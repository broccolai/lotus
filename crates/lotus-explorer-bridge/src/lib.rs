#![cfg(windows)]

#[cfg(not(target_pointer_width = "64"))]
compile_error!("the Lotus Explorer bridge supports only 64-bit Windows");

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use minhook::{MH_STATUS, MinHook};
use windows::Win32::Foundation::{
    CloseHandle, FreeLibrary, HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::System::LibraryLoader::{
    DisableThreadLibraryCalls, FreeLibraryAndExitThread,
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GetModuleHandleExW, GetModuleHandleW,
    GetProcAddress,
};
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows::Win32::System::Threading::{CreateThread, Sleep, THREAD_CREATION_FLAGS};
use windows::Win32::UI::WindowsAndMessaging::{
    CWPSTRUCT, CallNextHookEx, GetClassNameW, IsWindow, PostMessageW,
    RegisterWindowMessageW, SET_WINDOW_POS_FLAGS, SW_HIDE, SWP_HIDEWINDOW, SWP_SHOWWINDOW,
};
use windows::core::{BOOL, PCSTR, PCWSTR, w};

pub const CONFIG_MESSAGE_NAME: PCWSTR = w!("Lotus.ExplorerBridge.Configure.v1");
pub const ACK_MESSAGE_NAME: PCWSTR = w!("Lotus.ExplorerBridge.Acknowledge.v1");
pub const HOOK_EXPORT_NAME: &[u8] = b"lotus_explorer_bridge_hook\0";

type ShowWindowFn = unsafe extern "system" fn(HWND, i32) -> BOOL;
type SetWindowPosFn =
    unsafe extern "system" fn(HWND, HWND, i32, i32, i32, i32, SET_WINDOW_POS_FLAGS) -> BOOL;

static CONFIG_MESSAGE: AtomicU32 = AtomicU32::new(0);
static ACK_MESSAGE: AtomicU32 = AtomicU32::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
static HOOKS_READY: AtomicBool = AtomicBool::new(false);
static WORKER_RUNNING: AtomicBool = AtomicBool::new(false);
static OWNER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SHOW_WINDOW: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SHOW_WINDOW_ASYNC: AtomicUsize = AtomicUsize::new(0);
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
/// Receives configuration on Explorer's taskbar thread.
///
/// # Safety
///
/// `lparam` must follow the `WH_CALLWNDPROC` callback contract supplied by Windows.
pub unsafe extern "system" fn lotus_explorer_bridge_hook(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 && lparam.0 != 0 {
        // SAFETY: WH_CALLWNDPROC supplies a live CWPSTRUCT for this callback invocation.
        let message = unsafe { &*(lparam.0 as *const CWPSTRUCT) };
        handle_configuration(message);
    }

    // SAFETY: Every notification continues through Explorer's existing hook chain.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn handle_configuration(message: &CWPSTRUCT) {
    if message.message != registered_message(&CONFIG_MESSAGE, CONFIG_MESSAGE_NAME) {
        return;
    }

    let owner = message.wParam.0;
    let enable = message.lParam.0 != 0;
    let success = if enable {
        OWNER.store(owner, Ordering::Release);
        let installed = install_hooks();
        ENABLED.store(installed, Ordering::Release);
        if !installed {
            OWNER.store(0, Ordering::Release);
        }
        installed
    } else {
        ENABLED.store(false, Ordering::Release);
        uninstall_hooks();
        OWNER.store(0, Ordering::Release);
        true
    };
    acknowledge(owner, success);
}

fn install_hooks() -> bool {
    if HOOKS_READY.load(Ordering::Acquire) {
        return true;
    }
    if !start_owner_worker() {
        return false;
    }

    catch_unwind(AssertUnwindSafe(install_hooks_inner)).unwrap_or(false)
}

fn install_hooks_inner() -> bool {
    let Some(show_window) = user32_procedure(c"ShowWindow") else {
        return false;
    };
    let Some(show_window_async) = user32_procedure(c"ShowWindowAsync") else {
        return false;
    };
    let Some(set_window_pos) = user32_procedure(c"SetWindowPos") else {
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
        uninstall_hooks();
        return false;
    }

    for target in [show_window, show_window_async, set_window_pos] {
        if !matches!(
            // SAFETY: Each target was registered with MinHook and its trampoline was retained.
            unsafe { MinHook::enable_hook(target) },
            Ok(()) | Err(MH_STATUS::MH_ERROR_ENABLED)
        ) {
            uninstall_hooks();
            return false;
        }
    }

    HOOKS_READY.store(true, Ordering::Release);
    true
}

fn create_hook(target: *mut c_void, detour: *mut c_void, original: &AtomicUsize) -> bool {
    // SAFETY: The target is a live USER32 export and the supplied detour uses its exact ABI.
    match unsafe { MinHook::create_hook(target, detour) } {
        Ok(trampoline) => {
            original.store(trampoline.addr(), Ordering::Release);
            true
        }
        Err(MH_STATUS::MH_ERROR_ALREADY_CREATED) => original.load(Ordering::Acquire) != 0,
        Err(_) => false,
    }
}

fn uninstall_hooks() {
    let targets = [
        user32_procedure(c"ShowWindow"),
        user32_procedure(c"ShowWindowAsync"),
        user32_procedure(c"SetWindowPos"),
    ];
    for target in targets.into_iter().flatten() {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: Cleanup concerns only targets installed by this bridge in this process.
            let _ = unsafe { MinHook::disable_hook(target) };
            // SAFETY: The detour is disabled before its trampoline is released.
            let _ = unsafe { MinHook::remove_hook(target) };
        }));
    }

    HOOKS_READY.store(false, Ordering::Release);
    ORIGINAL_SHOW_WINDOW.store(0, Ordering::Release);
    ORIGINAL_SHOW_WINDOW_ASYNC.store(0, Ordering::Release);
    ORIGINAL_SET_WINDOW_POS.store(0, Ordering::Release);
}

