use std::ffi::c_void;
use std::sync::atomic::{AtomicU8, Ordering};

use windows::Win32::Foundation::{CloseHandle, HMODULE};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
    GetModuleHandleExW,
};
use windows::Win32::System::Threading::{CreateThread, Sleep, THREAD_CREATION_FLAGS};
use windows::core::PCWSTR;

use crate::{lotus_shell_bridge_hook, placement_hook};

const WORKER_IDLE: u8 = 0;
const WORKER_CONFIGURING: u8 = 1;
const WORKER_RUNNING: u8 = 2;
const WORKER_RETIRING: u8 = 3;
const WORKER_TEARING_DOWN: u8 = 4;

static CLEANUP_WORKER_STATE: AtomicU8 = AtomicU8::new(WORKER_IDLE);

pub(crate) struct ConfigurationGuard;

impl Drop for ConfigurationGuard {
    fn drop(&mut self) {
        CLEANUP_WORKER_STATE.store(WORKER_RUNNING, Ordering::Release);
    }
}

pub(crate) fn begin_configuration() -> Option<ConfigurationGuard> {
    loop {
        match CLEANUP_WORKER_STATE.load(Ordering::Acquire) {
            state @ (WORKER_RUNNING | WORKER_RETIRING) => {
                if CLEANUP_WORKER_STATE
                    .compare_exchange(
                        state,
                        WORKER_CONFIGURING,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok()
                {
                    return Some(ConfigurationGuard);
                }
            }
            WORKER_CONFIGURING | WORKER_TEARING_DOWN => unsafe { Sleep(1) },
            WORKER_IDLE => {
                if CLEANUP_WORKER_STATE
                    .compare_exchange(
                        WORKER_IDLE,
                        WORKER_CONFIGURING,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok()
                {
                    return if create_configured_worker() {
                        Some(ConfigurationGuard)
                    } else {
                        None
                    };
                }
            }
            _ => return None,
        }
    }
}

fn create_configured_worker() -> bool {
    let mut module = HMODULE::default();
    let address = PCWSTR::from_raw((lotus_shell_bridge_hook as *const ()).cast::<u16>());
    if unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
            address,
            &raw mut module,
        )
    }
    .is_err()
    {
        CLEANUP_WORKER_STATE.store(WORKER_IDLE, Ordering::Release);
        return false;
    }

    let worker = unsafe {
        CreateThread(
            None,
            0,
            Some(cleanup_worker),
            None,
            THREAD_CREATION_FLAGS::default(),
            None,
        )
    };
    let Ok(worker) = worker else {
        CLEANUP_WORKER_STATE.store(WORKER_IDLE, Ordering::Release);
        return false;
    };

    let _ = unsafe { CloseHandle(worker) };
    true
}

unsafe extern "system" fn cleanup_worker(_parameter: *mut c_void) -> u32 {
    while CLEANUP_WORKER_STATE.load(Ordering::Acquire) == WORKER_CONFIGURING {
        unsafe { Sleep(1) };
    }

    loop {
        while tick_count() <= placement_hook::lease_deadline() {
            unsafe { Sleep(25) };
        }

        if CLEANUP_WORKER_STATE
            .compare_exchange(
                WORKER_RUNNING,
                WORKER_RETIRING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            unsafe { Sleep(1) };
            continue;
        }

        if tick_count() <= placement_hook::lease_deadline()
            || CLEANUP_WORKER_STATE
                .compare_exchange(
                    WORKER_RETIRING,
                    WORKER_TEARING_DOWN,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_err()
        {
            let _ = CLEANUP_WORKER_STATE.compare_exchange(
                WORKER_RETIRING,
                WORKER_RUNNING,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            continue;
        }

        if placement_hook::disable_after_lease() {
            CLEANUP_WORKER_STATE.store(WORKER_IDLE, Ordering::Release);
            return 0;
        }

        CLEANUP_WORKER_STATE.store(WORKER_RUNNING, Ordering::Release);
        unsafe { Sleep(25) };
    }
}

fn tick_count() -> u64 {
    unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
}
