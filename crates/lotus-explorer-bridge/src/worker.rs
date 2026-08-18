use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{CloseHandle, FreeLibrary, HMODULE, HWND};
use windows::Win32::System::LibraryLoader::{
    FreeLibraryAndExitThread, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GetModuleHandleExW,
};
use windows::Win32::System::Threading::{CreateThread, Sleep, THREAD_CREATION_FLAGS};
use windows::Win32::UI::WindowsAndMessaging::IsWindow;
use windows::core::PCWSTR;

use crate::{hook, lotus_explorer_bridge_hook};

static WORKER_RUNNING: AtomicBool = AtomicBool::new(false);

pub(crate) fn start_owner_worker() -> bool {
    if WORKER_RUNNING.load(Ordering::Acquire) {
        return true;
    }

    let mut module = HMODULE::default();
    let address = PCWSTR::from_raw((lotus_explorer_bridge_hook as *const ()).cast::<u16>());
    if unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
            address,
            &raw mut module,
        )
    }
    .is_err()
    {
        return false;
    }

    let Ok(worker) = (unsafe {
        CreateThread(
            None,
            0,
            Some(owner_worker),
            Some(module.0.cast_const()),
            THREAD_CREATION_FLAGS::default(),
            None,
        )
    }) else {
        let _ = unsafe { FreeLibrary(module) };
        return false;
    };

    WORKER_RUNNING.store(true, Ordering::Release);
    let _ = unsafe { CloseHandle(worker) };
    true
}

unsafe extern "system" fn owner_worker(parameter: *mut c_void) -> u32 {
    while owner_is_live() {
        unsafe { Sleep(250) };
    }

    hook::uninstall();
    hook::clear_owner();
    WORKER_RUNNING.store(false, Ordering::Release);

    let module = HMODULE(parameter);
    unsafe { FreeLibraryAndExitThread(module, 0) }
}

fn owner_is_live() -> bool {
    let owner = hook::owner();
    owner != 0 && {
        let hwnd = HWND(std::ptr::with_exposed_provenance_mut(owner));
        unsafe { IsWindow(Some(hwnd)).as_bool() }
    }
}
