use std::sync::atomic::{AtomicU32, Ordering};

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;

use super::capture::HookState;
use super::{InputAction, Shared, health};
use crate::messages::INPUT_WAKE;
use crate::responsiveness::{InputFailOpenReason, METRICS};

pub(super) enum CyclePush {
    Wake,
    Coalesced,
    Exhausted,
}

pub(super) fn push_cycle(shared: &Shared, sequence: u64, delta: i32) -> CyclePush {
    let generation = u32::try_from(sequence).unwrap_or(u32::MAX);
    let slot = &shared.cycle_slots[(generation & 1) as usize];
    loop {
        let current = slot.load(Ordering::Acquire);
        let (current_generation, current_delta) = unpack_cycle(current);
        let (next_delta, outcome) = match current_generation {
            0 => (delta, CyclePush::Wake),
            current if current == generation => {
                (current_delta.saturating_add(delta), CyclePush::Coalesced)
            }
            _ => return CyclePush::Exhausted,
        };
        let next = pack_cycle(generation, next_delta);
        if slot
            .compare_exchange(current, next, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return outcome;
        }
    }
}

pub(super) fn take_cycles(shared: &Shared, sequence: u64) -> i32 {
    let generation = u32::try_from(sequence).unwrap_or(u32::MAX);
    let slot = &shared.cycle_slots[(generation & 1) as usize];
    loop {
        let current = slot.load(Ordering::Acquire);
        let (current_generation, delta) = unpack_cycle(current);
        if current_generation != generation {
            return 0;
        }
        if slot
            .compare_exchange(current, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return delta;
        }
    }
}

pub(super) fn clear_cycles(shared: &Shared, sequence: u64) {
    let _ = take_cycles(shared, sequence);
}

pub(super) fn enqueue(state: &HookState, action: InputAction) -> bool {
    let depth = state.shared.mailbox_depth.fetch_add(1, Ordering::AcqRel) + 1;
    if state.sender.try_send(action).is_err() {
        state.shared.mailbox_depth.fetch_sub(1, Ordering::AcqRel);
        METRICS.record_input_action_dropped();
        let _ = health::enter_fail_open(&state.shared, InputFailOpenReason::MailboxFull);
        return false;
    }
    METRICS.record_input_action_enqueued();
    METRICS.record_input_mailbox_depth(depth);
    if state.shared.wake_pending.swap(true, Ordering::AcqRel) {
        METRICS.record_input_wake_coalesced();
        return true;
    }
    if unsafe { PostThreadMessageW(state.ui_thread, INPUT_WAKE, WPARAM(0), LPARAM(0)) }
        .is_ok()
    {
        METRICS.record_input_wake_posted();
    } else {
        state.shared.wake_pending.store(false, Ordering::Release);
        METRICS.record_input_wake_failure();
        let _ = health::enter_fail_open(&state.shared, InputFailOpenReason::WakeFailure);
        return false;
    }
    true
}

pub(super) fn subtract_depth(depth: &AtomicU32, count: usize) {
    let count = u32::try_from(count).unwrap_or(u32::MAX);
    let _ = depth.try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
        Some(current.saturating_sub(count))
    });
}

pub(super) fn stamp_action(action: InputAction) -> InputAction {
    let captured_at = health::tick_count();
    match action {
        InputAction::ToggleSearch { sequence, .. } => InputAction::ToggleSearch {
            sequence,
            captured_at,
        },
        InputAction::AltTabBegin {
            sequence,
            direction,
            ..
        } => InputAction::AltTabBegin {
            sequence,
            direction,
            captured_at,
        },
        InputAction::AltTabCommit { sequence, .. } => InputAction::AltTabCommit {
            sequence,
            captured_at,
        },
        other => other,
    }
}

fn pack_cycle(generation: u32, delta: i32) -> u64 {
    (u64::from(generation) << 32) | u64::from(delta.cast_unsigned())
}

fn unpack_cycle(value: u64) -> (u32, i32) {
    let generation = u32::try_from(value >> 32).unwrap_or_default();
    let delta = u32::try_from(value & u64::from(u32::MAX))
        .unwrap_or_default()
        .cast_signed();
    (generation, delta)
}
