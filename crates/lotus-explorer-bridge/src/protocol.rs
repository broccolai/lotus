use std::sync::atomic::{AtomicU32, Ordering};

use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::UI::WindowsAndMessaging::{
    RegisterWindowMessageW, RemovePropW, SetPropW,
};
use windows::core::{PCWSTR, w};

pub const CONFIG_MESSAGE_NAME: PCWSTR = w!("Lotus.ExplorerBridge.Configure.v2");
pub const ACK_PROPERTY_NAME: PCWSTR = w!("Lotus.ExplorerBridge.Acknowledged.v2");
pub const OWNER_PROPERTY_NAME: PCWSTR = w!("Lotus.ExplorerBridge.Owner.v2");
pub const LEASE_PROPERTY_NAME: PCWSTR = w!("Lotus.ExplorerBridge.Lease.v3");
pub const HOOK_EXPORT_NAME: &[u8] = b"lotus_explorer_bridge_hook\0";

static CONFIG_MESSAGE: AtomicU32 = AtomicU32::new(0);

pub(crate) fn config_message() -> u32 {
    registered_message(&CONFIG_MESSAGE, CONFIG_MESSAGE_NAME)
}

pub(crate) fn decode_configuration(value: isize) -> Option<(usize, bool)> {
    let value = usize::try_from(value).ok()?;
    let token = value >> 1;
    (token != 0).then_some((token, value & 1 != 0))
}

pub(crate) fn acknowledge(owner: usize, configuration: isize, success: bool) {
    if owner == 0 {
        return;
    }
    let owner = HWND(std::ptr::with_exposed_provenance_mut(owner));
    if !success {
        let _ = unsafe { RemovePropW(owner, ACK_PROPERTY_NAME) };
        return;
    }
    let Ok(acknowledgement) = usize::try_from(configuration) else {
        return;
    };
    let _ = unsafe {
        SetPropW(
            owner,
            ACK_PROPERTY_NAME,
            Some(HANDLE(std::ptr::with_exposed_provenance_mut(
                acknowledgement,
            ))),
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
