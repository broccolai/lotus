use lotus_core::module::ModuleId;
use lotus_windows::graphics::DeviceState;
use lotus_windows::window::DockWindow;

use super::ModuleHost;
use crate::app::AppError;
use crate::app::context_menu::PopupOwner;
use crate::app::dock::DockRuntime;
use crate::app::launcher::LauncherEventOutcome;

impl ModuleHost {
    pub(in crate::app) fn toggle_launcher(
        &mut self,
        dock: &DockWindow,
        dock_model: &DockRuntime,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        if !self.lifecycle.is_enabled(ModuleId::Search) {
            return Ok(());
        }

        if self.launcher.is_visible() {
            self.hide_launcher();
            return Ok(());
        }

        self.applications.refresh_launcher_catalog_if_stale();
        let catalog = self.applications.prepare_launcher_catalog(
            dock_model.items(),
            &dock_model.settings().hidden_executables,
        );

        self.launcher.open(dock, dock_model, catalog, graphics)
    }

    pub(in crate::app) fn hide_launcher(&mut self) {
        self.context_menu.close_if_owned_by(PopupOwner::Search);
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
        if matches!(event, lotus_windows::window::SearchEvent::DismissRequested(request) if self.launcher.window.accepts_dismiss(request))
        {
            self.context_menu.close_if_owned_by(PopupOwner::Search);
        }
        let outcome = self
            .launcher
            .handle_event(event, dock, graphics, dock_model)?;
        Ok(outcome)
    }

    pub(in crate::app) fn dismiss_popups_for_activation(&mut self) {
        self.hide_launcher();
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
            self.hide_launcher();
        }
    }
}
