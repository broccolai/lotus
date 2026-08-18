mod discovery;
mod input;
mod placement;

use std::thread;
use std::time::Duration;

use thiserror::Error;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_A, VK_LWIN,
    VK_N, VK_RETURN,
};

use crate::WindowHandle;
use crate::shell_bridge::ShellBridgeLease;

const FOCUS_SETTLE_TIME: Duration = Duration::from_millis(60);
const VK_B: VIRTUAL_KEY = VIRTUAL_KEY(b'B' as u16);

#[derive(Debug, Error)]
pub enum TrayError {
    #[error("Windows accepted only {inserted} of {expected} shell-flyout key events")]
    InputIncomplete { inserted: u32, expected: u32 },
}

pub fn open_overflow(owner: WindowHandle) -> Result<(), TrayError> {
    open_overflow_with_anchor(owner, None)
}

pub fn open_overflow_at(owner: WindowHandle, screen_x: i32) -> Result<(), TrayError> {
    open_overflow_with_anchor(owner, Some(screen_x))
}

fn open_overflow_with_anchor(
    owner: WindowHandle,
    screen_x: Option<i32>,
) -> Result<(), TrayError> {
    input::send(&[
        input::key(VK_LWIN, KEYEVENTF_EXTENDEDKEY),
        input::key(VK_B, KEYBD_EVENT_FLAGS::default()),
        input::key(VK_B, KEYEVENTF_KEYUP),
        input::key(VK_LWIN, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP),
    ])?;
    thread::sleep(FOCUS_SETTLE_TIME);
    input::send(&[
        input::key(VK_RETURN, KEYBD_EVENT_FLAGS::default()),
        input::key(VK_RETURN, KEYEVENTF_KEYUP),
    ])?;
    place_from_owner(owner, screen_x, None, discovery::find_overflow);
    Ok(())
}

pub fn open_quick_settings(owner: WindowHandle) -> Result<bool, TrayError> {
    open_windows_11_panel(owner, None, VK_A)
}

pub fn open_quick_settings_at(
    owner: WindowHandle,
    screen_x: i32,
) -> Result<bool, TrayError> {
    open_windows_11_panel(owner, Some(screen_x), VK_A)
}

pub fn open_calendar(owner: WindowHandle) -> Result<bool, TrayError> {
    open_windows_11_panel(owner, None, VK_N)
}

pub fn open_calendar_at(owner: WindowHandle, screen_x: i32) -> Result<bool, TrayError> {
    open_windows_11_panel(owner, Some(screen_x), VK_N)
}

fn open_windows_11_panel(
    owner: WindowHandle,
    screen_x: Option<i32>,
    key_code: VIRTUAL_KEY,
) -> Result<bool, TrayError> {
    if !discovery::supports_windows_11_panels() {
        return Ok(false);
    }

    let owner_window = owner.raw();
    let Some(anchor) = discovery::window_anchor(owner_window) else {
        return Ok(true);
    };
    let bridge_window = discovery::find_shell_bridge_window();
    let bridge =
        bridge_window.and_then(|window| ShellBridgeLease::attach(window, owner_window));
    if let Some(bridge) = bridge.as_ref() {
        let _ = bridge.configure(screen_x.unwrap_or(anchor.0), anchor.1);
    }

    input::send(&[
        input::key(VK_LWIN, KEYEVENTF_EXTENDEDKEY),
        input::key(key_code, KEYBD_EVENT_FLAGS::default()),
        input::key(key_code, KEYEVENTF_KEYUP),
        input::key(VK_LWIN, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP),
    ])?;

    placement::place_flyout(
        screen_x,
        anchor.0,
        anchor.1,
        bridge.as_ref(),
        discovery::find_shell_panel,
    );
    Ok(true)
}

fn place_from_owner(
    owner: WindowHandle,
    screen_x: Option<i32>,
    bridge: Option<&ShellBridgeLease>,
    find_window: impl FnMut() -> Option<HWND>,
) {
    let Some(anchor) = discovery::window_anchor(owner.raw()) else {
        return;
    };
    placement::place_flyout(screen_x, anchor.0, anchor.1, bridge, find_window);
}
