use std::mem::size_of;
use std::thread;
use std::time::{Duration, Instant};

use thiserror::Error;
use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_A, VK_LWIN, VK_N, VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowW, GetClassNameW, GetWindowRect, GetWindowThreadProcessId,
    IsWindowVisible, SET_WINDOW_POS_FLAGS, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    SetWindowPos,
};
use windows::core::{BOOL, PCWSTR, w};

use crate::WindowHandle;
use crate::platform::windows::display::nearest_display_to_point;

const INPUT_MARKER: usize = 0x4C4F_5455;
const VK_B: VIRTUAL_KEY = VIRTUAL_KEY(b'B' as u16);
const FOCUS_SETTLE_TIME: Duration = Duration::from_millis(60);
const WINDOW_SETTLE_TIMEOUT: Duration = Duration::from_millis(400);
const WINDOW_SETTLE_RETRY: Duration = Duration::from_millis(16);
const REQUIRED_STABLE_SAMPLES: u8 = 5;
const EDGE_INSET_DIP: i32 = 12;
const WINDOWS_11_BUILD: u32 = 22_000;

#[derive(Debug, Error)]
pub enum TrayError {
    #[error("Windows accepted only {inserted} of {expected} shell-flyout key events")]
    InputIncomplete { inserted: u32, expected: u32 },
}

pub fn open_overflow(owner: WindowHandle) -> Result<(), TrayError> {
    send(&[
        key(VK_LWIN, KEYEVENTF_EXTENDEDKEY),
        key(VK_B, KEYBD_EVENT_FLAGS::default()),
        key(VK_B, KEYEVENTF_KEYUP),
        key(VK_LWIN, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP),
    ])?;
    thread::sleep(FOCUS_SETTLE_TIME);
    send(&[
        key(VK_RETURN, KEYBD_EVENT_FLAGS::default()),
        key(VK_RETURN, KEYEVENTF_KEYUP),
    ])?;
    place_from_owner(owner, find_overflow);
    Ok(())
}

pub fn open_quick_settings(owner: WindowHandle) -> Result<bool, TrayError> {
    open_windows_11_panel(owner, VK_A)
}

pub fn open_calendar(owner: WindowHandle) -> Result<bool, TrayError> {
    open_windows_11_panel(owner, VK_N)
}

fn open_windows_11_panel(
    owner: WindowHandle,
    key_code: VIRTUAL_KEY,
) -> Result<bool, TrayError> {
    if !supports_windows_11_panels() {
        return Ok(false);
    }

    send(&[
        key(VK_LWIN, KEYEVENTF_EXTENDEDKEY),
        key(key_code, KEYBD_EVENT_FLAGS::default()),
        key(key_code, KEYEVENTF_KEYUP),
        key(VK_LWIN, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP),
    ])?;
    place_from_owner(owner, find_shell_panel);
    Ok(true)
}

fn place_from_owner(owner: WindowHandle, find_window: impl FnMut() -> Option<HWND>) {
    let Some(anchor) = window_anchor(owner.raw()) else {
        return;
    };
    place_flyout(anchor.0, anchor.1, find_window);
}

fn place_flyout(
    anchor_x: i32,
    anchor_y: i32,
    mut find_window: impl FnMut() -> Option<HWND>,
) {
    let deadline = Instant::now() + WINDOW_SETTLE_TIMEOUT;
    let mut previous_size = None;
    let mut stable_samples = 0;

    while Instant::now() < deadline {
        let Some(window) = find_window() else {
            thread::sleep(WINDOW_SETTLE_RETRY);
            continue;
        };
        let Some(rect) = visible_window_rect(window) else {
            thread::sleep(WINDOW_SETTLE_RETRY);
            continue;
        };
        let size = (
            rect.right.saturating_sub(rect.left),
            rect.bottom.saturating_sub(rect.top),
        );
        if size.0 <= 0 || size.1 <= 0 {
            thread::sleep(WINDOW_SETTLE_RETRY);
            continue;
        }

        position_window(window, anchor_x, anchor_y, size.0, size.1);
        if previous_size == Some(size) {
            stable_samples += 1;
            if stable_samples >= REQUIRED_STABLE_SAMPLES {
                return;
            }
        } else {
            previous_size = Some(size);
            stable_samples = 1;
        }
        thread::sleep(WINDOW_SETTLE_RETRY);
    }
}

