use lotus_ui::frame::{FrameTrigger, ScheduledSurface};
use lotus_windows::appbar::fullscreen_notification;
use lotus_windows::graphics::{
    CompositionSurfaceState, DeviceState, GraphicsDeviceHealth, SurfaceError,
};
use lotus_windows::input::UiHeartbeatTimer;
use lotus_windows::interaction::{NativeMessage, next_message};
use lotus_windows::responsiveness::{METRICS, UiMessagePhase};
use lotus_windows::window::DockWindow;
use lotus_windows::window_tracker::WindowTracker;

use super::work::RuntimeWork;
use super::{dock_events, presentation, settings_events, window_events};
use crate::app::integration::IntegrationRecoveryContext;
use crate::app::modules::ModuleHost;
use crate::app::{AppError, DockRuntime, RuntimeServices};

mod frame;
mod graphics_recovery;
mod timing;
mod wakes;

use graphics_recovery::GraphicsRecoveryScheduler;
use timing::MessageTiming;
use wakes::{WakeEvents, is_input_wake};

pub(crate) fn run_message_loop(
    runtime: &mut RuntimeServices<'_>,
    dock: &mut DockWindow,
    graphics: &mut DeviceState,
    surface: &mut ScheduledSurface<CompositionSurfaceState>,
    window_tracker: &mut WindowTracker,
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    let heartbeat = UiHeartbeatTimer::start(auxiliary.input_enabled())?;
    MessageLoop {
        heartbeat,
        runtime,
        dock,
        graphics,
        surface,
        window_tracker,
        dock_model,
        auxiliary,
        last_monitor_key: None,
        graphics_recovery: GraphicsRecoveryScheduler::new(),
    }
    .run()
}

pub(crate) fn flush_frame(
    dock: &mut DockWindow,
    graphics: &mut DeviceState,
    surface: &mut ScheduledSurface<CompositionSurfaceState>,
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
    trigger: FrameTrigger,
) -> Result<(), AppError> {
    frame::flush_frame(dock, graphics, surface, dock_model, auxiliary, trigger)
}

struct MessageLoop<'a, 'runtime> {
    heartbeat: UiHeartbeatTimer,
    runtime: &'a mut RuntimeServices<'runtime>,
    dock: &'a mut DockWindow,
    graphics: &'a mut DeviceState,
    surface: &'a mut ScheduledSurface<CompositionSurfaceState>,
    window_tracker: &'a mut WindowTracker,
    dock_model: &'a mut DockRuntime,
    auxiliary: &'a mut ModuleHost,
    last_monitor_key: Option<presentation::MonitorPresentationKey>,
    graphics_recovery: GraphicsRecoveryScheduler,
}

