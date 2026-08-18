use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use minhook::{MH_STATUS, MinHook};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    CWPSTRUCT, SET_WINDOW_POS_FLAGS, SW_HIDE, SWP_HIDEWINDOW, SWP_SHOWWINDOW,
};
use windows::core::BOOL;

use crate::protocol::{acknowledge, config_message};
use crate::{target, worker};

type ShowWindowFn = unsafe extern "system" fn(HWND, i32) -> BOOL;
type SetWindowPosFn =
    unsafe extern "system" fn(HWND, HWND, i32, i32, i32, i32, SET_WINDOW_POS_FLAGS) -> BOOL;

static ENABLED: AtomicBool = AtomicBool::new(false);
static HOOKS_READY: AtomicBool = AtomicBool::new(false);
static OWNER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SHOW_WINDOW: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SHOW_WINDOW_ASYNC: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SET_WINDOW_POS: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn handle_configuration(message: &CWPSTRUCT) {
    if message.message != config_message() {
        return;
    }

    let owner = message.wParam.0;
    let enable = message.lParam.0 != 0;
    let success = if enable {
        OWNER.store(owner, Ordering::Release);
        let installed = install();
        ENABLED.store(installed, Ordering::Release);
        if !installed {
            OWNER.store(0, Ordering::Release);
        }
        installed
    } else {
        ENABLED.store(false, Ordering::Release);
        uninstall();
        OWNER.store(0, Ordering::Release);
        true
    };
    acknowledge(owner, success);
}

fn install() -> bool {
    if HOOKS_READY.load(Ordering::Acquire) {
        return true;
    }
    if !worker::start_owner_worker() {
        return false;
    }

    catch_unwind(AssertUnwindSafe(install_inner)).unwrap_or(false)
}

fn install_inner() -> bool {
    let Some(show_window) = target::user32_procedure(c"ShowWindow") else {
        return false;
    };
    let Some(show_window_async) = target::user32_procedure(c"ShowWindowAsync") else {
        return false;
    };
    let Some(set_window_pos) = target::user32_procedure(c"SetWindowPos") else {
        return false;
    };

    if !create_hook(
        show_window,
        hooked_show_window as *mut c_void,
        &ORIGINAL_SHOW_WINDOW,
    ) || !create_hook(
        show_window_async,
        hooked_show_window_async as *mut c_void,
        &ORIGINAL_SHOW_WINDOW_ASYNC,
    ) || !create_hook(
        set_window_pos,
        hooked_set_window_pos as *mut c_void,
        &ORIGINAL_SET_WINDOW_POS,
    ) {
        uninstall();
        return false;
    }

    for target in [show_window, show_window_async, set_window_pos] {
        if !matches!(
            unsafe { MinHook::enable_hook(target) },
            Ok(()) | Err(MH_STATUS::MH_ERROR_ENABLED)
        ) {
            uninstall();
            return false;
        }
    }

    HOOKS_READY.store(true, Ordering::Release);
    true
}

fn create_hook(target: *mut c_void, detour: *mut c_void, original: &AtomicUsize) -> bool {
    match unsafe { MinHook::create_hook(target, detour) } {
        Ok(trampoline) => {
            original.store(trampoline.addr(), Ordering::Release);
            true
        }
        Err(MH_STATUS::MH_ERROR_ALREADY_CREATED) => original.load(Ordering::Acquire) != 0,
        Err(_) => false,
    }
}

pub(crate) fn uninstall() {
    let targets = [
        target::user32_procedure(c"ShowWindow"),
        target::user32_procedure(c"ShowWindowAsync"),
        target::user32_procedure(c"SetWindowPos"),
    ];
    for target in targets.into_iter().flatten() {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _ = unsafe { MinHook::disable_hook(target) };
            let _ = unsafe { MinHook::remove_hook(target) };
        }));
    }

    HOOKS_READY.store(false, Ordering::Release);
    ORIGINAL_SHOW_WINDOW.store(0, Ordering::Release);
    ORIGINAL_SHOW_WINDOW_ASYNC.store(0, Ordering::Release);
    ORIGINAL_SET_WINDOW_POS.store(0, Ordering::Release);
}

pub(crate) fn owner() -> usize {
    OWNER.load(Ordering::Acquire)
}

pub(crate) fn clear_owner() {
    ENABLED.store(false, Ordering::Release);
    OWNER.store(0, Ordering::Release);
}

unsafe extern "system" fn hooked_show_window(window: HWND, command: i32) -> BOOL {
    call_show_window(&ORIGINAL_SHOW_WINDOW, window, command)
}

unsafe extern "system" fn hooked_show_window_async(window: HWND, command: i32) -> BOOL {
    call_show_window(&ORIGINAL_SHOW_WINDOW_ASYNC, window, command)
}

fn call_show_window(original: &AtomicUsize, window: HWND, command: i32) -> BOOL {
    let original = original.load(Ordering::Acquire);
    if original == 0 {
        return BOOL(0);
    }
    let original: ShowWindowFn = unsafe { std::mem::transmute(original) };

    if ENABLED.load(Ordering::Acquire)
        && command != SW_HIDE.0
        && target::is_taskbar_window(window)
    {
        return BOOL(1);
    }

    unsafe { original(window, command) }
}

unsafe extern "system" fn hooked_set_window_pos(
    window: HWND,
    insert_after: HWND,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    mut flags: SET_WINDOW_POS_FLAGS,
) -> BOOL {
    let original = ORIGINAL_SET_WINDOW_POS.load(Ordering::Acquire);
    if original == 0 {
        return BOOL(0);
    }
    let original: SetWindowPosFn = unsafe { std::mem::transmute(original) };

    if ENABLED.load(Ordering::Acquire) && target::is_taskbar_window(window) {
        flags.0 = (flags.0 & !SWP_SHOWWINDOW.0) | SWP_HIDEWINDOW.0;
    }

    unsafe { original(window, insert_after, x, y, width, height, flags) }
}
