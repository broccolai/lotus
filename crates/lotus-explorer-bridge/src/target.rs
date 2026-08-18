use std::ffi::{CStr, c_void};

use windows::Win32::Foundation::HWND;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;
use windows::core::{PCSTR, w};

pub(crate) fn user32_procedure(name: &'static CStr) -> Option<*mut c_void> {
    let module = unsafe { GetModuleHandleW(w!("user32.dll")) }.ok()?;
    let procedure = unsafe { GetProcAddress(module, PCSTR(name.as_ptr().cast())) }?;
    Some(procedure as *mut c_void)
}

pub(crate) fn is_taskbar_window(window: HWND) -> bool {
    let mut class_name = [0_u16; 32];
    let length = unsafe { GetClassNameW(window, &mut class_name) };
    let Ok(length) = usize::try_from(length) else {
        return false;
    };
    matches!(
        String::from_utf16_lossy(&class_name[..length]).as_str(),
        "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
    )
}
