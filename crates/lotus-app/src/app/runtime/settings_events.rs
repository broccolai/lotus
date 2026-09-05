use lotus_windows::graphics::{DeviceState, GraphicsDeviceHealth};
use lotus_windows::startup::StartupMode;
use lotus_windows::window_tracker::WindowTracker;

use super::settings_actions::execute_settings_action;
use crate::app::integration::IntegrationRecovery;
use crate::app::modules::ModuleHost;
use crate::app::primary_dock::PrimaryDock;
use crate::app::settings::SettingsCommand;
use crate::app::settings_persistence::SettingsPersistence;
use crate::app::{AppError, DockRuntime};

pub(super) struct SettingsEventContext<'a> {
    pub(super) primary_dock: &'a mut PrimaryDock,
    pub(super) graphics: &'a mut DeviceState,
    pub(super) window_tracker: &'a mut WindowTracker,
    pub(super) dock_model: &'a mut DockRuntime,
    pub(super) auxiliary: &'a mut ModuleHost,
    pub(super) integration: &'a mut IntegrationRecovery,
    pub(super) settings_persistence: &'a SettingsPersistence,
    pub(super) startup_mode: StartupMode,
    pub(super) startup_registration_allowed: bool,
}

pub(super) fn drain_settings_events(
    context: &mut SettingsEventContext<'_>,
) -> Result<bool, AppError> {
    let events = context.auxiliary.drain_settings_events();
    let had_events = !events.is_empty();

    for event in events {
        let result = (|| {
            let intent = context.auxiliary.handle_settings_event(
                event,
                context.graphics,
                context.dock_model.items(),
            )?;
            match intent {
                None => Ok(()),
                Some(SettingsCommand::PasteQuery) => {
                    if let Ok(clipboard) = lotus_windows::clipboard::read_text() {
                        context
                            .auxiliary
                            .paste_settings_query(&clipboard, context.dock_model.items());
                    }
                    Ok(())
                }
                Some(SettingsCommand::Action(action)) => {
                    execute_settings_action(action, context)
                }
            }
        })();
        match result {
            Ok(()) => {}
            Err(error)
                if error.mark_graphics_lost(context.graphics)
                    || context.graphics.health() == GraphicsDeviceHealth::Lost => {}
            Err(error) => return Err(error),
        }
    }
    Ok(had_events)
}