fn start_owner_worker() -> bool {
    if WORKER_RUNNING.load(Ordering::Acquire) {
        return true;
    }

    let mut module = HMODULE::default();
    let address = PCWSTR::from_raw((lotus_explorer_bridge_hook as *const ()).cast::<u16>());
    // SAFETY: FROM_ADDRESS acquires a reference to the module containing this callback.
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

    // SAFETY: The worker owns the acquired module reference until FreeLibraryAndExitThread.
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
        // SAFETY: Thread creation failed, so this path releases the independent reference.
        let _ = unsafe { FreeLibrary(module) };
        return false;
    };

    WORKER_RUNNING.store(true, Ordering::Release);
    // SAFETY: The worker owns its execution; Lotus does not need the duplicated thread handle.
    let _ = unsafe { CloseHandle(worker) };
    true
}

unsafe extern "system" fn owner_worker(parameter: *mut c_void) -> u32 {
    loop {
        let owner = OWNER.load(Ordering::Acquire);
        let live = owner != 0 && {
            let hwnd = HWND(std::ptr::with_exposed_provenance_mut(owner));
            // SAFETY: The stored value is an opaque HWND supplied by Lotus.
            unsafe { IsWindow(Some(hwnd)).as_bool() }
        };
        if !live {
            break;
        }

        // SAFETY: This bounded wait runs off Explorer's UI thread.
        unsafe { Sleep(250) };
    }

    ENABLED.store(false, Ordering::Release);
    uninstall_hooks();
    OWNER.store(0, Ordering::Release);
    WORKER_RUNNING.store(false, Ordering::Release);

    let module = HMODULE(parameter);
    // SAFETY: The worker owns this module reference and executes no bridge code afterward.
    unsafe { FreeLibraryAndExitThread(module, 0) }
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
    // SAFETY: MinHook supplied a trampoline with ShowWindow's exact system ABI.
    let original: ShowWindowFn = unsafe { std::mem::transmute(original) };

    if ENABLED.load(Ordering::Acquire) && command != SW_HIDE.0 && is_taskbar_window(window)
    {
        return BOOL(1);
    }

    // SAFETY: The trampoline ABI and caller-provided arguments are unchanged.
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
    // SAFETY: MinHook supplied a trampoline with SetWindowPos's exact system ABI.
    let original: SetWindowPosFn = unsafe { std::mem::transmute(original) };

    if ENABLED.load(Ordering::Acquire) && is_taskbar_window(window) {
        flags.0 = (flags.0 & !SWP_SHOWWINDOW.0) | SWP_HIDEWINDOW.0;
    }

    // SAFETY: The original ABI is preserved; only taskbar visibility flags may be constrained.
    unsafe { original(window, insert_after, x, y, width, height, flags) }
}

fn is_taskbar_window(window: HWND) -> bool {
    let mut class_name = [0_u16; 32];
    // SAFETY: The borrowed HWND is queried synchronously into writable storage.
    let length = unsafe { GetClassNameW(window, &mut class_name) };
    let Ok(length) = usize::try_from(length) else {
        return false;
    };
    matches!(
        String::from_utf16_lossy(&class_name[..length]).as_str(),
        "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
    )
}

fn user32_procedure(name: &'static std::ffi::CStr) -> Option<*mut c_void> {
    // SAFETY: USER32 is loaded in Explorer and the returned handle is borrowed.
    let module = unsafe { GetModuleHandleW(w!("user32.dll")) }.ok()?;
    // SAFETY: The module is live and the export name has static null-terminated storage.
    let procedure = unsafe { GetProcAddress(module, PCSTR(name.as_ptr().cast::<u8>())) }?;
    Some(procedure as *mut c_void)
}

fn acknowledge(owner: usize, success: bool) {
    if owner == 0 {
        return;
    }
    let message = registered_message(&ACK_MESSAGE, ACK_MESSAGE_NAME);
    if message == 0 {
        return;
    }

    let owner = HWND(std::ptr::with_exposed_provenance_mut(owner));
    // SAFETY: A stale owner HWND simply causes the asynchronous post to fail.
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
