use std::env;
use std::path::{Path, PathBuf};

use lotus_shell_bridge::{
    ACK_MESSAGE_NAME, CONFIG_MESSAGE_NAME, DISABLE_SENTINEL, HOOK_EXPORT_NAME,
    encode_anchor,
};
use windows::Win32::Foundation::{FreeLibrary, HINSTANCE, HMODULE, HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowThreadProcessId, HHOOK, HOOKPROC, MSG, PM_REMOVE, PeekMessageW,
    RegisterWindowMessageW, SEND_MESSAGE_TIMEOUT_FLAGS, SMTO_ABORTIFHUNG,
    SendMessageTimeoutW, SetWindowsHookExW, UnhookWindowsHookEx, WH_CALLWNDPROC,
};
use windows::core::{HSTRING, PCSTR};

const BRIDGE_FILE_NAME: &str = "lotus_shell_bridge.dll";
const MESSAGE_TIMEOUT_MILLISECONDS: u32 = 250;

pub(crate) struct ShellBridgeLease {
    module: HMODULE,
    hook: HHOOK,
    window: HWND,
    owner: HWND,
    message: u32,
    ack_message: u32,
}

impl ShellBridgeLease {
    pub(crate) fn attach(window: HWND, owner: HWND) -> Option<Self> {
        let thread_id = trusted_shell_thread(window)?;
        let path = bridge_path()?;
        let path = HSTRING::from(path.as_os_str());

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
        let ack_message = unsafe { RegisterWindowMessageW(ACK_MESSAGE_NAME) };
        if message == 0 || ack_message == 0 {
            let _ = unsafe { UnhookWindowsHookEx(hook) };
            let _ = unsafe { FreeLibrary(module) };
            return None;
        }

        Some(Self {
            module,
            hook,
            window,
            owner,
            message,
            ack_message,
        })
    }

    pub(crate) fn configure(&self, anchor_x: i32, anchor_y: i32) -> bool {
        self.send(encode_anchor(anchor_x, anchor_y))
    }

    fn send(&self, configuration: isize) -> bool {
        send_configuration(
            self.window,
            self.owner,
            self.message,
            self.ack_message,
            configuration,
        )
    }
}

impl Drop for ShellBridgeLease {
    fn drop(&mut self) {
        let _ = self.send(DISABLE_SENTINEL);
        let _ = unsafe { UnhookWindowsHookEx(self.hook) };
        let _ = unsafe { FreeLibrary(self.module) };
    }
}

fn send_configuration(
    window: HWND,
    owner: HWND,
    message: u32,
    ack_message: u32,
    configuration: isize,
) -> bool {
    let outcome = unsafe {
        SendMessageTimeoutW(
            window,
            message,
            WPARAM(owner.0.addr()),
            LPARAM(configuration),
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
            Some(owner),
            ack_message,
            ack_message,
            PM_REMOVE,
        )
    }
    .as_bool()
        && acknowledgement.wParam.0 == 1
}

fn trusted_shell_thread(window: HWND) -> Option<u32> {
    let mut class_name = [0_u16; 64];
    let length = unsafe { GetClassNameW(window, &mut class_name) };
    let length = usize::try_from(length).ok()?;
    let class_name = String::from_utf16_lossy(&class_name[..length]);
    if class_name != "ControlCenterWindow" && class_name != "Windows.UI.Core.CoreWindow" {
        return None;
    }

    let mut process_id = 0;
    let thread_id = unsafe { GetWindowThreadProcessId(window, Some(&raw mut process_id)) };
    if thread_id == 0 || process_id == 0 {
        return None;
    }

    let actual = crate::window_tracker::process_image_path(process_id)?;
    let system_root = env::var_os("SystemRoot")?;
    let expected = PathBuf::from(system_root)
        .join("System32")
        .join("ShellHost.exe");
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
