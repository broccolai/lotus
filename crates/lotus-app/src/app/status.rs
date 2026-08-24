use lotus_core::settings::{DockSettings, DockZone};
use lotus_dock::scene::DockPresenter;
use lotus_media::MediaHitTarget;
use lotus_settings::appearance::theme_for;
use lotus_ui::frame::{FrameOutcome, FramePass, ScheduledSurface};
use lotus_ui::geometry::NonZeroPhysicalSize;
use lotus_windows::graphics::assets::SvgAsset;
use lotus_windows::graphics::surface::FrameResult;
use lotus_windows::graphics::{CompositionSurfaceState, DeviceState, SurfaceSize};
use lotus_windows::responsiveness::{LayoutOperation, METRICS};
use lotus_windows::window::{
    DockWindow, PointerEvent, SignedPoint, StatusWindow, WindowEvent,
};

use crate::app::AppError;
use crate::app::dock::{
    dock_anchor, metrics, popup_overlap, status_items, status_popup_center,
};
use crate::app::runtime::resize_surface;
use crate::app::visuals::{
    DockHitTarget, DockIcon, DockScene, MediaItem, SystemStatusItem, SystemStatusKind,
    surface_size,
};

pub(super) enum AuxiliaryZoneAction {
    Media(MediaHitTarget),
    Status(SystemStatusKind),
}

pub(super) struct StatusRuntime {
    zones: Vec<ZoneSurface>,
}

struct ZoneSurface {
    window: StatusWindow,
    surface: Option<ScheduledSurface<CompositionSurfaceState>>,
    scene: DockScene,
    zone: Option<DockZone>,
    presenter: DockPresenter,
}

impl StatusRuntime {
    pub(super) fn diagnostic_surface_masks(&self) -> (bool, bool, bool) {
        self.zones
            .iter()
            .fold((false, false, false), |state, zone| {
                let surface = zone.surface.as_ref();
                (
                    state.0 || surface.is_some_and(ScheduledSurface::is_dirty),
                    state.1 || surface.is_some_and(ScheduledSurface::is_animating),
                    state.2 || zone.zone.is_some(),
                )
            })
    }

