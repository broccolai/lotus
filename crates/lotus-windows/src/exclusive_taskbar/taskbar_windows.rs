use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, FindWindowW, GetClassNameW};
use windows::core::{PCWSTR, w};

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
