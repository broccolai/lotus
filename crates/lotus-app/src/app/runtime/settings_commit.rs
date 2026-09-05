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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SettingsApplyMode {
    Ordinary,
    OnboardingRestart,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SettingsApplyResult {
    persistence: SettingsPersistence,
    restart: RestartDisposition,
    applications: ApplicationManagerRefresh,
    presentation: PresentationRefresh,
    integration: IntegrationRefresh,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingsPersistence {
    Unchanged,
    Persisted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RestartDisposition {
    None,
    Ordinary,
    Onboarding,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ApplicationManagerRefresh {
    None,
    IfApplicationsPageIsOpen,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PresentationRefresh {
    SettingsMaterialOnly,
    FullRuntime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IntegrationRefresh {
    None,
    StartupSyncOnly { start_with_windows: bool },
    FullRuntime { start_with_windows: bool },
}

pub(super) fn apply_color_outcome(
    target: ColorTarget,
    context: &mut SettingsEventContext<'_>,
) {
    if let ColorOutcome::Changed = context.auxiliary.choose_settings_color(target) {
        context.auxiliary.invalidate_settings();
    }
}

pub(super) fn apply_changed_settings(
    next: DockSettings,
    context: &mut SettingsEventContext<'_>,
    mode: SettingsApplyMode,
) -> Result<(), AppError> {
    let result = commit_settings(next, context, mode)?;
    apply_settings_result(result, context)
}

fn commit_settings(
    mut next: DockSettings,
    context: &mut SettingsEventContext<'_>,
    mode: SettingsApplyMode,
) -> Result<SettingsApplyResult, AppError> {
    next.application_icon_overrides = context
        .auxiliary
        .merge_application_icon_overrides(context.dock_model.settings());
    let next = next.retaining_externally_managed(context.dock_model.settings());

    let start_with_windows = next.start_with_windows;
    let impact = context
        .dock_model
        .apply_settings(next, context.window_tracker.current_windows())?;

    let persistence = if impact.changed {
        SettingsPersistence::Persisted
    } else {
        SettingsPersistence::Unchanged
    };

    let result = match mode {
        SettingsApplyMode::OnboardingRestart => SettingsApplyResult {
            persistence,
            restart: RestartDisposition::Onboarding,
            applications: ApplicationManagerRefresh::None,
            presentation: PresentationRefresh::SettingsMaterialOnly,
            integration: IntegrationRefresh::None,
        },
        SettingsApplyMode::Ordinary if impact.changed => SettingsApplyResult {
            persistence,
            restart: if impact.restart_required {
                RestartDisposition::Ordinary
            } else {
                RestartDisposition::None
            },
            applications: ApplicationManagerRefresh::IfApplicationsPageIsOpen,
            presentation: PresentationRefresh::FullRuntime,
            integration: IntegrationRefresh::FullRuntime { start_with_windows },
        },
        SettingsApplyMode::Ordinary => SettingsApplyResult {
            persistence,
            restart: RestartDisposition::None,
            applications: ApplicationManagerRefresh::None,
            presentation: PresentationRefresh::SettingsMaterialOnly,
            integration: IntegrationRefresh::StartupSyncOnly { start_with_windows },
        },
    };

    Ok(result)
}

fn apply_settings_result(
    result: SettingsApplyResult,
    context: &mut SettingsEventContext<'_>,
) -> Result<(), AppError> {
    apply_settings_material(result.presentation, context);

    if result.restart == RestartDisposition::Onboarding {
        return restart_after_onboarding(context);
    }

    synchronize_startup(result.integration, context);
    if result.persistence == SettingsPersistence::Unchanged {
        return Ok(());
    }

    let runtime_result = apply_runtime_integration(context, result.integration)
        .and_then(|()| apply_runtime_presentation(result.presentation, context));
    finish_runtime_refresh(result.applications, context);
    if let Err(error) = runtime_result {
        lotus_windows::diagnostics::record_error("settings.runtime_refresh_failed", &error);
        show_error(
            context.auxiliary.settings_owner(),
            "Lotus Settings",
            &format!(
                "Lotus saved your settings but could not fully apply them. Restart Lotus to finish applying the change.\n\n{error}"
            ),
        );
        return Ok(());
    }
    restart_if_required(result.restart, context);
    Ok(())
}

fn restart_after_onboarding(
    context: &mut SettingsEventContext<'_>,
) -> Result<(), AppError> {
    context
        .auxiliary
        .mark_settings_applied(context.dock_model.settings().clone());
    context.auxiliary.hide_settings();
    if let Err(error) = restart_current_process(context.startup_mode) {
        context.auxiliary.open_settings_without_refresh(
            context.dock_model.settings(),
            context.graphics,
        )?;
        show_restart_error(context, &error);
    } else {
        request_exit(0);
    }
    Ok(())
}

fn apply_settings_material(
    presentation: PresentationRefresh,
    context: &mut SettingsEventContext<'_>,
) {
    match presentation {
        PresentationRefresh::SettingsMaterialOnly | PresentationRefresh::FullRuntime => {
            context
                .auxiliary
                .apply_material_to_settings_window(context.dock_model.settings());
        }
    }
}

fn synchronize_startup(
    integration: IntegrationRefresh,
    context: &SettingsEventContext<'_>,
) {
    let start_with_windows = match integration {
        IntegrationRefresh::None => return,
        IntegrationRefresh::StartupSyncOnly { start_with_windows }
        | IntegrationRefresh::FullRuntime { start_with_windows } => start_with_windows,
    };

    if context.startup_registration_allowed
        && let Err(error) = startup_registration::sync(start_with_windows)
    {
        show_error(
            context.auxiliary.settings_owner(),
            "Lotus Settings",
            &format!(
                "Lotus saved your preference but could not update Windows startup.\n\n{error}"
            ),
        );
    }
}

fn apply_runtime_integration(
    context: &mut SettingsEventContext<'_>,
    integration: IntegrationRefresh,
) -> Result<(), AppError> {
    let IntegrationRefresh::FullRuntime { .. } = integration else {
        return Ok(());
    };
    context.auxiliary.reconcile(
        context.dock,
        context.dock_model.settings(),
        true,
        context.startup_mode.allows_shell_integration(),
    )?;
    let _changed = context.auxiliary.refresh_media(context.dock_model);

    Ok(())
}

fn apply_runtime_presentation(
    presentation: PresentationRefresh,
    context: &mut SettingsEventContext<'_>,
) -> Result<(), AppError> {
    if presentation != PresentationRefresh::FullRuntime {
        return Ok(());
    }

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
    if context.startup_mode.allows_shell_integration() {
        apply_fullscreen_visibility(
            context.dock,
            context.dock_surface,
            context.window_tracker,
            context.dock_model,
            context.auxiliary,
        )?;
    }

    Ok(())
}

fn finish_runtime_refresh(
    application_manager: ApplicationManagerRefresh,
    context: &mut SettingsEventContext<'_>,
) {
    context
        .auxiliary
        .mark_settings_applied(context.dock_model.settings().clone());
    context.auxiliary.clear_icon_caches();
    if application_manager == ApplicationManagerRefresh::IfApplicationsPageIsOpen
        && context.auxiliary.settings_on_apps_page()
    {
        refresh_application_manager(context);
    }
    context.auxiliary.invalidate_settings();
}

fn restart_if_required(restart: RestartDisposition, context: &SettingsEventContext<'_>) {
    if restart == RestartDisposition::Ordinary {
        if let Err(error) = restart_current_process(context.startup_mode) {
            show_restart_error(context, &error);
        } else {
            request_exit(0);
        }
    }
}

fn show_restart_error(context: &SettingsEventContext<'_>, error: &RestartError) {
    show_error(
        context.auxiliary.settings_owner(),
        "Lotus Settings",
        &format!("Lotus saved your settings but could not restart.\n\n{error}"),
    );
}

pub(super) fn restart_current_process(
    mode: lotus_windows::startup::StartupMode,
) -> Result<(), RestartError> {
    let executable = std::env::current_exe()?;
    let mut arguments = format!("--restart-after {} --open-settings", std::process::id());
    arguments.push(' ');
    arguments.push_str(mode.restart_argument());
    launch_target(&executable.to_string_lossy(), Some(&arguments))?;
    Ok(())
}
