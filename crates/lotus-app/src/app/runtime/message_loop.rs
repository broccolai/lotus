use lotus_ui::frame::FrameTrigger;
use lotus_windows::appbar::fullscreen_notification;
use lotus_windows::graphics::{DeviceState, GraphicsDeviceHealth, SurfaceError};
use lotus_windows::input::UiHeartbeatTimer;
use lotus_windows::interaction::{NativeMessage, next_message, take_priority_message};
use lotus_windows::responsiveness::{METRICS, UiMessagePhase};
use lotus_windows::window_tracker::WindowTracker;

use super::work::RuntimeWork;
use super::{dock_events, presentation, settings_events, window_events};
use crate::app::integration::IntegrationRecoveryContext;
use crate::app::modules::ModuleHost;
use crate::app::primary_dock::PrimaryDock;
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
    primary_dock: &mut PrimaryDock,
    graphics: &mut DeviceState,
    window_tracker: &mut WindowTracker,
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    let heartbeat = UiHeartbeatTimer::start(
        auxiliary.input_enabled(),
        runtime.integration.requires_maintenance(),
    )?;
    MessageLoop {
        heartbeat,
        runtime,
        primary_dock,
        graphics,
        window_tracker,
        dock_model,
        auxiliary,
        last_monitor_key: None,
        graphics_recovery: GraphicsRecoveryScheduler::new(),
        continuation_queued: false,
    }
    .run()
}

pub(crate) fn flush_frame(
    primary_dock: &mut PrimaryDock,
    graphics: &mut DeviceState,
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
    trigger: FrameTrigger,
) -> Result<(), AppError> {
    frame::flush_frame(primary_dock, graphics, dock_model, auxiliary, trigger)
}

struct MessageLoop<'a, 'runtime> {
    heartbeat: UiHeartbeatTimer,
    runtime: &'a mut RuntimeServices<'runtime>,
    primary_dock: &'a mut PrimaryDock,
    graphics: &'a mut DeviceState,
    window_tracker: &'a mut WindowTracker,
    dock_model: &'a mut DockRuntime,
    auxiliary: &'a mut ModuleHost,
    last_monitor_key: Option<presentation::MonitorPresentationKey>,
    graphics_recovery: GraphicsRecoveryScheduler,
    continuation_queued: bool,
}

