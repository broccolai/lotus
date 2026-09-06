use std::fmt;

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, PM_REMOVE, PeekMessageW, PostQuitMessage,
    PostThreadMessageW, TranslateMessage, WM_INPUT, WM_KEYFIRST, WM_KEYLAST, WM_MOUSEFIRST,
    WM_MOUSELAST, WM_QUIT, WM_TIMER,
};

use crate::WindowHandle;

pub struct NativeMessage(MSG);

impl NativeMessage {
    pub const fn id(&self) -> u32 {
        self.0.message
    }

    pub const fn parameter(&self) -> usize {
        self.0.wParam.0
    }

    pub const fn is_thread_message(&self) -> bool {
        self.0.hwnd.0.is_null()
    }

    pub const fn target_window(&self) -> Option<WindowHandle> {
        if self.is_thread_message() {
            None
        } else {
            Some(WindowHandle::from_raw(self.0.hwnd))
        }
    }

    pub fn dispatch(&self) {
        unsafe {
            let _ = TranslateMessage(&raw const self.0);
            DispatchMessageW(&raw const self.0);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MessagePumpError;

impl fmt::Display for MessagePumpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GetMessageW failed")
    }
}

pub fn next_message() -> Result<Option<NativeMessage>, MessagePumpError> {
    let mut message = MSG::default();
    match unsafe { GetMessageW(&raw mut message, None, 0, 0) }.0 {
        -1 => Err(MessagePumpError),
        0 => Ok(None),
        _ => Ok(Some(NativeMessage(message))),
    }
}

/// Removes one hardware-input or timer message ahead of an internal continuation.
/// Windows otherwise prioritizes the posted continuation queue over both categories.
pub fn take_priority_message() -> Option<NativeMessage> {
    for (first, last) in [
        (WM_KEYFIRST, WM_KEYLAST),
        (WM_MOUSEFIRST, WM_MOUSELAST),
        (WM_INPUT, WM_INPUT),
        (WM_TIMER, WM_TIMER),
    ] {
        let mut message = MSG::default();
        if unsafe { PeekMessageW(&raw mut message, None, first, last, PM_REMOVE) }.as_bool()
        {
            if message.message == WM_QUIT {
                let code = u32::try_from(message.wParam.0).unwrap_or_default();
                unsafe { PostQuitMessage(i32::from_ne_bytes(code.to_ne_bytes())) };
                return None;
            }
            return Some(NativeMessage(message));
        }
    }
    None
}

pub fn request_exit(exit_code: i32) {
    unsafe { PostQuitMessage(exit_code) };
}

pub fn post_runtime_continuation() -> bool {
    unsafe {
        PostThreadMessageW(
            GetCurrentThreadId(),
            crate::messages::RUNTIME_CONTINUATION,
            WPARAM(0),
            LPARAM(0),
        )
    }
    .is_ok()
}

/// A message-only wake target captured by the UI thread for a bounded worker.
#[derive(Clone, Copy, Debug)]
pub struct UiThreadWake {
    thread_id: u32,
    message: u32,
}

impl UiThreadWake {
    pub fn settings_persistence() -> Self {
        Self {
            thread_id: unsafe { GetCurrentThreadId() },
            message: crate::messages::SETTINGS_PERSISTENCE_WAKE,
        }
    }

    pub fn wake(self) -> bool {
        unsafe { PostThreadMessageW(self.thread_id, self.message, WPARAM(0), LPARAM(0)) }
            .is_ok()
    }
}

pub const fn is_settings_persistence_wake(message: u32) -> bool {
    message == crate::messages::SETTINGS_PERSISTENCE_WAKE
}

pub const fn is_runtime_continuation(message: u32) -> bool {
    message == crate::messages::RUNTIME_CONTINUATION
}

pub fn monotonic_millis() -> u64 {
    unsafe { GetTickCount64() }
}
