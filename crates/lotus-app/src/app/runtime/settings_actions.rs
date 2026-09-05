use lotus_settings::scene::SettingsAction;

use super::settings_commit::{
    SettingsApplyMode, apply_changed_settings, apply_color_outcome,
};
use super::settings_events::SettingsEventContext;
use super::settings_support::{export_diagnostics, export_settings, reset_lotus};
use super::update_events;
use crate::app::AppError;
use crate::app::integration::{IntegrationRecoveryContext, IntegrationRecoverySource};
use crate::app::modules::ModuleHost;
use crate::app::settings::{ApplicationIconOutcome, ColorTarget};

pub(super) fn execute_settings_action(
    action: SettingsAction,
    context: &mut SettingsEventContext<'_>,
) -> Result<(), AppError> {
    match action {
        SettingsAction::None => Ok(()),
        SettingsAction::Changed => {
            handle_changed_settings(context);
            context.auxiliary.invalidate_settings();
            Ok(())
        }
        SettingsAction::RefreshPresentation => {
            context.auxiliary.invalidate_settings();
            Ok(())
        }
        SettingsAction::Reverted | SettingsAction::OpenApplications => {
            refresh_application_manager(context);
            context.auxiliary.invalidate_settings();
            Ok(())
        }
        SettingsAction::ChooseBackgroundColor => {
            apply_color_outcome(ColorTarget::Background, context.auxiliary);
            Ok(())
        }
        SettingsAction::ChooseAccentColor => {
            apply_color_outcome(ColorTarget::Accent, context.auxiliary);
            Ok(())
        }
        SettingsAction::ChooseForegroundColor => {
            apply_color_outcome(ColorTarget::Foreground, context.auxiliary);
            Ok(())
        }
        SettingsAction::ChooseMascotImage => {
            let settings_directory = context.settings_persistence.directory();
            context
                .auxiliary
                .choose_settings_mascot_image(settings_directory);
            context.auxiliary.invalidate_settings();
            Ok(())
        }
        SettingsAction::ChooseApplicationIcon(id) => {
            let settings_directory = context.settings_persistence.directory();
            let outcome = context
                .auxiliary
                .choose_settings_application_icon(&id, settings_directory);
            if let ApplicationIconOutcome::Updated = outcome {
                context.auxiliary.clear_icon_caches();
                refresh_application_manager(context);
                context.auxiliary.invalidate_settings();
            }
            Ok(())
        }
        SettingsAction::ResetApplicationIcon(id) => {
            context.auxiliary.reset_application_icon_override(&id);
            context.auxiliary.clear_icon_caches();
            refresh_application_manager(context);
            context.auxiliary.invalidate_settings();
            Ok(())
        }
        action @ (SettingsAction::CheckForUpdates
        | SettingsAction::CancelUpdate
        | SettingsAction::AcceptUpdate) => {
            execute_update_action(&action, context.auxiliary, context.startup_mode);
            Ok(())
        }
        SettingsAction::RestartIntegration => {
            context.integration.recover(
                IntegrationRecoverySource::Settings,
                &mut IntegrationRecoveryContext {
                    primary_dock: context.primary_dock,
                    graphics: context.graphics,
                    window_tracker: context.window_tracker,
                    dock_model: context.dock_model,
                    auxiliary: context.auxiliary,
                },
            );
            Ok(())
        }
        SettingsAction::ReplaySetup => context.auxiliary.open_onboarding(
            context.dock_model.settings(),
            false,
            context.graphics,
        ),
        SettingsAction::ExportSettings => {
            export_settings(context);
            Ok(())
        }
        SettingsAction::ExportDiagnostics => {
            export_diagnostics(context);
            Ok(())
        }
        SettingsAction::ResetLotus => {
            reset_lotus(context);
            Ok(())
        }
        SettingsAction::Close => {
            if !context.auxiliary.onboarding_required_for_close() {
                context.auxiliary.hide_settings();
            }
            Ok(())
        }
        SettingsAction::Apply(next) => {
            apply_changed_settings(*next, context, SettingsApplyMode::Ordinary)
        }
        SettingsAction::CompleteOnboarding(next) => {
            let initial_setup = context.auxiliary.onboarding_required_for_close();
            context.auxiliary.end_onboarding();
            let mode = if initial_setup {
                SettingsApplyMode::OnboardingRestart
            } else {
                SettingsApplyMode::Ordinary
            };
            apply_changed_settings(*next, context, mode)?;
            if !initial_setup {
                context.auxiliary.hide_settings();
            }
            Ok(())
        }
    }
}

fn execute_update_action(
    action: &SettingsAction,
    auxiliary: &mut ModuleHost,
    mode: lotus_windows::startup::StartupMode,
) {
    match action {
        SettingsAction::CheckForUpdates => {
            update_events::start_update_check(auxiliary, mode);
        }
        SettingsAction::CancelUpdate => update_events::cancel_update(auxiliary),
        SettingsAction::AcceptUpdate => update_events::accept_update(auxiliary, mode),
        _ => {}
    }
}

fn handle_changed_settings(context: &mut SettingsEventContext<'_>) {
    if !context.auxiliary.settings_on_apps_page() {
        return;
    }
    if context.auxiliary.application_catalog_is_empty() {
        refresh_application_manager(context);
    } else {
        hydrate_application_previews(context);
    }
}

pub(super) fn refresh_application_manager(context: &mut SettingsEventContext<'_>) {
    context
        .auxiliary
        .refresh_application_manager(context.dock_model.items());
}

fn hydrate_application_previews(context: &mut SettingsEventContext<'_>) {
    context
        .auxiliary
        .hydrate_application_previews(context.dock_model.items());
}
