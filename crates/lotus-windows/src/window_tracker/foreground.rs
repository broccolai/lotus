use std::ffi::c_void;
use std::mem::size_of;

use lotus_core::fullscreen::{ScreenRect, is_fullscreen_foreground};
use lotus_core::window::WindowId;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId, IsIconic,
};

use super::enumeration::should_include_window;
const FULLSCREEN_EDGE_TOLERANCE: i32 = 2;
pub(super) fn observe_fullscreen_window(own_process_id: u32) -> Option<WindowId> {
    let id = observe_foreground_window(own_process_id)?;
    is_fullscreen_window(id).then_some(id)
}
pub(super) fn is_fullscreen_window(id: WindowId) -> bool {
    let Some(hwnd) = hwnd_from_window_id(id) else {
        return false;
    };
    if unsafe { IsIconic(hwnd) }.as_bool() || !should_include_window(hwnd) {
        return false;
    }
    let Some(window) = window_bounds(hwnd) else {
        return false;
    };
    let Some(monitor) = monitor_bounds(hwnd) else {
        return false;
    };
    is_fullscreen_foreground(
        true,
        screen_rect(window),
        screen_rect(monitor),
        FULLSCREEN_EDGE_TOLERANCE,
    )
}
pub(super) fn observe_foreground_window(own_process_id: u32) -> Option<WindowId> {
    let hwnd = unsafe { GetForegroundWindow() };
    let id = window_id(hwnd)?;
    let mut process_id = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut process_id)) };
    (process_id != 0 && process_id != own_process_id && should_include_window(hwnd))
        .then_some(id)
}
fn window_bounds(hwnd: HWND) -> Option<RECT> {
    let mut bounds = RECT::default();
    unsafe { GetWindowRect(hwnd, &raw mut bounds) }.ok()?;
    Some(bounds)
}
fn monitor_bounds(hwnd: HWND) -> Option<RECT> {
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_invalid() {
        return None;
    }
    let mut info = MONITORINFO {
        cbSize: u32_size::<MONITORINFO>(),
        ..MONITORINFO::default()
    };
    unsafe { GetMonitorInfoW(monitor, &raw mut info) }
        .as_bool()
        .then_some(info.rcMonitor)
}
const fn screen_rect(rect: RECT) -> ScreenRect {
    ScreenRect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}
fn window_id(hwnd: HWND) -> Option<WindowId> {
    (!hwnd.0.is_null())
        .then(|| u64::try_from(hwnd.0.addr()).ok())
        .flatten()
        .map(WindowId::new)
}
pub(super) fn hwnd_from_window_id(window: WindowId) -> Option<HWND> {
    let address = usize::try_from(window.get()).ok()?;
    (address != 0).then(|| HWND(std::ptr::with_exposed_provenance_mut::<c_void>(address)))
}
pub(super) fn same_monitor(left: HWND, right: HWND) -> bool {
    let (left, right) = unsafe {
        (
            MonitorFromWindow(left, MONITOR_DEFAULTTONEAREST),
            MonitorFromWindow(right, MONITOR_DEFAULTTONEAREST),
        )
    };
    !left.is_invalid() && left == right
}
#[allow(
    clippy::cast_possible_truncation,
    reason = "Win32 ABI scalar sizes are fixed and far below u32::MAX"
)]
const fn u32_size<T>() -> u32 {
    size_of::<T>() as u32
}
