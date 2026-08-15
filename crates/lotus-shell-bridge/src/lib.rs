#![cfg(windows)]

#[cfg(not(target_pointer_width = "64"))]
compile_error!("the Lotus shell bridge supports only 64-bit Windows");

use std::ffi::c_void;
use std::mem::size_of;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{
    AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicUsize, Ordering,
};

use minhook::{MH_STATUS, MinHook};
use windows::Win32::Foundation::{
    CloseHandle, FreeLibrary, HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, POINT, RECT,
    WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
};
use windows::Win32::System::LibraryLoader::{
    DisableThreadLibraryCalls, FreeLibraryAndExitThread,
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GetModuleHandleExW, GetModuleHandleW,
    GetProcAddress,
};
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows::Win32::System::Threading::{CreateThread, Sleep, THREAD_CREATION_FLAGS};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CWPSTRUCT, CallNextHookEx, GetClassNameW, GetWindowRect, PostMessageW,
    RegisterWindowMessageW, SET_WINDOW_POS_FLAGS, SWP_NOMOVE, SWP_NOSIZE,
};
use windows::core::{BOOL, PCSTR, PCWSTR, w};

pub const CONFIG_MESSAGE_NAME: PCWSTR = w!("Lotus.ShellBridge.Configure.v1");
pub const ACK_MESSAGE_NAME: PCWSTR = w!("Lotus.ShellBridge.Acknowledge.v1");
pub const HOOK_EXPORT_NAME: &[u8] = b"lotus_shell_bridge_hook\0";
pub const DISABLE_SENTINEL: isize = isize::MIN;

const EDGE_INSET_DIP: i32 = 12;
const LEASE_MILLISECONDS: u64 = 1_500;

type SetWindowPosFn =
    unsafe extern "system" fn(HWND, HWND, i32, i32, i32, i32, SET_WINDOW_POS_FLAGS) -> BOOL;

static CONFIG_MESSAGE: AtomicU32 = AtomicU32::new(0);
static ACK_MESSAGE: AtomicU32 = AtomicU32::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
static HOOK_READY: AtomicBool = AtomicBool::new(false);
static CLEANUP_WORKER_RUNNING: AtomicBool = AtomicBool::new(false);
static ANCHOR_X: AtomicI32 = AtomicI32::new(0);
static ANCHOR_Y: AtomicI32 = AtomicI32::new(0);
static LEASE_DEADLINE: AtomicU64 = AtomicU64::new(0);
static ORIGINAL_SET_WINDOW_POS: AtomicUsize = AtomicUsize::new(0);

#[unsafe(no_mangle)]
unsafe extern "system" fn DllMain(
    instance: HINSTANCE,
    reason: u32,
    _reserved: *mut c_void,
) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        // SAFETY: The loader supplied this live module handle during process attachment.
        let _ = unsafe { DisableThreadLibraryCalls(instance.into()) };
    }

    BOOL(1)
}

#[unsafe(no_mangle)]
/// Receives thread-hook notifications inside the allowlisted `ShellHost` process.
///
/// # Safety
///
/// `lparam` must follow the `WH_CALLWNDPROC` callback contract supplied by Windows.
pub unsafe extern "system" fn lotus_shell_bridge_hook(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 && lparam.0 != 0 {
        // SAFETY: WH_CALLWNDPROC supplies a live CWPSTRUCT for the duration of this callback.
        let message = unsafe { &*(lparam.0 as *const CWPSTRUCT) };
        handle_hook_message(message);
    }

    // SAFETY: Forwarding every hook notification preserves the target thread's hook chain.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn handle_hook_message(message: &CWPSTRUCT) {
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

    let active = install_process_hook();
    ENABLED.store(active, Ordering::Release);
    acknowledge(message.wParam, active);
}

pub const fn encode_anchor(x: i32, y: i32) -> isize {
    let [x0, x1, x2, x3] = x.to_ne_bytes();
    let [y0, y1, y2, y3] = y.to_ne_bytes();
    isize::from_ne_bytes([x0, x1, x2, x3, y0, y1, y2, y3])
}

const fn decode_anchor(value: isize) -> (i32, i32) {
    let [x0, x1, x2, x3, y0, y1, y2, y3] = value.to_ne_bytes();
    (
        i32::from_ne_bytes([x0, x1, x2, x3]),
        i32::from_ne_bytes([y0, y1, y2, y3]),
    )
}

fn config_message() -> u32 {
    registered_message(&CONFIG_MESSAGE, CONFIG_MESSAGE_NAME)
}

