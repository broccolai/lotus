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
static HOOK_CREATED: AtomicBool = AtomicBool::new(false);
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
        LEASE_DEADLINE.store(0, Ordering::Release);
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

    let Some(_configuration) = worker::begin_configuration() else {
        acknowledge(message.wParam, false);
        return;
    };

    let active = install();
    if active {
        LEASE_DEADLINE.store(
            tick_count().saturating_add(LEASE_MILLISECONDS),
            Ordering::Release,
        );
    }
    ENABLED.store(active, Ordering::Release);
    acknowledge(message.wParam, active);
}

fn install() -> bool {
    if HOOK_CREATED.load(Ordering::Acquire) {
        return enable_hook();
    }

    catch_unwind(AssertUnwindSafe(install_inner)).unwrap_or(false)
}

fn install_inner() -> bool {
    let Some(target) = target::set_window_pos_address() else {
        return false;
    };
    let original =
        match unsafe { MinHook::create_hook(target, hooked_set_window_pos as *mut c_void) }
        {
            Ok(original) => original.addr(),
            Err(MH_STATUS::MH_ERROR_ALREADY_CREATED) => {
                let original = ORIGINAL_SET_WINDOW_POS.load(Ordering::Acquire);
                if original == 0 {
                    return false;
                }
                original
            }
            Err(_) => return false,
        };
    ORIGINAL_SET_WINDOW_POS.store(original, Ordering::Release);
    HOOK_CREATED.store(true, Ordering::Release);

    if enable_hook_at(target) {
        return true;
    }

    let _ = disable_hook_at(target);
    false
}

fn enable_hook() -> bool {
    let Some(target) = target::set_window_pos_address() else {
        return false;
    };
    enable_hook_at(target)
}

fn enable_hook_at(target: *mut c_void) -> bool {
    match unsafe { MinHook::enable_hook(target) } {
        Ok(()) | Err(MH_STATUS::MH_ERROR_ENABLED) => true,
        Err(_) => false,
    }
}

pub(crate) fn disable_after_lease() -> bool {
    ENABLED.store(false, Ordering::Release);
    if !HOOK_CREATED.load(Ordering::Acquire) {
        return true;
    }
    let Some(target) = target::set_window_pos_address() else {
        return false;
    };

    disable_hook_at(target)
}

fn disable_hook_at(target: *mut c_void) -> bool {
    catch_unwind(AssertUnwindSafe(|| unsafe {
        matches!(
            MinHook::disable_hook(target),
            Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED)
        )
    }))
    .unwrap_or(false)
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
