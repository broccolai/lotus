mod applications;
mod input;
mod launcher;
mod lifecycle;
mod monitors;
mod popups;
mod presentation;
mod settings;
mod status;

use lotus_settings::scene::SettingsAction;
use lotus_windows::WindowHandle;
use lotus_windows::window::SignedPoint;

use crate::app::applications::ApplicationServices;
use crate::app::context_menu::ContextMenuRuntime;
use crate::app::launcher::LauncherRuntime;
use crate::app::media::MediaRuntime;
use crate::app::monitors::MonitorDocks;
use crate::app::settings::SettingsRuntime;
use crate::app::status::{AuxiliaryZoneAction, StatusRuntime};
use crate::app::switcher::SwitcherRuntime;

pub(super) struct ModuleHost {
    lifecycle: lifecycle::ModuleLifecycle,
    applications: ApplicationServices,
    launcher: LauncherRuntime,
    settings: SettingsRuntime,
    context_menu: ContextMenuRuntime,
    media: MediaRuntime,
    status: StatusRuntime,
    monitors: MonitorDocks,
    switcher: SwitcherRuntime,
}

pub(super) enum SettingsIntent {
    None,
    PasteQuery,
    Action(SettingsAction),
}

pub(super) struct StatusZoneActivation {
    pub(super) action: AuxiliaryZoneAction,
    pub(super) owner: WindowHandle,
    pub(super) anchor: Option<SignedPoint>,
}
