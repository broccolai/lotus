use lotus_core::launcher_model::{CursorMove as ModelCursorMove, QueryEdit};
use lotus_core::window::WindowInfo;
use lotus_search::command::CommandId;
use lotus_ui::frame::ScheduledSurface;
use lotus_windows::activation::launch_target;
use lotus_windows::clipboard::{read_text, write_text};
use lotus_windows::clock::local_time;
use lotus_windows::dialog::{confirm_restart, confirm_shutdown, show_error};
use lotus_windows::graphics::{
    CompositionSurfaceState, DeviceState, GraphicsDeviceHealth, SurfaceSize,
};
use lotus_windows::interaction::request_exit;
use lotus_windows::window::{
    CursorMove as WindowCursorMove, DockWindow, SearchEdit, SearchEvent,
};

use super::presentation::{present_dock_change, resize_launcher_surface};
use crate::app::launcher::{LauncherRuntime, LauncherSubmission};
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
    let catalog_changed = auxiliary.refresh_catalog(dock, dock_model, graphics)?;
    if !catalog_changed {
        return Ok(false);
    }
    let application_catalog = auxiliary.application_snapshot();
    dock_model.rebuild(windows, application_catalog.clone());
    auxiliary.reconcile_switcher_windows(
        windows,
        application_catalog,
        dock_model.application_assignments(),
        graphics,
    )?;
    present_dock_change(dock, graphics, surface, auxiliary, dock_model)?;
    auxiliary.refresh_open_application_manager(dock_model.items());
    if catalog_changed {
        auxiliary.invalidate_launcher_surface();
    }
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
        let submission = match handle_search_event(
            event,
            dock,
            graphics,
            dock_model,
            auxiliary.launcher_runtime(),
        ) {
            Ok(submission) => submission,
            Err(error)
                if error.mark_graphics_lost(graphics)
                    || graphics.health() == GraphicsDeviceHealth::Lost =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        if let Some(submission) = submission {
            execute_search_submission(submission, dock, graphics, dock_model, auxiliary)?;
        }
    }
    Ok(had_events)
}

pub(crate) fn handle_search_event(
    event: SearchEvent,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    dock_model: &DockRuntime,
    launcher: &mut LauncherRuntime,
) -> Result<Option<LauncherSubmission>, AppError> {
    let mut scene_changed = false;
    let mut command = None;
    match event {
        SearchEvent::TextInput(character) => {
            launcher.controller.push_character(character);
            launcher.rebuild_scene(launcher.window.dpi())?;
            scene_changed = true;
        }
        SearchEvent::Edit(edit) => {
            if launcher.controller.edit_query(model_query_edit(edit)) {
                launcher.rebuild_scene(launcher.window.dpi())?;
                scene_changed = true;
            }
        }
        SearchEvent::PasteRequested => {
            if let Ok(text) = read_text()
                && launcher.controller.insert_text(&text)
            {
                launcher.rebuild_scene(launcher.window.dpi())?;
                scene_changed = true;
            }
        }
        SearchEvent::MoveSelection(direction) => {
            launcher.move_selection(direction)?;
            scene_changed = true;
        }
        SearchEvent::DismissRequested => launcher.hide(),
        SearchEvent::SubmitRequested => command = launcher.submit(dock.handle()),
        SearchEvent::Resized { width, height } => {
            if let (Some(size), Some(surface)) =
                (SurfaceSize::new(width, height), launcher.surface.as_mut())
            {
                resize_launcher_surface(graphics, surface.value_mut(), size)?;
                scene_changed = true;
            }
        }
        SearchEvent::DpiChanged { dpi } => {
            launcher.rebuild_scene(dpi)?;
            scene_changed = true;
        }
        SearchEvent::ClockRefreshRequested => {
            scene_changed = launcher.scene.as_mut().is_some_and(|scene| {
                scene.set_footer_time(local_time(dock_model.settings().use_24_hour_time))
            });
        }
        SearchEvent::FocusRefreshRequested => {
            let _ = launcher.window.focus();
        }
        SearchEvent::RenderRequested => scene_changed = true,
        SearchEvent::PointerMoved { x, y } => {
            let hovered = launcher.result_at(x, y);
            scene_changed = launcher.set_hovered_result(hovered);
        }
        SearchEvent::PointerLeft => scene_changed = launcher.set_hovered_result(None),
        SearchEvent::PointerReleased { x, y } => {
            if let Some(index) = launcher.result_at(x, y) {
                let _ = launcher.select_result(index)?;
                command = launcher.submit(dock.handle());
            }
        }
    }

    if scene_changed && launcher.is_visible() {
        launcher.sync_size(dock, graphics)?;
        if let Some(surface) = &mut launcher.surface {
            surface.invalidate();
        }
    }
    Ok(command)
}

const fn model_query_edit(edit: SearchEdit) -> QueryEdit {
    match edit {
        SearchEdit::DeleteBackward => QueryEdit::DeleteBackward,
        SearchEdit::DeletePreviousWord => QueryEdit::DeletePreviousWord,
        SearchEdit::DeleteForward => QueryEdit::DeleteForward,
        SearchEdit::MoveCursor(movement) => QueryEdit::MoveCursor(match movement {
            WindowCursorMove::Home => ModelCursorMove::Home,
            WindowCursorMove::End => ModelCursorMove::End,
            WindowCursorMove::Previous => ModelCursorMove::Previous,
            WindowCursorMove::Next => ModelCursorMove::Next,
        }),
        SearchEdit::SelectAll => QueryEdit::SelectAll,
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
