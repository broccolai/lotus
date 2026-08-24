use std::ffi::c_void;
use std::ptr;

use lotus_core::activation::ActivationDecision;
use lotus_core::dock::DockItem;
use lotus_core::window::{TrackedWindowKey, WindowId};
use thiserror::Error;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Shell::{SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, IsIconic, IsWindow, PostMessageW, SC_CLOSE, SW_MINIMIZE,
    SW_RESTORE, SW_SHOWNORMAL, ShowWindow, SwitchToThisWindow, WM_SYSCOMMAND,
};
use windows::core::{BOOL, PCWSTR};

use super::launch::expand_environment_variables;
use crate::NativeError;
use crate::interaction::activate_exact_window;

#[link(name = "user32")]
unsafe extern "system" {
    fn EndTask(window: HWND, shutdown: BOOL, force: BOOL) -> BOOL;
}

#[derive(Debug, Error)]
pub enum ActivationError {
    #[error("window identity {0:?} cannot be represented as an HWND")]
    InvalidWindowId(WindowId),
    #[error("window {0:?} no longer exists")]
    MissingWindow(WindowId),
    #[error("window {key:?} now belongs to process {actual_process_id}")]
    IdentityMismatch {
        key: TrackedWindowKey,
        actual_process_id: u32,
    },
    #[error("window {0:?} is no longer the tracker-published incarnation")]
    RetiredWindow(TrackedWindowKey),
    #[error("Windows could not deliver the close request to {window:?}: {source}")]
    CloseDelivery {
        window: TrackedWindowKey,
        #[source]
        source: windows::core::Error,
    },
    #[error("Windows refused to force close {0:?}")]
    ForceCloseDenied(WindowId),
    #[error("Windows denied foreground activation for {0:?}")]
    ForegroundDenied(WindowId),
    #[error("the dock item has an empty launch target")]
    EmptyLaunchTarget,
    #[error("the launch target's environment variables could not be expanded")]
    EnvironmentExpansion,
    #[error("Windows could not shell-launch {target}: {source}")]
    Launch {
        target: String,
        #[source]
        source: NativeError,
    },
}

pub fn foreground_window() -> Option<WindowId> {
    let window = unsafe { GetForegroundWindow() };
    let address = window.0.addr();
    if address == 0 {
        None
    } else {
        u64::try_from(address).ok().map(WindowId::new)
    }
}

pub fn execute_activation(
    decision: ActivationDecision<TrackedWindowKey>,
    item: &DockItem,
) -> Result<(), ActivationError> {
    match decision {
        ActivationDecision::Launch => launch(item),
        ActivationDecision::Minimize(window) => minimize(window),
        ActivationDecision::Focus(window) => focus(window),
    }
}

fn minimize(window: TrackedWindowKey) -> Result<(), ActivationError> {
    let existing = existing_window(window)?;
    let _was_visible = unsafe { ShowWindow(existing.hwnd, SW_MINIMIZE) };
    drop(existing);
    ensure_current(window)?;
    Ok(())
}

fn focus(window: TrackedWindowKey) -> Result<(), ActivationError> {
    let existing = existing_window(window)?;
    if unsafe { IsIconic(existing.hwnd) }.as_bool() {
        let _was_visible = unsafe { ShowWindow(existing.hwnd, SW_RESTORE) };
    }
    drop(existing);

    ensure_current(window)?;
    let existing = existing_window(window)?;
    let activated = activate_exact_window(existing.hwnd).is_owned();
    drop(existing);
    if activated {
        ensure_current(window)?;
        Ok(())
    } else {
        ensure_current(window)?;
        Err(ActivationError::ForegroundDenied(window.id))
    }
}

pub fn focus_window(window: TrackedWindowKey) -> Result<(), ActivationError> {
    focus(window)
}

pub fn switch_window(window: TrackedWindowKey) -> Result<(), ActivationError> {
    let existing = existing_window(window)?;
    if unsafe { IsIconic(existing.hwnd) }.as_bool() {
        let _was_visible = unsafe { ShowWindow(existing.hwnd, SW_RESTORE) };
    }

    unsafe { SwitchToThisWindow(existing.hwnd, true) };
    drop(existing);
    ensure_current(window)?;
    let existing = existing_window(window)?;
    let activated = activate_exact_window(existing.hwnd).is_owned();
    drop(existing);
    if activated {
        ensure_current(window)?;
        Ok(())
    } else {
        ensure_current(window)?;
        Err(ActivationError::ForegroundDenied(window.id))
    }
}

