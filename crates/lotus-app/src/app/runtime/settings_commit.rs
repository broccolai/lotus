use lotus_core::settings::DockSettings;
use lotus_dock::model::SettingsImpact;
use lotus_windows::activation::launch_target;
use lotus_windows::dialog::show_error;
use lotus_windows::graphics::DeviceState;
use lotus_windows::interaction::request_exit;
use lotus_windows::startup::StartupMode;
use lotus_windows::{WindowHandle, startup as startup_registration};

use super::presentation::{apply_fullscreen_visibility, present_dock_change};
use super::settings_events::SettingsEventContext;
use crate::app::modules::ModuleHost;
use crate::app::settings::{ColorOutcome, ColorTarget};
use crate::app::{AppError, DockRuntime, RestartError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SettingsApplyMode {
    Ordinary,
    OnboardingRestart,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SettingsEffects {
    change: SettingsChangeKind,
    restart: RestartDisposition,
    applications: ApplicationManagerRefresh,
    presentation: PresentationRefresh,
    integration: IntegrationRefresh,
}

struct SettingsChange {
    settings: DockSettings,
    effects: SettingsEffects,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingsChangeKind {
    Unchanged,
    Changed,
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

pub(super) fn apply_color_outcome(target: ColorTarget, auxiliary: &mut ModuleHost) {
    if let ColorOutcome::Changed = auxiliary.choose_settings_color(target) {
        auxiliary.invalidate_settings();
    }
}

pub(super) fn apply_changed_settings(
    mut next: DockSettings,
    context: &mut SettingsEventContext<'_>,
    mode: SettingsApplyMode,
) -> Result<(), AppError> {
    next.application_icon_overrides = context
        .auxiliary
        .merge_application_icon_overrides(context.dock_model.settings());
    let change = prepare_settings_change(next, context.dock_model.settings(), mode);

    context.dock_model.apply_settings(
        change.settings,
        context.window_tracker.current_windows(),
        context.settings_persistence,
    )?;

    apply_committed_settings(change.effects, context)
}

fn prepare_settings_change(
    next: DockSettings,
    current: &DockSettings,
    mode: SettingsApplyMode,
) -> SettingsChange {
    let next = next.retaining_externally_managed(current).normalized();
    let impact = SettingsImpact::between(current, &next);
    let start_with_windows = next.start_with_windows;

    let change = if impact.changed {
        SettingsChangeKind::Changed
    } else {
        SettingsChangeKind::Unchanged
    };

    let effects = match mode {
        SettingsApplyMode::OnboardingRestart => SettingsEffects {
            change,
            restart: RestartDisposition::Onboarding,
            applications: ApplicationManagerRefresh::None,
            presentation: PresentationRefresh::SettingsMaterialOnly,
            integration: IntegrationRefresh::None,
        },
        SettingsApplyMode::Ordinary if impact.changed => SettingsEffects {
            change,
            restart: if impact.restart_required {
                RestartDisposition::Ordinary
            } else {
                RestartDisposition::None
            },
            applications: ApplicationManagerRefresh::IfApplicationsPageIsOpen,
            presentation: PresentationRefresh::FullRuntime,
            integration: IntegrationRefresh::FullRuntime { start_with_windows },
        },
        SettingsApplyMode::Ordinary => SettingsEffects {
            change,
            restart: RestartDisposition::None,
            applications: ApplicationManagerRefresh::None,
            presentation: PresentationRefresh::SettingsMaterialOnly,
            integration: IntegrationRefresh::StartupSyncOnly { start_with_windows },
        },
    };

    SettingsChange {
        settings: next,
        effects,
    }
}

fn apply_committed_settings(
    result: SettingsEffects,
    context: &mut SettingsEventContext<'_>,
) -> Result<(), AppError> {
    apply_settings_material(result.presentation, context);

    if result.restart == RestartDisposition::Onboarding {
        return restart_after_onboarding(
            context.auxiliary,
            context.dock_model.settings(),
            context.graphics,
            context.startup_mode,
        );
    }

    synchronize_startup(
        result.integration,
        context.auxiliary.settings_owner(),
        context.startup_registration_allowed,
    );
    if result.change == SettingsChangeKind::Unchanged {
        return Ok(());
    }

    let runtime_result = apply_runtime_integration(context, result.integration)
        .and_then(|()| apply_runtime_presentation(result.presentation, context));
    finish_runtime_refresh(result.applications, context.auxiliary, context.dock_model);
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
    restart_if_required(
        result.restart,
        context.auxiliary.settings_owner(),
        context.startup_mode,
    );
    Ok(())
}

fn restart_after_onboarding(
    auxiliary: &mut ModuleHost,
    settings: &DockSettings,
    graphics: &mut DeviceState,
    startup_mode: StartupMode,
) -> Result<(), AppError> {
    auxiliary.mark_settings_applied(settings.clone());
    auxiliary.hide_settings();
    if let Err(error) = restart_current_process(startup_mode) {
        auxiliary.open_settings_without_refresh(settings, graphics)?;
        show_restart_error(auxiliary.settings_owner(), &error);
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
    owner: WindowHandle,
    startup_registration_allowed: bool,
) {
    let start_with_windows = match integration {
        IntegrationRefresh::None => return,
        IntegrationRefresh::StartupSyncOnly { start_with_windows }
        | IntegrationRefresh::FullRuntime { start_with_windows } => start_with_windows,
    };

    if startup_registration_allowed
        && let Err(error) = startup_registration::sync(start_with_windows)
    {
        show_error(
            owner,
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
        context.primary_dock.window(),
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
        context.primary_dock.window().handle(),
        context.dock_model.settings(),
    );
    context.auxiliary.propagate_settings(
        context.dock_model.settings(),
        context.primary_dock.window(),
        context.graphics,
    )?;
    present_dock_change(
        context.primary_dock,
        context.graphics,
        context.auxiliary,
        context.dock_model,
    )?;
    if context.startup_mode.allows_shell_integration() {
        apply_fullscreen_visibility(
            context.primary_dock,
            context.window_tracker,
            context.dock_model,
            context.auxiliary,
        )?;
    }

    Ok(())
}

fn finish_runtime_refresh(
    application_manager: ApplicationManagerRefresh,
    auxiliary: &mut ModuleHost,
    dock_model: &DockRuntime,
) {
    auxiliary.mark_settings_applied(dock_model.settings().clone());
    auxiliary.clear_icon_caches();
    if application_manager == ApplicationManagerRefresh::IfApplicationsPageIsOpen
        && auxiliary.settings_on_apps_page()
    {
        auxiliary.refresh_application_manager(dock_model.items());
    }
    auxiliary.invalidate_settings();
}

fn restart_if_required(
    restart: RestartDisposition,
    owner: WindowHandle,
    startup_mode: StartupMode,
) {
    if restart == RestartDisposition::Ordinary {
        if let Err(error) = restart_current_process(startup_mode) {
            show_restart_error(owner, &error);
        } else {
            request_exit(0);
        }
    }
}

fn show_restart_error(owner: WindowHandle, error: &RestartError) {
    show_error(
        owner,
        "Lotus Settings",
        &format!("Lotus saved your settings but could not restart.\n\n{error}"),
    );
}

pub(super) fn restart_current_process(mode: StartupMode) -> Result<(), RestartError> {
    let executable = std::env::current_exe()?;
    let mut arguments = format!("--restart-after {} --open-settings", std::process::id());
    arguments.push(' ');
    arguments.push_str(mode.restart_argument());
    launch_target(&executable.to_string_lossy(), Some(&arguments))?;
    Ok(())
}
