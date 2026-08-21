use lotus_core::window::WindowInfo;
use lotus_settings::scene::SettingsApplicationRecord;
use lotus_ui::frame::ScheduledSurface;
use lotus_windows::WindowHandle;
use lotus_windows::activation::{
    ActivationError, force_window_close, launch_target, request_window_close,
};
use lotus_windows::dialog::show_error;
use lotus_windows::graphics::{
    AppMenuAction, CompositionSurfaceState, ContextMenuAction, DeviceState, PopupAction,
    PowerAction,
};
use lotus_windows::interaction::request_exit;
use lotus_windows::window::{ContextMenuEvent, DockWindow, SelectionDirection};

use super::presentation::resize_dock;
use crate::app::switcher::AuxiliaryWindows;
use crate::app::{AppError, DockRuntime};

pub(super) fn handle_context_menu_event(
    event: ContextMenuEvent,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    surface: &mut ScheduledSurface<CompositionSurfaceState>,
    windows: &[WindowInfo],
    dock_model: &mut DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    match event {
        ContextMenuEvent::PointerMoved { x, y } => {
            if auxiliary.context_menu.scene.pointer_move(x, y) {
                auxiliary.context_menu.invalidate();
            }
        }
        ContextMenuEvent::PointerLeft => {
            if auxiliary.context_menu.scene.pointer_left() {
                auxiliary.context_menu.invalidate();
            }
        }
        ContextMenuEvent::PointerReleased { x, y } => {
            let action = auxiliary.context_menu.scene.pointer_action(x, y);
            let source_index = auxiliary.context_menu.scene.source_index();
            if !action.is_some_and(opens_power_menu) {
                auxiliary.context_menu.hide();
            }
            if let Some(action) = action {
                let mut context = PopupActionContext {
                    dock,
                    graphics,
                    surface,
                    windows,
                    dock_model,
                    auxiliary,
                };
                execute_popup_action(action, source_index, &mut context)?;
            }
        }
        ContextMenuEvent::SelectionRequested => {
            let action = auxiliary.context_menu.scene.selected_action();
            let source_index = auxiliary.context_menu.scene.source_index();
            if !action.is_some_and(opens_power_menu) {
                auxiliary.context_menu.hide();
            }
            if let Some(action) = action {
                let mut context = PopupActionContext {
                    dock,
                    graphics,
                    surface,
                    windows,
                    dock_model,
                    auxiliary,
                };
                execute_popup_action(action, source_index, &mut context)?;
            }
        }
        ContextMenuEvent::MoveSelection(direction) => {
            let next = direction == SelectionDirection::Next;
            if auxiliary.context_menu.scene.move_selection(next) {
                auxiliary.context_menu.invalidate();
            }
        }
        ContextMenuEvent::Scroll(direction) => {
            let next = direction == SelectionDirection::Next;
            if auxiliary.context_menu.scene.scroll(next) {
                auxiliary.context_menu.invalidate();
            }
        }
        ContextMenuEvent::DismissRequested => auxiliary.context_menu.hide(),
        ContextMenuEvent::Resized { width, height } => {
            auxiliary.context_menu.resize(width, height)?;
            auxiliary.context_menu.invalidate();
        }
        ContextMenuEvent::DpiChanged { dpi } => {
            if auxiliary.context_menu.scene.set_dpi(dpi) {
                let desired = auxiliary.context_menu.scene.desired_size();
                if let Some(surface) = &mut auxiliary.context_menu.surface {
                    surface.value_mut().resize(desired)?;
                }
            }
            auxiliary.context_menu.invalidate();
        }
        ContextMenuEvent::RenderRequested => auxiliary.context_menu.invalidate(),
    }
    Ok(())
}

struct PopupActionContext<'a> {
    dock: &'a DockWindow,
    graphics: &'a mut DeviceState,
    surface: &'a mut ScheduledSurface<CompositionSurfaceState>,
    windows: &'a [WindowInfo],
    dock_model: &'a mut DockRuntime,
    auxiliary: &'a mut AuxiliaryWindows,
}