impl MessageLoop<'_, '_> {
    fn run(&mut self) -> Result<(), AppError> {
        self.schedule_graphics_recovery();
        loop {
            let Some(message) = next_message().map_err(|_error| AppError::MessageLoop)?
            else {
                return Ok(());
            };

            let started = std::time::Instant::now();
            let graphics_generation = self.graphics.generation();
            let mut timing = MessageTiming::default();
            let result = self.process_message(&message, &mut timing);
            let total = started.elapsed();
            METRICS.record_ui_message(total);
            self.record_slow_message(&message, total, timing, graphics_generation);
            match result {
                Ok(()) => {}
                Err(AppError::Surface(SurfaceError::DeviceLost(loss))) => {
                    self.graphics.mark_lost(loss);
                    self.dock.set_animation_active(false)?;
                    self.schedule_graphics_recovery();
                }
                Err(AppError::GraphicsUnavailable)
                    if self.graphics.health() == GraphicsDeviceHealth::Lost =>
                {
                    self.dock.set_animation_active(false)?;
                    self.schedule_graphics_recovery();
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn process_message(
        &mut self,
        message: &NativeMessage,
        timing: &mut MessageTiming,
    ) -> Result<(), AppError> {
        if self.handle_graphics_recovery_wake(message)? {
            return Ok(());
        }
        if message.is_thread_message()
            && self.heartbeat.matches(message.id(), message.parameter())
        {
            self.auxiliary.heartbeat_input();
            let started = std::time::Instant::now();
            let frame = self.handle_input_wake();
            timing.record(UiMessagePhase::Wake, started.elapsed());
            if frame {
                let frame_started = std::time::Instant::now();
                self.flush_frame(FrameTrigger::Changes)?;
                timing.record(UiMessagePhase::Frame, frame_started.elapsed());
            }
            METRICS.record_ui_work(false, false, frame);
            return Ok(());
        }
        if self.auxiliary.input_enabled() && is_input_wake(message.id()) {
            let started = std::time::Instant::now();
            message.dispatch();
            timing.record(UiMessagePhase::Dispatch, started.elapsed());
            let started = std::time::Instant::now();
            let frame = self.handle_input_wake();
            timing.record(UiMessagePhase::Wake, started.elapsed());
            if frame {
                let frame_started = std::time::Instant::now();
                self.flush_frame(FrameTrigger::Changes)?;
                timing.record(UiMessagePhase::Frame, frame_started.elapsed());
            }
            METRICS.record_ui_work(false, false, frame);
            return Ok(());
        }
        let mut work =
            message
                .target_window()
                .map_or_else(RuntimeWork::default, |window| {
                    if self.auxiliary.settings_owns_window(window) {
                        RuntimeWork::SETTINGS_EVENTS
                    } else if self.auxiliary.switcher_owns_window(window) {
                        RuntimeWork::SWITCHER_EVENTS
                    } else if self.auxiliary.monitor_docks_own_window(window) {
                        RuntimeWork::MONITOR_EVENTS
                    } else {
                        RuntimeWork::WINDOW_EVENTS
                    }
                });
        self.handle_shell_fullscreen(message, &mut work);

        let started = std::time::Instant::now();
        let tracker = window_events::handle_tracker_message(
            message,
            &mut window_events::TrackerEventContext {
                dock: self.dock,
                graphics: self.graphics,
                surface: self.surface,
                window_tracker: self.window_tracker,
                dock_model: self.dock_model,
                auxiliary: self.auxiliary,
            },
        )?;
        if tracker.monitor_sync {
            work.insert(RuntimeWork::MONITOR_SYNC);
        }
        if tracker.frame {
            work.insert(RuntimeWork::FRAME);
        }
        timing.record(UiMessagePhase::Tracker, started.elapsed());

        let wakes = WakeEvents::from_message(self.runtime, message.id());
        if wakes.any() {
            work.insert(RuntimeWork::WAKES);
        }
        let started = std::time::Instant::now();
        message.dispatch();
        timing.record(UiMessagePhase::Dispatch, started.elapsed());
        let integration_recovery =
            self.runtime.integration.recovery_source(message, self.dock);
        self.include_pending_event_work(&mut work);
        let drained = self.drain_events_until_idle(work, timing)?;
        if drained.changed {
            work.insert(RuntimeWork::FRAME);
        }
        if drained.animation_tick {
            work.insert(RuntimeWork::ANIMATION_TICK);
            work.insert(RuntimeWork::FRAME);
        }
        if work.contains(RuntimeWork::WAKES) {
            let started = std::time::Instant::now();
            if self.process_wakes(wakes)? {
                work.insert(RuntimeWork::FRAME);
            }
            timing.record(UiMessagePhase::Wake, started.elapsed());
        }
        if let Some(source) = integration_recovery {
            self.recover_integration(source, &mut work);
        }
        let monitor_sync = self.sync_monitor_presentation(work, timing)?;
        if work.contains(RuntimeWork::FRAME) {
            let started = std::time::Instant::now();
            self.flush_frame(if work.contains(RuntimeWork::ANIMATION_TICK) {
                FrameTrigger::AnimationTick
            } else {
                FrameTrigger::Changes
            })?;
            timing.record(UiMessagePhase::Frame, started.elapsed());
        }
        METRICS.record_ui_work(
            work.needs_event_drain(),
            monitor_sync,
            work.contains(RuntimeWork::FRAME),
        );
        Ok(())
    }

    fn include_pending_event_work(&self, work: &mut RuntimeWork) {
        if self.dock.has_pending_events() || self.auxiliary.has_pending_window_events() {
            work.insert(RuntimeWork::WINDOW_EVENTS);
        }
        if self.auxiliary.has_pending_settings_events() {
            work.insert(RuntimeWork::SETTINGS_EVENTS);
        }
        if self.auxiliary.has_pending_switcher_events() {
            work.insert(RuntimeWork::SWITCHER_EVENTS);
        }
        if self.auxiliary.has_pending_monitor_events() {
            work.insert(RuntimeWork::MONITOR_EVENTS);
        }
    }

    fn handle_shell_fullscreen(&mut self, message: &NativeMessage, work: &mut RuntimeWork) {
        if let Some(fullscreen) = fullscreen_notification(
            message.is_thread_message(),
            message.id(),
            message.parameter(),
        ) {
            self.window_tracker.set_shell_fullscreen(fullscreen);
            work.insert(RuntimeWork::MONITOR_SYNC);
        }
    }

    fn drain_events_until_idle(
        &mut self,
        work: RuntimeWork,
        timing: &mut MessageTiming,
    ) -> Result<EventDrainOutcome, AppError> {
        let mut outcome = self.drain_events(work, timing)?;
        loop {
            let mut pending = RuntimeWork::default();
            self.include_pending_event_work(&mut pending);
            if !pending.needs_event_drain() {
                return Ok(outcome);
            }
            let additional = self.drain_events(pending, timing)?;
            outcome.animation_tick |= additional.animation_tick;
            outcome.changed |= additional.changed;
        }
    }

    fn drain_events(
        &mut self,
        work: RuntimeWork,
        timing: &mut MessageTiming,
    ) -> Result<EventDrainOutcome, AppError> {
        let mut animation_tick = false;
        let mut changed = false;
        if work.contains(RuntimeWork::WINDOW_EVENTS) {
            let started = std::time::Instant::now();
            let outcome = window_events::drain_window_events(
                self.dock,
                self.graphics,
                self.surface,
                self.window_tracker.current_windows(),
                self.dock_model,
                self.auxiliary,
            )?;
            animation_tick = outcome.animation_tick;
            changed |= outcome.had_events;
            timing.record(UiMessagePhase::WindowDrain, started.elapsed());
        }
        if work.contains(RuntimeWork::SETTINGS_EVENTS) {
            let started = std::time::Instant::now();
            changed |= self.drain_settings_events()?;
            timing.record(UiMessagePhase::SettingsDrain, started.elapsed());
        }
        if work.contains(RuntimeWork::SWITCHER_EVENTS) {
            let started = std::time::Instant::now();
            changed |= self.auxiliary.drain_switcher_events(self.graphics);
            timing.record(UiMessagePhase::SwitcherDrain, started.elapsed());
        }
        if work.contains(RuntimeWork::MONITOR_EVENTS) {
            let started = std::time::Instant::now();
            let outcome = self.auxiliary.drain_monitor_dock_events(self.graphics)?;
            changed |= outcome.had_events;
            for action in outcome.actions {
                dock_events::execute_dock_action(
                    action,
                    self.dock,
                    self.graphics,
                    self.dock_model,
                    self.auxiliary,
                )?;
            }
            timing.record(UiMessagePhase::MonitorDrain, started.elapsed());
        }

        Ok(EventDrainOutcome {
            animation_tick,
            changed,
        })
    }

    fn sync_monitor_presentation(
        &mut self,
        work: RuntimeWork,
        timing: &mut MessageTiming,
    ) -> Result<bool, AppError> {
        let key = presentation::monitor_presentation_key(
            self.window_tracker,
            self.dock_model,
            self.auxiliary,
        );
        if !work.contains(RuntimeWork::MONITOR_SYNC) && self.last_monitor_key == Some(key) {
            return Ok(false);
        }
        let started = std::time::Instant::now();
        presentation::sync_monitor_presentation(
            self.runtime,
            self.dock,
            self.surface,
            self.graphics,
            self.window_tracker,
            self.dock_model,
            self.auxiliary,
        )?;
        self.last_monitor_key = Some(key);
        timing.record(UiMessagePhase::MonitorSync, started.elapsed());
        Ok(true)
    }

    fn drain_settings_events(&mut self) -> Result<bool, AppError> {
        let had_events = settings_events::drain_settings_events(
            &mut settings_events::SettingsEventContext {
                dock: self.dock,
                graphics: self.graphics,
                dock_surface: self.surface,
                window_tracker: self.window_tracker,
                dock_model: self.dock_model,
                auxiliary: self.auxiliary,
                integration: self.runtime.integration,
            },
        )?;
        self.heartbeat.set_enabled(self.auxiliary.input_enabled())?;
        Ok(had_events)
    }

    fn flush_frame(&mut self, trigger: FrameTrigger) -> Result<(), AppError> {
        flush_frame(
            self.dock,
            self.graphics,
            self.surface,
            self.dock_model,
            self.auxiliary,
            trigger,
        )?;
        self.schedule_graphics_recovery();
        Ok(())
    }

    fn recover_integration(
        &mut self,
        source: crate::app::integration::IntegrationRecoverySource,
        work: &mut RuntimeWork,
    ) {
        if matches!(
            source,
            crate::app::integration::IntegrationRecoverySource::Settings
        ) && self.graphics.health() == GraphicsDeviceHealth::Lost
        {
            self.graphics_recovery.reset();
        }
        self.runtime.integration.recover(
            source,
            &mut IntegrationRecoveryContext {
                dock: self.dock,
                graphics: self.graphics,
                dock_surface: self.surface,
                window_tracker: self.window_tracker,
                dock_model: self.dock_model,
                auxiliary: self.auxiliary,
            },
        );
        self.last_monitor_key = None;
        work.insert(RuntimeWork::FRAME);
    }
}

#[derive(Clone, Copy, Default)]
struct EventDrainOutcome {
    animation_tick: bool,
    changed: bool,
}
