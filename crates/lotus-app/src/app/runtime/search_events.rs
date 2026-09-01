use lotus_core::window::WindowInfo;
use lotus_search::command::CommandId;
use lotus_ui::frame::ScheduledSurface;
use lotus_windows::activation::launch_target;
use lotus_windows::clipboard::write_text;
use lotus_windows::dialog::{confirm_restart, confirm_shutdown, show_error};
use lotus_windows::graphics::{CompositionSurfaceState, DeviceState, GraphicsDeviceHealth};
use lotus_windows::interaction::request_exit;
use lotus_windows::window::DockWindow;

use super::presentation::present_dock_change;
use crate::app::launcher::{LauncherEventOutcome, LauncherSubmission};
use crate::app::modules::ModuleHost;
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
    let context_menu_request = events.iter().position(|event| {
        matches!(
            event,
            lotus_windows::window::SearchEvent::ContextMenuRequested(_)
        )
    });
    for (index, event) in events.into_iter().enumerate() {
        // The outside-click hook sees the right-button press before Windows sends the
        // context-menu request on release. Preserve that in-Search handoff only.
        if context_menu_request.is_some_and(|request_index| index < request_index)
            && matches!(event, lotus_windows::window::SearchEvent::DismissRequested)
        {
            continue;
        }
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
            if let Err(error) = launch_target("sndvol.exe", None) {
                show_error(
                    dock.handle(),
                    "Lotus",
                    &format!("Lotus could not open the Windows volume mixer.\n\n{error}"),
                );
            }
        }
        CommandId::OpenNotificationArea => {
            if let Err(error) = lotus_windows::tray::open_overflow(dock.handle()) {
                show_error(
                    dock.handle(),
                    "Lotus",
                    &format!(
                        "Lotus could not open the Windows notification area.\n\n{error}"
                    ),
                );
            }
        }
        CommandId::ShowDesktop => {
            if let Err(error) = lotus_windows::desktop::toggle() {
                show_error(
                    dock.handle(),
                    "Lotus",
                    &format!("Lotus could not show the desktop.\n\n{error}"),
                );
            }
        }
        CommandId::LockComputer => {
            if let Err(error) = lotus_windows::desktop::lock() {
                show_error(
                    dock.handle(),
                    "Lotus",
                    &format!("Lotus could not lock Windows.\n\n{error}"),
                );
            }
        }
        CommandId::RestartComputer => {
            if confirm_restart(dock.handle())
                && let Err(error) = launch_target("shutdown.exe", Some("/r /t 0"))
            {
                show_error(
                    dock.handle(),
                    "Lotus",
                    &format!("Lotus could not restart Windows.\n\n{error}"),
                );
            }
        }
        CommandId::ShutDownComputer => {
            if confirm_shutdown(dock.handle())
                && let Err(error) = launch_target("shutdown.exe", Some("/s /t 0"))
            {
                show_error(
                    dock.handle(),
                    "Lotus",
                    &format!("Lotus could not shut down Windows.\n\n{error}"),
                );
            }
        }
        CommandId::QuitLotus => request_exit(0),
    }
    Ok(())
}
