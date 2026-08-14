use std::ffi::c_void;
use std::ptr;

use lotus_core::activation::ActivationDecision;
use lotus_core::dock::DockItem;
use lotus_core::window::WindowId;
use thiserror::Error;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Shell::{SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, IsIconic, IsWindow, PostMessageW, SW_MINIMIZE, SW_RESTORE,
    SW_SHOWNORMAL, ShowWindow, SwitchToThisWindow, WM_CLOSE,
};
use windows::core::PCWSTR;

use super::launch::expand_environment_variables;
use crate::NativeError;
use crate::interaction::activate_window;

#[derive(Debug, Error)]
pub enum ActivationError {
    #[error("window identity {0:?} cannot be represented as an HWND")]
    InvalidWindowId(WindowId),
    #[error("window {0:?} no longer exists")]
    MissingWindow(WindowId),
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
    // SAFETY: GetForegroundWindow takes no pointers and returns either a live
    // borrowed HWND or null. Lotus does not own or destroy the handle.
    let window = unsafe { GetForegroundWindow() };
    let address = window.0.addr();
    if address == 0 {
        None
    } else {
        u64::try_from(address).ok().map(WindowId::new)
    }
}

pub fn execute_activation(
    decision: ActivationDecision<WindowId>,
    item: &DockItem,
) -> Result<(), ActivationError> {
    match decision {
        ActivationDecision::Launch => launch(item),
        ActivationDecision::Minimize(window) => minimize(window),
        ActivationDecision::Focus(window) => focus(window),
    }
}

fn minimize(window: WindowId) -> Result<(), ActivationError> {
    let hwnd = existing_window(window)?;
    // SAFETY: `existing_window` verified the borrowed HWND immediately before
    // this non-blocking request. The return value is prior visibility, not an
    // operation-success indicator.
    let _was_visible = unsafe { ShowWindow(hwnd, SW_MINIMIZE) };
    Ok(())
}

fn focus(window: WindowId) -> Result<(), ActivationError> {
    let hwnd = existing_window(window)?;
    // SAFETY: `hwnd` was validated immediately above and remains borrowed.
    if unsafe { IsIconic(hwnd) }.as_bool() {
        // SAFETY: ShowWindow does not transfer ownership; its BOOL is the prior
        // visibility state rather than a failure signal.
        let _was_visible = unsafe { ShowWindow(hwnd, SW_RESTORE) };
    }

    if activate_window(hwnd).is_owned() {
        Ok(())
    } else {
        Err(ActivationError::ForegroundDenied(window))
    }
}

pub fn focus_window(window: WindowId) -> Result<(), ActivationError> {
    focus(window)
}

pub fn switch_window(window: WindowId) -> Result<(), ActivationError> {
    let hwnd = existing_window(window)?;
    // SAFETY: `hwnd` was validated immediately above and remains borrowed.
    if unsafe { IsIconic(hwnd) }.as_bool() {
        // SAFETY: ShowWindow does not transfer ownership; its BOOL is the prior
        // visibility state rather than a failure signal.
        let _was_visible = unsafe { ShowWindow(hwnd, SW_RESTORE) };
    }

    // SAFETY: `hwnd` is a live borrowed top-level window. Passing true selects
    // the native Alt/Ctrl+Tab switching behavior documented by Windows.
    unsafe { SwitchToThisWindow(hwnd, true) };
    if activate_window(hwnd).is_owned() {
        Ok(())
    } else {
        Err(ActivationError::ForegroundDenied(window))
    }
}

pub fn request_window_close(window: WindowId) -> Result<(), ActivationError> {
    let hwnd = existing_window(window)?;
    // SAFETY: The HWND was validated immediately above. Posting WM_CLOSE transfers no pointers
    // and lets the owning application run its normal close and save-confirmation path.
    unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }
        .map_err(|_| ActivationError::MissingWindow(window))
}

fn existing_window(window: WindowId) -> Result<HWND, ActivationError> {
    let hwnd = hwnd_from_id(window)?;
    // SAFETY: IsWindow only inspects the numeric handle and accepts stale
    // values; no ownership or lifetime claim is made by this check.
    unsafe { IsWindow(Some(hwnd)) }
        .as_bool()
        .then_some(hwnd)
        .ok_or(ActivationError::MissingWindow(window))
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

    // SAFETY: `execute` has the documented size, all optional fields are null,
    // and its UTF-16 buffers remain alive through this synchronous call. No
    // process handle is requested, so there is no returned ownership to close.
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
