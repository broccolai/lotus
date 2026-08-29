use std::sync::atomic::Ordering;

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;

use super::capture::{self, HOOK_STATE};
use super::{Shared, lock_decision, replay};
use crate::messages::{INPUT_RESYNC, INPUT_WAKE};
use crate::responsiveness::{InputFailOpenReason, METRICS};

pub(super) const HEARTBEAT_INTERVAL_MS: u32 = 75;
const UI_HEARTBEAT_STALE_MS: u64 = 1_000;

pub(super) fn heartbeat_stale(shared: &Shared) -> bool {
    tick_count().saturating_sub(shared.heartbeat.load(Ordering::Acquire))
        > UI_HEARTBEAT_STALE_MS
}

pub(super) fn enter_fail_open(shared: &Shared, reason: InputFailOpenReason) -> bool {
    let was_open = shared.fail_open.swap(true, Ordering::AcqRel);
    if !was_open {
        METRICS.record_input_fail_open(reason);
        let _ = request_cleanup(shared);
    }
    shared.healthy_heartbeats.store(0, Ordering::Release);
    for slot in &shared.cycle_slots {
        slot.store(0, Ordering::Release);
    }
    !was_open
}

pub(super) fn request_cleanup(shared: &Shared) -> bool {
    let mut requested = shared.cleanup_requested_epoch.load(Ordering::Acquire);
    loop {
        if shared.cleanup_completed_epoch.load(Ordering::Acquire) != requested {
            METRICS.record_input_cleanup_redundant_suppressed();
            return false;
        }
        match shared.cleanup_requested_epoch.compare_exchange_weak(
            requested,
            requested.saturating_add(1),
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                METRICS.record_input_cleanup_requested();
                return true;
            }
            Err(current) => requested = current,
        }
    }
}

pub(super) fn request_cleanup_for_sequence(shared: &Shared, sequence: u64) -> bool {
    if sequence == 0 {
        return false;
    }
    let mut covered = shared.cleanup_sequence.load(Ordering::Acquire);
    while sequence > covered {
        match shared.cleanup_sequence.compare_exchange_weak(
            covered,
            sequence,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return request_cleanup(shared),
            Err(current) => covered = current,
        }
    }
    METRICS.record_input_cleanup_redundant_suppressed();
    false
}

pub(super) fn cleanup_is_complete(shared: &Shared) -> bool {
    shared.cleanup_completed_epoch.load(Ordering::Acquire)
        == shared.cleanup_requested_epoch.load(Ordering::Acquire)
}

pub(super) fn try_recover_from_fail_open(shared: &Shared) {
    let _decision = lock_decision(shared);
    let requested = shared.cleanup_requested_epoch.load(Ordering::Acquire);
    let completed = shared.cleanup_completed_epoch.load(Ordering::Acquire);
    if shared.fail_open.load(Ordering::Acquire) && requested == completed {
        if shared.healthy_heartbeats.fetch_add(1, Ordering::AcqRel) >= 2 {
            shared.fail_open.store(false, Ordering::Release);
            if shared.cleanup_requested_epoch.load(Ordering::Acquire) == completed {
                request_pressed_key_resync(shared);
            } else {
                shared.fail_open.store(true, Ordering::Release);
            }
            shared.healthy_heartbeats.store(0, Ordering::Release);
        }
    } else {
        shared.healthy_heartbeats.store(0, Ordering::Release);
    }
}

fn request_pressed_key_resync(shared: &Shared) {
    shared
        .pressed_resync_requested
        .store(true, Ordering::Release);
    let thread_id = shared.worker_thread.load(Ordering::Acquire);
    if thread_id != 0 {
        let _ =
            unsafe { PostThreadMessageW(thread_id, INPUT_RESYNC, WPARAM(0), LPARAM(0)) };
    }
}

pub(super) fn watchdog_tick() {
    resync_pressed_keys_if_requested();
    if heartbeat_stale_shared() {
        HOOK_STATE.with(|slot| {
            if let Some(state) = slot.borrow().as_ref() {
                let _ = enter_fail_open(&state.shared, InputFailOpenReason::HeartbeatStale);
            }
        });
    }
    cleanup_input_state();
}

fn heartbeat_stale_shared() -> bool {
    HOOK_STATE.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|state| heartbeat_stale(&state.shared))
    })
}

pub(super) fn resync_pressed_keys_if_requested() {
    let requested = HOOK_STATE.with(|slot| {
        slot.borrow().as_ref().is_some_and(|state| {
            state
                .shared
                .pressed_resync_requested
                .swap(false, Ordering::AcqRel)
        })
    });
    if requested {
        capture::resync_pressed_keys();
    }
}

pub(super) fn cleanup_input_state() {
    let mut direct_alt_tab_fallback = false;
    HOOK_STATE.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        let Some(state) = borrowed.as_mut() else {
            return;
        };
        let requested = state.shared.cleanup_requested_epoch.load(Ordering::Acquire);
        if state.shared.cleanup_completed_epoch.load(Ordering::Acquire) == requested {
            return;
        }

        let active_cleanup_state = state.sequence.has_active_cleanup_state();
        let sequence = state.sequence.invalidate();
        state
            .shared
            .cleanup_sequence
            .fetch_max(sequence, Ordering::AcqRel);
        let (acknowledged, rejected) = {
            let mut decision = lock_decision(&state.shared);
            decision.invalid_before =
                decision.invalid_before.max(sequence.saturating_add(1));
            (
                decision.claimed_sequence,
                decision.rejected_sequence == sequence,
            )
        };
        let (_, alt_fallback) = state.sequence.fail_open_cleanup(if rejected {
            0
        } else {
            acknowledged
        });
        state.alt_fallback = state.alt_fallback.or(alt_fallback);
        for slot in &state.shared.cycle_slots {
            slot.store(0, Ordering::Release);
        }
        if active_cleanup_state {
            state
                .shared
                .cancelled_sequence
                .store(sequence, Ordering::Release);
            METRICS.record_input_sequence_cancel();
            METRICS.record_input_cleanup_active_sequence_cancel();
            signal_input_wake(state);
        } else {
            METRICS.record_input_cleanup_idle();
        }
        if state.alt_fallback.is_none() {
            mark_cleanup_complete(&state.shared, requested);
        } else if !replay::queue_alt_tab_fallback(
            state.shared.worker_thread.load(Ordering::Acquire),
        ) {
            direct_alt_tab_fallback = true;
        }
    });
    if direct_alt_tab_fallback {
        let _ = replay::flush_alt_tab_fallback();
    }
}

pub(super) fn mark_cleanup_complete(shared: &Shared, epoch: u64) {
    let _decision = lock_decision(shared);
    if shared.cleanup_requested_epoch.load(Ordering::Acquire) == epoch
        && shared.cleanup_completed_epoch.load(Ordering::Acquire) != epoch
    {
        shared
            .cleanup_completed_epoch
            .store(epoch, Ordering::Release);
        METRICS.record_input_cleanup_completed();
    }
}

fn signal_input_wake(state: &capture::HookState) {
    if state.shared.wake_pending.swap(true, Ordering::AcqRel) {
        return;
    }
    if unsafe { PostThreadMessageW(state.ui_thread, INPUT_WAKE, WPARAM(0), LPARAM(0)) }
        .is_err()
    {
        state.shared.wake_pending.store(false, Ordering::Release);
    }
}

pub(super) fn tick_count() -> u64 {
    unsafe { GetTickCount64() }
}
