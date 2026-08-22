use std::fmt;

use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, PostQuitMessage, TranslateMessage,
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

pub fn request_exit(exit_code: i32) {
    unsafe { PostQuitMessage(exit_code) };
}

pub fn monotonic_millis() -> u64 {
    unsafe { GetTickCount64() }
}
