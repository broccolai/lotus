use lotus_core::module::{ModuleId, ModuleSet};
use lotus_core::search::SearchUsage;
use lotus_core::settings::DockSettings;
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
use crate::app::search_usage::SearchUsageStore;
use crate::app::settings::SettingsRuntime;
use crate::app::status::StatusRuntime;
use crate::app::switcher::SwitcherRuntime;

pub(in crate::app) struct ModuleHostServices {
    pub(in crate::app) usage: SearchUsage,
    pub(in crate::app) usage_store: SearchUsageStore,
    pub(in crate::app) applications: ApplicationServices,
}

impl ModuleHost {
    pub(in crate::app) fn create(
        dock: &DockWindow,
        dock_model: &mut DockRuntime,
        services: ModuleHostServices,
        modules_active: bool,
        shell_effects_allowed: bool,
        updates_allowed: bool,
    ) -> Result<Self, AppError> {
        let mut search_window = dock.create_search_window()?;
        search_window.set_outside_click_allowed(shell_effects_allowed);
        dock_model.attach_icon_hydrator(services.applications.dock_icon_client());
        lotus_windows::backdrop::apply_search_settings(
            search_window.handle(),
            dock_model.settings(),
        );
        let launcher = LauncherRuntime::new(
            search_window,
            dock_model.settings().clone(),
            &theme_for(dock_model.settings()),
            services.usage,
            services.usage_store,
            services.applications.launcher_icon_client(),
        );
        let settings = SettingsRuntime::new(
            dock.create_settings_window()?,
            dock_model.settings().clone(),
            updates_allowed && is_installed().unwrap_or(false),
            updates_allowed,
            services.applications.settings_icon_client(),
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
            services.applications.switcher_icon_client(),
            services.applications.view(),
        );
        let status = StatusRuntime::new(
            [dock.create_status_window()?, dock.create_status_window()?],
            dock_model.settings(),
        )?;

        let mut host = Self {
            lifecycle: ModuleLifecycle::new(),
            applications: services.applications,
            launcher,
            settings,
            context_menu,
            media: MediaRuntime::new(false),
            status,
            monitors: MonitorDocks::new(shell_effects_allowed),
            switcher,
        };
        host.reconcile(
            dock,
            dock_model.settings(),
            modules_active,
            shell_effects_allowed,
        )?;
        Ok(host)
    }

    pub(in crate::app) fn reconcile(
        &mut self,
        dock: &DockWindow,
        settings: &DockSettings,
        active: bool,
        shell_effects_allowed: bool,
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
        self.lifecycle
            .reconcile_input(settings, transition.next, shell_effects_allowed);
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
    input_unavailable: bool,
    search_presentation_ready: bool,
    switcher_presentation_ready: bool,
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
            input_unavailable: false,
            search_presentation_ready: false,
            switcher_presentation_ready: false,
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
        !self.input_unavailable
            && self.input.as_ref().is_none_or(InputController::is_healthy)
    }

    pub(super) fn heartbeat_input(&self) {
        if let Some(input) = &self.input {
            input.heartbeat();
        }
    }

    pub(super) fn input_controller(&self) -> Option<&InputController> {
        self.input.as_ref()
    }

    pub(super) fn set_search_presentation_ready(&mut self, ready: bool) {
        self.search_presentation_ready = ready;
        self.publish_input_readiness();
    }

    pub(super) fn set_switcher_presentation_ready(&mut self, ready: bool) {
        self.switcher_presentation_ready = ready;
        self.publish_input_readiness();
    }

    pub(super) fn mark_presentations_unavailable(&mut self) {
        self.search_presentation_ready = false;
        self.switcher_presentation_ready = false;
        self.publish_input_readiness();
    }

    pub(super) fn runtime_capabilities_ready(&self) -> bool {
        !self.input_unavailable
            && self.input.as_ref().is_none_or(InputController::is_healthy)
            && self.input_config.is_none_or(|config| {
                (!config.windows_key_search || self.search_presentation_ready)
                    && (!config.custom_alt_tab || self.switcher_presentation_ready)
            })
    }

    pub(super) fn search_takeover_requested(&self) -> bool {
        self.input_config
            .is_some_and(|config| config.windows_key_search)
    }

    pub(super) fn switcher_takeover_requested(&self) -> bool {
        self.input_config
            .is_some_and(|config| config.custom_alt_tab)
    }

    fn publish_input_readiness(&self) {
        if let Some(input) = &self.input {
            input.set_capability_readiness(
                self.search_presentation_ready,
                self.switcher_presentation_ready,
            );
        }
    }

    fn reconcile_input(
        &mut self,
        settings: &DockSettings,
        modules: ModuleSet,
        shell_effects_allowed: bool,
    ) {
        let next = InputConfig {
            windows_key_search: shell_effects_allowed
                && modules.contains(ModuleId::Search)
                && settings.search_open_with_windows_key,
            custom_alt_tab: shell_effects_allowed && modules.contains(ModuleId::AltTab),
        };
        let next = (next.windows_key_search || next.custom_alt_tab).then_some(next);
        if next.is_none() {
            self.input = None;
            self.input_config = None;
            self.input_unavailable = false;
            return;
        }
        if self.input_config == next {
            return;
        }

        self.input = None;
        self.input_config = None;
        self.input_unavailable = false;
        let Some(config) = next else {
            return;
        };

        match InputController::start(config) {
            Ok(controller) => {
                controller.set_capability_readiness(
                    self.search_presentation_ready,
                    self.switcher_presentation_ready,
                );
                self.input = Some(controller);
                self.input_config = Some(config);
            }
            Err(error) => {
                self.input_unavailable = true;
                lotus_windows::diagnostics::record_error("input.enable", &error);
                lotus_windows::diagnostics::record_diagnostic(
                    "input.unavailable",
                    "configured=true fail_open=true",
                );
            }
        }
    }
}
