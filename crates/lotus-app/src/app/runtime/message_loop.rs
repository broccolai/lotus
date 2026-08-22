use lotus_ui::frame::{FrameOutcome, FramePass, FrameTrigger, ScheduledSurface};
use lotus_windows::appbar::fullscreen_notification;
use lotus_windows::graphics::{CompositionSurfaceState, DeviceState};
use lotus_windows::icon_hydrator::is_icon_hydration_wake;
use lotus_windows::input::{UiHeartbeatTimer, is_input_wake};
use lotus_windows::interaction::{NativeMessage, next_message};
use lotus_windows::media::is_media_wake;
use lotus_windows::responsiveness::{METRICS, SlowUiEvent, UiMessagePhase};
use lotus_windows::search_catalog::is_search_catalog_wake;
use lotus_windows::taskbar_badges::is_taskbar_badge_wake;
use lotus_windows::update::is_update_wake;
use lotus_windows::window::DockWindow;
use lotus_windows::window_tracker::WindowTracker;

use super::work::RuntimeWork;
use super::{
    controllers, dock_events, present_dock_change, presentation, search_events,
    settings_events, update_events, window_events,
};
use crate::app::modules::ModuleHost;
use crate::app::{AppError, DockRuntime, RuntimePolicy};

pub(crate) fn run_message_loop(
    runtime: &RuntimePolicy<'_>,
    dock: &mut DockWindow,
    graphics: &mut DeviceState,
    surface: &mut ScheduledSurface<CompositionSurfaceState>,
    window_tracker: &mut WindowTracker,
    dock_model: &mut DockRuntime,
    auxiliary: &mut ModuleHost,
) -> Result<(), AppError> {
    let heartbeat = UiHeartbeatTimer::start(auxiliary.input().is_some())?;
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
    let mut pass = FramePass::new(trigger);
    let device_generation = graphics.generation();
    let animation_allowed = !dock.is_fullscreen_occluded();
    pass.render(surface, |surface| {
        presentation::render_surface(graphics, surface, dock_model).map(|outcome| {
            match outcome {
                FrameOutcome::Complete {
                    continues_animation,
                } => FrameOutcome::complete(continues_animation && animation_allowed),
                FrameOutcome::Retry => FrameOutcome::Retry,
            }
        })
    })?;
    auxiliary.render_frames(&mut pass, graphics)?;

    if graphics.generation() != device_generation {
        surface.invalidate();
        auxiliary.invalidate_surfaces();
        pass.request_next_frame();
    }

    dock.set_animation_active(pass.animation_active())?;
    Ok(())
}

struct MessageLoop<'a, 'runtime> {
    heartbeat: UiHeartbeatTimer,
    runtime: &'a RuntimePolicy<'runtime>,
    dock: &'a mut DockWindow,
    graphics: &'a mut DeviceState,
    surface: &'a mut ScheduledSurface<CompositionSurfaceState>,
    window_tracker: &'a mut WindowTracker,
    dock_model: &'a mut DockRuntime,
    auxiliary: &'a mut ModuleHost,
    last_monitor_key: Option<presentation::MonitorPresentationKey>,
}

