use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    IsWindow, IsWindowVisible, SW_HIDE, SW_SHOWNOACTIVATE, ShowWindowAsync,
};

use super::taskbar_windows::taskbar_windows;

/// Restores every taskbar window that this transaction actually hid.
pub(super) struct TaskbarVisibilityTransaction {
    hidden_windows: Vec<HWND>,
}

impl TaskbarVisibilityTransaction {
    pub(super) const fn start() -> Self {
        Self {
            hidden_windows: Vec::new(),
        }
    }

    pub(super) fn hide_existing(&mut self) {
        for hwnd in taskbar_windows() {
            self.hide(hwnd);
        }
    }

    pub(super) fn hide(&mut self, hwnd: HWND) {
        // SAFETY: The callback or current shell lookup supplied a live top-level taskbar HWND.
        if !unsafe { IsWindowVisible(hwnd).as_bool() } {
            return;
        }
        if !self.hidden_windows.contains(&hwnd) {
            self.hidden_windows.push(hwnd);
        }
        // SAFETY: Hiding an exact taskbar-class HWND is reversible and its visibility is journaled.
        let _ = unsafe { ShowWindowAsync(hwnd, SW_HIDE) };
    }

    fn restore(&self) {
        for &hwnd in &self.hidden_windows {
            // SAFETY: The handle is only used when Windows still recognizes it.
            if unsafe { IsWindow(Some(hwnd)).as_bool() } {
                // SAFETY: This restores only a taskbar window that was visible when
                // the guardian first observed it, without activating it.
                let _ = unsafe { ShowWindowAsync(hwnd, SW_SHOWNOACTIVATE) };
            }
        }
    }
}

impl Drop for TaskbarVisibilityTransaction {
    fn drop(&mut self) {
        self.restore();
    }
}
