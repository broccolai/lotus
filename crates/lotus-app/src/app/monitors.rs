use lotus_core::settings::DockSettings;
use lotus_ui::frame::FramePass;
use lotus_windows::WindowHandle;
use lotus_windows::graphics::{DeviceState, GraphicsDevice};
use lotus_windows::window::{DockReplicaWindow, DockWindow, PopupAlignment, SignedPoint};
use lotus_windows::window_tracker::WindowTracker;

use crate::app::AppError;
use crate::app::visuals::{DockHitTarget, DockScene};

mod replica;

use self::replica::{MonitorDock, ReplicaEventDrain};

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
    pub(super) drained_events: usize,
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

impl MonitorDocks {
    pub(super) fn owns_window(&self, window: WindowHandle) -> bool {
        self.docks.iter().any(|dock| dock.handle() == window)
    }

    pub(super) fn has_pending_events(&self) -> bool {
        self.docks.iter().any(MonitorDock::has_pending_events)
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
        self.docks.iter().any(MonitorDock::is_visible)
    }

    pub(super) fn diagnostic_surface_masks(&self) -> (bool, bool, bool) {
        self.docks
            .iter()
            .fold((false, false, false), |state, dock| {
                let (dirty, animating) = dock.diagnostic_surface_state();
                (
                    state.0 || dirty,
                    state.1 || animating,
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
            replica.invalidate();
        }
    }

    pub(super) fn recover_surfaces(
        &mut self,
        device: &GraphicsDevice,
    ) -> Result<(), AppError> {
        for replica in &mut self.docks {
            replica.recover_surface(device)?;
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
                && tracker.fullscreen_on_same_monitor(replica.handle());
            let occluded = settings.hide_when_fullscreen && fullscreen;
            replica.set_fullscreen_occluded(occluded)?;
        }
        Ok(())
    }

    pub(super) fn drain_events_up_to(
        &mut self,
        graphics: &mut DeviceState,
        limit: usize,
    ) -> Result<MonitorDockEventDrain, AppError> {
        let mut actions = Vec::new();
        let mut refresh = false;
        let mut had_events = false;
        let mut drained = 0;
        for replica in &mut self.docks {
            let remaining = limit.saturating_sub(drained);
            if remaining == 0 {
                break;
            }
            let ReplicaEventDrain {
                actions: replica_actions,
                had_events: replica_had_events,
                drained_events: replica_drained,
                topology_refresh_requested,
            } = replica.drain_events_up_to(graphics, remaining)?;
            actions.extend(replica_actions);
            drained += replica_drained;
            had_events |= replica_had_events;
            refresh |= topology_refresh_requested;
        }
        if refresh {
            self.mark_topology_dirty();
        }
        Ok(MonitorDockEventDrain {
            actions,
            had_events,
            drained_events: drained,
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
            docks.push(MonitorDock::create(
                dock, window, scene, settings, graphics,
            )?);
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
        if !replica_inputs_match(self.docks.iter().map(MonitorDock::handle), &inputs) {
            return Err(AppError::InvalidScene);
        }

        for (replica, replica_input) in self.docks.iter_mut().zip(inputs) {
            replica.refresh(dock, replica_input.scene, settings, graphics)?;
        }
        Ok(())
    }

    fn replica_targets(&self) -> Vec<MonitorReplicaTarget> {
        self.docks
            .iter()
            .map(|replica| MonitorReplicaTarget {
                dpi: replica.dpi(),
                owner: replica.handle(),
            })
            .collect()
    }

    fn record_recovery_failure<E: std::error::Error + 'static>(&mut self, error: &E) {
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
