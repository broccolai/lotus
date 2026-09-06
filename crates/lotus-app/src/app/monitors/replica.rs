use std::time::Instant;

use lotus_core::settings::DockSettings;
use lotus_dock::scene::DockPresenter;
use lotus_ui::frame::{FramePass, ScheduledSurface};
use lotus_ui::geometry::NonZeroPhysicalSize;
use lotus_windows::graphics::{
    CompositionSurfaceState, DeviceState, GraphicsDevice, SurfaceSize,
};
use lotus_windows::responsiveness::{LayoutOperation, METRICS};
use lotus_windows::window::{
    DockContextRequest, DockEvent, DockReplicaWindow, DockWindow, PointerEvent,
    PopupAlignment, SignedPoint,
};

use crate::app::AppError;
use crate::app::dock::{popup_overlap, status_popup_center};
use crate::app::monitors::DockAction;
use crate::app::runtime::resize_surface;
use crate::app::surface_render::frame_outcome;
use crate::app::visuals::{DockAnchor, DockHitTarget, DockScene, surface_size};

pub(super) struct MonitorDock {
    window: DockReplicaWindow,
    surface: ScheduledSurface<CompositionSurfaceState>,
    scene: DockScene,
    presenter: DockPresenter,
}

pub(super) struct ReplicaEventDrain {
    pub(super) actions: Vec<DockAction>,
    pub(super) had_events: bool,
    pub(super) drained_events: usize,
    pub(super) topology_refresh_requested: bool,
}

impl MonitorDock {
    pub(super) fn create(
        dock: &DockWindow,
        window: DockReplicaWindow,
        scene: DockScene,
        settings: &DockSettings,
        graphics: &mut DeviceState,
    ) -> Result<Self, AppError> {
        let size = scene.desired_size();
        let physical = NonZeroPhysicalSize::new(size.width(), size.height())
            .ok_or(AppError::ZeroSizedSurface)?;
        dock.place_secondary_dock_window(&window, physical, settings)?;
        lotus_windows::backdrop::apply_dock_settings(window.handle(), settings);
        let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
        let surface =
            CompositionSurfaceState::create(device, window.handle(), surface_size(size))?;
        Ok(Self {
            window,
            surface: ScheduledSurface::new(surface),
            scene,
            presenter: DockPresenter::default(),
        })
    }

    pub(super) fn handle(&self) -> lotus_windows::WindowHandle {
        self.window.handle()
    }

    pub(super) fn dpi(&self) -> u32 {
        self.window.dpi()
    }

    pub(super) fn has_pending_events(&self) -> bool {
        self.window.has_pending_events()
    }

    pub(super) fn is_visible(&self) -> bool {
        self.window.is_visible() && !self.window.is_fullscreen_occluded()
    }

    pub(super) fn diagnostic_surface_state(&self) -> (bool, bool) {
        (self.surface.is_dirty(), self.surface.is_animating())
    }

    pub(super) fn set_fullscreen_occluded(
        &mut self,
        occluded: bool,
    ) -> Result<(), AppError> {
        self.window.set_fullscreen_occluded(occluded)?;
        if occluded {
            self.surface.stop_animation();
        }
        Ok(())
    }

    pub(super) fn refresh(
        &mut self,
        dock: &DockWindow,
        scene: DockScene,
        settings: &DockSettings,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.scene = scene;
        let size = self.scene.desired_size();
        let physical = NonZeroPhysicalSize::new(size.width(), size.height())
            .ok_or(AppError::ZeroSizedSurface)?;
        dock.place_secondary_dock_window(&self.window, physical, settings)?;
        lotus_windows::backdrop::apply_dock_settings(self.window.handle(), settings);
        resize_surface(graphics, self.surface.value_mut(), surface_size(size))?;
        Ok(())
    }

    pub(super) fn invalidate(&mut self) {
        self.surface.invalidate();
    }

    pub(super) fn recover_surface(
        &mut self,
        device: &GraphicsDevice,
    ) -> Result<(), AppError> {
        self.surface.value_mut().recover(device)?;
        Ok(())
    }

    pub(super) fn drain_events_up_to(
        &mut self,
        graphics: &mut DeviceState,
        limit: usize,
    ) -> Result<ReplicaEventDrain, AppError> {
        let events = self.window.drain_events_up_to(limit).collect::<Vec<_>>();
        let mut actions = Vec::new();
        let mut topology_refresh_requested = false;
        for event in &events {
            match event {
                DockEvent::Pointer(pointer) => {
                    if let Some(action) = self.handle_pointer(*pointer) {
                        actions.push(action);
                    }
                }
                DockEvent::ContextMenuRequested(request) => {
                    if let Some((target, anchor, alignment)) =
                        self.popup_target_anchor(*request)
                    {
                        actions.push(DockAction::Context {
                            target,
                            anchor,
                            alignment,
                            shift_held: request.shift_held(),
                        });
                    }
                }
                DockEvent::Resized { width, height } => {
                    if let Some(size) = SurfaceSize::new(*width, *height) {
                        resize_surface(graphics, self.surface.value_mut(), size)?;
                    }
                }
                DockEvent::DpiChanged { .. } | DockEvent::PlacementRefreshRequested => {
                    topology_refresh_requested = true;
                }
                DockEvent::RenderRequested => self.surface.invalidate(),
                DockEvent::AnimationFrame
                | DockEvent::MascotAnimationDeadline
                | DockEvent::StatusRefreshRequested => {}
            }
        }
        Ok(ReplicaEventDrain {
            had_events: !events.is_empty(),
            drained_events: events.len(),
            actions,
            topology_refresh_requested,
        })
    }

