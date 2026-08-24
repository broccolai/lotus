use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use crate::NativeError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WindowTimer {
    id: usize,
    interval_ms: u32,
}

impl WindowTimer {
    pub(crate) const fn new(id: usize, interval_ms: u32) -> Self {
        Self { id, interval_ms }
    }

    pub(crate) const fn matches(self, id: usize) -> bool {
        self.id == id
    }

    pub(crate) fn start(self, hwnd: HWND) -> Result<(), NativeError> {
        self.start_with_interval(hwnd, self.interval_ms)
    }

    pub(crate) fn start_with_interval(
        self,
        hwnd: HWND,
        interval_ms: u32,
    ) -> Result<(), NativeError> {
        if unsafe { SetTimer(Some(hwnd), self.id, interval_ms, None) } == 0 {
            return Err(windows::core::Error::from_thread().into());
        }
        Ok(())
    }

    pub(crate) fn stop(self, hwnd: HWND) {
        let _ = unsafe { KillTimer(Some(hwnd), self.id) };
    }
}
