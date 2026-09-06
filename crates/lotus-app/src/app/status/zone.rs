use std::time::Instant;

use lotus_core::settings::{DockSettings, DockZone};
use lotus_dock::scene::DockPresenter;
use lotus_settings::appearance::theme_for;
use lotus_ui::embedded_icon::EmbeddedIcon;
use lotus_ui::frame::{FramePass, ScheduledSurface};
use lotus_ui::geometry::NonZeroPhysicalSize;
use lotus_windows::WindowHandle;
use lotus_windows::graphics::{
    CompositionSurfaceState, DeviceState, GraphicsDevice, SurfaceSize,
};
use lotus_windows::responsiveness::{LayoutOperation, METRICS};
use lotus_windows::window::{
    DockWindow, PointerEvent, SignedPoint, StatusEvent, StatusWindow,
};

use crate::app::AppError;
use crate::app::dock::{
    dock_anchor, metrics, popup_overlap, status_items, status_popup_center,
};
use crate::app::runtime::resize_surface;
use crate::app::status::AuxiliaryZoneAction;
use crate::app::surface_render::frame_outcome;
use crate::app::visuals::{
    DockHitTarget, DockIcon, DockScene, MediaItem, SystemStatusItem, surface_size,
};

pub(super) struct StatusZone {
    window: StatusWindow,
    surface: Option<ScheduledSurface<CompositionSurfaceState>>,
    scene: DockScene,
    zone: Option<DockZone>,
    presenter: DockPresenter,
}

impl StatusZone {
    pub(super) fn new(
        window: StatusWindow,
        settings: &DockSettings,
    ) -> Result<Self, AppError> {
        Ok(Self {
            scene: build_scene(window.dpi(), DockZone::Center, settings, None, None)?,
            window,
            surface: None,
            zone: None,
            presenter: DockPresenter::default(),
        })
    }

    pub(super) fn diagnostic_state(&self) -> (bool, bool, bool) {
        let surface = self.surface.as_ref();
        (
            surface.is_some_and(ScheduledSurface::is_dirty),
            surface.is_some_and(ScheduledSurface::is_animating),
            self.zone.is_some(),
        )
    }

    pub(super) fn sync(
        &mut self,
        dock: &DockWindow,
        zone: Option<DockZone>,
        settings: &DockSettings,
        media: Option<&MediaItem>,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.zone = zone;
        let Some(zone) = zone else {
            self.window.set_visible(false);
            return Ok(());
        };

        self.scene = build_scene(
            self.window.dpi(),
            zone,
            settings,
            media.filter(|_| settings.media_zone == zone).cloned(),
            (settings.system_status_zone == zone)
                .then(|| status_items(settings, self.window.handle())),
        )?;
        let size = self.scene.desired_size();
        let physical = NonZeroPhysicalSize::new(size.width(), size.height())
            .ok_or(AppError::ZeroSizedSurface)?;

        dock.place_status_window(&self.window, physical, zone, settings)?;
        lotus_windows::backdrop::apply_dock_settings(self.window.handle(), settings);

        if let Some(surface) = &mut self.surface {
            resize_surface(graphics, surface.value_mut(), surface_size(size))?;
        } else {
            let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
            self.surface = Some(ScheduledSurface::new(CompositionSurfaceState::create(
                device,
                self.window.handle(),
                surface_size(size),
            )?));
        }
        self.window.set_visible(dock.is_visible());
        Ok(())
    }

    pub(super) fn set_visible(&self, dock_visible: bool) {
        self.window.set_visible(dock_visible && self.zone.is_some());
    }

    pub(super) fn set_fullscreen_occluded(
        &mut self,
        occluded: bool,
    ) -> Result<(), AppError> {
        if self.zone.is_some() {
            self.window.set_fullscreen_occluded(occluded)?;
            if occluded && let Some(surface) = &mut self.surface {
                surface.stop_animation();
            }
        }
        Ok(())
    }

