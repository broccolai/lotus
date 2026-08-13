use std::mem::size_of;

use thiserror::Error;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_LWIN,
};

const LOTUS_INPUT_MARKER: usize = 0x4C4F_5455;
const VK_D: VIRTUAL_KEY = VIRTUAL_KEY(b'D' as u16);

#[derive(Debug, Error)]
#[error("Windows accepted only {inserted} of {expected} Show Desktop key events")]
pub struct ShowDesktopError {
    inserted: u32,
    expected: u32,
}

pub fn toggle() -> Result<(), ShowDesktopError> {
    let inputs = [
        key(VK_LWIN, KEYEVENTF_EXTENDEDKEY),
        key(VK_D, KEYBD_EVENT_FLAGS::default()),
        key(VK_D, KEYEVENTF_KEYUP),
        key(VK_LWIN, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP),
    ];
    let expected = u32::try_from(inputs.len()).unwrap_or(u32::MAX);
    // SAFETY: Each value is a fully initialized keyboard INPUT and the size
    // matches the exact ABI type supplied to SendInput.
    let inserted =
        unsafe { SendInput(&inputs, i32::try_from(size_of::<INPUT>()).unwrap_or(i32::MAX)) };
    if inserted == expected { Ok(()) } else { Err(ShowDesktopError { inserted, expected }) }
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
                dwExtraInfo: LOTUS_INPUT_MARKER,
            },
        },
    }
}
