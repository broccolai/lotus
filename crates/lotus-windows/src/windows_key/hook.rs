use std::mem::size_of;
use std::sync::{Arc, Mutex, Weak};

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HHOOK, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use super::controller::{
    HookContext, WindowsKeyError, WindowsKeyEvent, emit_event, lock, perform_action,
};
use super::sequence::{HookDecision, KeyEvent, KeyTransition, ReplayKey};
use crate::NativeError;

const LOTUS_INPUT_MARKER: usize = 0x4C4F_5455;
const SUPPRESS_EVENT: LRESULT = LRESULT(1);

static ACTIVE_CONTROLLER: Mutex<Option<Weak<HookContext>>> = Mutex::new(None);

pub(super) struct OwnedKeyboardHook(HHOOK);

impl Drop for OwnedKeyboardHook {
    fn drop(&mut self) {
        // SAFETY: This guard owns the successful SetWindowsHookExW result and
        // releases it exactly once while the callback function remains loaded.
        unsafe {
            let _ = UnhookWindowsHookEx(self.0);
        }
    }
}

pub(super) fn install_hook() -> Result<OwnedKeyboardHook, NativeError> {
    // SAFETY: A null module name requests this process module. Its handle stays
    // loaded for the process lifetime.
    let module = unsafe { GetModuleHandleW(None) }?;
    // SAFETY: The callback has the required ABI and static lifetime. Thread id
    // zero is required for the documented global low-level keyboard hook.
    let hook = unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard_hook),
            Some(HINSTANCE(module.0)),
            0,
        )
    }?;
    Ok(OwnedKeyboardHook(hook))
}

pub(super) fn claim_active_controller(
    context: &Arc<HookContext>,
) -> Result<(), WindowsKeyError> {
    let mut active = lock(&ACTIVE_CONTROLLER);
    if active.as_ref().and_then(Weak::upgrade).is_some() {
        return Err(WindowsKeyError::AlreadyEnabled);
    }

    *active = Some(Arc::downgrade(context));
    Ok(())
}

pub(super) fn release_active_controller(context: &Arc<HookContext>) {
    let mut active = lock(&ACTIVE_CONTROLLER);
    let owns_slot = active
        .as_ref()
        .and_then(Weak::upgrade)
        .is_some_and(|owner| Arc::ptr_eq(&owner, context));
    if owns_slot {
        *active = None;
    }
}

unsafe extern "system" fn keyboard_hook(
    code: i32,
    message: WPARAM,
    data: LPARAM,
) -> LRESULT {
    if code < 0 || data.0 == 0 {
        return call_next(code, message, data);
    }

    let Some(context) = lock(&ACTIVE_CONTROLLER).as_ref().and_then(Weak::upgrade) else {
        return call_next(code, message, data);
    };
    // SAFETY: For nonnegative WH_KEYBOARD_LL callbacks, Windows documents
    // lParam as a valid pointer to KBDLLHOOKSTRUCT for the callback duration.
    let keyboard = unsafe { &*(data.0 as *const KBDLLHOOKSTRUCT) };
    let Ok(message_id) = u32::try_from(message.0) else {
        return call_next(code, message, data);
    };
    let Some(transition) = transition_from_message(message_id) else {
        return call_next(code, message, data);
    };
    let Ok(virtual_key) = u16::try_from(keyboard.vkCode) else {
        return call_next(code, message, data);
    };
    let event = KeyEvent {
        virtual_key,
        transition,
        extended: keyboard.flags.contains(LLKHF_EXTENDED),
        self_injected: keyboard.dwExtraInfo == LOTUS_INPUT_MARKER,
    };
    let decision = lock(&context.sequence).transition(event);

    match decision {
        HookDecision::Pass => call_next(code, message, data),
        HookDecision::Suppress => SUPPRESS_EVENT,
        HookDecision::Act(action) => {
            perform_action(&context, action);
            SUPPRESS_EVENT
        }
    }
}

pub(super) fn send_replay(context: &HookContext, replay: &[ReplayKey]) -> bool {
    let inputs = replay
        .iter()
        .copied()
        .map(input_from_replay)
        .collect::<Vec<_>>();
    let expected = u32::try_from(inputs.len()).unwrap_or(u32::MAX);
    // SAFETY: INPUT values are fully initialized keyboard variants and the
    // byte size matches the Win32 INPUT structure used by this crate version.
    let inserted = unsafe {
        SendInput(
            &inputs,
            i32::try_from(size_of::<INPUT>()).unwrap_or(i32::MAX),
        )
    };
    if inserted == expected {
        return true;
    }

    emit_event(
        context,
        WindowsKeyEvent::ReplayIncomplete { inserted, expected },
    );
    false
}

fn call_next(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    // SAFETY: Forwarding the untouched callback parameters is required for all
    // events Lotus does not consume. The hook handle parameter is ignored for
    // low-level hooks and may be None.
    unsafe { CallNextHookEx(None, code, message, data) }
}

fn input_from_replay(replay: ReplayKey) -> INPUT {
    let mut flags = if replay.extended {
        KEYEVENTF_EXTENDEDKEY
    } else {
        KEYBD_EVENT_FLAGS::default()
    };
    if replay.transition == KeyTransition::Up {
        flags |= KEYEVENTF_KEYUP;
    }

    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(replay.virtual_key),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: LOTUS_INPUT_MARKER,
            },
        },
    }
}

fn transition_from_message(message: u32) -> Option<KeyTransition> {
    match message {
        WM_KEYDOWN | WM_SYSKEYDOWN => Some(KeyTransition::Down),
        WM_KEYUP | WM_SYSKEYUP => Some(KeyTransition::Up),
        _ => None,
    }
}
