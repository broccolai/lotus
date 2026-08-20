use lotus_ui::frame::{FrameOutcome, FramePass, ScheduledSurface};
use lotus_ui::geometry::NonZeroPhysicalSize;
use lotus_windows::WindowHandle;
use lotus_windows::graphics::scene::{DockAnchor, DockHitTarget, DockScene};
use lotus_windows::graphics::surface::FrameResult;
use lotus_windows::graphics::{CompositionSurfaceState, DeviceState, SurfaceSize};
use lotus_windows::window::{
    DockContextRequest, DockWindow, PointerEvent, PopupAlignment, SignedPoint,
    StatusWindow, WindowEvent,
};
use lotus_windows::window_tracker::WindowTracker;

use crate::app::AppError;
use crate::app::dock::{DockRuntime, popup_overlap, status_popup_center};
use crate::app::runtime::resize_surface;

#[derive(Clone, Copy)]
pub(super) enum MonitorDockAction {
    Activate {
        target: DockHitTarget,
        owner: WindowHandle,
        anchor: Option<SignedPoint>,
    },
    Context {
        target: DockHitTarget,
        anchor: SignedPoint,
        alignment: PopupAlignment,
    },
}

pub(super) struct MonitorDocks {
    docks: Vec<MonitorDock>,
    rendered_revision: u64,
    topology_dirty: bool,
}

struct MonitorDock {
    window: StatusWindow,
    surface: ScheduledSurface<CompositionSurfaceState>,
    scene: DockScene,
}

impl MonitorDocks {
    pub(super) const fn new() -> Self {
        Self {
            docks: Vec::new(),
            rendered_revision: u64::MAX,
            topology_dirty: true,
        }
    }

    pub(super) fn sync(
        &mut self,
        dock: &DockWindow,
        model: &mut DockRuntime,
        graphics: &mut DeviceState,
        tracker: &WindowTracker,
    ) -> Result<(), AppError> {
        if !model.settings().show_on_all_monitors {
            self.docks.clear();
            self.rendered_revision = model.revision();
            return Ok(());
        }

        if self.topology_dirty {
            self.recreate(dock, model, graphics)?;
        } else if self.rendered_revision != model.revision() {
            self.refresh_content(dock, model, graphics)?;
        }
        self.rendered_revision = model.revision();
        self.sync_visibility(model, tracker)?;
        Ok(())
    }

    pub(super) fn mark_topology_dirty(&mut self) {
        self.topology_dirty = true;
    }

    pub(super) fn render_frame(
        &mut self,
        pass: &mut FramePass,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        for replica in &mut self.docks {
            replica.render_frame(pass, graphics)?;
        }
        Ok(())
    }

    pub(super) fn invalidate(&mut self) {
        for replica in &mut self.docks {
            replica.surface.invalidate();
        }
    }

    pub(super) fn sync_visibility(
        &mut self,
        model: &DockRuntime,
        tracker: &WindowTracker,
    ) -> Result<(), AppError> {
        for replica in &mut self.docks {
            let fullscreen = tracker.fullscreen_on_same_monitor(replica.window.handle());
            let occluded = model.settings().hide_when_fullscreen && fullscreen;
            replica.window.set_fullscreen_occluded(occluded)?;
            if occluded {
                replica.surface.stop_animation();
            }
        }
        Ok(())
    }

    pub(super) fn drain_events(
        &mut self,
        graphics: &mut DeviceState,
    ) -> Result<Vec<MonitorDockAction>, AppError> {
        let mut actions = Vec::new();
        let mut refresh = false;
        for replica in &mut self.docks {
            let events = replica.window.drain_events().collect::<Vec<_>>();
            for event in events {
                match event {
                    WindowEvent::Pointer(pointer) => {
                        if let Some(action) = replica.handle_pointer(pointer) {
                            actions.push(action);
                        }
                    }
                    WindowEvent::ContextMenuRequested(request) => {
                        if let Some((target, anchor, alignment)) =
                            replica.popup_target_anchor(request)
                        {
                            actions.push(MonitorDockAction::Context {
                                target,
                                anchor,
                                alignment,
                            });
                        }
                    }
                    WindowEvent::Resized { width, height } => {
                        if let Some(size) = SurfaceSize::new(width, height) {
                            resize_surface(graphics, replica.surface.value_mut(), size)?;
                        }
                    }
                    WindowEvent::DpiChanged { .. }
                    | WindowEvent::PlacementRefreshRequested => refresh = true,
                    WindowEvent::RenderRequested => {
                        replica.surface.invalidate();
                    }
                    WindowEvent::AnimationFrame
                    | WindowEvent::StatusRefreshRequested
                    | WindowEvent::Search(_)
                    | WindowEvent::Settings(_)
                    | WindowEvent::ContextMenu(_)
                    | WindowEvent::Switcher(_) => {}
                }
            }
        }
        if refresh {
            self.topology_dirty = true;
        }
        Ok(actions)
    }

