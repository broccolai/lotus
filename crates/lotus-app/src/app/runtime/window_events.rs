use lotus_core::window::WindowInfo;
use lotus_ui::frame::ScheduledSurface;
use lotus_windows::graphics::{CompositionSurfaceState, DeviceState};
use lotus_windows::interaction::NativeMessage;
use lotus_windows::window::{DockWindow, WindowEvent};
use lotus_windows::window_tracker::{WindowTracker, WindowTrackerEvent};

use super::presentation::present_dock_change;
use super::{dock_events, popup_events, search_events};
use crate::app::modules::ModuleHost;
use crate::app::{AppError, DockRuntime};

pub(super) struct WindowDrainOutcome {
    pub(super) animation_tick: bool,
    pub(super) had_events: bool,
}

pub(super) fn drain_window_events(
    dock: &mut DockWindow,
    graphics: &mut DeviceState,
    surface: &mut ScheduledSurface<CompositionSurfaceState>,
    windows: &[WindowInfo],
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<WindowDrainOutcome, AppError> {
    let events = dock.drain_events().collect::<Vec<_>>();
    let mut had_events = !events.is_empty();
    let animation_tick = events
        .iter()
        .any(|event| matches!(event, WindowEvent::AnimationFrame));
    for event in events {
        dock_events::handle_window_event(
            event, dock, graphics, surface, dock_model, auxiliary,
        )?;
    }
    had_events |=
        search_events::drain_search_events(dock, graphics, dock_model, auxiliary)?;
    for event in auxiliary.drain_context_menu_events() {
        had_events = true;
        popup_events::handle_context_menu_event(
            event, dock, graphics, surface, windows, dock_model, auxiliary,
        )?;
    }
    for (zone, event) in auxiliary.drain_status_events() {
        had_events = true;
        auxiliary.hide_launcher_on_status_press(&event);
        if let Some(activation) = auxiliary.handle_status_event(zone, event, graphics)? {
            match activation.action {
                crate::app::status::AuxiliaryZoneAction::Media(target) => {
                    auxiliary.activate_media(target, dock_model, activation.owner);
                }
                crate::app::status::AuxiliaryZoneAction::Status(kind) => {
                    dock_events::activate_system_status(
                        kind,
                        activation.owner,
                        activation.anchor,
                    );
                }
            }
        }
    }
    Ok(WindowDrainOutcome {
        animation_tick,
        had_events,
    })
}

pub(super) struct TrackerEventContext<'a> {
    pub(super) dock: &'a DockWindow,
    pub(super) graphics: &'a mut DeviceState,
    pub(super) surface: &'a mut ScheduledSurface<CompositionSurfaceState>,
    pub(super) window_tracker: &'a mut WindowTracker,
    pub(super) dock_model: &'a mut DockRuntime,
    pub(super) auxiliary: &'a mut ModuleHost,
}

#[derive(Clone, Copy, Default)]
pub(super) struct TrackerMessageOutcome {
    pub(super) monitor_sync: bool,
    pub(super) frame: bool,
}

pub(super) fn handle_tracker_message(
    message: &NativeMessage,
    context: &mut TrackerEventContext<'_>,
) -> Result<TrackerMessageOutcome, AppError> {
    let Some(event) = context.window_tracker.handle_message(
        message.is_thread_message(),
        message.id(),
        message.parameter(),
    )?
    else {
        return Ok(TrackerMessageOutcome::default());
    };
    if event == WindowTrackerEvent::FullscreenRefreshed {
        let foreground = lotus_windows::activation::foreground_window();
        context.dock_model.record_foreground(foreground);
        context.auxiliary.record_switcher_foreground(foreground);
    }
    if event == WindowTrackerEvent::SnapshotRefreshed {
        let previous_size = context.dock_model.scene().desired_size();
        let windows = context.window_tracker.current_windows();
        let pins_reconciled = context
            .dock_model
            .reconcile_unpinned_pins(windows, context.auxiliary.application_catalog())?;
        if pins_reconciled {
            context.auxiliary.adapt_to_pin_changes(
                context.dock,
                context.dock_model,
                context.graphics,
            )?;
        }
        context.dock_model.rebuild(windows);
        context
            .dock_model
            .record_foreground(lotus_windows::activation::foreground_window());
        context
            .auxiliary
            .reconcile_visible_window_picker(context.dock_model, context.graphics)?;
        if context.dock_model.scene().desired_size() != previous_size {
            present_dock_change(
                context.dock,
                context.graphics,
                context.surface,
                context.auxiliary,
                context.dock_model,
            )?;
        } else if pins_reconciled {
            context.auxiliary.sync_status(
                context.dock,
                context.dock_model,
                context.graphics,
            )?;
        }
        context.surface.invalidate();
    }
    Ok(TrackerMessageOutcome {
        monitor_sync: true,
        frame: event == WindowTrackerEvent::SnapshotRefreshed,
    })
}