fn acknowledge(owner: WPARAM, success: bool) {
    if owner.0 == 0 {
        return;
    }
    let message = registered_message(&ACK_MESSAGE, ACK_MESSAGE_NAME);
    if message == 0 {
        return;
    }

    let owner = HWND(std::ptr::with_exposed_provenance_mut(owner.0));
    // SAFETY: The owner handle is supplied by Lotus and a stale handle simply rejects the post.
    let _ = unsafe {
        PostMessageW(
            Some(owner),
            message,
            WPARAM(usize::from(success)),
            LPARAM(0),
        )
    };
}

fn registered_message(storage: &AtomicU32, name: PCWSTR) -> u32 {
    let current = storage.load(Ordering::Acquire);
    if current != 0 {
        return current;
    }

    // SAFETY: The message name has static process-lifetime storage.
    let registered = unsafe { RegisterWindowMessageW(name) };
    if registered != 0 {
        let _ =
            storage.compare_exchange(0, registered, Ordering::AcqRel, Ordering::Acquire);
    }
    storage.load(Ordering::Acquire)
}

fn install_process_hook() -> bool {
    if HOOK_READY.load(Ordering::Acquire) {
        return true;
    }
    if !start_cleanup_worker() {
        return false;
    }

    catch_unwind(AssertUnwindSafe(install_process_hook_inner)).unwrap_or(false)
}

fn install_process_hook_inner() -> bool {
    let Some(target) = set_window_pos_address() else {
        return false;
    };
    // SAFETY: The target is USER32's live SetWindowPos entry point and the detour has the same
    // system ABI. The pinned bridge keeps both the detour and trampoline live for ShellHost.
    let Ok(original) =
        (unsafe { MinHook::create_hook(target, hooked_set_window_pos as *mut c_void) })
    else {
        return HOOK_READY.load(Ordering::Acquire);
    };
    ORIGINAL_SET_WINDOW_POS.store(original.addr(), Ordering::Release);

    // SAFETY: The hook was created for this exact live target and its trampoline was saved first.
    match unsafe { MinHook::enable_hook(target) } {
        Ok(()) | Err(MH_STATUS::MH_ERROR_ENABLED) => {
            HOOK_READY.store(true, Ordering::Release);
            true
        }
        Err(_) => false,
    }
}

fn start_cleanup_worker() -> bool {
    if CLEANUP_WORKER_RUNNING.load(Ordering::Acquire) {
        return true;
    }

    let mut module = HMODULE::default();
    let address = PCWSTR::from_raw((lotus_shell_bridge_hook as *const ()).cast::<u16>());
    // SAFETY: FROM_ADDRESS interprets the pointer as an address inside this loaded bridge rather
    // than a string and acquires the worker's independent module reference.
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

    // SAFETY: The module reference keeps the callback live until it removes the process hook and
    // exits through FreeLibraryAndExitThread.
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
        // SAFETY: Releases the independent reference acquired above after thread creation failed.
        let _ = unsafe { FreeLibrary(module) };
        return false;
    };

    CLEANUP_WORKER_RUNNING.store(true, Ordering::Release);
    // SAFETY: The worker owns its lifetime independently; only this duplicated thread handle is
    // closed here.
    let _ = unsafe { CloseHandle(worker) };
    true
}

unsafe extern "system" fn cleanup_worker(parameter: *mut c_void) -> u32 {
    while tick_count() <= LEASE_DEADLINE.load(Ordering::Acquire) {
        // SAFETY: A short wait keeps cleanup off the shell UI thread and has no pointer inputs.
        unsafe { Sleep(25) };
    }

    ENABLED.store(false, Ordering::Release);
    if let Some(target) = set_window_pos_address() {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: This worker owns cleanup for the exact process hook created by this module.
            let _ = unsafe { MinHook::disable_hook(target) };
            // SAFETY: The detour is disabled before its trampoline and bookkeeping are removed.
            let _ = unsafe { MinHook::remove_hook(target) };
        }));
    }
    HOOK_READY.store(false, Ordering::Release);
    ORIGINAL_SET_WINDOW_POS.store(0, Ordering::Release);
    CLEANUP_WORKER_RUNNING.store(false, Ordering::Release);

    let module = HMODULE(parameter);
    // SAFETY: This raw worker owns the independent module reference and no bridge code executes
    // after the atomic cleanup above.
    unsafe { FreeLibraryAndExitThread(module, 0) }
}