    fn recreate(
        &mut self,
        dock: &DockWindow,
        model: &mut DockRuntime,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.docks.clear();
        for window in dock.create_secondary_dock_windows()? {
            let scene = model.replica_scene(window.dpi())?;
            let size = scene.desired_size();
            let physical = NonZeroPhysicalSize::new(size.width(), size.height())
                .ok_or(AppError::ZeroSizedSurface)?;
            dock.place_secondary_dock_window(&window, physical, model.settings())?;
            lotus_windows::backdrop::apply_dock_settings(window.handle(), model.settings());
            let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
            let surface = CompositionSurfaceState::create(
                device,
                window.handle(),
                SurfaceSize::from(size),
            )?;
            let replica = MonitorDock {
                window,
                surface: ScheduledSurface::new(surface),
                scene,
            };
            self.docks.push(replica);
        }
        self.topology_dirty = false;
        Ok(())
    }

    fn refresh_content(
        &mut self,
        dock: &DockWindow,
        model: &mut DockRuntime,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        for replica in &mut self.docks {
            replica.scene = model.replica_scene(replica.window.dpi())?;
            let size = replica.scene.desired_size();
            let physical = NonZeroPhysicalSize::new(size.width(), size.height())
                .ok_or(AppError::ZeroSizedSurface)?;
            dock.place_secondary_dock_window(&replica.window, physical, model.settings())?;
            resize_surface(
                graphics,
                replica.surface.value_mut(),
                SurfaceSize::from(size),
            )?;
        }
        Ok(())
    }
}

impl MonitorDock {
    fn handle_pointer(&mut self, event: PointerEvent) -> Option<MonitorDockAction> {
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
                    target.map(|target| MonitorDockAction::Activate {
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
            let layout = self.scene.layout(size.width(), size.height());
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
        let DockContextRequest::Pointer { screen, client } = request else {
            return None;
        };
        let target = hit_test(&self.scene, client.x, client.y)?;
        let size = self.scene.desired_size();
        let layout = self.scene.layout(size.width(), size.height());
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

    fn render_frame(
        &mut self,
        pass: &mut FramePass,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let animation_allowed = !self.window.is_fullscreen_occluded();
        pass.render(&mut self.surface, |surface| {
            match surface.render_scene(&self.scene) {
                Ok(FrameResult::Presented { needs_animation }) => {
                    Ok(FrameOutcome::complete(needs_animation && animation_allowed))
                }
                Ok(FrameResult::TargetRecreated) => Ok(FrameOutcome::Retry),
                Err(lotus_windows::graphics::SurfaceError::DeviceLost(_)) => {
                    let _ = graphics.poll();
                    graphics.recover()?;
                    let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
                    surface.recover(device)?;
                    match surface.render_scene(&self.scene)? {
                        FrameResult::Presented { needs_animation } => {
                            Ok(FrameOutcome::complete(needs_animation && animation_allowed))
                        }
                        FrameResult::TargetRecreated => Ok(FrameOutcome::Retry),
                    }
                }
                Err(error) => Err(error.into()),
            }
        })
    }
}

fn hit_test(scene: &DockScene, x: i32, y: i32) -> Option<DockHitTarget> {
    let x = u32::try_from(x).ok()?;
    let y = u32::try_from(y).ok()?;
    let size = scene.desired_size();
    scene.layout(size.width(), size.height()).hit_test(x, y)
}
