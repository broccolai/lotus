use std::env;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{
    FreeLibrary, HANDLE, HINSTANCE, HMODULE, HWND, LPARAM, WPARAM,
};
use windows::Win32::System::Com::CoCreateGuid;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetClassNameW, GetPropW, GetWindowThreadProcessId, HHOOK, HOOKPROC, MSG,
    PM_REMOVE, PeekMessageW, RegisterWindowMessageW, RemovePropW,
    SEND_MESSAGE_TIMEOUT_FLAGS, SMTO_ABORTIFHUNG, SendMessageTimeoutW, SetPropW,
    SetWindowsHookExW, UnhookWindowsHookEx, WH_CALLWNDPROC,
};
use windows::core::{HSTRING, PCSTR, PCWSTR, w};

const CONFIG_MESSAGE_NAME: PCWSTR = w!("Lotus.ExplorerBridge.Configure.v2");
const ACK_MESSAGE_NAME: PCWSTR = w!("Lotus.ExplorerBridge.Acknowledge.v2");
const OWNER_PROPERTY_NAME: PCWSTR = w!("Lotus.ExplorerBridge.Owner.v2");
const HOOK_EXPORT_NAME: &[u8] = b"lotus_explorer_bridge_hook\0";
const MESSAGE_TIMEOUT_MILLISECONDS: u32 = 500;

pub(crate) struct ExplorerBridgeLease {
    module: HMODULE,
    hook: HHOOK,
    taskbar: HWND,
    owner: HWND,
    message: u32,
    acknowledgement: u32,
    token: usize,
}

impl ExplorerBridgeLease {
    pub(crate) fn attach(owner: HWND) -> Option<Self> {
        let taskbar = primary_taskbar()?;
        let thread_id = trusted_explorer_thread(taskbar)?;
        let path = HSTRING::from(
            crate::bridge_cache::cached_bridge_path(
                crate::bridge_cache::BridgeBinary::Explorer,
            )?
            .as_os_str(),
        );

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
            release_controller_module(hook, module);
            return None;
        }

        let Some(token) = lease_token() else {
            release_controller_module(hook, module);
            return None;
        };
        if unsafe {
            SetPropW(
                owner,
                OWNER_PROPERTY_NAME,
                Some(HANDLE(std::ptr::with_exposed_provenance_mut(token))),
            )
        }
        .is_err()
        {
            release_controller_module(hook, module);
            return None;
        }
        let lease = Self {
            module,
            hook,
            taskbar,
            owner,
            message,
            acknowledgement,
            token,
        };
        lease.configure(true).then_some(lease)
    }

    fn configure(&self, enabled: bool) -> bool {
        send_configuration(
            self.taskbar,
            self.owner,
            self.message,
            self.acknowledgement,
            enabled,
            self.token,
        )
    }
}

impl Drop for ExplorerBridgeLease {
    fn drop(&mut self) {
        let _ = self.configure(false);
        if unsafe { GetPropW(self.owner, OWNER_PROPERTY_NAME) }
            .0
            .addr()
            == self.token
        {
            let _ = unsafe { RemovePropW(self.owner, OWNER_PROPERTY_NAME) };
        }
        let _ = release_controller_module(self.hook, self.module);
    }
}

fn send_configuration(
    taskbar: HWND,
    owner: HWND,
    message: u32,
    acknowledgement_message: u32,
    enabled: bool,
    token: usize,
) -> bool {
    let configuration = ((token << 1) | usize::from(enabled)).cast_signed();
    let outcome = unsafe {
        SendMessageTimeoutW(
            taskbar,
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

    loop {
        let mut acknowledgement = MSG::default();
        if !unsafe {
            PeekMessageW(
                &raw mut acknowledgement,
                Some(owner),
                acknowledgement_message,
                acknowledgement_message,
                PM_REMOVE,
            )
        }
        .as_bool()
        {
            return false;
        }
        if acknowledgement.lParam.0 == configuration {
            return acknowledgement.wParam.0 == 1;
        }
    }
}

#[allow(clippy::cast_possible_truncation)]
fn lease_token() -> Option<usize> {
    let guid = unsafe { CoCreateGuid() }.ok()?;
    let value = guid.to_u128();
    let token =
        ((value as u64 ^ (value >> 64) as u64) as usize) & (isize::MAX as usize >> 1);
    (token != 0).then_some(token)
}

fn release_controller_module(hook: HHOOK, module: HMODULE) -> bool {
    if unsafe { UnhookWindowsHookEx(hook) }.is_ok() {
        let _ = unsafe { FreeLibrary(module) };
        true
    } else {
        false
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

fn same_windows_path(left: &Path, right: &Path) -> bool {
    left.as_os_str()
        .to_string_lossy()
        .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
}
