use std::time::Instant;

use lotus_core::window::WindowInfo;
use lotus_ui::frame::ScheduledSurface;
use lotus_windows::graphics::{CompositionSurfaceState, DeviceState, GraphicsDeviceHealth};
use lotus_windows::interaction::NativeMessage;
use lotus_windows::responsiveness::{METRICS, TrackerUiPhase};
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
        complete_after_graphics_loss(
            dock_events::handle_window_event(
                event, dock, graphics, surface, dock_model, auxiliary,
            ),
            graphics,
        )?;
    }
    had_events |= complete_after_graphics_loss(
        search_events::drain_search_events(dock, graphics, dock_model, auxiliary),
        graphics,
    )?
    .unwrap_or(false);
    for event in auxiliary.drain_context_menu_events() {
        had_events = true;
        complete_after_graphics_loss(
            popup_events::handle_context_menu_event(
                event, dock, graphics, surface, windows, dock_model, auxiliary,
            ),
            graphics,
        )?;
    }
    for (zone, event) in auxiliary.drain_status_events() {
        had_events = true;
        auxiliary.hide_launcher_on_status_press(&event);
        let activation = complete_after_graphics_loss(
            auxiliary.handle_status_event(zone, event, graphics),
            graphics,
        )?;
        if let Some(activation) = activation.flatten() {
            match activation.action {
                crate::app::status::AuxiliaryZoneAction::Media(target) => {
                    auxiliary.activate_media(target, dock_model, activation.owner);
                }
                crate::app::status::AuxiliaryZoneAction::Status(kind) => {
                    if dock_events::activate_system_status(
                        kind,
                        activation.owner,
                        activation.anchor,
                    ) {
                        dock_model.advanced_color_changed();
                        auxiliary.refresh_status(dock_model.settings());
                    }
                }
            }
        }
    }
    Ok(WindowDrainOutcome {
        animation_tick,
        had_events,
    })
}

fn complete_after_graphics_loss<T>(
    result: Result<T, AppError>,
    graphics: &mut DeviceState,
) -> Result<Option<T>, AppError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error)
            if error.mark_graphics_lost(graphics)
                || graphics.health() == GraphicsDeviceHealth::Lost =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
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
    if !WindowTracker::is_refresh_message(message.is_thread_message(), message.id()) {
        return Ok(TrackerMessageOutcome::default());
    }
    let Some(event) =
        measure_tracker_ui_phase(TrackerUiPhase::PublishedSnapshotObservation, || {
            context.window_tracker.handle_message(
                message.is_thread_message(),
                message.id(),
                message.parameter(),
            )
        })?
    else {
        return Ok(TrackerMessageOutcome::default());
    };
    if event == WindowTrackerEvent::FullscreenRefreshed {
        measure_tracker_ui_phase(TrackerUiPhase::DockModelRebuildForegroundUpdate, || {
            let foreground = lotus_windows::activation::foreground_window();
            context.dock_model.record_foreground(foreground);
            context.auxiliary.record_switcher_foreground(
                foreground,
                context.window_tracker.current_windows(),
            );
        });
    }
    if event == WindowTrackerEvent::SnapshotRefreshed {
        let previous_size = context.dock_model.scene().desired_size();
        let windows = context.window_tracker.current_windows();
        context.dock_model.prune_recent_windows(windows);
        let application_catalog = context.auxiliary.application_snapshot();
        measure_tracker_ui_phase(TrackerUiPhase::DockModelRebuildForegroundUpdate, || {
            context
                .dock_model
                .rebuild(windows, application_catalog.clone());
            context
                .dock_model
                .record_foreground(lotus_windows::activation::foreground_window());
        });
        measure_tracker_ui_phase(TrackerUiPhase::SwitcherReconciliation, || {
            complete_after_graphics_loss(
                context.auxiliary.reconcile_switcher_windows(
                    windows,
                    application_catalog,
                    context.dock_model.application_assignments(),
                    context.graphics,
                ),
                context.graphics,
            )
        })?;
        measure_tracker_ui_phase(TrackerUiPhase::VisiblePickerReconciliation, || {
            complete_after_graphics_loss(
                context
                    .auxiliary
                    .reconcile_visible_window_picker(context.dock_model, context.graphics),
                context.graphics,
            )
        })?;
        if context.dock_model.scene().desired_size() != previous_size {
            measure_tracker_ui_phase(
                TrackerUiPhase::PresentationStatusSynchronization,
                || {
                    complete_after_graphics_loss(
                        present_dock_change(
                            context.dock,
                            context.graphics,
                            context.surface,
                            context.auxiliary,
                            context.dock_model,
                        ),
                        context.graphics,
                    )
                },
            )?;
        }
        context.surface.invalidate();
    }
    Ok(TrackerMessageOutcome {
        monitor_sync: true,
        frame: event == WindowTrackerEvent::SnapshotRefreshed,
    })
}

fn measure_tracker_ui_phase<T>(phase: TrackerUiPhase, operation: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let result = operation();
    METRICS.record_tracker_ui_phase(phase, started.elapsed());
    result
}
