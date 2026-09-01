use lotus_core::module::{ModuleId, ModuleSet};
use lotus_core::search::SearchUsage;
use lotus_core::settings::DockSettings;
use lotus_search::usage::SearchUsageStore;
use lotus_settings::appearance::theme_for;
use lotus_windows::graphics::DeviceState;
use lotus_windows::input::{InputConfig, InputController};
use lotus_windows::update::is_installed;
use lotus_windows::window::DockWindow;

use super::ModuleHost;
use crate::app::AppError;
use crate::app::applications::ApplicationServices;
use crate::app::context_menu::ContextMenuRuntime;
use crate::app::dock::DockRuntime;
use crate::app::launcher::LauncherRuntime;
use crate::app::media::MediaRuntime;
use crate::app::monitors::MonitorDocks;
use crate::app::settings::SettingsRuntime;
use crate::app::status::StatusRuntime;
use crate::app::switcher::SwitcherRuntime;

impl ModuleHost {
    pub(in crate::app) fn create(
        dock: &DockWindow,
        dock_model: &mut DockRuntime,
        usage: SearchUsage,
        usage_store: SearchUsageStore,
        modules_active: bool,
    ) -> Result<Self, AppError> {
        let search_window = dock.create_search_window()?;
        let applications = ApplicationServices::new()?;
        dock_model.attach_icon_hydrator(applications.dock_icon_client());
        lotus_windows::backdrop::apply_search_settings(
            search_window.handle(),
            dock_model.settings(),
        );
        let launcher = LauncherRuntime::new(
            search_window,
            dock_model.settings().clone(),
            &theme_for(dock_model.settings()),
            usage,
            usage_store,
            applications.launcher_icon_client(),
        );
        let settings = SettingsRuntime::new(
            dock.create_settings_window()?,
            dock_model.settings().clone(),
            is_installed().unwrap_or(false),
        )?;
        let context_menu_window = dock.create_context_menu_window()?;
        lotus_windows::backdrop::apply_context_menu_settings(
            context_menu_window.handle(),
            dock_model.settings(),
        );
        let context_menu = ContextMenuRuntime::new(
            context_menu_window,
            &theme_for(dock_model.settings()),
        )?;
        let switcher_window = dock.create_switcher_window()?;
        lotus_windows::backdrop::apply_popup_settings(
            switcher_window.handle(),
            dock_model.settings(),
        );
        let switcher = SwitcherRuntime::new(
            switcher_window,
            dock_model.settings(),
            &theme_for(dock_model.settings()),
            applications.switcher_icon_client(),
        );
        let status = StatusRuntime::new(
            [dock.create_status_window()?, dock.create_status_window()?],
            dock_model.settings(),
        )?;

        let mut host = Self {
            lifecycle: ModuleLifecycle::new(),
            applications,
            launcher,
            settings,
            context_menu,
            media: MediaRuntime::new(false),
            status,
            monitors: MonitorDocks::new(),
            switcher,
        };
        host.reconcile(dock, dock_model.settings(), modules_active)?;
        Ok(host)
    }

    pub(in crate::app) fn reconcile(
        &mut self,
        dock: &DockWindow,
        settings: &DockSettings,
        active: bool,
    ) -> Result<(), AppError> {
        let transition = self.lifecycle.transition(settings, active);

        if transition.disabled(ModuleId::Search) {
            self.hide_launcher();
        }
        if transition.disabled(ModuleId::AltTab) {
            self.switcher.abandon();
        }
        if transition.disabled(ModuleId::Status) {
            self.status.set_visible(false);
        }

        self.media.set_enabled(transition.enabled(ModuleId::Media));
        dock.set_status_refresh_active(
            transition.enabled(ModuleId::Status) && settings.show_date_time_status,
        )?;
        self.lifecycle.reconcile_input(settings, transition.next);
        self.lifecycle.enabled = transition.next;
        Ok(())
    }

    pub(in crate::app) fn propagate_settings(
        &mut self,
        settings: &DockSettings,
        dock: &DockWindow,
        graphics: &mut DeviceState,
    ) -> Result<(), AppError> {
        self.launcher.apply_settings(settings, dock, graphics)?;
        self.context_menu.apply_settings(settings);
        self.switcher.apply_settings(settings);
        Ok(())
    }
}

pub(super) struct ModuleLifecycle {
    enabled: ModuleSet,
    input_config: Option<InputConfig>,
    input: Option<InputController>,
}

struct LifecycleTransition {
    previous: ModuleSet,
    next: ModuleSet,
}

impl LifecycleTransition {
    const fn disabled(&self, module: ModuleId) -> bool {
        self.previous.contains(module) && !self.next.contains(module)
    }

    const fn enabled(&self, module: ModuleId) -> bool {
        self.next.contains(module)
    }
}

impl ModuleLifecycle {
    fn new() -> Self {
        Self {
            enabled: ModuleSet::default(),
            input_config: None,
            input: None,
        }
    }

    fn transition(&self, settings: &DockSettings, active: bool) -> LifecycleTransition {
        let next = if active {
            ModuleSet::from_settings(settings)
        } else {
            ModuleSet::default()
        };

        LifecycleTransition {
            previous: self.enabled,
            next,
        }
    }

    pub(super) const fn is_enabled(&self, module: ModuleId) -> bool {
        self.enabled.contains(module)
    }

    pub(super) const fn input_enabled(&self) -> bool {
        self.input.is_some()
    }

    pub(super) fn input_healthy(&self) -> bool {
        self.input.as_ref().is_none_or(InputController::is_healthy)
    }

    pub(super) fn heartbeat_input(&self) {
        if let Some(input) = &self.input {
            input.heartbeat();
        }
    }

    pub(super) fn input_controller(&self) -> Option<&InputController> {
        self.input.as_ref()
    }

    fn reconcile_input(&mut self, settings: &DockSettings, modules: ModuleSet) {
        let next = InputConfig {
            windows_key_search: modules.contains(ModuleId::Search)
                && settings.search_open_with_windows_key,
            custom_alt_tab: modules.contains(ModuleId::AltTab),
        };
        let next = (next.windows_key_search || next.custom_alt_tab).then_some(next);
        if self.input_config == next {
            return;
        }

        self.input = None;
        self.input_config = None;
        let Some(config) = next else {
            return;
        };

        match InputController::start(config) {
            Ok(controller) => {
                self.input = Some(controller);
                self.input_config = Some(config);
            }
            Err(error) => lotus_windows::diagnostics::record_error("input.enable", &error),
        }
    }
}
