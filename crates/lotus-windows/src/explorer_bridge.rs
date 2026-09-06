use std::env;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    FreeLibrary, HANDLE, HINSTANCE, HMODULE, HWND, LPARAM, WPARAM,
};
use windows::Win32::System::Com::CoCreateGuid;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetClassNameW, GetPropW, GetWindowThreadProcessId, HHOOK, HOOKPROC,
    RegisterWindowMessageW, RemovePropW, SendNotifyMessageW, SetPropW, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_CALLWNDPROC,
};
use windows::core::{HSTRING, PCSTR, PCWSTR, w};

const CONFIG_MESSAGE_NAME: PCWSTR = w!("Lotus.ExplorerBridge.Configure.v2");
const ACK_PROPERTY_NAME: PCWSTR = w!("Lotus.ExplorerBridge.Acknowledged.v2");
const OWNER_PROPERTY_NAME: PCWSTR = w!("Lotus.ExplorerBridge.Owner.v2");
const LEASE_PROPERTY_NAME: PCWSTR = w!("Lotus.ExplorerBridge.Lease.v3");
const HOOK_EXPORT_NAME: &[u8] = b"lotus_explorer_bridge_hook\0";
const UI_PROGRESS_LEASE_MILLISECONDS: u64 = 2_000;
const CONFIGURATION_DEADLINE: Duration = Duration::from_secs(1);

pub(crate) struct ExplorerBridgeLease {
    module: HMODULE,
    hook: HHOOK,
    taskbar: HWND,
    explorer_process: u32,
    explorer_thread: u32,
    owner: HWND,
    message: u32,
    token: usize,
    configured_at: Instant,
}

impl ExplorerBridgeLease {
    pub(crate) fn attach(owner: HWND) -> Option<Self> {
        let taskbar = primary_taskbar()?;
        let (explorer_process, explorer_thread) = trusted_explorer_identity(taskbar)?;
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
                explorer_thread,
            )
        }) else {
            let _ = unsafe { FreeLibrary(module) };
            return None;
        };

        let message = unsafe { RegisterWindowMessageW(CONFIG_MESSAGE_NAME) };
        if message == 0 {
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
        let _ = unsafe { RemovePropW(owner, ACK_PROPERTY_NAME) };
        let lease = Self {
            module,
            hook,
            taskbar,
            explorer_process,
            explorer_thread,
            owner,
            message,
            token,
            configured_at: Instant::now(),
        };
        // The hook may be configured now, but suppression remains fail-open until the UI owns
        // a usable presentation and explicitly grants its first short lease.
        lease.report_ui_progress(false);
        lease.configure(true).then_some(lease)
    }

    pub(crate) fn reassert(&self) -> bool {
        self.configure(true)
    }

    /// Renews the short lease only from UI-owned progress. The injected bridge reads this
    /// property directly, so a responsive helper cannot conceal a stalled UI thread.
    pub(crate) fn report_ui_progress(&self, takeover_allowed: bool) {
        let value = lease_value(takeover_allowed);
        let _ = unsafe {
            SetPropW(
                self.owner,
                LEASE_PROPERTY_NAME,
                Some(HANDLE(std::ptr::with_exposed_provenance_mut(value))),
            )
        };
    }

    /// Stops suppression immediately before the caller restores native taskbars.
    pub(crate) fn revoke_takeover(&self) {
        self.report_ui_progress(false);
        let _ = self.configure(false);
    }

    pub(crate) fn is_usable(&self) -> bool {
        self.has_current_explorer_identity()
            && unsafe { GetPropW(self.owner, ACK_PROPERTY_NAME) }.0.addr()
                == configuration_value(self.token, true)
    }

    pub(crate) fn should_replace(&self) -> bool {
        !self.has_current_explorer_identity()
            || (!self.is_usable() && self.configured_at.elapsed() >= CONFIGURATION_DEADLINE)
    }

    fn configure(&self, enabled: bool) -> bool {
        send_configuration(self.taskbar, self.owner, self.message, enabled, self.token)
    }

    fn has_current_explorer_identity(&self) -> bool {
        trusted_explorer_identity(self.taskbar).is_some_and(|(process, thread)| {
            process == self.explorer_process && thread == self.explorer_thread
        })
    }
}

impl Drop for ExplorerBridgeLease {
    fn drop(&mut self) {
        self.revoke_takeover();
        if unsafe { GetPropW(self.owner, OWNER_PROPERTY_NAME) }
            .0
            .addr()
            == self.token
        {
            let _ = unsafe { RemovePropW(self.owner, OWNER_PROPERTY_NAME) };
        }
        let _ = unsafe { RemovePropW(self.owner, LEASE_PROPERTY_NAME) };
        let _ = unsafe { RemovePropW(self.owner, ACK_PROPERTY_NAME) };
        let _ = release_controller_module(self.hook, self.module);
    }
}

fn send_configuration(
    taskbar: HWND,
    owner: HWND,
    message: u32,
    enabled: bool,
    token: usize,
) -> bool {
    let configuration = configuration_value(token, enabled).cast_signed();
    unsafe {
        SendNotifyMessageW(
            taskbar,
            message,
            WPARAM(owner.0.addr()),
            LPARAM(configuration),
        )
    }
    .is_ok()
}

fn configuration_value(token: usize, enabled: bool) -> usize {
    (token << 1) | usize::from(enabled)
}

#[allow(clippy::cast_possible_truncation)]
fn lease_value(takeover_allowed: bool) -> usize {
    let expires_at =
        unsafe { GetTickCount64() }.saturating_add(UI_PROGRESS_LEASE_MILLISECONDS);
    let encoded = (expires_at << 1) | u64::from(takeover_allowed);
    usize::try_from(encoded).unwrap_or(0)
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

fn trusted_explorer_identity(window: HWND) -> Option<(u32, u32)> {
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
    same_windows_path(&actual, &expected).then_some((process_id, thread_id))
}

fn same_windows_path(left: &Path, right: &Path) -> bool {
    left.as_os_str()
        .to_string_lossy()
        .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
}
