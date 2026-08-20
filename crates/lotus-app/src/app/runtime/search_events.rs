use lotus_core::launcher_model::{CursorMove as ModelCursorMove, QueryEdit};
use lotus_core::window::WindowInfo;
use lotus_search::command::CommandId;
use lotus_ui::frame::ScheduledSurface;
use lotus_windows::activation::launch_target;
use lotus_windows::clipboard::{read_text, write_text};
use lotus_windows::clock::local_time;
use lotus_windows::dialog::{confirm_restart, confirm_shutdown, show_error};
use lotus_windows::graphics::{CompositionSurfaceState, DeviceState, SurfaceSize};
use lotus_windows::interaction::request_exit;
use lotus_windows::window::{
    CursorMove as WindowCursorMove, DockWindow, SearchEdit, SearchEvent,
};

use super::presentation::{resize_dock, resize_launcher_surface};
use crate::app::launcher::{LauncherRuntime, LauncherSubmission};
use crate::app::switcher::AuxiliaryWindows;
use crate::app::{AppError, DockRuntime};

pub(super) fn refresh_catalog(
    dock: &DockWindow,
    graphics: &mut DeviceState,
    surface: &mut ScheduledSurface<CompositionSurfaceState>,
    windows: &[WindowInfo],
    dock_model: &mut DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    let catalog_changed = auxiliary.launcher.refresh_catalog_if_ready(
        dock,
        dock_model,
        &auxiliary.applications,
        graphics,
    )?;
    let pins_upgraded = dock_model.upgrade_legacy_pins(&auxiliary.applications)?;
    let pins_reconciled =
        dock_model.reconcile_unpinned_pins(windows, &auxiliary.applications)?;
    let pins_changed = pins_upgraded || pins_reconciled;
    if !catalog_changed && !pins_changed {
        return Ok(());
    }
    if pins_changed {
        auxiliary
            .settings
            .scene
            .reconcile_application_icon_overrides(dock_model.settings());
        auxiliary
            .launcher
            .apply_settings(dock_model.settings(), dock, graphics)?;
        auxiliary.context_menu.apply_settings(dock_model.settings());
        auxiliary.switcher.apply_settings(dock_model.settings());
        let _changed = auxiliary.media.refresh(dock_model);
        dock_model.rebuild(windows);
        resize_dock(dock, graphics, surface, dock_model)?;
        surface.invalidate();
        auxiliary
            .status
            .sync(dock, dock_model.settings(), dock_model.media(), graphics)?;
    }
    refresh_open_application_manager(dock_model, auxiliary);
    if catalog_changed && let Some(surface) = &mut auxiliary.launcher.surface {
        surface.invalidate();
    }
    Ok(())
}

pub(super) fn refresh_open_application_manager(
    dock_model: &DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) {
    if !auxiliary.settings.visible
        || auxiliary.settings.scene.page() != lotus_settings::scene::SettingsPage::Apps
    {
        return;
    }
    let selected = auxiliary
        .settings
        .scene
        .selected_application()
        .map(|application| application.id.clone());
    let settings_draft = auxiliary.settings.scene.draft().clone();
    let applications = super::settings_events::application_records(
        &auxiliary.applications,
        dock_model.items(),
        &settings_draft,
    );
    let _ = auxiliary.settings.scene.set_applications(applications);
    if let Some(selected) = selected {
        let _ = auxiliary.settings.scene.open_application_manager(&selected);
    }
    super::settings_events::hydrate_application_previews(
        &auxiliary.applications,
        dock_model.items(),
        &mut auxiliary.settings,
    );
    auxiliary.settings.invalidate();
}

pub(super) fn drain_search_events(
    dock: &DockWindow,
    graphics: &mut DeviceState,
    dock_model: &DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    for event in auxiliary.launcher.drain_events() {
        if let Some(submission) =
            handle_search_event(event, dock, graphics, dock_model, &mut auxiliary.launcher)?
        {
            execute_search_submission(submission, dock, graphics, dock_model, auxiliary)?;
        }
    }
    Ok(())
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
    auxiliary: &mut AuxiliaryWindows,
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
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    match command {
        CommandId::OpenSettings => {
            auxiliary.settings.open(dock_model.settings(), graphics)?;
            refresh_open_application_manager(dock_model, auxiliary);
        }
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
