use std::path::Path;

use lotus_core::window::WindowInfo;
use lotus_ui::frame::ScheduledSurface;
use lotus_windows::WindowHandle;
use lotus_windows::activation::launch_target;
use lotus_windows::dialog::show_error;
use lotus_windows::graphics::{CompositionSurfaceState, DeviceState};
use lotus_windows::interaction::request_exit;
use lotus_windows::window::{ContextMenuEvent, DockWindow};

use super::presentation::present_dock_change;
use crate::app::modules::ModuleHost;
use crate::app::visuals::{AppMenuAction, ContextMenuAction, PopupAction, PowerAction};
use crate::app::{AppError, DockRuntime, activation};

pub(super) fn handle_context_menu_event(
    event: ContextMenuEvent,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    surface: &mut ScheduledSurface<CompositionSurfaceState>,
    windows: &[WindowInfo],
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    let refocus_launcher = matches!(event, ContextMenuEvent::DismissRequested)
        && auxiliary.context_menu_is_search_owned()
        && auxiliary.launcher_is_visible();
    if let Some(invocation) = auxiliary.handle_context_menu_event(event)? {
        let mut context = PopupActionContext {
            dock,
            graphics,
            surface,
            windows,
            dock_model,
            auxiliary,
        };
        execute_popup_action(invocation.action, &mut context)?;
    } else if refocus_launcher {
        auxiliary.focus_launcher_if_visible();
    }
    Ok(())
}

struct PopupActionContext<'a> {
    dock: &'a DockWindow,
    graphics: &'a mut DeviceState,
    surface: &'a mut ScheduledSurface<CompositionSurfaceState>,
    windows: &'a [WindowInfo],
    dock_model: &'a mut DockRuntime,
    auxiliary: &'a mut ModuleHost,
}

fn execute_popup_action(
    action: PopupAction,
    context: &mut PopupActionContext<'_>,
) -> Result<(), AppError> {
    match action {
        PopupAction::System(action) => {
            execute_context_menu_action(
                action,
                context.graphics,
                context.dock_model,
                context.auxiliary,
            )?;
        }
        PopupAction::Power(action) => execute_power_action(action, context.dock.handle()),
        PopupAction::App { action, identity } => {
            let Some(source_index) = context.dock_model.source_index(&identity) else {
                lotus_windows::diagnostics::record_diagnostic(
                    "activation.app_menu_source_absent",
                    "application context-menu source disappeared before its action ran",
                );
                return Ok(());
            };
            execute_app_menu_action(action, source_index, context)?;
        }
        PopupAction::Activate(key) => match activation::activate_exact(key) {
            Ok(outcome) => {
                if let Some(window) = outcome.focused_key()
                    && let Some(source_index) =
                        context.dock_model.source_index_for_key(window)
                {
                    context
                        .dock_model
                        .record_window_activation(source_index, window);
                }
                if matches!(outcome, activation::ActivationOutcome::ForegroundDenied) {
                    lotus_windows::diagnostics::record_diagnostic(
                        "activation.popup_foreground_denied",
                        "Windows denied a picker foreground request",
                    );
                }
            }
            Err(error) => {
                lotus_windows::diagnostics::record_error("activation.popup_window", &error);
                show_error(
                    context.dock.handle(),
                    "Lotus",
                    &format!("Lotus could not activate that window.\n\n{error}"),
                );
            }
        },
        PopupAction::CloseWindow(key) => {
            if let Err(error) = activation::request_close(key, false) {
                lotus_windows::diagnostics::record_error("activation.popup_close", &error);
                show_error(
                    context.dock.handle(),
                    "Lotus",
                    &format!("Lotus could not close that window.\n\n{error}"),
                );
            }
        }
        PopupAction::OpenFileLocation(path) => {
            match lotus_windows::activation::reveal_in_file_explorer(Path::new(&path)) {
                Ok(()) => context.auxiliary.hide_launcher(),
                Err(error) => {
                    lotus_windows::diagnostics::record_error(
                        "activation.open_file_location",
                        &error,
                    );
                    show_error(
                        context.dock.handle(),
                        "Lotus Search",
                        &format!(
                            "Lotus could not open that application's location.\n\n{error}"
                        ),
                    );
                    context.auxiliary.focus_launcher_if_visible();
                }
            }
        }
    }
    Ok(())
}

