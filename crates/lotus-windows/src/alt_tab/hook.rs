use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HHOOK, KBDLLHOOKSTRUCT, LLKHF_ALTDOWN, PostThreadMessageW,
    SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL,
};

use super::sequence::{Decision, KeyEvent, Sequence, Transition};
use super::{ALT_TAB_WAKE_MESSAGE, AltTabError, AltTabEvent};
use crate::{NativeError, diagnostics};

const SUPPRESS: LRESULT = LRESULT(1);

static ACTIVE_CONTROLLER: Mutex<Option<Weak<HookContext>>> = Mutex::new(None);

pub(super) struct HookContext {
    state: Mutex<Sequence>,
    events: Sender<AltTabEvent>,
    owner_thread: u32,
}

impl HookContext {
    pub(super) fn new(events: Sender<AltTabEvent>) -> Self {
        let owner_thread = unsafe { GetCurrentThreadId() };

        Self {
            state: Mutex::new(Sequence::default()),
            events,
            owner_thread,
        }
    }

    pub(super) fn cancel(&self) -> bool {
        lock(&self.state).cancel()
    }

    pub(super) fn emit(&self, event: AltTabEvent) {
        if self.events.send(event).is_err() {
            return;
        }

        let _ = unsafe {
            PostThreadMessageW(
                self.owner_thread,
                ALT_TAB_WAKE_MESSAGE,
                WPARAM(0),
                LPARAM(0),
            )
        };
    }
}

pub(super) struct OwnedHook(HHOOK);

impl Drop for OwnedHook {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWindowsHookEx(self.0);
        }
    }
}

pub(super) fn install() -> Result<OwnedHook, NativeError> {
    let module = unsafe { GetModuleHandleW(None) }?;
    let hook = unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard_hook),
            Some(HINSTANCE(module.0)),
            0,
        )
    }?;

    Ok(OwnedHook(hook))
}

pub(super) fn claim(context: &Arc<HookContext>) -> Result<(), AltTabError> {
    let mut active = lock(&ACTIVE_CONTROLLER);
    if active.as_ref().and_then(Weak::upgrade).is_some() {
        return Err(AltTabError::AlreadyEnabled);
    }

    *active = Some(Arc::downgrade(context));
    Ok(())
}

pub(super) fn release(context: &Arc<HookContext>) {
    let mut active = lock(&ACTIVE_CONTROLLER);
    if active
        .as_ref()
        .and_then(Weak::upgrade)
        .is_some_and(|owner| Arc::ptr_eq(&owner, context))
    {
        *active = None;
    }
}

unsafe extern "system" fn keyboard_hook(
    code: i32,
    message: WPARAM,
    data: LPARAM,
) -> LRESULT {
    if let Ok(result) = catch_unwind(AssertUnwindSafe(|| unsafe {
        keyboard_hook_inner(code, message, data)
    })) {
        result
    } else {
        recover_from_hook_panic();
        call_next(code, message, data)
    }
}

unsafe fn keyboard_hook_inner(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    if code < 0 || data.0 == 0 {
        return call_next(code, message, data);
    }
    let Some(context) = lock(&ACTIVE_CONTROLLER).as_ref().and_then(Weak::upgrade) else {
        return call_next(code, message, data);
    };
    let keyboard = unsafe { &*(data.0 as *const KBDLLHOOKSTRUCT) };
    let original_message = message;
    let Ok(message) = u32::try_from(original_message.0) else {
        return call_next(code, original_message, data);
    };
    let Some(transition) = Transition::from_message(message) else {
        return call_next(code, WPARAM(message as usize), data);
    };
    let Ok(key) = u16::try_from(keyboard.vkCode) else {
        return call_next(code, WPARAM(message as usize), data);
    };
    let action = lock(&context.state).transition(KeyEvent {
        key,
        transition,
        alt_flag: keyboard.flags.contains(LLKHF_ALTDOWN),
    });

    match action {
        Decision::Pass => call_next(code, WPARAM(message as usize), data),
        Decision::Suppress => SUPPRESS,
        Decision::Emit(event) => {
            context.emit(event);
            SUPPRESS
        }
        Decision::EmitAndPass(event) => {
            context.emit(event);
            call_next(code, WPARAM(message as usize), data)
        }
    }
}

fn recover_from_hook_panic() {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Some(context) = lock(&ACTIVE_CONTROLLER).as_ref().and_then(Weak::upgrade)
            && context.cancel()
        {
            context.emit(AltTabEvent::Cancel);
        }
        diagnostics::record_message("alt_tab.callback", "the custom Alt+Tab hook panicked");
    }));
}

fn call_next(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    unsafe { CallNextHookEx(None, code, message, data) }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