fn execute_popup_action(
    action: PopupAction,
    source_index: Option<usize>,
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
        PopupAction::App(action) => {
            let Some(source_index) = source_index else {
                return Ok(());
            };
            execute_app_menu_action(action, source_index, context)?;
        }
        PopupAction::Activate(window) => {
            if let Some(source_index) = source_index {
                context
                    .dock_model
                    .record_window_activation(source_index, window);
            }
            if let Err(error) = lotus_windows::activation::switch_window(window) {
                show_error(
                    context.dock.handle(),
                    "Lotus",
                    &format!("Lotus could not activate that window.\n\n{error}"),
                );
            }
        }
        PopupAction::CloseWindow(window) => {
            if let Err(error) = request_window_close(window) {
                show_error(
                    context.dock.handle(),
                    "Lotus",
                    &format!("Lotus could not close that window.\n\n{error}"),
                );
            }
        }
    }
    Ok(())
}

const fn opens_power_menu(action: PopupAction) -> bool {
    matches!(
        action,
        PopupAction::System(ContextMenuAction::RequestShutdown)
    )
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
            open_application_icon_manager(source_index, context)?;
        }
        AppMenuAction::TogglePin => {
            let pinned = context
                .dock_model
                .item(source_index)
                .is_some_and(|item| item.is_pinned);
            let registered = (!pinned)
                .then(|| context.dock_model.item(source_index))
                .flatten()
                .and_then(|item| {
                    item.windows.first().and_then(|window| {
                        context
                            .auxiliary
                            .applications
                            .registered_application(window, &item.display_name)
                    })
                });
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
                resize_dock(
                    context.dock,
                    context.graphics,
                    context.surface,
                    context.dock_model,
                )?;
                context.auxiliary.status.sync(
                    context.dock,
                    context.dock_model.settings(),
                    context.dock_model.media(),
                    context.graphics,
                )?;
                context.surface.invalidate();
            }
        }
        AppMenuAction::Close => {
            let window_ids = context
                .dock_model
                .item(source_index)
                .map(|item| {
                    item.windows
                        .iter()
                        .map(|window| window.id)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for window in window_ids {
                if let Err(error) = request_window_close(window) {
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
            let window_ids = context
                .dock_model
                .item(source_index)
                .map(|item| {
                    item.windows
                        .iter()
                        .map(|window| window.id)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for window in window_ids {
                match force_window_close(window) {
                    Ok(()) | Err(ActivationError::MissingWindow(_)) => {}
                    Err(error) => {
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
    }
    Ok(())
}

fn open_application_icon_manager(
    source_index: usize,
    context: &mut PopupActionContext<'_>,
) -> Result<(), AppError> {
    let icon = context.dock_model.application_icon_preview(source_index);
    let Some(item) = context.dock_model.item(source_index) else {
        return Ok(());
    };
    let custom = context
        .dock_model
        .settings()
        .application_icon_override_for(&item.application_identity());
    let id = custom.map_or_else(|| item.id.clone(), |override_| override_.id.clone());
    let record = SettingsApplicationRecord {
        id: id.clone(),
        name: item.display_name.clone(),
        icon,
        app_user_model_id: item.app_user_model_id.clone(),
        match_executables: std::path::Path::new(&item.executable_path)
            .file_name()
            .and_then(|name| name.to_str().map(str::to_owned))
            .into_iter()
            .collect(),
        customized: custom.is_some(),
        missing_icon: false,
    };
    let mut applications = super::settings_events::application_records(
        &context.auxiliary.applications,
        context.dock_model.items(),
        context.dock_model.settings(),
    );
    if !applications.iter().any(|application| application.id == id) {
        applications.push(record);
    }
    context
        .auxiliary
        .settings
        .open(context.dock_model.settings(), context.graphics)?;
    let _ = context
        .auxiliary
        .settings
        .scene
        .set_applications(applications);
    let _ = context
        .auxiliary
        .settings
        .scene
        .open_application_manager(&id);
    super::settings_events::hydrate_application_previews(
        &context.auxiliary.applications,
        context.dock_model.items(),
        &mut context.auxiliary.settings,
    );
    context.auxiliary.settings.invalidate();
    Ok(())
}

fn execute_context_menu_action(
    action: ContextMenuAction,
    graphics: &mut DeviceState,
    dock_model: &DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    match action {
        ContextMenuAction::OpenSettings => {
            auxiliary.settings.open(dock_model.settings(), graphics)?;
            super::search_events::refresh_open_application_manager(dock_model, auxiliary);
        }
        ContextMenuAction::RequestShutdown => {
            auxiliary.context_menu.open_power(graphics)?;
        }
        ContextMenuAction::QuitLotus => request_exit(0),
    }
    Ok(())
}
