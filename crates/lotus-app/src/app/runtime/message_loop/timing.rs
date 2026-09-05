use lotus_windows::interaction::NativeMessage;
use lotus_windows::responsiveness::{METRICS, SlowUiEvent, UiMessagePhase};

use super::MessageLoop;
use crate::app::PresentationSurface;

#[derive(Clone, Copy, Default)]
pub(super) struct MessageTiming {
    phase_us: [u64; 9],
    pub(super) accounted_us: u64,
}

impl MessageTiming {
    pub(super) fn record(&mut self, phase: UiMessagePhase, duration: std::time::Duration) {
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

impl MessageLoop<'_, '_> {
    pub(super) fn record_slow_message(
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
        let dirty_surface_mask = aux_dirty
            | (u32::from(self.primary_dock.is_dirty()) * PresentationSurface::Dock.bit());
        let animating_surface_mask = aux_animating
            | (u32::from(self.primary_dock.is_animating())
                * PresentationSurface::Dock.bit());
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
            visible_feature_mask: visible_features
                | (u32::from(self.primary_dock.window().is_visible())
                    * PresentationSurface::Dock.bit()),
            input_fail_open: !self.auxiliary.input_healthy(),
        });
    }
}

pub(super) fn duration_micros(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}
