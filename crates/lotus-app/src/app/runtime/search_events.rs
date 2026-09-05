use lotus_core::window::WindowInfo;
use lotus_search::command::CommandId;
use lotus_windows::activation::launch_target;
use lotus_windows::clipboard::{read_text, write_text};
use lotus_windows::dialog::show_error;
use lotus_windows::graphics::{DeviceState, GraphicsDeviceHealth};
use lotus_windows::window::DockWindow;

use super::presentation::present_dock_change;
use crate::app::launcher::{LauncherEventOutcome, LauncherSubmission};
use crate::app::modules::ModuleHost;
use crate::app::primary_dock::PrimaryDock;
use crate::app::settings_persistence::SettingsPersistence;
use crate::app::system_actions::{Confirmation, SystemAction, execute_system_action};
use crate::app::{AppError, DockRuntime};

pub(super) fn refresh_catalog(
    primary_dock: &mut PrimaryDock,
    graphics: &mut DeviceState,
    windows: &[WindowInfo],
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
    settings_persistence: &SettingsPersistence,
) -> Result<bool, AppError> {
    if !auxiliary.launcher_catalog_refresh_pending() {
        return Ok(false);
    }
    let application_catalog = auxiliary.application_snapshot();
    dock_model.adopt_catalogue_pins(&application_catalog, settings_persistence)?;
    dock_model.rebuild(windows, application_catalog.clone());
    let catalog_changed =
        auxiliary.refresh_catalog(primary_dock.window(), dock_model, graphics)?;
    if !catalog_changed {
        return Ok(false);
    }
    auxiliary.reconcile_switcher_windows(
        windows,
        application_catalog,
        dock_model.application_assignments(),
        graphics,
    )?;
    present_dock_change(primary_dock, graphics, auxiliary, dock_model)?;
    auxiliary.refresh_open_application_manager(dock_model.items());
    auxiliary.invalidate_launcher_surface();
    Ok(true)
}

pub(super) fn drain_search_events(
    dock: &DockWindow,
    graphics: &mut DeviceState,
    dock_model: &DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<bool, AppError> {
    let events = auxiliary.drain_launcher_events();
    let had_events = !events.is_empty();
    for event in events {
        let outcome =
            match auxiliary.handle_launcher_event(event, dock, graphics, dock_model) {
                Ok(outcome) => outcome,
                Err(error)
                    if error.mark_graphics_lost(graphics)
                        || graphics.health() == GraphicsDeviceHealth::Lost =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            };
        match outcome {
            LauncherEventOutcome::None => {}
            LauncherEventOutcome::PasteRequested => {
                paste_search_clipboard(dock, graphics, auxiliary)?;
            }
            LauncherEventOutcome::Submission(submission) => {
                execute_search_submission(
                    submission, dock, graphics, dock_model, auxiliary,
                )?;
            }
            LauncherEventOutcome::OpenFileLocation { anchor, path } => {
                auxiliary.open_search_file_location_menu(anchor, path, graphics)?;
            }
        }
    }
    Ok(had_events)
}

fn paste_search_clipboard(
    dock: &DockWindow,
    graphics: &mut DeviceState,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    let Ok(text) = read_text() else {
        return Ok(());
    };

    match auxiliary.paste_into_launcher(&text, dock, graphics) {
        Ok(()) => Ok(()),
        Err(error)
            if error.mark_graphics_lost(graphics)
                || graphics.health() == GraphicsDeviceHealth::Lost =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn execute_search_submission(
    submission: LauncherSubmission,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    dock_model: &DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    match submission {
        LauncherSubmission::Command(command) => {
            execute_search_command(command, dock, graphics, dock_model, auxiliary)
        }
        LauncherSubmission::Calculation(value) => {
            if let Err(error) = write_text(&value) {
                show_error(
                    dock.handle(),
                    "Lotus Calculator",
                    &format!("Lotus could not copy the result.\n\n{error}"),
                );
            }
            Ok(())
        }
        LauncherSubmission::Application(entry) => {
            match launch_target(&entry.launch_target, entry.invocation_arguments()) {
                Ok(()) => auxiliary.record_successful_launcher_launch(&entry.launch_target),
                Err(error) => show_error(
                    dock.handle(),
                    "Lotus Search",
                    &format!("Lotus could not open {}.\n\n{error}", entry.name),
                ),
            }
            Ok(())
        }
    }
}

fn execute_search_command(
    command: CommandId,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    dock_model: &DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    let action = match command {
        CommandId::OpenSettings => return auxiliary.open_settings(dock_model, graphics),
        CommandId::OpenVolumeMixer => SystemAction::OpenVolumeMixer,
        CommandId::OpenNotificationArea => {
            SystemAction::OpenNotificationArea { anchor: None }
        }
        CommandId::ShowDesktop => SystemAction::ShowDesktop,
        CommandId::LockComputer => SystemAction::LockComputer,
        CommandId::RestartComputer => SystemAction::RestartComputer {
            confirmation: Confirmation::Required,
        },
        CommandId::ShutDownComputer => SystemAction::ShutDownComputer {
            confirmation: Confirmation::Required,
        },
        CommandId::QuitLotus => SystemAction::QuitLotus,
    };

    execute_system_action(action, dock.handle());
    Ok(())
}
