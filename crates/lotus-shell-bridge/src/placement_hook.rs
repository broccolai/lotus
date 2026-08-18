use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering};

use minhook::{MH_STATUS, MinHook};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::UI::WindowsAndMessaging::{
    CWPSTRUCT, SET_WINDOW_POS_FLAGS, SWP_NOMOVE,
};
use windows::core::BOOL;

use crate::protocol::{DISABLE_SENTINEL, acknowledge, config_message, decode_anchor};
use crate::{target, worker};

const LEASE_MILLISECONDS: u64 = 1_500;

type SetWindowPosFn =
    unsafe extern "system" fn(HWND, HWND, i32, i32, i32, i32, SET_WINDOW_POS_FLAGS) -> BOOL;

static ENABLED: AtomicBool = AtomicBool::new(false);
static HOOK_READY: AtomicBool = AtomicBool::new(false);
static ANCHOR_X: AtomicI32 = AtomicI32::new(0);
static ANCHOR_Y: AtomicI32 = AtomicI32::new(0);
static LEASE_DEADLINE: AtomicU64 = AtomicU64::new(0);
static ORIGINAL_SET_WINDOW_POS: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn handle_message(message: &CWPSTRUCT) {
    if message.message != config_message() {
        return;
    }
    if message.lParam.0 == DISABLE_SENTINEL {
        ENABLED.store(false, Ordering::Release);
        acknowledge(message.wParam, true);
        return;
    }

    let (anchor_x, anchor_y) = decode_anchor(message.lParam.0);
    ANCHOR_X.store(anchor_x, Ordering::Release);
    ANCHOR_Y.store(anchor_y, Ordering::Release);
    LEASE_DEADLINE.store(
        tick_count().saturating_add(LEASE_MILLISECONDS),
        Ordering::Release,
    );

    let active = install();
    ENABLED.store(active, Ordering::Release);
    acknowledge(message.wParam, active);
}

fn install() -> bool {
    if HOOK_READY.load(Ordering::Acquire) {
        return true;
    }
    if !worker::start_cleanup_worker() {
        return false;
    }

    catch_unwind(AssertUnwindSafe(install_inner)).unwrap_or(false)
}

fn install_inner() -> bool {
    let Some(target) = target::set_window_pos_address() else {
        return false;
    };
    let Ok(original) =
        (unsafe { MinHook::create_hook(target, hooked_set_window_pos as *mut c_void) })
    else {
        return HOOK_READY.load(Ordering::Acquire);
    };
    ORIGINAL_SET_WINDOW_POS.store(original.addr(), Ordering::Release);

    match unsafe { MinHook::enable_hook(target) } {
        Ok(()) | Err(MH_STATUS::MH_ERROR_ENABLED) => {
            HOOK_READY.store(true, Ordering::Release);
            true
        }
        Err(_) => false,
    }
}

pub(crate) fn uninstall_after_lease() {
    ENABLED.store(false, Ordering::Release);
    if let Some(target) = target::set_window_pos_address() {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _ = unsafe { MinHook::disable_hook(target) };
            let _ = unsafe { MinHook::remove_hook(target) };
        }));
    }
    HOOK_READY.store(false, Ordering::Release);
    ORIGINAL_SET_WINDOW_POS.store(0, Ordering::Release);
}

pub(crate) fn lease_deadline() -> u64 {
    LEASE_DEADLINE.load(Ordering::Acquire)
}

unsafe extern "system" fn hooked_set_window_pos(
    window: HWND,
    insert_after: HWND,
    mut x: i32,
    mut y: i32,
    width: i32,
    height: i32,
    mut flags: SET_WINDOW_POS_FLAGS,
) -> BOOL {
    let original = ORIGINAL_SET_WINDOW_POS.load(Ordering::Acquire);
    if original == 0 {
        return BOOL(0);
    }
    let original: SetWindowPosFn = unsafe { std::mem::transmute(original) };

    if lease_active()
        && target::is_control_center_window(window)
        && let Some(position) = target::desired_position(
            window,
            width,
            height,
            flags,
            (
                ANCHOR_X.load(Ordering::Acquire),
                ANCHOR_Y.load(Ordering::Acquire),
            ),
        )
    {
        x = position.0;
        y = position.1;
        flags.0 &= !SWP_NOMOVE.0;
    }

    unsafe { original(window, insert_after, x, y, width, height, flags) }
}

fn lease_active() -> bool {
    if !ENABLED.load(Ordering::Acquire) {
        return false;
    }
    if tick_count() <= LEASE_DEADLINE.load(Ordering::Acquire) {
        return true;
    }

    ENABLED.store(false, Ordering::Release);
    false
}

fn tick_count() -> u64 {
    unsafe { GetTickCount64() }
}
