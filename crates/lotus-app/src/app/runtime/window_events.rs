use lotus_core::window::WindowInfo;
use lotus_windows::graphics::{CompositionSurfaceState, DeviceState};
use lotus_windows::interaction::NativeMessage;
use lotus_windows::window::{DockWindow, PointerEvent, WindowEvent};
use lotus_windows::window_tracker::{WindowTracker, WindowTrackerEvent};

use super::presentation::{apply_fullscreen_visibility, render_and_schedule, resize_dock};
use super::{dock_events, popup_events, search_events};
use crate::app::context_menu::ContextMenuRuntime;
use crate::app::status::AuxiliaryZoneAction;
use crate::app::switcher::AuxiliaryWindows;
use crate::app::{AppError, DockRuntime, RuntimePolicy};

pub(super) fn drain_window_events(
    dock: &mut DockWindow,
    graphics: &mut DeviceState,
    surface: &mut CompositionSurfaceState,
    windows: &[WindowInfo],
    dock_model: &mut DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<(), AppError> {
    let events = dock.drain_events().collect::<Vec<_>>();
    for event in events {
        dock_events::handle_window_event(
            event, dock, graphics, surface, dock_model, auxiliary,
        )?;
    }
    search_events::drain_search_events(dock, graphics, surface, dock_model, auxiliary)?;
    for event in auxiliary.context_menu.drain_events() {
        popup_events::handle_context_menu_event(
            event, dock, graphics, surface, windows, dock_model, auxiliary,
        )?;
    }
    for (zone, event) in auxiliary.status.drain_events() {
        if matches!(
            event,
            WindowEvent::Pointer(PointerEvent::LeftButtonPressed { .. })
        ) {
            auxiliary.launcher.hide();
        }
        if let Some((action, owner, anchor)) =
            auxiliary.status.handle_event(zone, event, graphics)?
        {
            match action {
                AuxiliaryZoneAction::Media(target) => {
                    auxiliary.media.activate(target, dock_model, owner);
                }
                AuxiliaryZoneAction::Status(kind) => {
                    dock_events::activate_system_status(kind, owner, anchor);
                }
            }
        }
    }
    Ok(())
}

pub(super) struct TrackerEventContext<'a, 'runtime> {
    pub(super) runtime: &'a RuntimePolicy<'runtime>,
    pub(super) dock: &'a DockWindow,
    pub(super) graphics: &'a mut DeviceState,
    pub(super) surface: &'a mut CompositionSurfaceState,
    pub(super) window_tracker: &'a mut WindowTracker,
    pub(super) dock_model: &'a mut DockRuntime,
    pub(super) auxiliary: &'a mut AuxiliaryWindows,
}

pub(super) fn handle_tracker_message(
    message: &NativeMessage,
    context: &mut TrackerEventContext<'_, '_>,
) -> Result<(), AppError> {
    let Some(event) = context.window_tracker.handle_message(
        message.is_thread_message(),
        message.id(),
        message.parameter(),
    )?
    else {
        return Ok(());
    };
    if event == WindowTrackerEvent::SnapshotRefreshed {
        context
            .dock_model
            .rebuild(context.window_tracker.current_windows());
        context
            .dock_model
            .record_foreground(lotus_windows::activation::foreground_window());
        reconcile_visible_picker(
            context.dock_model,
            &mut context.auxiliary.context_menu,
            context.graphics,
        )?;
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
        render_and_schedule(
            context.dock,
            context.graphics,
            context.surface,
            context.dock_model.scene(),
            context.auxiliary.launcher.needs_animation(),
        )?;
    }
    if context.runtime.onboarding_required {
        Ok(())
    } else {
        apply_fullscreen_visibility(
            context.dock,
            context.window_tracker,
            context.dock_model,
            &mut context.auxiliary.launcher,
            &context.auxiliary.status,
        )
    }
}

fn reconcile_visible_picker(
    dock_model: &mut DockRuntime,
    popup: &mut ContextMenuRuntime,
    graphics: &mut DeviceState,
) -> Result<(), AppError> {
    let Some(identity) = popup.picker_identity().map(str::to_owned) else {
        return Ok(());
    };
    let Some(source_index) = dock_model.source_index(&identity) else {
        popup.hide();
        return Ok(());
    };
    let windows = dock_model
        .picker_windows(source_index, lotus_windows::activation::foreground_window());
    let style = dock_model.settings().window_picker_style;
    popup.replace_picker(source_index, style, windows, graphics)
}
