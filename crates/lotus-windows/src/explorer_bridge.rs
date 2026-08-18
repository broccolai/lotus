use std::env;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{FreeLibrary, HINSTANCE, HMODULE, HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetClassNameW, GetWindowThreadProcessId, HHOOK, HOOKPROC, MSG, PM_REMOVE,
    PeekMessageW, RegisterWindowMessageW, SEND_MESSAGE_TIMEOUT_FLAGS, SMTO_ABORTIFHUNG,
    SendMessageTimeoutW, SetWindowsHookExW, UnhookWindowsHookEx, WH_CALLWNDPROC,
};
use windows::core::{HSTRING, PCSTR, PCWSTR, w};

const BRIDGE_FILE_NAME: &str = "lotus_explorer_bridge.dll";
const CONFIG_MESSAGE_NAME: PCWSTR = w!("Lotus.ExplorerBridge.Configure.v1");
const ACK_MESSAGE_NAME: PCWSTR = w!("Lotus.ExplorerBridge.Acknowledge.v1");
const HOOK_EXPORT_NAME: &[u8] = b"lotus_explorer_bridge_hook\0";
const MESSAGE_TIMEOUT_MILLISECONDS: u32 = 500;

pub(crate) struct ExplorerBridgeLease {
    module: HMODULE,
    hook: HHOOK,
    taskbar: HWND,
    owner: HWND,
    message: u32,
    acknowledgement: u32,
}

impl ExplorerBridgeLease {
    pub(crate) fn attach(owner: HWND) -> Option<Self> {
        let taskbar = primary_taskbar()?;
        let thread_id = trusted_explorer_thread(taskbar)?;
        let path = HSTRING::from(bridge_path()?.as_os_str());

        let module = unsafe { LoadLibraryW(&path) }.ok()?;
        let procedure =
            unsafe { GetProcAddress(module, PCSTR::from_raw(HOOK_EXPORT_NAME.as_ptr())) };
        let Some(procedure) = procedure else {
            let _ = unsafe { FreeLibrary(module) };
            return None;
        };
        let hook_procedure: HOOKPROC = unsafe { std::mem::transmute(procedure) };
        let Ok(hook) = (unsafe {
            SetWindowsHookExW(
                WH_CALLWNDPROC,
                hook_procedure,
                Some(HINSTANCE(module.0)),
                thread_id,
            )
        }) else {
            let _ = unsafe { FreeLibrary(module) };
            return None;
        };

        let message = unsafe { RegisterWindowMessageW(CONFIG_MESSAGE_NAME) };
        let acknowledgement = unsafe { RegisterWindowMessageW(ACK_MESSAGE_NAME) };
        if message == 0 || acknowledgement == 0 {
            let _ = unsafe { UnhookWindowsHookEx(hook) };
            let _ = unsafe { FreeLibrary(module) };
            return None;
        }

        let lease = Self {
            module,
            hook,
            taskbar,
            owner,
            message,
            acknowledgement,
        };
        lease.configure(true).then_some(lease)
    }

    fn configure(&self, enabled: bool) -> bool {
        let outcome = unsafe {
            SendMessageTimeoutW(
                self.taskbar,
                self.message,
                WPARAM(self.owner.0.addr()),
                LPARAM(isize::from(enabled)),
                SEND_MESSAGE_TIMEOUT_FLAGS(SMTO_ABORTIFHUNG.0),
                MESSAGE_TIMEOUT_MILLISECONDS,
                None,
            )
        };
        if outcome.0 == 0 {
            return false;
        }

        let mut acknowledgement = MSG::default();
        unsafe {
            PeekMessageW(
                &raw mut acknowledgement,
                Some(self.owner),
                self.acknowledgement,
                self.acknowledgement,
                PM_REMOVE,
            )
        }
        .as_bool()
            && acknowledgement.wParam.0 == 1
    }
}

impl Drop for ExplorerBridgeLease {
    fn drop(&mut self) {
        let _ = self.configure(false);
        let _ = unsafe { UnhookWindowsHookEx(self.hook) };
        let _ = unsafe { FreeLibrary(self.module) };
    }
}

fn primary_taskbar() -> Option<HWND> {
    unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) }.ok()
}

fn trusted_explorer_thread(window: HWND) -> Option<u32> {
    let mut class_name = [0_u16; 32];
    let length = unsafe { GetClassNameW(window, &mut class_name) };
    let length = usize::try_from(length).ok()?;
    if String::from_utf16_lossy(&class_name[..length]) != "Shell_TrayWnd" {
        return None;
    }

    let mut process_id = 0;
    let thread_id = unsafe { GetWindowThreadProcessId(window, Some(&raw mut process_id)) };
    if thread_id == 0 || process_id == 0 {
        return None;
    }

    let actual = crate::window_tracker::process_image_path(process_id)?;
    let expected = PathBuf::from(env::var_os("SystemRoot")?).join("explorer.exe");
    same_windows_path(&actual, &expected).then_some(thread_id)
}

fn bridge_path() -> Option<PathBuf> {
    let path = env::current_exe().ok()?.parent()?.join(BRIDGE_FILE_NAME);
    path.is_file().then_some(path)
}

fn same_windows_path(left: &Path, right: &Path) -> bool {
    left.as_os_str()
        .to_string_lossy()
        .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
}