fn execute_power_action(action: PowerAction, owner: WindowHandle) {
    let result = match action {
        PowerAction::Lock => {
            lotus_windows::desktop::lock().map_err(|error| error.to_string())
        }
        PowerAction::Restart => launch_target("shutdown.exe", Some("/r /t 0"))
            .map_err(|error| error.to_string()),
        PowerAction::ShutDown => launch_target("shutdown.exe", Some("/s /t 0"))
            .map_err(|error| error.to_string()),
        PowerAction::Cancel => return,
    };
    if let Err(error) = result {
        show_error(
            owner,
            "Lotus",
            &format!("Lotus could not complete that power action.\n\n{error}"),
        );
    }
}

fn execute_app_menu_action(
    action: AppMenuAction,
    source_index: usize,
    context: &mut PopupActionContext<'_>,
) -> Result<(), AppError> {
    match action {
        AppMenuAction::Open => context
            .dock_model
            .open_new(source_index, context.dock.handle()),
        AppMenuAction::CustomizeIcon => {
            context.auxiliary.open_application_icon_manager(
                context.dock_model,
                source_index,
                context.graphics,
            )?;
        }
        AppMenuAction::TogglePin => {
            let pinned = context
                .dock_model
                .item(source_index)
                .is_some_and(|item| item.is_pinned);
            let registered = (!pinned)
                .then(|| context.dock_model.item(source_index))
                .flatten()
                .and_then(|item| context.dock_model.registered_application_for_item(item));
            let changed = match context.dock_model.set_pinned(
                source_index,
                !pinned,
                context.windows,
                registered,
            ) {
                Ok(changed) => changed,
                Err(error) => {
                    show_error(
                        context.dock.handle(),
                        "Lotus",
                        &format!("Lotus could not save that pin.\n\n{error}"),
                    );
                    false
                }
            };
            if changed {
                present_dock_change(
                    context.dock,
                    context.graphics,
                    context.surface,
                    context.auxiliary,
                    context.dock_model,
                )?;
            }
        }
        AppMenuAction::Close => {
            let window_keys = context
                .dock_model
                .item(source_index)
                .map(|item| item.windows.iter().map(WindowInfo::key).collect::<Vec<_>>())
                .unwrap_or_default();
            for key in window_keys {
                if let Err(error) = activation::request_close(key, false) {
                    lotus_windows::diagnostics::record_error(
                        "activation.app_menu_close",
                        &error,
                    );
                    show_error(
                        context.dock.handle(),
                        "Lotus",
                        &format!("Lotus could not close that window.\n\n{error}"),
                    );
                    break;
                }
            }
        }
        AppMenuAction::ForceClose => {
            let window_keys = context
                .dock_model
                .item(source_index)
                .map(|item| item.windows.iter().map(WindowInfo::key).collect::<Vec<_>>())
                .unwrap_or_default();
            for key in window_keys {
                if let Err(error) = activation::request_close(key, true) {
                    lotus_windows::diagnostics::record_error(
                        "activation.app_menu_force_close",
                        &error,
                    );
                    show_error(
                        context.dock.handle(),
                        "Lotus",
                        &format!("Lotus could not force close that window.\n\n{error}"),
                    );
                    break;
                }
            }
        }
    }
    Ok(())
}

fn execute_context_menu_action(
    action: ContextMenuAction,
    graphics: &mut DeviceState,
    dock_model: &DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    match action {
        ContextMenuAction::OpenSettings => auxiliary.open_settings(dock_model, graphics)?,
        ContextMenuAction::RequestShutdown => auxiliary.open_power_menu(graphics)?,
        ContextMenuAction::QuitLotus => request_exit(0),
    }
    Ok(())
}
