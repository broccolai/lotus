use lotus_core::module::ModuleId;
use lotus_windows::graphics::DeviceState;
use lotus_windows::search_catalog::ApplicationCatalogSnapshot;
use lotus_windows::window::DockWindow;

use super::ModuleHost;
use crate::app::AppError;
use crate::app::dock::DockRuntime;
use crate::app::launcher::LauncherEventOutcome;

impl ModuleHost {
    pub(in crate::app) fn toggle_launcher(
        &mut self,
        dock: &DockWindow,
        dock_model: &DockRuntime,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        if !self.modules.is_enabled(ModuleId::Search) {
            return Ok(());
        }
        self.launcher
            .toggle(dock, dock_model, &self.applications, graphics)
    }

    pub(in crate::app) fn hide_launcher(&mut self) {
        self.launcher.hide();
    }

    pub(in crate::app) fn launcher_is_visible(&self) -> bool {
        self.launcher.is_visible()
    }

    pub(in crate::app) fn advance_launcher_animation(&mut self) {
        self.launcher.advance_animation();
    }

    pub(in crate::app) fn invalidate_launcher_surface(&mut self) {
        self.launcher.invalidate();
    }

    pub(in crate::app) fn drain_launcher_events(
        &mut self,
    ) -> Vec<lotus_windows::window::SearchEvent> {
        self.launcher.drain_events()
    }

    pub(in crate::app) fn handle_launcher_event(
        &mut self,
        event: lotus_windows::window::SearchEvent,
        dock: &DockWindow,
        graphics: &mut DeviceState,
        dock_model: &DockRuntime,
    ) -> Result<LauncherEventOutcome, AppError> {
        self.launcher
            .handle_event(event, dock, graphics, dock_model)
    }

    pub(in crate::app) fn refresh_catalog(
        &mut self,
        dock: &DockWindow,
        dock_model: &mut DockRuntime,
        graphics: &mut DeviceState,
    ) -> Result<bool, AppError> {
        self.launcher.refresh_catalog_if_ready(
            dock,
            dock_model,
            &self.applications,
            graphics,
        )
    }

    pub(in crate::app) fn application_snapshot(
        &self,
    ) -> std::sync::Arc<ApplicationCatalogSnapshot> {
        self.applications.snapshot()
    }

    pub(in crate::app) fn launcher_catalog_refresh_pending(&self) -> bool {
        self.applications
            .ready_generation()
            .is_some_and(|generation| {
                self.launcher.catalog_generation() != Some(generation)
            })
    }

    pub(in crate::app) fn dismiss_popups_for_activation(&mut self) {
        self.launcher.hide();
    }

    pub(in crate::app) fn hide_launcher_on_status_press(
        &mut self,
        event: &lotus_windows::window::StatusEvent,
    ) {
        if matches!(
            event,
            lotus_windows::window::StatusEvent::Pointer(
                lotus_windows::window::PointerEvent::LeftButtonPressed { .. }
            )
        ) {
            self.launcher.hide();
        }
    }
}
