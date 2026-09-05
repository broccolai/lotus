use std::error::Error;

use lotus_core::settings::DockSettings;
use lotus_dock::scene::DockPresenter;
use lotus_ui::frame::{FrameOutcome, FramePass, ScheduledSurface};
use lotus_ui::geometry::NonZeroPhysicalSize;
use lotus_windows::WindowHandle;
use lotus_windows::graphics::surface::FrameResult;
use lotus_windows::graphics::{
    CompositionSurfaceState, DeviceState, GraphicsDevice, SurfaceSize,
};
use lotus_windows::responsiveness::{LayoutOperation, METRICS};
use lotus_windows::window::{
    DockContextRequest, DockEvent, DockReplicaWindow, DockWindow, PointerEvent,
    PopupAlignment, SignedPoint,
};
use lotus_windows::window_tracker::WindowTracker;

use crate::app::AppError;
use crate::app::dock::{popup_overlap, status_popup_center};
use crate::app::runtime::resize_surface;
use crate::app::visuals::{DockAnchor, DockHitTarget, DockScene, surface_size};

#[derive(Clone, Copy)]
pub(super) enum DockAction {
    Activate {
        target: DockHitTarget,
        owner: WindowHandle,
        anchor: Option<SignedPoint>,
    },
    Context {
        target: DockHitTarget,
        anchor: SignedPoint,
        alignment: PopupAlignment,
        shift_held: bool,
    },
}

pub(super) struct MonitorDockEventDrain {
    pub(super) actions: Vec<DockAction>,
    pub(super) had_events: bool,
}

pub(super) struct MonitorPresentationInput {
    pub(super) settings: DockSettings,
    pub(super) revision: u64,
    pub(super) replicas: Vec<MonitorReplicaInput>,
}

pub(super) struct MonitorReplicaInput {
    pub(super) owner: WindowHandle,
    pub(super) scene: DockScene,
}

#[derive(Clone, Copy)]
pub(super) struct MonitorReplicaTarget {
    pub(super) dpi: u32,
    pub(super) owner: WindowHandle,
}

pub(super) enum MonitorPresentationRequest {
    Disabled,
    Recreate(Vec<DockReplicaWindow>),
    Refresh(Vec<MonitorReplicaTarget>),
    Current,
}

impl MonitorPresentationRequest {
    pub(super) fn take_targets(&mut self) -> Vec<MonitorReplicaTarget> {
        match self {
            Self::Disabled | Self::Current => Vec::new(),
            Self::Recreate(windows) => windows
                .iter()
                .map(|window| MonitorReplicaTarget {
                    dpi: window.dpi(),
                    owner: window.handle(),
                })
                .collect(),
            Self::Refresh(targets) => std::mem::take(targets),
        }
    }
}

pub(super) struct MonitorDocks {
    fullscreen_occlusion_allowed: bool,
    docks: Vec<MonitorDock>,
    rendered_revision: u64,
    topology_dirty: bool,
    topology_generation: u64,
    health: MonitorIntegrationHealth,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MonitorIntegrationHealth {
    Disabled,
    Healthy,
    Degraded,
}

struct MonitorDock {
    window: DockReplicaWindow,
    surface: ScheduledSurface<CompositionSurfaceState>,
    scene: DockScene,
    presenter: DockPresenter,
}

impl MonitorDocks {
    pub(super) fn owns_window(&self, window: WindowHandle) -> bool {
        self.docks.iter().any(|dock| dock.window.handle() == window)
    }

    pub(super) fn has_pending_events(&self) -> bool {
        self.docks
            .iter()
            .any(|dock| dock.window.has_pending_events())
    }

    pub(super) const fn new(fullscreen_occlusion_allowed: bool) -> Self {
        Self {
            fullscreen_occlusion_allowed,
            docks: Vec::new(),
            rendered_revision: u64::MAX,
            topology_dirty: true,
            topology_generation: 0,
            health: MonitorIntegrationHealth::Disabled,
        }
    }

    pub(super) fn begin_sync(
        &mut self,
        dock: &DockWindow,
        settings: &DockSettings,
        revision: u64,
    ) -> Result<MonitorPresentationRequest, AppError> {
        if !settings.show_on_all_monitors {
            self.docks.clear();
            self.rendered_revision = revision;
            self.topology_dirty = true;
            self.health = MonitorIntegrationHealth::Disabled;
            return Ok(MonitorPresentationRequest::Disabled);
        }

        if self.topology_dirty {
            return match dock.create_secondary_dock_windows() {
                Ok(windows) => Ok(MonitorPresentationRequest::Recreate(windows)),
                Err(error) => {
                    self.record_recovery_failure(&error);
                    Err(error.into())
                }
            };
        }

        if self.rendered_revision != revision {
            return Ok(MonitorPresentationRequest::Refresh(self.replica_targets()));
        }

        Ok(MonitorPresentationRequest::Current)
    }

