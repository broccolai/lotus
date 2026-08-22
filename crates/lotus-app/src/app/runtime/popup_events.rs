use lotus_core::window::WindowInfo;
use lotus_ui::frame::ScheduledSurface;
use lotus_windows::WindowHandle;
use lotus_windows::activation::{
    ActivationError, force_window_close, launch_target, request_window_close,
};
use lotus_windows::dialog::show_error;
use lotus_windows::graphics::{CompositionSurfaceState, DeviceState};
use lotus_windows::interaction::request_exit;
use lotus_windows::window::{ContextMenuEvent, DockWindow, SelectionDirection};

use super::presentation::present_dock_change;
use crate::app::modules::ModuleHost;
use crate::app::visuals::{AppMenuAction, ContextMenuAction, PopupAction, PowerAction};
use crate::app::{AppError, DockRuntime};

pub(super) fn handle_context_menu_event(
    event: ContextMenuEvent,
    dock: &DockWindow,
    graphics: &mut DeviceState,
    surface: &mut ScheduledSurface<CompositionSurfaceState>,
    windows: &[WindowInfo],
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    match event {
        ContextMenuEvent::PointerMoved { x, y } => {
            if auxiliary.context_menu_runtime().scene.pointer_move(x, y) {
                auxiliary.context_menu_runtime().invalidate();
            }
        }
        ContextMenuEvent::PointerLeft => {
            if auxiliary.context_menu_runtime().scene.pointer_left() {
                auxiliary.context_menu_runtime().invalidate();
            }
        }
        ContextMenuEvent::PointerReleased { x, y } => {
            let action = auxiliary.context_menu_runtime().scene.pointer_action(x, y);
            let source_index = auxiliary.context_menu_runtime().scene.source_index();
            if !action.is_some_and(opens_power_menu) {
                auxiliary.context_menu_runtime().hide();
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
            let action = auxiliary.context_menu_runtime().scene.selected_action();
            let source_index = auxiliary.context_menu_runtime().scene.source_index();
            if !action.is_some_and(opens_power_menu) {
                auxiliary.context_menu_runtime().hide();
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
            if auxiliary.context_menu_runtime().scene.move_selection(next) {
                auxiliary.context_menu_runtime().invalidate();
            }
        }
        ContextMenuEvent::Scroll(direction) => {
            let next = direction == SelectionDirection::Next;
            if auxiliary.context_menu_runtime().scene.scroll(next) {
                auxiliary.context_menu_runtime().invalidate();
            }
        }
        ContextMenuEvent::DismissRequested => auxiliary.context_menu_runtime().hide(),
        ContextMenuEvent::Resized { width, height } => {
            auxiliary.context_menu_runtime().resize(width, height)?;
            auxiliary.context_menu_runtime().invalidate();
        }
        ContextMenuEvent::DpiChanged { dpi } => {
            if auxiliary.context_menu_runtime().scene.set_dpi(dpi) {
                let desired = auxiliary.context_menu_runtime().scene.desired_size();
                if let Some(surface) = &mut auxiliary.context_menu_runtime().surface {
                    surface.value_mut().resize(desired)?;
                }
            }
            auxiliary.context_menu_runtime().invalidate();
        }
        ContextMenuEvent::RenderRequested => auxiliary.context_menu_runtime().invalidate(),
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
                .and_then(|item| {
                    item.windows.first().and_then(|window| {
                        context
                            .auxiliary
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
