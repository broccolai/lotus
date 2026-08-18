#![cfg(windows)]

#[cfg(not(target_pointer_width = "64"))]
compile_error!("the Lotus Explorer bridge supports only 64-bit Windows");

use std::ffi::c_void;

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::DisableThreadLibraryCalls;
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows::Win32::UI::WindowsAndMessaging::{CWPSTRUCT, CallNextHookEx};
use windows::core::BOOL;

mod hook;
mod protocol;
mod target;
mod worker;

pub use protocol::{ACK_MESSAGE_NAME, CONFIG_MESSAGE_NAME, HOOK_EXPORT_NAME};

#[unsafe(no_mangle)]
unsafe extern "system" fn DllMain(
    instance: HINSTANCE,
    reason: u32,
    _reserved: *mut c_void,
) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        let _ = unsafe { DisableThreadLibraryCalls(instance.into()) };
    }

    BOOL(1)
}

#[unsafe(no_mangle)]
/// Receives configuration on Explorer's taskbar thread.
///
/// # Safety
///
/// `lparam` must follow the `WH_CALLWNDPROC` callback contract supplied by Windows.
pub unsafe extern "system" fn lotus_explorer_bridge_hook(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 && lparam.0 != 0 {
        let message = unsafe { &*(lparam.0 as *const CWPSTRUCT) };
        hook::handle_configuration(message);
    }

    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}