    pub(super) fn finish_sync(
        &mut self,
        dock: &DockWindow,
        request: MonitorPresentationRequest,
        input: MonitorPresentationInput,
        graphics: &mut DeviceState,
        tracker: &WindowTracker,
    ) -> Result<(), AppError> {
        let MonitorPresentationInput {
            settings,
            revision,
            replicas,
        } = input;
        let recreating = matches!(request, MonitorPresentationRequest::Recreate(_));
        let result = match request {
            MonitorPresentationRequest::Disabled | MonitorPresentationRequest::Current => {
                Ok(())
            }
            MonitorPresentationRequest::Recreate(windows) => {
                self.recreate(dock, windows, replicas, &settings, graphics)
            }
            MonitorPresentationRequest::Refresh(_) => {
                self.refresh_content(dock, replicas, &settings, graphics)
            }
        };
        if let Err(error) = result {
            if recreating {
                self.record_recovery_failure(&error);
            }
            return Err(error);
        }

        self.rendered_revision = revision;
        self.sync_visibility(&settings, tracker)?;
        Ok(())
    }

    pub(super) fn abort_sync(
        &mut self,
        request: &MonitorPresentationRequest,
        error: &AppError,
    ) {
        if matches!(request, MonitorPresentationRequest::Recreate(_)) {
            self.record_recovery_failure(error);
        }
    }

    pub(super) fn mark_topology_dirty(&mut self) {
        self.topology_dirty = true;
        self.topology_generation = self.topology_generation.wrapping_add(1);
    }

    pub(super) const fn topology_generation(&self) -> u64 {
        self.topology_generation
    }

    pub(super) const fn health(&self) -> MonitorIntegrationHealth {
        self.health
    }

    pub(super) fn replica_count(&self) -> usize {
        self.docks.len()
    }

    pub(super) fn has_visible_dock(&self) -> bool {
        self.docks
            .iter()
            .any(|dock| dock.window.is_visible() && !dock.window.is_fullscreen_occluded())
    }