    pub(super) fn refresh(&mut self, settings: &DockSettings) {
        let Some(active) = self.zone else {
            return;
        };

        let next = if settings.system_status_zone == active {
            status_items(settings, self.window.handle())
        } else {
            Vec::new()
        };
        if self.scene.status_items() != next {
            self.scene.replace_status_items(next);
            self.invalidate();
        }
    }

    pub(super) fn drain_events_up_to(
        &mut self,
        limit: usize,
    ) -> impl Iterator<Item = StatusEvent> + '_ {
        self.window.drain_events_up_to(limit)
    }

    pub(super) fn has_pending_events(&self) -> bool {
        self.window.has_pending_events()
    }

    pub(super) fn handle_event(
        &mut self,
        event: StatusEvent,
        graphics: &mut DeviceState,
    ) -> Result<Option<(AuxiliaryZoneAction, WindowHandle, Option<SignedPoint>)>, AppError>
    {
        let (action, scene_changed) = match event {
            StatusEvent::Pointer(pointer) => match pointer {
                PointerEvent::Moved { x, y } => {
                    let target = self.hit_test(x, y);
                    (None, self.scene.set_hovered(target))
                }
                PointerEvent::Left => (None, self.scene.set_hovered(None)),
                PointerEvent::LeftButtonPressed { x, y } => {
                    let target = self.hit_test(x, y);
                    (None, self.scene.set_pressed(target))
                }
                PointerEvent::LeftButtonReleased { x, y } => {
                    let target = self.hit_test(x, y);
                    let pressed = self.scene.interaction().pressed;
                    let changed = self.scene.set_pressed(None);
                    ((pressed == target).then_some(target).flatten(), changed)
                }
                PointerEvent::Cancelled => (None, self.scene.set_pressed(None)),
            },
            StatusEvent::Resized { width, height } => {
                if let (Some(surface), Some(size)) =
                    (&mut self.surface, SurfaceSize::new(width, height))
                {
                    resize_surface(graphics, surface.value_mut(), size)?;
                }
                (None, true)
            }
            StatusEvent::DpiChanged { dpi } => (None, self.scene.set_dpi(dpi)),
            StatusEvent::RenderRequested => (None, true),
        };
        let anchor = action.and_then(|target| self.target_anchor(target));
        if scene_changed {
            self.invalidate();
        }

        Ok(action
            .and_then(auxiliary_action)
            .map(|action| (action, self.window.handle(), anchor)))
    }

    pub(super) fn render_frame(
        &mut self,
        pass: &mut FramePass,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        if self.zone.is_none() {
            if let Some(surface) = &mut self.surface {
                surface.stop_animation();
            }
            return Ok(());
        }
        let Some(surface) = &mut self.surface else {
            return Ok(());
        };

        let animation_allowed = !self.window.is_fullscreen_occluded();
        let size = self.scene.desired_size();
        let (presentation, animating) =
            self.presenter
                .present(&self.scene, size.width(), size.height());
        pass.render(surface, |surface| {
            frame_outcome(graphics, surface.render_scene(&presentation, animating))
                .map(|frame| frame.with_animation_allowed(animation_allowed))
        })
    }

    pub(super) fn invalidate(&mut self) {
        if let Some(surface) = &mut self.surface {
            surface.invalidate();
        }
    }

    pub(super) fn recover_surface(
        &mut self,
        device: &GraphicsDevice,
    ) -> Result<(), AppError> {
        if let Some(surface) = &mut self.surface {
            surface.value_mut().recover(device)?;
        }
        Ok(())
    }

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
}

fn build_scene(
    dpi: u32,
    zone: DockZone,
    settings: &DockSettings,
    media: Option<MediaItem>,
    status: Option<Vec<SystemStatusItem>>,
) -> Result<DockScene, AppError> {
    let mut scene = DockScene::new(
        dpi,
        metrics(settings)?,
        DockIcon::Embedded(EmbeddedIcon::LotusPixel),
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
