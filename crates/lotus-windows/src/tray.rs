use std::mem::size_of;
use std::thread;
use std::time::Duration;

use thiserror::Error;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_LWIN, VK_RETURN,
};

const INPUT_MARKER: usize = 0x4C4F_5455;
const VK_B: VIRTUAL_KEY = VIRTUAL_KEY(b'B' as u16);
const FOCUS_SETTLE_TIME: Duration = Duration::from_millis(60);

#[derive(Debug, Error)]
pub enum TrayError {
    #[error("Windows accepted only {inserted} of {expected} notification-area key events")]
    InputIncomplete { inserted: u32, expected: u32 },
}

pub fn open_overflow() -> Result<(), TrayError> {
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
    ])
}

fn send(inputs: &[INPUT]) -> Result<(), TrayError> {
    let expected = u32::try_from(inputs.len()).unwrap_or(u32::MAX);
    // SAFETY: Each value is a fully initialized keyboard INPUT and the size
    // matches the exact ABI type supplied to SendInput.
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
