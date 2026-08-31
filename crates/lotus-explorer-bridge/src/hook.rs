use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use minhook::{MH_STATUS, MinHook};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    CWPSTRUCT, GetPropW, GetWindowThreadProcessId, SET_WINDOW_POS_FLAGS, SW_HIDE,
    SWP_HIDEWINDOW, SWP_SHOWWINDOW,
};
use windows::core::BOOL;

use crate::protocol::{
    OWNER_PROPERTY_NAME, acknowledge, config_message, decode_configuration,
};
use crate::{target, worker};

type ShowWindowFn = unsafe extern "system" fn(HWND, i32) -> BOOL;
type SetWindowPosFn =
    unsafe extern "system" fn(HWND, HWND, i32, i32, i32, i32, SET_WINDOW_POS_FLAGS) -> BOOL;

static ENABLED: AtomicBool = AtomicBool::new(false);
static HOOKS_CREATED: AtomicBool = AtomicBool::new(false);
static OWNER: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_TOKEN: AtomicUsize = AtomicUsize::new(0);
static OWNER_PROCESS: AtomicUsize = AtomicUsize::new(0);
static OWNER_THREAD: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SHOW_WINDOW: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SHOW_WINDOW_ASYNC: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SET_WINDOW_POS: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn handle_configuration(message: &CWPSTRUCT) {
    if message.message != config_message() {
        return;
    }

    let owner = message.wParam.0;
    let configuration = message.lParam.0;
    let Some((token, enable_requested)) = decode_configuration(configuration) else {
        return;
    };
    let success = if enable_requested {
        enable(owner, token)
    } else {
        disable(token)
    };
    acknowledge(owner, configuration, success);
}

fn enable(owner: usize, token: usize) -> bool {
    let Some((process, thread)) = owner_identity(owner, token) else {
        return false;
    };
    if HOOKS_CREATED.load(Ordering::Acquire) && worker::begin_configuration() {
        if !enable_hooks() {
            worker::finish_configuration();
            return false;
        }
        publish_owner(owner, token, process, thread);
        worker::finish_configuration();
        return true;
    }
    if !worker::start_owner_worker() {
        return false;
    }

    let installed = catch_unwind(AssertUnwindSafe(install_inner)).unwrap_or(false);
    if installed {
        publish_owner(owner, token, process, thread);
        return worker::activate_owner_worker();
    }

    ENABLED.store(false, Ordering::Release);
    OWNER.store(0, Ordering::Release);
    if disable_hooks() {
        worker::cancel_owner_worker();
    } else {
        let _ = worker::activate_owner_worker();
    }
    false
}

fn publish_owner(owner: usize, token: usize, process: usize, thread: usize) {
    OWNER.store(owner, Ordering::Release);
    ACTIVE_TOKEN.store(token, Ordering::Release);
    OWNER_PROCESS.store(process, Ordering::Release);
    OWNER_THREAD.store(thread, Ordering::Release);
    ENABLED.store(true, Ordering::Release);
}

fn owner_identity(owner: usize, token: usize) -> Option<(usize, usize)> {
    let hwnd = HWND(std::ptr::with_exposed_provenance_mut(owner));
    let mut process = 0;
    let thread = unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut process)) };
    (thread != 0
        && process != 0
        && unsafe { GetPropW(hwnd, OWNER_PROPERTY_NAME) }.0.addr() == token)
        .then_some((process as usize, thread as usize))
}

fn disable(token: usize) -> bool {
    if ACTIVE_TOKEN.load(Ordering::Acquire) != token {
        return true;
    }
    if !worker::begin_configuration() {
        return worker::is_idle() && hooks_are_absent();
    }

    ENABLED.store(false, Ordering::Release);
    let disabled = disable_hooks();
    if disabled {
        OWNER.store(0, Ordering::Release);
        worker::release_after_disable();
    } else {
        worker::finish_configuration();
    }
    disabled
}

fn hooks_are_absent() -> bool {
    !HOOKS_CREATED.load(Ordering::Acquire)
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
        disable_hooks();
        return false;
    }

    for target in [show_window, show_window_async, set_window_pos] {
        if !matches!(
            unsafe { MinHook::enable_hook(target) },
            Ok(()) | Err(MH_STATUS::MH_ERROR_ENABLED)
        ) {
            disable_hooks();
            return false;
        }
    }

    HOOKS_CREATED.store(true, Ordering::Release);
    true
}

fn enable_hooks() -> bool {
    let Some(targets) = hook_targets() else {
        return false;
    };
    let enabled = targets.into_iter().all(|target| {
        matches!(
            unsafe { MinHook::enable_hook(target) },
            Ok(()) | Err(MH_STATUS::MH_ERROR_ENABLED)
        )
    });
    if !enabled {
        let _ = disable_hooks();
    }
    enabled
}

fn hook_targets() -> Option<[*mut c_void; 3]> {
    Some([
        target::user32_procedure(c"ShowWindow")?,
        target::user32_procedure(c"ShowWindowAsync")?,
        target::user32_procedure(c"SetWindowPos")?,
    ])
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

pub(crate) fn disable_hooks() -> bool {
    ENABLED.store(false, Ordering::Release);
    let mut all_disabled = true;
    for hook in [
        (c"ShowWindow", &ORIGINAL_SHOW_WINDOW),
        (c"ShowWindowAsync", &ORIGINAL_SHOW_WINDOW_ASYNC),
        (c"SetWindowPos", &ORIGINAL_SET_WINDOW_POS),
    ] {
        all_disabled = disable_hook(hook) && all_disabled;
    }
    all_disabled
}

fn disable_hook((name, original): (&'static std::ffi::CStr, &AtomicUsize)) -> bool {
    if original.load(Ordering::Acquire) == 0 {
        return true;
    }
    let Some(target) = target::user32_procedure(name) else {
        return false;
    };

    let result = catch_unwind(AssertUnwindSafe(|| {
        match unsafe { MinHook::disable_hook(target) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) => {}
            Err(_) => return false,
        }

        true
    }));
    result.unwrap_or(false)
}

pub(crate) fn owner() -> usize {
    OWNER.load(Ordering::Acquire)
}

pub(crate) fn clear_owner() {
    ENABLED.store(false, Ordering::Release);
    OWNER.store(0, Ordering::Release);
    ACTIVE_TOKEN.store(0, Ordering::Release);
    OWNER_PROCESS.store(0, Ordering::Release);
    OWNER_THREAD.store(0, Ordering::Release);
}

pub(crate) fn owner_is_live() -> bool {
    let owner = owner();
    let token = ACTIVE_TOKEN.load(Ordering::Acquire);
    let mut process = 0;
    let hwnd = HWND(std::ptr::with_exposed_provenance_mut(owner));
    owner != 0
        && token != 0
        && unsafe {
            windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(hwnd)).as_bool()
        }
        && unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut process)) } as usize
            == OWNER_THREAD.load(Ordering::Acquire)
        && process as usize == OWNER_PROCESS.load(Ordering::Acquire)
        && unsafe { GetPropW(hwnd, OWNER_PROPERTY_NAME) }.0.addr() == token
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
