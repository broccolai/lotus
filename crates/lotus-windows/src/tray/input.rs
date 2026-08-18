use std::mem::size_of;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, SendInput, VIRTUAL_KEY,
};

use super::TrayError;

const INPUT_MARKER: usize = 0x4C4F_5455;

pub(super) fn send(inputs: &[INPUT]) -> Result<(), TrayError> {
    let expected = u32::try_from(inputs.len()).unwrap_or(u32::MAX);
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

pub(super) const fn key(virtual_key: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
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
