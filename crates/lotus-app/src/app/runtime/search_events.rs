use lotus_core::window::WindowInfo;
use lotus_search::command::CommandId;
use lotus_ui::frame::ScheduledSurface;
use lotus_windows::clipboard::write_text;
use lotus_windows::dialog::show_error;
use lotus_windows::graphics::{CompositionSurfaceState, DeviceState, GraphicsDeviceHealth};
use lotus_windows::window::DockWindow;

use super::presentation::present_dock_change;
use crate::app::launcher::{LauncherEventOutcome, LauncherSubmission};
use crate::app::modules::ModuleHost;
use crate::app::system_actions::{Confirmation, SystemAction, execute_system_action};
use crate::app::{AppError, DockRuntime};

pub(super) fn refresh_catalog(
    dock: &DockWindow,
    graphics: &mut DeviceState,
    surface: &mut ScheduledSurface<CompositionSurfaceState>,
    windows: &[WindowInfo],
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<bool, AppError> {
    if !auxiliary.launcher_catalog_refresh_pending() {
        return Ok(false);
    }
    let application_catalog = auxiliary.application_snapshot();
    dock_model.adopt_catalogue_pins(&application_catalog)?;
    dock_model.rebuild(windows, application_catalog.clone());
    let catalog_changed = auxiliary.refresh_catalog(dock, dock_model, graphics)?;
    if !catalog_changed {
        return Ok(false);
    }
    auxiliary.reconcile_switcher_windows(
        windows,
        application_catalog,
        dock_model.application_assignments(),
        graphics,
    )?;
    present_dock_change(dock, graphics, surface, auxiliary, dock_model)?;
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
    }
}

fn execute_search_command(
    command: CommandId,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    dock_model: &DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    match command {
        CommandId::OpenSettings => auxiliary.open_settings(dock_model, graphics)?,
        CommandId::OpenVolumeMixer => {
            execute_system_action(SystemAction::OpenVolumeMixer, dock.handle());
        }
        CommandId::OpenNotificationArea => {
            execute_system_action(
                SystemAction::OpenNotificationArea { anchor: None },
                dock.handle(),
            );
        }
        CommandId::ShowDesktop => {
            execute_system_action(SystemAction::ShowDesktop, dock.handle());
        }
        CommandId::LockComputer => {
            execute_system_action(SystemAction::LockComputer, dock.handle());
        }
        CommandId::RestartComputer => {
            execute_system_action(
                SystemAction::RestartComputer {
                    confirmation: Confirmation::Required,
                },
                dock.handle(),
            );
        }
        CommandId::ShutDownComputer => {
            execute_system_action(
                SystemAction::ShutDownComputer {
                    confirmation: Confirmation::Required,
                },
                dock.handle(),
            );
        }
        CommandId::QuitLotus => {
            execute_system_action(SystemAction::QuitLotus, dock.handle());
        }
    }
    Ok(())
}
