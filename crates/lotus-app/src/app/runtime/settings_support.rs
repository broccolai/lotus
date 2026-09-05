use lotus_windows::dialog::{confirm_reset_settings, show_error, show_information};
use lotus_windows::interaction::request_exit;
use lotus_windows::startup as startup_registration;

use super::settings_commit::restart_current_process;
use super::settings_events::SettingsEventContext;

pub(super) fn export_settings(context: &mut SettingsEventContext<'_>) {
    let owner = context.auxiliary.settings_owner();
    let destination = match lotus_windows::settings_file::choose_export_path(owner) {
        Ok(Some(destination)) => destination,
        Ok(None) => return,
        Err(error) => {
            lotus_windows::diagnostics::record_error(
                "settings.export_dialog_failed",
                &error,
            );
            show_error(
                owner,
                "Lotus Settings",
                &format!("Lotus could not open the settings export dialog.\n\n{error}"),
            );
            return;
        }
    };

    match context
        .settings_persistence
        .export(context.dock_model.settings(), &destination)
    {
        Ok(()) => {
            lotus_windows::diagnostics::record_diagnostic(
                "settings.exported",
                &format!("path={}", destination.display()),
            );
            show_information(
                owner,
                "Lotus Settings",
                &format!(
                    "Settings exported successfully.\n\n{}",
                    destination.display()
                ),
            );
        }
        Err(error) => {
            lotus_windows::diagnostics::record_error("settings.export_failed", &error);
            show_error(
                owner,
                "Lotus Settings",
                &format!(
                    "Lotus could not export settings to {}.\n\n{error}",
                    destination.display()
                ),
            );
        }
    }
}

pub(super) fn export_diagnostics(context: &mut SettingsEventContext<'_>) {
    let owner = context.auxiliary.settings_owner();
    let destination =
        match lotus_windows::settings_file::choose_diagnostics_export_path(owner) {
            Ok(Some(destination)) => destination,
            Ok(None) => return,
            Err(error) => {
                lotus_windows::diagnostics::record_error(
                    "diagnostics.export_dialog_failed",
                    &error,
                );
                show_error(
                    owner,
                    "Lotus diagnostics",
                    &format!(
                        "Lotus could not open the diagnostics export dialog.\n\n{error}"
                    ),
                );
                return;
            }
        };

    if let Err(error) = context
        .settings_persistence
        .validate_export_destination(&destination)
    {
        lotus_windows::diagnostics::record_error("diagnostics.export_rejected", &error);
        show_error(
            owner,
            "Lotus diagnostics",
            &format!("Lotus could not export diagnostics.\n\n{error}"),
        );
        return;
    }
    let integration = context
        .integration
        .diagnostic_snapshot(context.graphics, context.auxiliary);
    match lotus_windows::diagnostics::export_support_report(
        &destination,
        context.dock_model.settings(),
        &integration,
    ) {
        Ok(()) => lotus_windows::diagnostics::record_diagnostic(
            "diagnostics.exported",
            "support report exported",
        ),
        Err(error) => {
            lotus_windows::diagnostics::record_error("diagnostics.export_failed", &error);
            show_error(
                owner,
                "Lotus diagnostics",
                &format!("Lotus could not export diagnostics.\n\n{error}"),
            );
        }
    }
}

pub(super) fn reset_lotus(context: &mut SettingsEventContext<'_>) {
    let owner = context.auxiliary.settings_owner();
    if !confirm_reset_settings(owner) {
        return;
    }

    let reset = match context.settings_persistence.reset() {
        Ok(reset) => reset,
        Err(error) => {
            lotus_windows::diagnostics::record_error("settings.reset_failed", &error);
            show_error(
                owner,
                "Reset Lotus safely",
                &format!("Lotus could not reset your settings.\n\n{error}"),
            );
            return;
        }
    };
    lotus_windows::diagnostics::record_diagnostic(
        "settings.reset_persisted",
        &format!(
            "settings_path={} backup_path={}",
            context
                .settings_persistence
                .directory()
                .join("settings.json")
                .display(),
            reset.backup_path.display(),
        ),
    );
    if context.startup_registration_allowed
        && let Err(error) = startup_registration::sync(reset.settings.start_with_windows)
    {
        lotus_windows::diagnostics::record_error(
            "settings.reset_startup_sync_failed",
            &error,
        );
        show_error(
            owner,
            "Reset Lotus safely",
            &format!(
                "Lotus reset its settings, but could not update Windows startup. The reset will still take effect after restart.\n\nBackup: {}\n\n{error}",
                reset.backup_path.display(),
            ),
        );
    }
    match restart_current_process(context.startup_mode) {
        Ok(()) => request_exit(0),
        Err(error) => {
            lotus_windows::diagnostics::record_error(
                "settings.reset_restart_failed",
                &error,
            );
            show_error(
                owner,
                "Reset Lotus safely",
                &format!(
                    "Lotus reset its settings and kept a backup, but could not restart. The reset will take effect the next time Lotus starts.\n\nBackup: {}\n\n{error}",
                    reset.backup_path.display(),
                ),
            );
        }
    }
}
