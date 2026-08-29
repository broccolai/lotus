use lotus_core::settings::DockSettings;
use lotus_settings::scene::SettingsAction;
use lotus_ui::frame::ScheduledSurface;
use lotus_windows::activation::launch_target;
use lotus_windows::dialog::{confirm_reset_settings, show_error, show_information};
use lotus_windows::graphics::{CompositionSurfaceState, DeviceState, GraphicsDeviceHealth};
use lotus_windows::interaction::request_exit;
use lotus_windows::startup as startup_registration;
use lotus_windows::window::DockWindow;
use lotus_windows::window_tracker::WindowTracker;

use super::presentation::{apply_fullscreen_visibility, present_dock_change};
use super::update_events;
use crate::app::integration::{
    IntegrationRecovery, IntegrationRecoveryContext, IntegrationRecoverySource,
};
use crate::app::modules::{ModuleHost, SettingsIntent};
use crate::app::settings::{ApplicationIconOutcome, ColorOutcome, ColorTarget};
use crate::app::{AppError, DockRuntime, RestartError};

pub(super) struct SettingsEventContext<'a> {
    pub(super) dock: &'a DockWindow,
    pub(super) graphics: &'a mut DeviceState,
    pub(super) dock_surface: &'a mut ScheduledSurface<CompositionSurfaceState>,
    pub(super) window_tracker: &'a mut WindowTracker,
    pub(super) dock_model: &'a mut DockRuntime,
    pub(super) auxiliary: &'a mut ModuleHost,
    pub(super) integration: &'a mut IntegrationRecovery,
}

pub(super) fn drain_settings_events(
    context: &mut SettingsEventContext<'_>,
) -> Result<bool, AppError> {
    let events = context.auxiliary.drain_settings_events();
    let had_events = !events.is_empty();
    for event in events {
        let result = (|| {
            let intent = context.auxiliary.handle_settings_event(
                event,
                context.graphics,
                context.dock_model.items(),
            )?;
            match intent {
                SettingsIntent::None => Ok(()),
                SettingsIntent::PasteQuery => {
                    if let Ok(clipboard) = lotus_windows::clipboard::read_text() {
                        context
                            .auxiliary
                            .paste_settings_query(&clipboard, context.dock_model.items());
                    }
                    Ok(())
                }
                SettingsIntent::Action(action) => execute_settings_action(action, context),
            }
        })();
        match result {
            Ok(()) => {}
            Err(error)
                if error.mark_graphics_lost(context.graphics)
                    || context.graphics.health() == GraphicsDeviceHealth::Lost => {}
            Err(error) => return Err(error),
        }
    }
    Ok(had_events)
}

fn execute_settings_action(
    action: SettingsAction,
    context: &mut SettingsEventContext<'_>,
) -> Result<(), AppError> {
    match action {
        SettingsAction::None => Ok(()),
        SettingsAction::Changed => {
            if context.auxiliary.settings_on_apps_page() {
                if context.auxiliary.application_catalog_is_empty() {
                    refresh_application_manager(context);
                } else {
                    hydrate_application_previews(context);
                }
            }
            context.auxiliary.invalidate_settings();
            Ok(())
        }
        SettingsAction::Reverted | SettingsAction::OpenApplications => {
            refresh_application_manager(context);
            context.auxiliary.invalidate_settings();
            Ok(())
        }
        SettingsAction::ChooseBackgroundColor => {
            apply_color_outcome(ColorTarget::Background, context);
            Ok(())
        }
        SettingsAction::ChooseAccentColor => {
            apply_color_outcome(ColorTarget::Accent, context);
            Ok(())
        }
        SettingsAction::ChooseForegroundColor => {
            apply_color_outcome(ColorTarget::Foreground, context);
            Ok(())
        }
        SettingsAction::ChooseMascotImage => {
            let settings_directory = context.dock_model.settings_directory();
            context
                .auxiliary
                .choose_settings_mascot_image(settings_directory);
            context.auxiliary.invalidate_settings();
            Ok(())
        }
        SettingsAction::ChooseApplicationIcon(id) => {
            let settings_directory = context.dock_model.settings_directory();
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
        SettingsAction::CheckForUpdates => {
            update_events::start_update_check(context.auxiliary);
            Ok(())
        }
        SettingsAction::RestartIntegration => {
            context.integration.recover(
                IntegrationRecoverySource::Settings,
                &mut IntegrationRecoveryContext {
                    dock: context.dock,
                    graphics: context.graphics,
                    dock_surface: context.dock_surface,
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
        SettingsAction::Apply(next) => apply_changed_settings(*next, context, false),
        SettingsAction::CompleteOnboarding(next) => {
            let initial_setup = context.auxiliary.onboarding_required_for_close();
            context.auxiliary.end_onboarding();
            apply_changed_settings(*next, context, initial_setup)?;
            if !initial_setup {
                context.auxiliary.hide_settings();
            }
            Ok(())
        }
    }
}

fn export_settings(context: &mut SettingsEventContext<'_>) {
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

    match context.dock_model.export_settings(&destination) {
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

fn export_diagnostics(context: &mut SettingsEventContext<'_>) {
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

    if let Err(error) = context.dock_model.validate_export_destination(&destination) {
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

fn reset_lotus(context: &mut SettingsEventContext<'_>) {
    let owner = context.auxiliary.settings_owner();
    if !confirm_reset_settings(owner) {
        return;
    }

    let reset = match context.dock_model.reset_settings() {
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
                .dock_model
                .settings_directory()
                .join("settings.json")
                .display(),
            reset.backup_path.display(),
        ),
    );
    if let Err(error) = startup_registration::sync(reset.settings.start_with_windows) {
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
    match restart_current_process() {
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

fn apply_color_outcome(target: ColorTarget, context: &mut SettingsEventContext<'_>) {
    if let ColorOutcome::Changed = context.auxiliary.choose_settings_color(target) {
        context.auxiliary.invalidate_settings();
    }
}

fn apply_changed_settings(
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

fn restart_current_process() -> Result<(), RestartError> {
    let executable = std::env::current_exe()?;
    let arguments = format!("--restart-after {} --open-settings", std::process::id());
    launch_target(&executable.to_string_lossy(), Some(&arguments))?;
    Ok(())
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
