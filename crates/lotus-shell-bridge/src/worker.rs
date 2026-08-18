use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{CloseHandle, FreeLibrary, HMODULE};
use windows::Win32::System::LibraryLoader::{
    FreeLibraryAndExitThread, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GetModuleHandleExW,
};
use windows::Win32::System::Threading::{CreateThread, Sleep, THREAD_CREATION_FLAGS};
use windows::core::PCWSTR;

use crate::{lotus_shell_bridge_hook, placement_hook};

static CLEANUP_WORKER_RUNNING: AtomicBool = AtomicBool::new(false);

pub(crate) fn start_cleanup_worker() -> bool {
    if CLEANUP_WORKER_RUNNING.load(Ordering::Acquire) {
        return true;
    }

    let mut module = HMODULE::default();
    let address = PCWSTR::from_raw((lotus_shell_bridge_hook as *const ()).cast::<u16>());
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

    let worker = unsafe {
        CreateThread(
            None,
            0,
            Some(cleanup_worker),
            Some(module.0.cast_const()),
            THREAD_CREATION_FLAGS::default(),
            None,
        )
    };
    let Ok(worker) = worker else {
        let _ = unsafe { FreeLibrary(module) };
        return false;
    };

    CLEANUP_WORKER_RUNNING.store(true, Ordering::Release);
    let _ = unsafe { CloseHandle(worker) };
    true
}

unsafe extern "system" fn cleanup_worker(parameter: *mut c_void) -> u32 {
    while tick_count() <= placement_hook::lease_deadline() {
        unsafe { Sleep(25) };
    }

    placement_hook::uninstall_after_lease();
    CLEANUP_WORKER_RUNNING.store(false, Ordering::Release);

    let module = HMODULE(parameter);
    unsafe { FreeLibraryAndExitThread(module, 0) }
}

fn tick_count() -> u64 {
    unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
}
