use lotus_core::settings::DockSettings;
use lotus_windows::activation::launch_target;
use lotus_windows::dialog::show_error;
use lotus_windows::interaction::request_exit;
use lotus_windows::startup as startup_registration;

use super::presentation::{apply_fullscreen_visibility, present_dock_change};
use super::settings_actions::refresh_application_manager;
use super::settings_events::SettingsEventContext;
use crate::app::settings::{ColorOutcome, ColorTarget};
use crate::app::{AppError, RestartError};

pub(super) fn apply_color_outcome(
    target: ColorTarget,
    context: &mut SettingsEventContext<'_>,
) {
    if let ColorOutcome::Changed = context.auxiliary.choose_settings_color(target) {
        context.auxiliary.invalidate_settings();
    }
}

pub(super) fn apply_changed_settings(
    mut next: DockSettings,
    context: &mut SettingsEventContext<'_>,
    restart_after_apply: bool,
) -> Result<(), AppError> {
    next.application_icon_overrides = context
        .auxiliary
        .merge_application_icon_overrides(context.dock_model.settings());
    let next = next.retaining_externally_managed(context.dock_model.settings());

    let start_with_windows = next.start_with_windows;
    let impact = context
        .dock_model
        .apply_settings(next, context.window_tracker.current_windows())?;
    context
        .auxiliary
        .apply_material_to_settings_window(context.dock_model.settings());
    if restart_after_apply {
        context
            .auxiliary
            .mark_settings_applied(context.dock_model.settings().clone());
        context.auxiliary.hide_settings();
        if let Err(error) = restart_current_process() {
            context.auxiliary.open_settings_without_refresh(
                context.dock_model.settings(),
                context.graphics,
            )?;
            show_error(
                context.auxiliary.settings_owner(),
                "Lotus Settings",
                &format!("Lotus saved your settings but could not restart.\n\n{error}"),
            );
        } else {
            request_exit(0);
        }
        return Ok(());
    }
    if let Err(error) = startup_registration::sync(start_with_windows) {
        show_error(
            context.auxiliary.settings_owner(),
            "Lotus Settings",
            &format!(
                "Lotus saved your preference but could not update Windows startup.\n\n{error}"
            ),
        );
    }
    if !impact.changed {
        return Ok(());
    }

    context
        .auxiliary
        .reconcile(context.dock, context.dock_model.settings(), true)?;
    let _changed = context.auxiliary.refresh_media(context.dock_model);

    lotus_windows::backdrop::apply_dock_settings(
        context.dock.handle(),
        context.dock_model.settings(),
    );
    context.auxiliary.propagate_settings(
        context.dock_model.settings(),
        context.dock,
        context.graphics,
    )?;
    present_dock_change(
        context.dock,
        context.graphics,
        context.dock_surface,
        context.auxiliary,
        context.dock_model,
    )?;
    apply_fullscreen_visibility(
        context.dock,
        context.dock_surface,
        context.window_tracker,
        context.dock_model,
        context.auxiliary,
    )?;
    context
        .auxiliary
        .mark_settings_applied(context.dock_model.settings().clone());
    context.auxiliary.clear_icon_caches();
    if context.auxiliary.settings_on_apps_page() {
        refresh_application_manager(context);
    }
    context.auxiliary.invalidate_settings();

    if impact.restart_required {
        if let Err(error) = restart_current_process() {
            show_error(
                context.auxiliary.settings_owner(),
                "Lotus Settings",
                &format!("Lotus saved your settings but could not restart.\n\n{error}"),
            );
        } else {
            request_exit(0);
        }
    }
    Ok(())
}

pub(super) fn restart_current_process() -> Result<(), RestartError> {
    let executable = std::env::current_exe()?;
    let arguments = format!("--restart-after {} --open-settings", std::process::id());
    launch_target(&executable.to_string_lossy(), Some(&arguments))?;
    Ok(())
}
