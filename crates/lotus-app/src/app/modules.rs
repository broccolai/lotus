use lotus_core::module::{ModuleId, ModuleSet};
use lotus_core::settings::DockSettings;
use lotus_windows::icon_hydrator::IconHydrator;
use lotus_windows::input::{InputConfig, InputController};
use lotus_windows::search_catalog::SearchCatalogCache;
use lotus_windows::window::DockWindow;

use crate::app::AppError;
use crate::app::context_menu::ContextMenuRuntime;
use crate::app::launcher::LauncherRuntime;
use crate::app::media::MediaRuntime;
use crate::app::monitors::MonitorDocks;
use crate::app::settings::SettingsRuntime;
use crate::app::status::StatusRuntime;
use crate::app::switcher::SwitcherRuntime;

pub(super) struct ModuleHost {
    pub(super) modules: ModuleRuntime,
    pub(super) icon_hydrator: IconHydrator,
    pub(super) applications: SearchCatalogCache,
    pub(super) launcher: LauncherRuntime,
    pub(super) settings: SettingsRuntime,
    pub(super) context_menu: ContextMenuRuntime,
    pub(super) media: MediaRuntime,
    pub(super) status: StatusRuntime,
    pub(super) monitors: MonitorDocks,
    pub(super) switcher: SwitcherRuntime,
}

impl ModuleHost {
    pub(super) const fn is_enabled(&self, module: ModuleId) -> bool {
        self.modules.is_enabled(module)
    }

    pub(super) const fn input(&self) -> Option<&InputController> {
        self.modules.input()
    }

    pub(super) fn reconcile(
        &mut self,
        dock: &DockWindow,
        settings: &DockSettings,
        active: bool,
    ) -> Result<(), AppError> {
        self.modules.reconcile(
            settings,
            active,
            &mut ModuleResources {
                dock,
                launcher: &mut self.launcher,
                switcher: &mut self.switcher,
                media: &mut self.media,
                status: &mut self.status,
            },
        )
    }

    pub(super) fn invalidate_surfaces(&mut self) {
        self.launcher.invalidate();
        self.settings.invalidate();
        self.context_menu.invalidate();
        self.switcher.invalidate();
        self.status.invalidate();
        self.monitors.invalidate();
    }
}

pub(super) struct ModuleRuntime {
    enabled: ModuleSet,
    input_config: Option<InputConfig>,
    input: Option<InputController>,
}

pub(super) struct ModuleResources<'a> {
    pub(super) dock: &'a DockWindow,
    pub(super) launcher: &'a mut LauncherRuntime,
    pub(super) switcher: &'a mut SwitcherRuntime,
    pub(super) media: &'a mut MediaRuntime,
    pub(super) status: &'a mut StatusRuntime,
}

impl ModuleRuntime {
    pub(super) fn new() -> Self {
        Self {
            enabled: ModuleSet::default(),
            input_config: None,
            input: None,
        }
    }

    pub(super) fn reconcile(
        &mut self,
        settings: &DockSettings,
        active: bool,
        resources: &mut ModuleResources<'_>,
    ) -> Result<(), AppError> {
        let next = if active {
            ModuleSet::from_settings(settings)
        } else {
            ModuleSet::default()
        };

        if self.was_disabled(ModuleId::Search, next) {
            resources.launcher.hide();
        }
        if self.was_disabled(ModuleId::AltTab, next) {
            resources.switcher.abandon();
        }
        if self.was_disabled(ModuleId::Status, next) {
            resources.status.set_visible(false);
        }

        resources.media.set_enabled(next.contains(ModuleId::Media));
        resources.dock.set_status_refresh_active(
            next.contains(ModuleId::Status) && settings.show_date_time_status,
        )?;
        self.reconcile_input(settings, next);
        self.enabled = next;
        Ok(())
    }

    pub(super) const fn is_enabled(&self, module: ModuleId) -> bool {
        self.enabled.contains(module)
    }

    pub(super) const fn input(&self) -> Option<&InputController> {
        self.input.as_ref()
    }

    fn was_disabled(&self, module: ModuleId, next: ModuleSet) -> bool {
        self.enabled.contains(module) && !next.contains(module)
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
