use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetClassNameW, GetWindowThreadProcessId, IsWindow,
    SW_SHOWNOACTIVATE, ShowWindowAsync,
};
use windows::core::{PCWSTR, w};

use crate::window_tracker::process_image_path;

/// The exact native taskbar classes Lotus is permitted to hide.
pub(super) fn is_taskbar_window(hwnd: HWND) -> bool {
    let mut class_name = [0u16; 32];
    // SAFETY: `class_name` is writable for the duration of this synchronous query.
    let length = unsafe { GetClassNameW(hwnd, &mut class_name) };
    let Ok(length) = usize::try_from(length) else {
        return false;
    };

    matches!(
        String::from_utf16_lossy(&class_name[..length]).as_str(),
        "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
    )
}

#[derive(Clone, Copy)]
pub(super) struct TaskbarWindowIdentity {
    hwnd: HWND,
    process_id: u32,
    thread_id: u32,
}

impl TaskbarWindowIdentity {
    pub(super) fn capture(hwnd: HWND) -> Option<Self> {
        if !is_taskbar_window(hwnd) {
            return None;
        }
        let mut process_id = 0;
        let thread_id =
            unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut process_id)) };
        (thread_id != 0 && trusted_explorer(process_id)).then_some(Self {
            hwnd,
            process_id,
            thread_id,
        })
    }

    pub(super) fn hwnd(self) -> HWND {
        self.hwnd
    }

    pub(super) fn still_matches(self) -> bool {
        if !unsafe { IsWindow(Some(self.hwnd)) }.as_bool() || !is_taskbar_window(self.hwnd)
        {
            return false;
        }
        let mut process_id = 0;
        let thread_id =
            unsafe { GetWindowThreadProcessId(self.hwnd, Some(&raw mut process_id)) };
        thread_id == self.thread_id
            && process_id == self.process_id
            && trusted_explorer(process_id)
    }
}

fn trusted_explorer(process_id: u32) -> bool {
    let Some(actual) = process_image_path(process_id) else {
        return false;
    };
    let Some(system_root) = env::var_os("SystemRoot") else {
        return false;
    };
    actual.as_os_str().to_string_lossy().eq_ignore_ascii_case(
        PathBuf::from(system_root)
            .join("explorer.exe")
            .as_os_str()
            .to_string_lossy()
            .as_ref(),
    )
}

pub(super) fn taskbar_windows() -> Vec<HWND> {
    let mut windows = Vec::new();
    // SAFETY: Static class strings are NUL-terminated and a null title accepts any title.
    if let Ok(primary) = unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) } {
        windows.push(primary);
    }

    let mut previous = None;
    loop {
        // SAFETY: The search is restricted to top-level windows of the exact secondary
        // taskbar class; `previous` is either null or a handle returned by this loop.
        let Ok(hwnd) = (unsafe {
            FindWindowExW(None, previous, w!("Shell_SecondaryTrayWnd"), PCWSTR::null())
        }) else {
            break;
        };
        windows.push(hwnd);
        previous = Some(hwnd);
    }

    windows
}

pub(super) fn restore_verified_taskbars() {
    for hwnd in taskbar_windows() {
        if TaskbarWindowIdentity::capture(hwnd).is_some() {
            let _ = unsafe { ShowWindowAsync(hwnd, SW_SHOWNOACTIVATE) };
        }
    }
}
use std::env;
use std::path::PathBuf;
