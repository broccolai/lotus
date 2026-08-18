use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, GetWindowThreadProcessId,
};
use windows::core::{BOOL, Error as WindowsError, PCWSTR, w};

pub fn taskbar_window() -> Result<HWND, WindowsError> {
    // SAFETY: Both class and title pointers are static or null and valid for the call.
    unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) }
}

pub fn taskbar_content_host(taskbar: HWND) -> Result<HWND, WindowsError> {
    // SAFETY: `taskbar` is live and both child and title pointers are valid for the call.
    unsafe {
        FindWindowExW(
            Some(taskbar),
            None,
            w!("Windows.UI.Composition.DesktopWindowContentBridge"),
            PCWSTR::null(),
        )
    }
}

pub fn discord_windows(executable_name: &str) -> Vec<HWND> {
    let mut state = WindowSearch {
        executable_name,
        results: Vec::new(),
    };
    // SAFETY: EnumWindows invokes the callback synchronously while `state` remains live.
    let _ = unsafe {
        EnumWindows(
            Some(find_window),
            LPARAM((&raw mut state).addr().cast_signed()),
        )
    };
    state.results
}

struct WindowSearch<'a> {
    executable_name: &'a str,
    results: Vec<HWND>,
}

unsafe extern "system" fn find_window(window: HWND, parameter: LPARAM) -> BOOL {
    // SAFETY: `parameter` points to the live WindowSearch supplied to synchronous EnumWindows.
    let state = unsafe { &mut *(parameter.0 as *mut WindowSearch<'_>) };
    let mut process_id = 0;
    // SAFETY: EnumWindows supplied a valid HWND and the process ID pointer is writable.
    unsafe { GetWindowThreadProcessId(window, Some(&raw mut process_id)) };
    let matches = crate::window_tracker::process_image_path(process_id)
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .is_some_and(|name| name.eq_ignore_ascii_case(state.executable_name));
    if matches {
        state.results.push(window);
    }
    BOOL(1)
}