pub fn request_window_close(window: TrackedWindowKey) -> Result<(), ActivationError> {
    let existing = existing_window(window)?;
    let posted = unsafe {
        PostMessageW(
            Some(existing.hwnd),
            WM_SYSCOMMAND,
            WPARAM(SC_CLOSE as usize),
            LPARAM(0),
        )
    };
    drop(existing);
    match posted {
        Ok(()) => ensure_current(window),
        Err(source) => classify_close_delivery(window, source, ensure_current(window)),
    }
}

pub fn force_window_close(window: TrackedWindowKey) -> Result<(), ActivationError> {
    let existing = existing_window(window)?;
    // `existing_window` established this HWND is a current top-level window identity.
    let ended =
        unsafe { EndTask(existing.hwnd, BOOL::from(false), BOOL::from(true)) }.as_bool();
    drop(existing);
    if ended {
        return ensure_current(window);
    }
    ensure_current(window)?;
    Err(ActivationError::ForceCloseDenied(window.id))
}

struct ExistingWindow {
    hwnd: HWND,
    _current: crate::window_tracker::CurrentTrackedWindow,
}

fn existing_window(key: TrackedWindowKey) -> Result<ExistingWindow, ActivationError> {
    let Some(current) = crate::window_tracker::hold_current_tracked_window(key) else {
        crate::window_tracker::report_stale_target(key);
        return Err(ActivationError::RetiredWindow(key));
    };
    let hwnd = hwnd_from_id(key.id)?;
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        drop(current);
        crate::window_tracker::report_stale_target(key);
        return Err(ActivationError::MissingWindow(key.id));
    }
    let mut process_id = 0;
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(
            hwnd,
            Some(&raw mut process_id),
        )
    };
    if process_id != key.process_id {
        drop(current);
        crate::window_tracker::report_stale_target(key);
        return Err(ActivationError::IdentityMismatch {
            key,
            actual_process_id: process_id,
        });
    }
    Ok(ExistingWindow {
        hwnd,
        _current: current,
    })
}

fn ensure_current(key: TrackedWindowKey) -> Result<(), ActivationError> {
    existing_window(key).map(|_| ())
}

fn classify_close_delivery(
    window: TrackedWindowKey,
    source: windows::core::Error,
    revalidation: Result<(), ActivationError>,
) -> Result<(), ActivationError> {
    revalidation?;
    Err(ActivationError::CloseDelivery { window, source })
}

fn hwnd_from_id(window: WindowId) -> Result<HWND, ActivationError> {
    let address = usize::try_from(window.get())
        .map_err(|_| ActivationError::InvalidWindowId(window))?;
    if address == 0 {
        return Err(ActivationError::InvalidWindowId(window));
    }

    Ok(HWND(ptr::with_exposed_provenance_mut::<c_void>(address)))
}

fn launch(item: &DockItem) -> Result<(), ActivationError> {
    launch_target(&item.launch_target, item.arguments.as_deref())
}

pub fn launch_target(target: &str, arguments: Option<&str>) -> Result<(), ActivationError> {
    let request = LaunchRequest::new(target, arguments)?;
    let file = wide_null(&request.target);
    let parameters = request.arguments.as_deref().map(wide_null);
    let mut execute = SHELLEXECUTEINFOW {
        cbSize: u32::try_from(size_of::<SHELLEXECUTEINFOW>())
            .expect("SHELLEXECUTEINFOW size fits u32"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: parameters
            .as_ref()
            .map_or_else(PCWSTR::null, |value| PCWSTR(value.as_ptr())),
        nShow: SW_SHOWNORMAL.0,
        ..SHELLEXECUTEINFOW::default()
    };

    unsafe { ShellExecuteExW(&raw mut execute) }.map_err(|source| ActivationError::Launch {
        target: request.target,
        source: source.into(),
    })
}

#[derive(Debug, Eq, PartialEq)]
struct LaunchRequest {
    target: String,
    arguments: Option<String>,
}

impl LaunchRequest {
    fn new(target: &str, arguments: Option<&str>) -> Result<Self, ActivationError> {
        if target.trim().is_empty() {
            return Err(ActivationError::EmptyLaunchTarget);
        }

        let target = expand_environment_variables(target)
            .ok_or(ActivationError::EnvironmentExpansion)?;
        Ok(Self {
            target,
            arguments: arguments.map(str::to_owned),
        })
    }
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}
