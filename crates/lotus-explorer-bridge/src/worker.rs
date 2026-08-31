use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use windows::Win32::Foundation::{CloseHandle, HMODULE};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
    GetModuleHandleExW,
};
use windows::Win32::System::Threading::{CreateThread, Sleep, THREAD_CREATION_FLAGS};
use windows::core::PCWSTR;

use crate::{hook, lotus_explorer_bridge_hook};

const IDLE: usize = 0;
const STARTING: usize = 1;
const CONFIGURING: usize = 2;
const RUNNING: usize = 3;
const STOPPING: usize = 4;

static WORKER_STATE: AtomicUsize = AtomicUsize::new(IDLE);

pub(crate) fn start_owner_worker() -> bool {
    if WORKER_STATE
        .compare_exchange(IDLE, STARTING, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }

    let mut module = HMODULE::default();
    let address = PCWSTR::from_raw((lotus_explorer_bridge_hook as *const ()).cast::<u16>());
    if unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
            address,
            &raw mut module,
        )
    }
    .is_err()
    {
        WORKER_STATE.store(IDLE, Ordering::Release);
        return false;
    }

    let Ok(worker) = (unsafe {
        CreateThread(
            None,
            0,
            Some(owner_worker),
            None,
            THREAD_CREATION_FLAGS::default(),
            None,
        )
    }) else {
        WORKER_STATE.store(IDLE, Ordering::Release);
        return false;
    };

    let _ = unsafe { CloseHandle(worker) };
    true
}

pub(crate) fn activate_owner_worker() -> bool {
    WORKER_STATE
        .compare_exchange(STARTING, RUNNING, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

pub(crate) fn cancel_owner_worker() {
    let _ = WORKER_STATE.compare_exchange(
        STARTING,
        STOPPING,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
}

pub(crate) fn begin_configuration() -> bool {
    WORKER_STATE
        .compare_exchange(RUNNING, CONFIGURING, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

pub(crate) fn finish_configuration() {
    let _ = WORKER_STATE.compare_exchange(
        CONFIGURING,
        RUNNING,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
}

pub(crate) fn release_after_disable() {
    let _ = WORKER_STATE.compare_exchange(
        CONFIGURING,
        STOPPING,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
}

pub(crate) fn is_idle() -> bool {
    WORKER_STATE.load(Ordering::Acquire) == IDLE
}

unsafe extern "system" fn owner_worker(_parameter: *mut c_void) -> u32 {
    loop {
        match WORKER_STATE.load(Ordering::Acquire) {
            STARTING | CONFIGURING => unsafe { Sleep(25) },
            RUNNING if owner_is_live() => unsafe { Sleep(250) },
            RUNNING => {
                let _ = WORKER_STATE.compare_exchange(
                    RUNNING,
                    STOPPING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
            }
            STOPPING => {
                hook::clear_owner();
                while !hook::disable_hooks() {
                    unsafe { Sleep(250) };
                }
                WORKER_STATE.store(IDLE, Ordering::Release);
                break;
            }
            _ => break,
        }
    }

    0
}

fn owner_is_live() -> bool {
    hook::owner_is_live()
}