    pub(super) fn diagnostic_surface_masks(&self) -> (bool, bool, bool) {
        self.docks
            .iter()
            .fold((false, false, false), |state, dock| {
                (
                    state.0 || dock.surface.is_dirty(),
                    state.1 || dock.surface.is_animating(),
                    state.2 || !self.docks.is_empty(),
                )
            })
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

    pub(super) fn recover_surfaces(
        &mut self,
        device: &GraphicsDevice,
    ) -> Result<(), AppError> {
        for replica in &mut self.docks {
            replica.surface.value_mut().recover(device)?;
        }
        Ok(())
    }

    pub(super) fn sync_visibility(
        &mut self,
        settings: &DockSettings,
        tracker: &WindowTracker,
    ) -> Result<(), AppError> {
        for replica in &mut self.docks {
            let fullscreen = self.fullscreen_occlusion_allowed
                && tracker.fullscreen_on_same_monitor(replica.window.handle());
            let occluded = settings.hide_when_fullscreen && fullscreen;
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
    ) -> Result<MonitorDockEventDrain, AppError> {
        let mut actions = Vec::new();
        let mut refresh = false;
        let mut had_events = false;
        for replica in &mut self.docks {
            let events = replica.window.drain_events().collect::<Vec<_>>();
            had_events |= !events.is_empty();
            for event in events {
                match event {
                    DockEvent::Pointer(pointer) => {
                        if let Some(action) = replica.handle_pointer(pointer) {
                            actions.push(action);
                        }
                    }
                    DockEvent::ContextMenuRequested(request) => {
                        if let Some((target, anchor, alignment)) =
                            replica.popup_target_anchor(request)
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
                        if let Some(size) = SurfaceSize::new(width, height) {
                            resize_surface(graphics, replica.surface.value_mut(), size)?;
                        }
                    }
                    DockEvent::DpiChanged { .. } | DockEvent::PlacementRefreshRequested => {
                        refresh = true;
                    }
                    DockEvent::RenderRequested => {
                        replica.surface.invalidate();
                    }
                    DockEvent::AnimationFrame
                    | DockEvent::MascotAnimationDeadline
                    | DockEvent::StatusRefreshRequested => {}
                }
            }
        }
        if refresh {
            self.mark_topology_dirty();
        }
        Ok(MonitorDockEventDrain {
            actions,
            had_events,
        })
    }

    fn recreate(
        &mut self,
        dock: &DockWindow,
        windows: Vec<DockReplicaWindow>,
        inputs: Vec<MonitorReplicaInput>,
        settings: &DockSettings,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        if !replica_inputs_match(windows.iter().map(DockReplicaWindow::handle), &inputs) {
            return Err(AppError::InvalidScene);
        }

        lotus_windows::diagnostics::record_diagnostic(
            "monitors.recovery_requested",
            &format!(
                "previous_replicas={} topology={}",
                self.docks.len(),
                self.topology_generation
            ),
        );
        let mut docks = Vec::new();
        for (window, replica_input) in windows.into_iter().zip(inputs) {
            let scene = replica_input.scene;
            let size = scene.desired_size();
            let physical = NonZeroPhysicalSize::new(size.width(), size.height())
                .ok_or(AppError::ZeroSizedSurface)?;
            dock.place_secondary_dock_window(&window, physical, settings)?;
            lotus_windows::backdrop::apply_dock_settings(window.handle(), settings);
            let device = graphics.ready().ok_or(AppError::GraphicsUnavailable)?;
            let surface = CompositionSurfaceState::create(
                device,
                window.handle(),
                surface_size(size),
            )?;
            let replica = MonitorDock {
                window,
                surface: ScheduledSurface::new(surface),
                scene,
                presenter: DockPresenter::default(),
            };
            docks.push(replica);
        }
        self.docks = docks;
        self.topology_dirty = false;
        self.health = MonitorIntegrationHealth::Healthy;
        lotus_windows::diagnostics::record_diagnostic(
            "monitors.recovered",
            &format!(
                "replicas={} topology={}",
                self.docks.len(),
                self.topology_generation
            ),
        );
        Ok(())
    }

    fn refresh_content(
        &mut self,
        dock: &DockWindow,
        inputs: Vec<MonitorReplicaInput>,
        settings: &DockSettings,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        if !replica_inputs_match(
            self.docks.iter().map(|replica| replica.window.handle()),
            &inputs,
        ) {
            return Err(AppError::InvalidScene);
        }

        for (replica, replica_input) in self.docks.iter_mut().zip(inputs) {
            replica.scene = replica_input.scene;
            let size = replica.scene.desired_size();
            let physical = NonZeroPhysicalSize::new(size.width(), size.height())
                .ok_or(AppError::ZeroSizedSurface)?;
            dock.place_secondary_dock_window(&replica.window, physical, settings)?;
            lotus_windows::backdrop::apply_dock_settings(replica.window.handle(), settings);
            resize_surface(graphics, replica.surface.value_mut(), surface_size(size))?;
        }
        Ok(())
    }

    fn replica_targets(&self) -> Vec<MonitorReplicaTarget> {
        self.docks
            .iter()
            .map(|replica| MonitorReplicaTarget {
                dpi: replica.window.dpi(),
                owner: replica.window.handle(),
            })
            .collect()
    }

    fn record_recovery_failure<E: Error + 'static>(&mut self, error: &E) {
        self.health = MonitorIntegrationHealth::Degraded;
        lotus_windows::diagnostics::record_error("monitors.recovery_failed", error);
    }
}

fn replica_inputs_match(
    expected: impl ExactSizeIterator<Item = WindowHandle>,
    inputs: &[MonitorReplicaInput],
) -> bool {
    expected.len() == inputs.len()
        && expected
            .zip(inputs)
            .all(|(owner, input)| owner == input.owner)
}

impl MonitorDock {
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

    fn render_frame(
        &mut self,
        pass: &mut FramePass,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        let animation_allowed = !self.window.is_fullscreen_occluded();
        let size = self.scene.desired_size();
        let (presentation, animating) =
            self.presenter
                .present(&self.scene, size.width(), size.height());
        let render = |surface: &mut CompositionSurfaceState| {
            surface.render_scene(&presentation, animating)
        };
        pass.render(&mut self.surface, |surface| match render(surface) {
            Ok(FrameResult::Presented { needs_animation }) => {
                Ok(FrameOutcome::complete(needs_animation && animation_allowed))
            }
            Ok(FrameResult::TargetRecreated) => Ok(FrameOutcome::Retry),
            Err(lotus_windows::graphics::SurfaceError::DeviceLost(loss)) => {
                graphics.mark_lost(loss);
                Ok(FrameOutcome::complete(false))
            }
            Err(error) => Err(error.into()),
        })
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
use std::time::Instant;
