use std::sync::atomic::{AtomicU32, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, RegisterWindowMessageW};
use windows::core::{PCWSTR, w};

pub const CONFIG_MESSAGE_NAME: PCWSTR = w!("Lotus.ShellBridge.Configure.v1");
pub const ACK_MESSAGE_NAME: PCWSTR = w!("Lotus.ShellBridge.Acknowledge.v1");
pub const HOOK_EXPORT_NAME: &[u8] = b"lotus_shell_bridge_hook\0";
pub const DISABLE_SENTINEL: isize = isize::MIN;

static CONFIG_MESSAGE: AtomicU32 = AtomicU32::new(0);
static ACK_MESSAGE: AtomicU32 = AtomicU32::new(0);

pub const fn encode_anchor(x: i32, y: i32) -> isize {
    let [x0, x1, x2, x3] = x.to_ne_bytes();
    let [y0, y1, y2, y3] = y.to_ne_bytes();
    isize::from_ne_bytes([x0, x1, x2, x3, y0, y1, y2, y3])
}

pub(crate) const fn decode_anchor(value: isize) -> (i32, i32) {
    let [x0, x1, x2, x3, y0, y1, y2, y3] = value.to_ne_bytes();
    (
        i32::from_ne_bytes([x0, x1, x2, x3]),
        i32::from_ne_bytes([y0, y1, y2, y3]),
    )
}

pub(crate) fn config_message() -> u32 {
    registered_message(&CONFIG_MESSAGE, CONFIG_MESSAGE_NAME)
}

pub(crate) fn acknowledge(owner: WPARAM, success: bool) {
    if owner.0 == 0 {
        return;
    }
    let message = registered_message(&ACK_MESSAGE, ACK_MESSAGE_NAME);
    if message == 0 {
        return;
    }

    let owner = HWND(std::ptr::with_exposed_provenance_mut(owner.0));
    let _ = unsafe {
        PostMessageW(
            Some(owner),
            message,
            WPARAM(usize::from(success)),
            LPARAM(0),
        )
    };
}

fn registered_message(storage: &AtomicU32, name: PCWSTR) -> u32 {
    let current = storage.load(Ordering::Acquire);
    if current != 0 {
        return current;
    }

    let registered = unsafe { RegisterWindowMessageW(name) };
    if registered != 0 {
        let _ =
            storage.compare_exchange(0, registered, Ordering::AcqRel, Ordering::Acquire);
    }
    storage.load(Ordering::Acquire)
}