fn position_window(window: HWND, anchor_x: i32, anchor_y: i32, width: i32, height: i32) {
    let Ok(display) = nearest_display_to_point(anchor_x, anchor_y) else {
        return;
    };
    let dpi = display.dpi().map_or(96, lotus_ui::geometry::DpiScale::dpi);
    let inset = EDGE_INSET_DIP.saturating_mul(i32::try_from(dpi).unwrap_or(96)) / 96;
    let maximum_x = display.work_area.right.saturating_sub(width);
    let maximum_y = display.work_area.bottom.saturating_sub(height);
    let x = display
        .work_area
        .right
        .saturating_sub(width)
        .saturating_sub(inset)
        .clamp(
            display.work_area.left,
            maximum_x.max(display.work_area.left),
        );
    let y = anchor_y
        .saturating_sub(height)
        .saturating_sub(inset)
        .clamp(display.work_area.top, maximum_y.max(display.work_area.top));

    // SAFETY: The live shell HWND is only repositioned; size, activation, and z-order remain
    // owned by Windows.
    let _ = unsafe {
        SetWindowPos(
            window,
            None,
            x,
            y,
            0,
            0,
            SET_WINDOW_POS_FLAGS(SWP_NOSIZE.0 | SWP_NOZORDER.0 | SWP_NOACTIVATE.0),
        )
    };
}

fn window_anchor(window: HWND) -> Option<(i32, i32)> {
    let rect = window_rect(window)?;
    Some((rect.right, rect.top))
}

fn visible_window_rect(window: HWND) -> Option<RECT> {
    // SAFETY: The shell owns the HWND and the query tolerates a stale handle.
    if !unsafe { IsWindowVisible(window) }.as_bool() {
        return None;
    }
    window_rect(window)
}

fn window_rect(window: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    // SAFETY: `rect` remains writable for the synchronous window query.
    unsafe { GetWindowRect(window, &raw mut rect) }.ok()?;
    Some(rect)
}

fn find_overflow() -> Option<HWND> {
    [
        w!("TopLevelWindowForOverflowXamlIsland"),
        w!("NotifyIconOverflowWindow"),
    ]
    .into_iter()
    .find_map(|class_name| {
        // SAFETY: The class name is static UTF-16 and the title pointer is null.
        unsafe { FindWindowW(class_name, PCWSTR::null()) }.ok()
    })
}

fn find_shell_panel() -> Option<HWND> {
    let mut result = None;
    // SAFETY: EnumWindows invokes the callback synchronously while `result` remains live.
    let _ = unsafe {
        EnumWindows(
            Some(find_shell_panel_window),
            LPARAM((&raw mut result).addr().cast_signed()),
        )
    };
    result
}

unsafe extern "system" fn find_shell_panel_window(window: HWND, state: LPARAM) -> BOOL {
    let mut process_id = 0;
    // SAFETY: EnumWindows supplied a valid HWND and the process ID output remains writable.
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

    // SAFETY: `state` points to the live result supplied to synchronous EnumWindows.
    unsafe { *(state.0 as *mut Option<HWND>) = Some(window) };
    BOOL(0)
}

fn shell_panel_class(window: HWND) -> bool {
    let mut buffer = [0_u16; 128];
    // SAFETY: The HWND is used only for this query and the UTF-16 buffer is writable.
    let length = unsafe { GetClassNameW(window, &mut buffer) };
    let class_name =
        String::from_utf16_lossy(&buffer[..usize::try_from(length).unwrap_or(0)]);
    class_name == "ControlCenterWindow" || class_name == "Windows.UI.Core.CoreWindow"
}

fn supports_windows_11_panels() -> bool {
    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: u32::try_from(size_of::<OSVERSIONINFOW>()).unwrap_or(u32::MAX),
        ..OSVERSIONINFOW::default()
    };
    // SAFETY: The initialized structure has the required size and remains writable.
    unsafe { RtlGetVersion(&raw mut version) }.is_ok()
        && version.dwBuildNumber >= WINDOWS_11_BUILD
}

fn send(inputs: &[INPUT]) -> Result<(), TrayError> {
    let expected = u32::try_from(inputs.len()).unwrap_or(u32::MAX);
    // SAFETY: Each value is a fully initialized keyboard INPUT and the size matches the exact ABI
    // type supplied to SendInput.
    let inserted = unsafe {
        SendInput(
            inputs,
            i32::try_from(size_of::<INPUT>()).unwrap_or(i32::MAX),
        )
    };
    if inserted == expected {
        Ok(())
    } else {
        Err(TrayError::InputIncomplete { inserted, expected })
    }
}

const fn key(virtual_key: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: virtual_key,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: INPUT_MARKER,
            },
        },
    }
}
