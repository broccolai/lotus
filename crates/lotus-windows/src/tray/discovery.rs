use std::mem::size_of;

use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowW, GetClassNameW, GetWindowRect, GetWindowThreadProcessId,
    IsWindowVisible,
};
use windows::core::{BOOL, PCWSTR, w};

const WINDOWS_11_BUILD: u32 = 22_000;

pub(super) fn window_anchor(window: HWND) -> Option<(i32, i32)> {
    let rect = window_rect(window)?;
    Some((rect.right, rect.top))
}

pub(super) fn visible_window_rect(window: HWND) -> Option<RECT> {
    if !unsafe { IsWindowVisible(window) }.as_bool() {
        return None;
    }
    window_rect(window)
}

fn window_rect(window: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(window, &raw mut rect) }.ok()?;
    Some(rect)
}

pub(super) fn find_overflow() -> Option<HWND> {
    [
        w!("TopLevelWindowForOverflowXamlIsland"),
        w!("NotifyIconOverflowWindow"),
    ]
    .into_iter()
    .find_map(|class_name| unsafe { FindWindowW(class_name, PCWSTR::null()) }.ok())
}

pub(super) fn find_shell_panel() -> Option<HWND> {
    let mut result = None;
    let _ = unsafe {
        EnumWindows(
            Some(find_shell_panel_window),
            LPARAM((&raw mut result).addr().cast_signed()),
        )
    };
    result
}

pub(super) fn find_shell_bridge_window() -> Option<HWND> {
    let mut result = None;
    let _ = unsafe {
        EnumWindows(
            Some(find_shell_bridge_window_callback),
            LPARAM((&raw mut result).addr().cast_signed()),
        )
    };
    result
}

unsafe extern "system" fn find_shell_bridge_window_callback(
    window: HWND,
    state: LPARAM,
) -> BOOL {
    let Some(class_name) = window_class_name(window) else {
        return BOOL(1);
    };
    if !shell_panel_class_name(&class_name) {
        return BOOL(1);
    }
    let mut process_id = 0;
    unsafe { GetWindowThreadProcessId(window, Some(&raw mut process_id)) };
    if !process_is_shell_host(process_id) {
        return BOOL(1);
    }
    unsafe { *(state.0 as *mut Option<HWND>) = Some(window) };
    BOOL(0)
}

unsafe extern "system" fn find_shell_panel_window(window: HWND, state: LPARAM) -> BOOL {
    if !unsafe { IsWindowVisible(window) }.as_bool() {
        return BOOL(1);
    }
    let Some(rect) = window_rect(window) else {
        return BOOL(1);
    };
    if rect.right <= rect.left || rect.bottom <= rect.top {
        return BOOL(1);
    }
    let mut process_id = 0;
    unsafe { GetWindowThreadProcessId(window, Some(&raw mut process_id)) };
    let shell_host = crate::window_tracker::process_image_path(process_id)
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .is_some_and(|name| {
            name.eq_ignore_ascii_case("ShellHost.exe")
                || name.eq_ignore_ascii_case("ShellExperienceHost.exe")
        });
    if !shell_host || !shell_panel_class(window) {
        return BOOL(1);
    }
    unsafe { *(state.0 as *mut Option<HWND>) = Some(window) };
    BOOL(0)
}

fn shell_panel_class(window: HWND) -> bool {
    window_class_name(window).is_some_and(|class_name| shell_panel_class_name(&class_name))
}

fn window_class_name(window: HWND) -> Option<String> {
    let mut buffer = [0_u16; 128];
    let length = unsafe { GetClassNameW(window, &mut buffer) };
    let length = usize::try_from(length).ok()?;
    (length != 0).then(|| String::from_utf16_lossy(&buffer[..length]))
}

fn shell_panel_class_name(class_name: &str) -> bool {
    class_name == "ControlCenterWindow" || class_name == "Windows.UI.Core.CoreWindow"
}

fn process_is_shell_host(process_id: u32) -> bool {
    crate::window_tracker::process_image_path(process_id)
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .is_some_and(|name| name.eq_ignore_ascii_case("ShellHost.exe"))
}

pub(super) fn supports_windows_11_panels() -> bool {
    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: u32::try_from(size_of::<OSVERSIONINFOW>()).unwrap_or(u32::MAX),
        ..OSVERSIONINFOW::default()
    };
    unsafe { RtlGetVersion(&raw mut version) }.is_ok()
        && version.dwBuildNumber >= WINDOWS_11_BUILD
}
