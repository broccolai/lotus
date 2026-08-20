use lotus_core::window::WindowInfo;
use lotus_ui::frame::ScheduledSurface;
use lotus_windows::graphics::{CompositionSurfaceState, DeviceState};
use lotus_windows::interaction::NativeMessage;
use lotus_windows::window::{DockWindow, PointerEvent, WindowEvent};
use lotus_windows::window_tracker::{WindowTracker, WindowTrackerEvent};

use super::presentation::{apply_fullscreen_visibility, resize_dock};
use super::{dock_events, popup_events, search_events};
use crate::app::context_menu::ContextMenuRuntime;
use crate::app::status::AuxiliaryZoneAction;
use crate::app::switcher::AuxiliaryWindows;
use crate::app::{AppError, DockRuntime, RuntimePolicy};

pub(super) fn drain_window_events(
    dock: &mut DockWindow,
    graphics: &mut DeviceState,
    surface: &mut ScheduledSurface<CompositionSurfaceState>,
    windows: &[WindowInfo],
    dock_model: &mut DockRuntime,
    auxiliary: &mut AuxiliaryWindows,
) -> Result<bool, AppError> {
    let events = dock.drain_events().collect::<Vec<_>>();
    let animation_tick = events
        .iter()
        .any(|event| matches!(event, WindowEvent::AnimationFrame));
    for event in events {
        dock_events::handle_window_event(
            event, dock, graphics, surface, dock_model, auxiliary,
        )?;
    }
    search_events::drain_search_events(dock, graphics, dock_model, auxiliary)?;
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
    Ok(animation_tick)
}

pub(super) struct TrackerEventContext<'a, 'runtime> {
    pub(super) runtime: &'a RuntimePolicy<'runtime>,
    pub(super) dock: &'a DockWindow,
    pub(super) graphics: &'a mut DeviceState,
    pub(super) surface: &'a mut ScheduledSurface<CompositionSurfaceState>,
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
    if event == WindowTrackerEvent::FullscreenRefreshed {
        let foreground = lotus_windows::activation::foreground_window();
        context.dock_model.record_foreground(foreground);
        context.auxiliary.switcher.record_foreground(foreground);
    }
    if event == WindowTrackerEvent::SnapshotRefreshed {
        let previous_size = context.dock_model.scene().desired_size();
        let windows = context.window_tracker.current_windows();
        let pins_reconciled = context
            .dock_model
            .reconcile_unpinned_pins(windows, &context.auxiliary.applications)?;
        if pins_reconciled {
            context
                .auxiliary
                .settings
                .scene
                .reconcile_application_icon_overrides(context.dock_model.settings());
            context.auxiliary.launcher.apply_settings(
                context.dock_model.settings(),
                context.dock,
                context.graphics,
            )?;
            context
                .auxiliary
                .context_menu
                .apply_settings(context.dock_model.settings());
            context
                .auxiliary
                .switcher
                .apply_settings(context.dock_model.settings());
            let _changed = context.auxiliary.media.refresh(context.dock_model);
        }
        context.dock_model.rebuild(windows);
        context
            .dock_model
            .record_foreground(lotus_windows::activation::foreground_window());
        reconcile_visible_picker(
            context.dock_model,
            &mut context.auxiliary.context_menu,
            context.graphics,
        )?;
        if context.dock_model.scene().desired_size() != previous_size {
            resize_dock(
                context.dock,
                context.graphics,
                context.surface,
                context.dock_model,
            )?;
        }
        if pins_reconciled {
            context.auxiliary.status.sync(
                context.dock,
                context.dock_model.settings(),
                context.dock_model.media(),
                context.graphics,
            )?;
        }
        context.surface.invalidate();
    }
    if context.runtime.onboarding_required {
        Ok(())
    } else {
        apply_fullscreen_visibility(
            context.dock,
            context.surface,
            context.window_tracker,
            context.dock_model,
            &mut context.auxiliary.launcher,
            &mut context.auxiliary.status,
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
