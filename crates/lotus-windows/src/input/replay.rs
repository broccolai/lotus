use std::mem::size_of;
use std::sync::atomic::Ordering;

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;

use super::capture::HOOK_STATE;
use super::health;
use super::state::{AltFallback, Transition};
use crate::messages::ALT_TAB_FALLBACK_REPLAY;
use crate::responsiveness::{InputFailOpenReason, METRICS};

pub(super) const LOTUS_INPUT_MARKER: usize = 0x4C4F_5455;
const SILENT_START_CANCELLATION_KEY: VIRTUAL_KEY = VIRTUAL_KEY(0xE8);
const ALT_TAB_FALLBACK_INPUT_CAPACITY: usize = 16;

#[derive(Clone, Copy)]
struct InjectedKey {
    key: u16,
    transition: Transition,
    extended: bool,
}

pub(super) fn queue_alt_tab_fallback(thread_id: u32) -> bool {
    if thread_id == 0 {
        return false;
    }

    unsafe { PostThreadMessageW(thread_id, ALT_TAB_FALLBACK_REPLAY, WPARAM(0), LPARAM(0)) }
        .is_ok()
}

pub(super) fn flush_alt_tab_fallback() -> bool {
    HOOK_STATE.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        let Some(state) = borrowed.as_mut() else {
            return true;
        };
        if state.alt_fallback.is_none() {
            let requested = state.shared.cleanup_requested_epoch.load(Ordering::Acquire);
            health::mark_cleanup_complete(&state.shared, requested);
            return true;
        }
        let shared = std::sync::Arc::clone(&state.shared);
        let requested = shared.cleanup_requested_epoch.load(Ordering::Acquire);
        let alt_fallback = state.alt_fallback;
        drop(borrowed);
        if let Some(fallback) = alt_fallback
            && !replay_alt_fallback(fallback)
        {
            METRICS.record_input_replay_failure();
            let mut borrowed = slot.borrow_mut();
            if let Some(state) = borrowed.as_mut() {
                let _ = health::enter_fail_open(
                    &state.shared,
                    InputFailOpenReason::WakeFailure,
                );
            }
            return false;
        }
        let mut borrowed = slot.borrow_mut();
        if let Some(state) = borrowed.as_mut() {
            state.alt_fallback = None;
        }
        health::mark_cleanup_complete(&shared, requested);
        true
    })
}

fn replay_alt_fallback(fallback: AltFallback) -> bool {
    let direction = fallback.steps.signum();
    for _ in 0..fallback.steps.unsigned_abs() {
        let keys = alt_fallback_keys(fallback, direction);
        if !send_alt_tab_fallback_keys(&keys) {
            return false;
        }
    }
    true
}

fn alt_fallback_keys(
    fallback: AltFallback,
    direction: i32,
) -> [Option<InjectedKey>; ALT_TAB_FALLBACK_INPUT_CAPACITY] {
    let reverse = direction < 0;
    let mut keys = [None; ALT_TAB_FALLBACK_INPUT_CAPACITY];
    let mut index = 0;
    if !reverse {
        index = append_shift_keys(&mut keys, index, fallback.shift_mask, Transition::Up);
    }
    if !fallback.alt_is_held {
        keys[index] = Some(alt_key(fallback.alt_key, Transition::Down));
        index += 1;
    }
    if reverse && fallback.shift_mask == 0 {
        keys[index] = Some(InjectedKey {
            key: 0x10,
            transition: Transition::Down,
            extended: false,
        });
        index += 1;
    }
    keys[index] = Some(InjectedKey {
        key: 0x09,
        transition: Transition::Down,
        extended: false,
    });
    index += 1;
    keys[index] = Some(InjectedKey {
        key: 0x09,
        transition: Transition::Up,
        extended: false,
    });
    index += 1;
    if reverse && fallback.shift_mask == 0 {
        keys[index] = Some(InjectedKey {
            key: 0x10,
            transition: Transition::Up,
            extended: false,
        });
        index += 1;
    }
    if !fallback.alt_is_held {
        keys[index] = Some(alt_key(fallback.alt_key, Transition::Up));
        index += 1;
    }
    if !reverse {
        let _end =
            append_shift_keys(&mut keys, index, fallback.shift_mask, Transition::Down);
    }
    keys
}

fn append_shift_keys(
    keys: &mut [Option<InjectedKey>; ALT_TAB_FALLBACK_INPUT_CAPACITY],
    mut index: usize,
    mask: u8,
    transition: Transition,
) -> usize {
    for (bit, key) in [(0b001, 0xA0), (0b010, 0xA1), (0b100, 0x10)] {
        if mask & bit != 0 {
            keys[index] = Some(InjectedKey {
                key,
                transition,
                extended: false,
            });
            index += 1;
        }
    }
    index
}

fn alt_key(key: u16, transition: Transition) -> InjectedKey {
    InjectedKey {
        key,
        transition,
        extended: key == 0xA5,
    }
}

fn send_alt_tab_fallback_keys(keys: &[Option<InjectedKey>]) -> bool {
    let mut inputs = [empty_input(); ALT_TAB_FALLBACK_INPUT_CAPACITY];
    let count = keys
        .iter()
        .flatten()
        .enumerate()
        .map(|(index, key)| {
            inputs[index] = input_from_injected_key(*key);
            index + 1
        })
        .last()
        .unwrap_or_default();
    if count == 0 {
        return true;
    }
    let expected = u32::try_from(count).unwrap_or_default();
    let inserted = unsafe {
        SendInput(
            &inputs[..count],
            i32::try_from(size_of::<INPUT>()).unwrap_or_default(),
        )
    };
    let inserted_count = usize::try_from(inserted).unwrap_or_default().min(count);
    let retry = if inserted_count < count {
        unsafe {
            SendInput(
                &inputs[inserted_count..count],
                i32::try_from(size_of::<INPUT>()).unwrap_or_default(),
            )
        }
    } else {
        0
    };
    inserted.saturating_add(retry) == expected
}

fn empty_input() -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT::default(),
        },
    }
}

fn input_from_injected_key(key: InjectedKey) -> INPUT {
    let mut flags = if key.extended {
        KEYEVENTF_EXTENDEDKEY
    } else {
        KEYBD_EVENT_FLAGS::default()
    };
    if key.transition == Transition::Up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(key.key),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: LOTUS_INPUT_MARKER,
            },
        },
    }
}

pub(super) fn cancel_native_start() -> bool {
    let keys = [
        InjectedKey {
            key: SILENT_START_CANCELLATION_KEY.0,
            transition: Transition::Down,
            extended: false,
        },
        InjectedKey {
            key: SILENT_START_CANCELLATION_KEY.0,
            transition: Transition::Up,
            extended: false,
        },
    ];
    let inputs = keys.map(input_from_injected_key);
    let inserted = unsafe {
        SendInput(
            &inputs,
            i32::try_from(size_of::<INPUT>()).unwrap_or_default(),
        )
    };
    if inserted == 2 {
        return true;
    }
    if inserted == 1 {
        let key_up = [input_from_injected_key(keys[1])];
        let _ = unsafe {
            SendInput(
                &key_up,
                i32::try_from(size_of::<INPUT>()).unwrap_or_default(),
            )
        };
    }
    false
}