fn set_window_pos_address() -> Option<*mut c_void> {
    // SAFETY: USER32 is loaded in the GUI process and the returned module handle is borrowed.
    let module = unsafe { GetModuleHandleW(w!("user32.dll")) }.ok()?;
    // SAFETY: The module is live and the exported name is static and null terminated.
    let procedure =
        unsafe { GetProcAddress(module, PCSTR(c"SetWindowPos".as_ptr().cast::<u8>())) }?;
    Some(procedure as *mut c_void)
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
    // SAFETY: MinHook returned a trampoline with SetWindowPos's exact system ABI.
    let original: SetWindowPosFn = unsafe { std::mem::transmute(original) };

    if lease_active()
        && is_control_center_window(window)
        && let Some(position) = desired_position(window, width, height, flags)
    {
        x = position.0;
        y = position.1;
        flags.0 &= !SWP_NOMOVE.0;
    }

    // SAFETY: The original ABI and all arguments are preserved apart from the validated position.
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
    // SAFETY: Reading the monotonic system tick count has no preconditions or side effects.
    unsafe { GetTickCount64() }
}

fn is_control_center_window(window: HWND) -> bool {
    let mut class_name = [0_u16; 64];
    // SAFETY: The borrowed HWND is queried synchronously into writable storage.
    let length = unsafe { GetClassNameW(window, &mut class_name) };
    let Ok(length) = usize::try_from(length) else {
        return false;
    };
    // SAFETY: The macro produces a static, null-terminated UTF-16 string.
    let expected_name = w!("ControlCenterWindow");
    // SAFETY: The macro produces a static, null-terminated UTF-16 string.
    let expected = unsafe { expected_name.as_wide() };
    class_name.get(..length) == Some(expected)
}

fn desired_position(
    window: HWND,
    width: i32,
    height: i32,
    flags: SET_WINDOW_POS_FLAGS,
) -> Option<(i32, i32)> {
    if flags.0 & (SWP_NOMOVE.0 | SWP_NOSIZE.0) == SWP_NOMOVE.0 | SWP_NOSIZE.0 {
        return None;
    }

    let mut current = RECT::default();
    // SAFETY: The live target HWND is queried synchronously into writable storage.
    unsafe { GetWindowRect(window, &raw mut current) }.ok()?;
    let actual_width = if flags.0 & SWP_NOSIZE.0 != 0 {
        current.right.saturating_sub(current.left)
    } else {
        width
    };
    let actual_height = if flags.0 & SWP_NOSIZE.0 != 0 {
        current.bottom.saturating_sub(current.top)
    } else {
        height
    };
    if actual_width <= 0 || actual_height <= 0 {
        return None;
    }

    let anchor_x = ANCHOR_X.load(Ordering::Acquire);
    let anchor_y = ANCHOR_Y.load(Ordering::Acquire);
    // SAFETY: Selecting the nearest monitor for a physical screen point has no side effects.
    let monitor = unsafe {
        MonitorFromPoint(
            POINT {
                x: anchor_x,
                y: anchor_y,
            },
            MONITOR_DEFAULTTONEAREST,
        )
    };
    let mut monitor_info = MONITORINFO {
        cbSize: u32::try_from(size_of::<MONITORINFO>()).ok()?,
        ..MONITORINFO::default()
    };
    // SAFETY: The monitor handle is live and the correctly sized output remains writable.
    if !unsafe { GetMonitorInfoW(monitor, &raw mut monitor_info) }.as_bool() {
        return None;
    }

    // SAFETY: Reading the DPI for this live HWND has no side effects.
    let dpi = unsafe { GetDpiForWindow(window) }.max(96);
    let inset = EDGE_INSET_DIP.saturating_mul(i32::try_from(dpi).unwrap_or(96)) / 96;
    let minimum_x = monitor_info.rcWork.left.saturating_add(inset);
    let maximum_x = monitor_info
        .rcWork
        .right
        .saturating_sub(actual_width)
        .saturating_sub(inset)
        .max(minimum_x);
    let minimum_y = monitor_info.rcWork.top;
    let maximum_y = monitor_info
        .rcWork
        .bottom
        .saturating_sub(actual_height)
        .max(minimum_y);
    let x = anchor_x
        .saturating_sub(actual_width / 2)
        .clamp(minimum_x, maximum_x);
    let y = anchor_y
        .saturating_sub(actual_height)
        .clamp(minimum_y, maximum_y);
    Some((x, y))
}