impl MessageLoop<'_, '_> {
    fn run(&mut self) -> Result<(), AppError> {
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
            result?;
        }
    }

    fn process_message(
        &mut self,
        message: &NativeMessage,
        timing: &mut MessageTiming,
    ) -> Result<(), AppError> {
        if message.is_thread_message()
            && self.heartbeat.matches(message.id(), message.parameter())
        {
            if let Some(input) = self.auxiliary.input() {
                input.heartbeat();
            }
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
        if self.auxiliary.input().is_some() && is_input_wake(message.id()) {
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
        let shell_fullscreen = fullscreen_notification(
            message.is_thread_message(),
            message.id(),
            message.parameter(),
        );
        if let Some(fullscreen) = shell_fullscreen {
            self.window_tracker.set_shell_fullscreen(fullscreen);
            work.insert(RuntimeWork::MONITOR_SYNC);
        }

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
            changed |= self.auxiliary.drain_switcher_events();
            timing.record(UiMessagePhase::SwitcherDrain, started.elapsed());
        }
        if work.contains(RuntimeWork::MONITOR_EVENTS) {
            let started = std::time::Instant::now();
            let outcome = self.auxiliary.drain_monitor_dock_events(self.graphics)?;
            changed |= outcome.had_events;
            for action in outcome.actions {
                dock_events::handle_monitor_dock_action(
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

    fn process_wakes(&mut self, wakes: WakeEvents) -> Result<bool, AppError> {
        let mut changed = false;
        if wakes.update {
            update_events::handle_update_results(self.auxiliary.settings_runtime());
            changed = true;
        }
        if wakes.badges
            && let Some(controller) = self.runtime.taskbar_badges
            && let Ok(snapshot) = controller.snapshot()
        {
            self.dock_model.set_notifications(snapshot);
            self.render_dock();
            changed = true;
        }
        if wakes.media && self.auxiliary.drain_media(self.dock_model) {
            present_dock_change(
                self.dock,
                self.graphics,
                self.surface,
                self.auxiliary,
                self.dock_model,
            )?;
            self.render_dock();
            changed = true;
        }
        if wakes.search_catalog {
            changed |= search_events::refresh_catalog(
                self.dock,
                self.graphics,
                self.surface,
                self.window_tracker.current_windows(),
                self.dock_model,
                self.auxiliary,
            )?;
        }
        if wakes.icon_hydration {
            self.auxiliary.drain_hydrated_icons()?;
            changed = true;
        }

        Ok(changed)
    }

    fn handle_input_wake(&mut self) -> bool {
        self.auxiliary
            .with_input_modules(|controller, launcher, switcher, catalog| {
                controllers::handle_input_actions(&mut controllers::InputEventContext {
                    controller,
                    dock: self.dock,
                    tracker: self.window_tracker,
                    dock_model: self.dock_model,
                    graphics: self.graphics,
                    catalog,
                    launcher,
                    switcher,
                })
            })
            .unwrap_or(false)
    }

    fn render_dock(&mut self) {
        self.surface.invalidate();
    }

    fn drain_settings_events(&mut self) -> Result<bool, AppError> {
        let events = self.auxiliary.drain_settings_events();
        let had_events = !events.is_empty();
        for event in events {
            settings_events::handle_settings_event(
                event,
                &mut settings_events::SettingsEventContext {
                    dock: self.dock,
                    graphics: self.graphics,
                    dock_surface: self.surface,
                    window_tracker: self.window_tracker,
                    dock_model: self.dock_model,
                    auxiliary: self.auxiliary,
                },
            )?;
        }
        self.heartbeat
            .set_enabled(self.auxiliary.input().is_some())?;
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
        )
    }

    fn record_slow_message(
        &self,
        message: &NativeMessage,
        total: std::time::Duration,
        timing: MessageTiming,
        graphics_generation: u64,
    ) {
        let total_us = duration_micros(total);
        if total_us < 50_000 {
            return;
        }
        let (phase, phase_us) = timing.slowest();
        let (aux_dirty, aux_animating, visible_features) =
            self.auxiliary.diagnostic_surface_masks();
        let dirty_surface_mask = aux_dirty | u32::from(self.surface.is_dirty());
        let animating_surface_mask = aux_animating | u32::from(self.surface.is_animating());
        METRICS.record_slow_ui_event(SlowUiEvent {
            timestamp_ms: lotus_windows::interaction::monotonic_millis(),
            message_id: message.id(),
            category: if message.is_thread_message() {
                "thread"
            } else {
                "window"
            },
            total_us,
            accounted_us: timing.accounted_us,
            slowest_phase: phase.name(),
            slowest_phase_us: phase_us,
            window_count: self.window_tracker.current_windows().len(),
            monitor_replica_count: self.auxiliary.monitor_replica_count(),
            dirty_surface_mask,
            animating_surface_mask,
            graphics_generation: self.graphics.generation(),
            graphics_recovered: graphics_generation != self.graphics.generation(),
            visible_feature_mask: visible_features | u32::from(self.dock.is_visible()),
            input_fail_open: self
                .auxiliary
                .input()
                .is_some_and(|input| !input.is_healthy()),
        });
    }
}

#[derive(Clone, Copy, Default)]
struct MessageTiming {
    phase_us: [u64; 9],
    accounted_us: u64,
}

#[derive(Clone, Copy, Default)]
struct EventDrainOutcome {
    animation_tick: bool,
    changed: bool,
}

impl MessageTiming {
    fn record(&mut self, phase: UiMessagePhase, duration: std::time::Duration) {
        let micros = METRICS.record_ui_phase(phase, duration);
        self.phase_us[phase.index()] = self.phase_us[phase.index()].saturating_add(micros);
        self.accounted_us = self.accounted_us.saturating_add(micros);
    }

    fn slowest(self) -> (UiMessagePhase, u64) {
        [
            UiMessagePhase::Tracker,
            UiMessagePhase::Dispatch,
            UiMessagePhase::WindowDrain,
            UiMessagePhase::SettingsDrain,
            UiMessagePhase::SwitcherDrain,
            UiMessagePhase::MonitorDrain,
            UiMessagePhase::Wake,
            UiMessagePhase::MonitorSync,
            UiMessagePhase::Frame,
        ]
        .into_iter()
        .map(|phase| (phase, self.phase_us[phase.index()]))
        .max_by_key(|(_, micros)| *micros)
        .unwrap_or((UiMessagePhase::Dispatch, 0))
    }
}

fn duration_micros(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

#[derive(Clone, Copy)]
struct WakeEvents {
    search_catalog: bool,
    update: bool,
    media: bool,
    badges: bool,
    icon_hydration: bool,
}

impl WakeEvents {
    const fn any(self) -> bool {
        self.search_catalog
            || self.update
            || self.media
            || self.badges
            || self.icon_hydration
    }

    fn from_message(runtime: &RuntimePolicy<'_>, message: u32) -> Self {
        Self {
            search_catalog: is_search_catalog_wake(message),
            update: is_update_wake(message),
            media: is_media_wake(message),
            badges: runtime.taskbar_badges.is_some() && is_taskbar_badge_wake(message),
            icon_hydration: is_icon_hydration_wake(message),
        }
    }
}