    pub(super) fn render_frame(
        &mut self,
        pass: &mut FramePass,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let animation_allowed = !self.window.is_fullscreen_occluded();
        let size = self.scene.desired_size();
        let (presentation, animating) =
            self.presenter
                .present(&self.scene, size.width(), size.height());
        pass.render(&mut self.surface, |surface| {
            frame_outcome(graphics, surface.render_scene(&presentation, animating))
                .map(|frame| frame.with_animation_allowed(animation_allowed))
        })
    }

    fn handle_pointer(&mut self, event: PointerEvent) -> Option<DockAction> {
        let (action, scene_changed) = match event {
            PointerEvent::Moved { x, y } => {
                let target = hit_test(&self.scene, x, y);
                (None, self.scene.set_hovered(target))
            }
            PointerEvent::Left => (None, self.scene.set_hovered(None)),
            PointerEvent::LeftButtonPressed { x, y } => {
                let target = hit_test(&self.scene, x, y);
                (None, self.scene.set_pressed(target))
            }
            PointerEvent::LeftButtonReleased { x, y } => {
                let target = hit_test(&self.scene, x, y);
                let pressed = self.scene.interaction().pressed;
                let changed = self.scene.set_pressed(None);
                let action = if pressed == target {
                    target.map(|target| DockAction::Activate {
                        target,
                        owner: self.window.handle(),
                        anchor: self.activation_anchor(target, x, y),
                    })
                } else {
                    None
                };
                (action, changed)
            }
            PointerEvent::Cancelled => (None, self.scene.set_pressed(None)),
        };
        if scene_changed {
            self.surface.invalidate();
        }
        action
    }

    fn activation_anchor(
        &self,
        target: DockHitTarget,
        pointer_x: i32,
        pointer_y: i32,
    ) -> Option<SignedPoint> {
        let (x, y) = if let DockHitTarget::SystemStatus(kind) = target {
            let size = self.scene.desired_size();
            let started = Instant::now();
            let layout = self.scene.layout(size.width(), size.height());
            METRICS.record_layout(LayoutOperation::MonitorPopup, started.elapsed());
            let bounds = layout
                .status_items
                .iter()
                .find(|item| item.kind == kind)?
                .hit_bounds;
            (
                i32::try_from(status_popup_center(&layout.status_items)?).ok()?,
                i32::try_from(bounds.top)
                    .ok()?
                    .saturating_add(popup_overlap(self.scene.dpi())),
            )
        } else {
            (pointer_x, pointer_y)
        };
        self.window.client_to_screen(SignedPoint { x, y }).ok()
    }

    fn popup_target_anchor(
        &self,
        request: DockContextRequest,
    ) -> Option<(DockHitTarget, SignedPoint, PopupAlignment)> {
        let DockContextRequest::Pointer { screen, client, .. } = request else {
            return None;
        };
        let target = hit_test(&self.scene, client.x, client.y)?;
        let size = self.scene.desired_size();
        let started = Instant::now();
        let layout = self.scene.layout(size.width(), size.height());
        METRICS.record_layout(LayoutOperation::MonitorPopup, started.elapsed());
        let bounds = match target {
            DockHitTarget::Item(source_index) => layout
                .items
                .iter()
                .find(|item| item.source_index == source_index)
                .map(|item| item.bounds)?,
            DockHitTarget::Jirachi => layout.jirachi,
            DockHitTarget::Media(_)
            | DockHitTarget::SystemStatus(_)
            | DockHitTarget::ShowDesktop => return None,
        };
        let (anchor_x, alignment) = match (target, self.scene.anchor()) {
            (DockHitTarget::Jirachi, DockAnchor::Left) => (0, PopupAlignment::Start),
            (DockHitTarget::Jirachi, DockAnchor::Right) => {
                (size.width(), PopupAlignment::End)
            }
            _ => (
                bounds.left.saturating_add(bounds.width / 2),
                PopupAlignment::Center,
            ),
        };
        let anchor_x = i32::try_from(anchor_x).ok()?;
        let overlap = i32::try_from((u64::from(self.scene.dpi()) * 6 + 48) / 96).ok()?;
        let top = i32::try_from(bounds.top).ok()?;
        Some((
            target,
            SignedPoint {
                x: screen.x.saturating_sub(client.x).saturating_add(anchor_x),
                y: screen
                    .y
                    .saturating_sub(client.y)
                    .saturating_add(top)
                    .saturating_add(overlap),
            },
            alignment,
        ))
    }
}

fn hit_test(scene: &DockScene, x: i32, y: i32) -> Option<DockHitTarget> {
    let x = u32::try_from(x).ok()?;
    let y = u32::try_from(y).ok()?;
    let size = scene.desired_size();
    let started = Instant::now();
    let target = scene.layout(size.width(), size.height()).hit_test(x, y);
    METRICS.record_layout(LayoutOperation::MonitorHitTest, started.elapsed());
    target
}
