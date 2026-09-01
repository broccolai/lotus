use lotus_core::settings::DockSettings;
use lotus_windows::graphics::DeviceState;
use lotus_windows::window::DockWindow;
use lotus_windows::window_tracker::WindowTracker;

use super::ModuleHost;
use crate::app::AppError;
use crate::app::dock::DockRuntime;
use crate::app::monitors::{MonitorDockEventDrain, MonitorIntegrationHealth};

impl ModuleHost {
    pub(in crate::app) fn sync_status(
        &mut self,
        dock: &DockWindow,
        dock_model: &DockRuntime,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.status
            .sync(dock, dock_model.settings(), dock_model.media(), graphics)
    }

    pub(in crate::app) fn refresh_placement(
        &mut self,
        dock: &DockWindow,
        dock_model: &DockRuntime,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.monitors.mark_topology_dirty();
        self.sync_status(dock, dock_model, graphics)?;
        if self.launcher.is_visible() {
            self.launcher.refresh_placement(dock, graphics)?;
        }
        Ok(())
    }

    pub(in crate::app) fn sync_monitor_docks(
        &mut self,
        dock: &DockWindow,
        dock_model: &mut DockRuntime,
        graphics: &mut DeviceState,
        window_tracker: &WindowTracker,
    ) -> Result<(), AppError> {
        let mut request =
            self.monitors
                .begin_sync(dock, dock_model.settings(), dock_model.revision())?;
        let input = match dock_model.prepare_monitor_presentation(request.take_targets()) {
            Ok(input) => input,
            Err(error) => {
                self.monitors.abort_sync(&request, &error);
                return Err(error);
            }
        };
        self.monitors
            .finish_sync(dock, request, input, graphics, window_tracker)
    }

    pub(in crate::app) fn drain_monitor_dock_events(
        &mut self,
        graphics: &mut DeviceState,
    ) -> Result<MonitorDockEventDrain, AppError> {
        self.monitors.drain_events(graphics)
    }

    pub(in crate::app) fn has_pending_monitor_events(&self) -> bool {
        self.monitors.has_pending_events()
    }

    pub(in crate::app) const fn monitor_topology_generation(&self) -> u64 {
        self.monitors.topology_generation()
    }

    pub(in crate::app) const fn monitor_integration_health(
        &self,
    ) -> MonitorIntegrationHealth {
        self.monitors.health()
    }

    pub(in crate::app) fn monitor_replica_count(&self) -> usize {
        self.monitors.replica_count()
    }

    pub(in crate::app) fn has_visible_monitor_dock(&self) -> bool {
        self.monitors.has_visible_dock()
    }

    pub(in crate::app) fn monitor_docks_own_window(
        &self,
        window: lotus_windows::WindowHandle,
    ) -> bool {
        self.monitors.owns_window(window)
    }

    pub(in crate::app) fn refresh_status(&mut self, settings: &DockSettings) {
        self.status.refresh(settings);
    }
}