    pub(super) fn new(
        windows: [StatusWindow; 2],
        settings: &DockSettings,
    ) -> Result<Self, AppError> {
        let zones = windows
            .into_iter()
            .map(|window| {
                Ok(ZoneSurface {
                    scene: empty_zone_scene(window.dpi(), settings)?,
                    window,
                    surface: None,
                    zone: None,
                    presenter: DockPresenter::default(),
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        Ok(Self { zones })
    }

    pub(super) fn sync(
        &mut self,
        dock: &DockWindow,
        settings: &DockSettings,
        media: Option<&MediaItem>,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let occupied = occupied_external_zones(settings, media);
        for (index, zone_surface) in self.zones.iter_mut().enumerate() {
            let zone = occupied.get(index).copied();
            let Some(zone) = zone else {
                zone_surface.zone = None;
                zone_surface.window.set_visible(false);
                continue;
            };

            zone_surface.zone = Some(zone);
            zone_surface.scene = build_zone_scene(
                zone_surface.window.dpi(),
                zone,
                settings,
                media.filter(|_| settings.media_zone == zone).cloned(),
                (settings.system_status_zone == zone).then(|| status_items(settings)),
            )?;
            let size = zone_surface.scene.desired_size();
            let physical = NonZeroPhysicalSize::new(size.width(), size.height())
                .ok_or(AppError::ZeroSizedSurface)?;
            dock.place_status_window(&zone_surface.window, physical, zone, settings)?;
            lotus_windows::backdrop::apply_dock_settings(
                zone_surface.window.handle(),
                settings,
            );

            if let Some(surface) = &mut zone_surface.surface {
                resize_surface(graphics, surface.value_mut(), surface_size(size))?;
            } else {
                let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
                zone_surface.surface =
                    Some(ScheduledSurface::new(CompositionSurfaceState::create(
                        device,
                        zone_surface.window.handle(),
                        surface_size(size),
                    )?));
            }
            zone_surface.window.set_visible(dock.is_visible());
        }
        Ok(())
    }

    pub(super) fn set_visible(&self, dock_visible: bool) {
        for zone in &self.zones {
            zone.window.set_visible(dock_visible && zone.zone.is_some());
        }
    }

    pub(super) fn set_fullscreen_occluded(
        &mut self,
        occluded: bool,
    ) -> Result<(), AppError> {
        for zone in &mut self.zones {
            if zone.zone.is_some() {
                zone.window.set_fullscreen_occluded(occluded)?;
                if occluded && let Some(surface) = &mut zone.surface {
                    surface.stop_animation();
                }
            }
        }
        Ok(())
    }

    pub(super) fn refresh(&mut self, settings: &DockSettings) {
        for zone in &mut self.zones {
            let Some(active) = zone.zone else {
                continue;
            };
            let next = if settings.system_status_zone == active {
                status_items(settings)
            } else {
                Vec::new()
            };
            if zone.scene.status_items() != next {
                zone.scene.replace_status_items(next);
                zone.invalidate();
            }
        }
    }

    pub(super) fn drain_events(&mut self) -> Vec<(usize, WindowEvent)> {
        self.zones
            .iter_mut()
            .enumerate()
            .flat_map(|(index, zone)| {
                zone.window.drain_events().map(move |event| (index, event))
            })
            .collect()
    }

    pub(super) fn has_pending_events(&self) -> bool {
        self.zones
            .iter()
            .any(|zone| zone.window.has_pending_events())
    }

    pub(super) fn handle_event(
        &mut self,
        zone_index: usize,
        event: WindowEvent,
        graphics: &mut DeviceState,
    ) -> Result<
        Option<(
            AuxiliaryZoneAction,
            lotus_windows::WindowHandle,
            Option<SignedPoint>,
        )>,
        AppError,
    > {
        let Some(zone) = self.zones.get_mut(zone_index) else {
            return Ok(None);
        };
        let (action, scene_changed) = match event {
            WindowEvent::Pointer(pointer) => match pointer {
                PointerEvent::Moved { x, y } => {
                    let target = zone.hit_test(x, y);
                    (None, zone.scene.set_hovered(target))
                }
                PointerEvent::Left => (None, zone.scene.set_hovered(None)),
                PointerEvent::LeftButtonPressed { x, y } => {
                    let target = zone.hit_test(x, y);
                    (None, zone.scene.set_pressed(target))
                }
                PointerEvent::LeftButtonReleased { x, y } => {
                    let target = zone.hit_test(x, y);
                    let pressed = zone.scene.interaction().pressed;
                    let changed = zone.scene.set_pressed(None);
                    ((pressed == target).then_some(target).flatten(), changed)
                }
                PointerEvent::Cancelled => (None, zone.scene.set_pressed(None)),
            },
            WindowEvent::Resized { width, height } => {
                if let (Some(surface), Some(size)) =
                    (&mut zone.surface, SurfaceSize::new(width, height))
                {
                    resize_surface(graphics, surface.value_mut(), size)?;
                }
                (None, true)
            }
            WindowEvent::DpiChanged { dpi } => (None, zone.scene.set_dpi(dpi)),
            WindowEvent::RenderRequested => (None, true),
            WindowEvent::AnimationFrame
            | WindowEvent::MascotAnimationDeadline
            | WindowEvent::PlacementRefreshRequested
            | WindowEvent::ContextMenuRequested(_)
            | WindowEvent::Search(_)
            | WindowEvent::Settings(_)
            | WindowEvent::ContextMenu(_)
            | WindowEvent::Switcher(_)
            | WindowEvent::StatusRefreshRequested => (None, false),
        };
        let anchor = action.and_then(|target| zone.target_anchor(target));
        if scene_changed {
            zone.invalidate();
        }
        Ok(action
            .and_then(auxiliary_action)
            .map(|action| (action, zone.window.handle(), anchor)))
    }
    pub(super) fn render_frame(
        &mut self,
        pass: &mut FramePass,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        for zone in &mut self.zones {
            if zone.zone.is_none() {
                if let Some(surface) = &mut zone.surface {
                    surface.stop_animation();
                }
                continue;
            }
            let Some(surface) = &mut zone.surface else {
                continue;
            };
            let animation_allowed = !zone.window.is_fullscreen_occluded();
            let size = zone.scene.desired_size();
            let (presentation, animating) =
                zone.presenter
                    .present(&zone.scene, size.width(), size.height());
            let render = |surface: &mut CompositionSurfaceState| {
                surface.render_scene(&presentation, animating)
            };
            pass.render(surface, |surface| match render(surface) {
                Ok(FrameResult::Presented { needs_animation }) => Ok::<_, AppError>(
                    FrameOutcome::complete(needs_animation && animation_allowed),
                ),
                Ok(FrameResult::TargetRecreated) => Ok(FrameOutcome::Retry),
                Err(lotus_windows::graphics::SurfaceError::DeviceLost(_)) => {
                    let _ = graphics.poll();
                    graphics.recover()?;
                    let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
                    surface.recover(device)?;
                    match render(surface)? {
                        FrameResult::Presented { needs_animation } => {
                            Ok(FrameOutcome::complete(needs_animation && animation_allowed))
                        }
                        FrameResult::TargetRecreated => Ok(FrameOutcome::Retry),
                    }
                }
                Err(error) => Err(error.into()),
            })?;
        }
        Ok(())
    }

    pub(super) fn invalidate(&mut self) {
        for zone in &mut self.zones {
            zone.invalidate();
        }
    }
}

impl ZoneSurface {
    fn hit_test(&self, x: i32, y: i32) -> Option<DockHitTarget> {
        let x = u32::try_from(x).ok()?;
        let y = u32::try_from(y).ok()?;
        let size = self.scene.desired_size();
        let started = Instant::now();
        let target = self
            .scene
            .layout(size.width(), size.height())
            .hit_test(x, y);
        METRICS.record_layout(LayoutOperation::StatusHitTest, started.elapsed());
        target
    }

    fn target_anchor(&self, target: DockHitTarget) -> Option<SignedPoint> {
        let DockHitTarget::SystemStatus(kind) = target else {
            return None;
        };
        let size = self.scene.desired_size();
        let started = Instant::now();
        let layout = self.scene.layout(size.width(), size.height());
        METRICS.record_layout(LayoutOperation::StatusPopup, started.elapsed());
        let bounds = layout
            .status_items
            .iter()
            .find(|item| item.kind == kind)?
            .hit_bounds;
        let x = i32::try_from(status_popup_center(&layout.status_items)?).ok()?;
        let y = i32::try_from(bounds.top)
            .ok()?
            .saturating_add(popup_overlap(self.scene.dpi()));
        self.window.client_to_screen(SignedPoint { x, y }).ok()
    }

    fn invalidate(&mut self) {
        if let Some(surface) = &mut self.surface {
            surface.invalidate();
        }
    }
}

fn occupied_external_zones(
    settings: &DockSettings,
    media: Option<&MediaItem>,
) -> Vec<DockZone> {
    [DockZone::Left, DockZone::Center, DockZone::Right]
        .into_iter()
        .filter(|zone| *zone != settings.dock_zone)
        .filter(|zone| {
            (media.is_some()
                && settings.show_media_controls
                && *zone == settings.media_zone)
                || (settings.show_system_status
                    && *zone == settings.system_status_zone
                    && !status_items(settings).is_empty())
        })
        .collect()
}

fn empty_zone_scene(dpi: u32, settings: &DockSettings) -> Result<DockScene, AppError> {
    build_zone_scene(dpi, DockZone::Center, settings, None, None)
}

fn build_zone_scene(
    dpi: u32,
    zone: DockZone,
    settings: &DockSettings,
    media: Option<MediaItem>,
    status: Option<Vec<SystemStatusItem>>,
) -> Result<DockScene, AppError> {
    let mut scene = DockScene::new(
        dpi,
        metrics(settings)?,
        DockIcon::Embedded(SvgAsset::LotusPixel),
        Vec::new(),
    )
    .ok_or(AppError::InvalidScene)?;
    scene.set_anchor(dock_anchor(zone));
    scene.set_launcher_button_visible(false);
    scene.replace_media(media);
    scene.replace_status_items(status.unwrap_or_default());
    let _ = scene.set_theme(theme_for(settings));
    Ok(scene)
}

fn auxiliary_action(target: DockHitTarget) -> Option<AuxiliaryZoneAction> {
    match target {
        DockHitTarget::Media(target) => Some(AuxiliaryZoneAction::Media(target)),
        DockHitTarget::SystemStatus(kind) => Some(AuxiliaryZoneAction::Status(kind)),
        DockHitTarget::Item(_) | DockHitTarget::Jirachi | DockHitTarget::ShowDesktop => {
            None
        }
    }
}
use std::time::Instant;
