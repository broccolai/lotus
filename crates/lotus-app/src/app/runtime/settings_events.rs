use lotus_windows::graphics::{DeviceState, GraphicsDeviceHealth};
use lotus_windows::startup::StartupMode;
use lotus_windows::window_tracker::WindowTracker;

use super::settings_actions::execute_settings_action;
use super::settings_commit::{apply_persisted_settings, show_settings_persistence_failure};
use super::settings_support::complete_reset_lotus;
use crate::app::integration::IntegrationRecovery;
use crate::app::modules::ModuleHost;
use crate::app::primary_dock::PrimaryDock;
use crate::app::settings::SettingsCommand;
use crate::app::settings_persistence::{
    PersistenceOperation, PersistenceOutcome, SettingsPersistence,
};
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

pub(super) fn drain_settings_events_up_to(
    context: &mut SettingsEventContext<'_>,
    limit: usize,
) -> Result<usize, AppError> {
    let events = context.auxiliary.drain_settings_events_up_to(limit);
    let drained = events.len();

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
    Ok(drained)
}

pub(super) fn apply_persistence_completion(
    context: &mut SettingsEventContext<'_>,
) -> Result<bool, AppError> {
    let Some(completion) = context.settings_persistence.drain_completion() else {
        return Ok(false);
    };
    lotus_windows::diagnostics::record_diagnostic(
        "settings.persistence_completion",
        &format!("operation_id={}", completion.operation_id),
    );
    match (completion.operation, completion.outcome) {
        (PersistenceOperation::Save { reason, settings }, PersistenceOutcome::Saved) => {
            apply_persisted_settings(reason, *settings, context)?;
        }
        (PersistenceOperation::Save { reason, .. }, PersistenceOutcome::Failed(error)) => {
            match reason {
                crate::app::settings_persistence::SettingsSaveReason::Pin => {
                    lotus_windows::dialog::show_error(
                        context.primary_dock.window().handle(),
                        "Lotus",
                        &format!("Lotus could not save that pin.\n\n{error}"),
                    );
                }
                crate::app::settings_persistence::SettingsSaveReason::Reorder => {
                    lotus_windows::dialog::show_error(
                        context.primary_dock.window().handle(),
                        "Lotus",
                        &format!("Lotus could not save that dock order.\n\n{error}"),
                    );
                }
                crate::app::settings_persistence::SettingsSaveReason::Apply { .. } => {
                    show_settings_persistence_failure(
                        PersistenceOutcome::Failed(error),
                        context,
                    );
                }
            }
        }
        (PersistenceOperation::Save { .. }, PersistenceOutcome::Reset(_)) => {}
        (PersistenceOperation::Reset, outcome) => complete_reset_lotus(outcome, context),
    }
    Ok(true)
}