impl MessageLoop<'_, '_> {
    fn run(&mut self) -> Result<(), AppError> {
        self.schedule_graphics_recovery();
        if self.runtime.post_install_health.is_some() {
            self.schedule_event_continuation();
        }
        loop {
            let Some(message) = next_message().map_err(|_error| AppError::MessageLoop)?
            else {
                return Ok(());
            };
            let message = self.prioritize_continuation_boundary(message);

            let started = std::time::Instant::now();
            let graphics_generation = self.graphics.generation();
            let mut timing = MessageTiming::default();
            let result = self.process_message(&message, &mut timing);
            let total = started.elapsed();
            METRICS.record_ui_message(total);
            self.record_slow_message(&message, total, timing, graphics_generation);
            match result {
                Ok(()) => self.complete_post_install_health_if_ready(),
                Err(AppError::Surface(SurfaceError::DeviceLost(loss))) => {
                    self.graphics.mark_lost(loss);
                    self.auxiliary.mark_presentations_unavailable();
                    self.runtime.integration.report_ui_progress(false);
                    self.primary_dock.window().set_animation_active(false)?;
                    self.schedule_graphics_recovery();
                }
                Err(AppError::GraphicsUnavailable)
                    if self.graphics.health() == GraphicsDeviceHealth::Lost =>
                {
                    self.auxiliary.mark_presentations_unavailable();
                    self.runtime.integration.report_ui_progress(false);
                    self.primary_dock.window().set_animation_active(false)?;
                    self.schedule_graphics_recovery();
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn prioritize_continuation_boundary(
        &mut self,
        message: NativeMessage,
    ) -> NativeMessage {
        if !message.is_thread_message()
            || !lotus_windows::interaction::is_runtime_continuation(message.id())
        {
            return message;
        }
        self.continuation_queued = false;
        let Some(priority) = take_priority_message() else {
            return message;
        };

        self.schedule_event_continuation();
        priority
    }

    fn complete_post_install_health_if_ready(&mut self) {
        if self.graphics.health() != GraphicsDeviceHealth::Healthy
            || !self.primary_dock.presentation_ready()
            || !self.auxiliary.runtime_capabilities_ready()
        {
            return;
        }
        let Some(health) = self.runtime.post_install_health.take() else {
            return;
        };
        if let Err(error) =
            lotus_windows::update::complete_post_install_runtime_readiness(health)
        {
            lotus_windows::diagnostics::record_error(
                "update.runtime_readiness_writer",
                &error,
            );
        }
    }

    fn process_message(
        &mut self,
        message: &NativeMessage,
        timing: &mut MessageTiming,
    ) -> Result<(), AppError> {
        if message.is_thread_message()
            && lotus_windows::interaction::is_runtime_continuation(message.id())
        {
            self.continuation_queued = false;
        }
        let recovery_started = std::time::Instant::now();
        if self.handle_graphics_recovery_wake(message)? {
            timing.record(UiMessagePhase::GraphicsRecovery, recovery_started.elapsed());
            return Ok(());
        }
        if message.is_thread_message()
            && self.heartbeat.matches(message.id(), message.parameter())
        {
            self.heartbeat.acknowledge();
            return self.handle_heartbeat(timing);
        }
        if self.heartbeat.due() {
            self.heartbeat.acknowledge();
            self.handle_heartbeat(timing)?;
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
                primary_dock: self.primary_dock,
                graphics: self.graphics,
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
        let integration_recovery = self
            .runtime
            .integration
            .recovery_source(message, self.primary_dock.window());
        self.include_pending_event_work(&mut work);
        let drained = self.drain_events_with_budget(work, timing)?;
        if drained.changed {
            work.insert(RuntimeWork::FRAME);
        }
        if drained.animation_tick {
            work.insert(RuntimeWork::ANIMATION_TICK);
            work.insert(RuntimeWork::FRAME);
        }
        if self.apply_pending_persistence(timing)? {
            work.insert(RuntimeWork::FRAME);
        }
        if work.contains(RuntimeWork::WAKES) && self.process_wakes(wakes, timing)? {
            work.insert(RuntimeWork::FRAME);
        }
        if let Some(source) = integration_recovery {
            self.recover_integration(source, &mut work, timing);
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

    fn handle_heartbeat(&mut self, timing: &mut MessageTiming) -> Result<(), AppError> {
        self.auxiliary.heartbeat_input();
        let integration_started = std::time::Instant::now();
        let integration_changed = self
            .runtime
            .integration
            .maintain(self.dock_model.settings(), self.primary_dock.window());
        timing.record(UiMessagePhase::Integration, integration_started.elapsed());
        if integration_changed {
            self.heartbeat.set_modes(
                self.auxiliary.input_enabled(),
                self.runtime.integration.requires_maintenance(),
            )?;
        }
        let started = std::time::Instant::now();
        let frame = self.handle_input_wake()
            || integration_changed
            || self.apply_pending_persistence(timing)?;
        timing.record(UiMessagePhase::Wake, started.elapsed());
        if frame {
            let frame_started = std::time::Instant::now();
            self.flush_frame(FrameTrigger::Changes)?;
            timing.record(UiMessagePhase::Frame, frame_started.elapsed());
        }
        self.runtime.integration.report_ui_progress(
            self.graphics.health() == GraphicsDeviceHealth::Healthy
                && self.primary_dock.presentation_ready()
                && self.auxiliary.runtime_capabilities_ready(),
        );
        METRICS.record_ui_work(false, false, frame);
        Ok(())
    }

    fn include_pending_event_work(&self, work: &mut RuntimeWork) {
        if self.primary_dock.window().has_pending_events()
            || self.auxiliary.has_pending_window_events()
        {
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
        ) && self.runtime.integration.shell_effects_allowed()
        {
            self.window_tracker.set_shell_fullscreen(fullscreen);
            work.insert(RuntimeWork::MONITOR_SYNC);
        }
    }

    fn drain_events_with_budget(
        &mut self,
        work: RuntimeWork,
        timing: &mut MessageTiming,
    ) -> Result<EventDrainOutcome, AppError> {
        const MAX_DRAIN_ROUNDS: usize = 4;

        let mut budget = EventDrainBudget::new();
        let mut rounds = 1;
        let mut outcome = self.drain_events(work, timing, &mut budget)?;
        loop {
            let mut pending = RuntimeWork::default();
            self.include_pending_event_work(&mut pending);
            if !pending.needs_event_drain() {
                return Ok(outcome);
            }
            if rounds >= MAX_DRAIN_ROUNDS || budget.exhausted() {
                self.schedule_event_continuation();
                return Ok(outcome);
            }
            let additional = self.drain_events(pending, timing, &mut budget)?;
            outcome.animation_tick |= additional.animation_tick;
            outcome.changed |= additional.changed;
            rounds += 1;
        }
    }

    fn schedule_event_continuation(&mut self) {
        if self.continuation_queued {
            return;
        }
        self.continuation_queued = lotus_windows::interaction::post_runtime_continuation();
        if !self.continuation_queued {
            lotus_windows::diagnostics::record_message(
                "runtime.continuation_wake_failed",
                "pending UI work will resume on the next native message",
            );
        }
    }

    fn drain_events(
        &mut self,
        work: RuntimeWork,
        timing: &mut MessageTiming,
        budget: &mut EventDrainBudget,
    ) -> Result<EventDrainOutcome, AppError> {
        let mut animation_tick = false;
        let mut changed = false;
        if work.contains(RuntimeWork::WINDOW_EVENTS) {
            let started = std::time::Instant::now();
            let allowance = budget.allowance(32);
            let outcome = window_events::drain_window_events(
                self.primary_dock,
                self.graphics,
                self.dock_model,
                self.auxiliary,
                &self.runtime.settings_persistence,
                allowance,
            )?;
            budget.consume(outcome.drained_events);
            animation_tick = outcome.animation_tick;
            changed |= outcome.had_events;
            timing.record(UiMessagePhase::WindowDrain, started.elapsed());
        }
        if work.contains(RuntimeWork::SETTINGS_EVENTS) && !budget.exhausted() {
            let started = std::time::Instant::now();
            let drained = self.drain_settings_events(budget.allowance(8))?;
            budget.consume(drained);
            changed |= drained != 0;
            timing.record(UiMessagePhase::SettingsDrain, started.elapsed());
        }
        if work.contains(RuntimeWork::SWITCHER_EVENTS) && !budget.exhausted() {
            let started = std::time::Instant::now();
            let drained = self
                .auxiliary
                .drain_switcher_events_up_to(self.graphics, budget.allowance(8));
            budget.consume(drained);
            changed |= drained != 0;
            timing.record(UiMessagePhase::SwitcherDrain, started.elapsed());
        }
        if work.contains(RuntimeWork::MONITOR_EVENTS) && !budget.exhausted() {
            let started = std::time::Instant::now();
            let outcome = self
                .auxiliary
                .drain_monitor_dock_events_up_to(self.graphics, budget.allowance(8))?;
            budget.consume(outcome.drained_events);
            changed |= outcome.had_events;
            for action in outcome.actions {
                dock_events::execute_dock_action(
                    action,
                    self.primary_dock.window(),
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
            self.primary_dock,
            self.graphics,
            self.window_tracker,
            self.dock_model,
            self.auxiliary,
        )?;
        self.last_monitor_key = Some(key);
        timing.record(UiMessagePhase::MonitorSync, started.elapsed());
        Ok(true)
    }

    fn drain_settings_events(&mut self, limit: usize) -> Result<usize, AppError> {
        let drained = settings_events::drain_settings_events_up_to(
            &mut settings_events::SettingsEventContext {
                primary_dock: self.primary_dock,
                graphics: self.graphics,
                window_tracker: self.window_tracker,
                dock_model: self.dock_model,
                auxiliary: self.auxiliary,
                integration: self.runtime.integration,
                settings_persistence: &self.runtime.settings_persistence,
                startup_mode: self.runtime.startup_mode,
                startup_registration_allowed: self.runtime.startup_registration_allowed,
            },
            limit,
        )?;
        self.heartbeat.set_modes(
            self.auxiliary.input_enabled(),
            self.runtime.integration.requires_maintenance(),
        )?;
        Ok(drained)
    }

    fn apply_pending_persistence(
        &mut self,
        timing: &mut MessageTiming,
    ) -> Result<bool, AppError> {
        let started = std::time::Instant::now();
        let changed = settings_events::apply_persistence_completion(
            &mut settings_events::SettingsEventContext {
                primary_dock: self.primary_dock,
                graphics: self.graphics,
                window_tracker: self.window_tracker,
                dock_model: self.dock_model,
                auxiliary: self.auxiliary,
                integration: self.runtime.integration,
                settings_persistence: &self.runtime.settings_persistence,
                startup_mode: self.runtime.startup_mode,
                startup_registration_allowed: self.runtime.startup_registration_allowed,
            },
        )?;
        timing.record(UiMessagePhase::Persistence, started.elapsed());
        Ok(changed)
    }

    fn flush_frame(&mut self, trigger: FrameTrigger) -> Result<(), AppError> {
        flush_frame(
            self.primary_dock,
            self.graphics,
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
        timing: &mut MessageTiming,
    ) {
        let started = std::time::Instant::now();
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
                primary_dock: self.primary_dock,
                graphics: self.graphics,
                window_tracker: self.window_tracker,
                dock_model: self.dock_model,
                auxiliary: self.auxiliary,
            },
        );
        timing.record(UiMessagePhase::Integration, started.elapsed());
        self.last_monitor_key = None;
        work.insert(RuntimeWork::FRAME);
    }
}

#[derive(Clone, Copy, Default)]
struct EventDrainOutcome {
    animation_tick: bool,
    changed: bool,
}

struct EventDrainBudget {
    started: std::time::Instant,
    remaining: usize,
}

impl EventDrainBudget {
    const MAX_EVENTS: usize = 64;
    const MAX_TIME: std::time::Duration = std::time::Duration::from_millis(2);

    fn new() -> Self {
        Self {
            started: std::time::Instant::now(),
            remaining: Self::MAX_EVENTS,
        }
    }

    fn allowance(&self, quantum: usize) -> usize {
        if self.exhausted() {
            0
        } else {
            self.remaining.min(quantum)
        }
    }

    fn consume(&mut self, processed: usize) {
        self.remaining = self.remaining.saturating_sub(processed);
    }

    fn exhausted(&self) -> bool {
        self.remaining == 0 || self.started.elapsed() >= Self::MAX_TIME
    }
}
