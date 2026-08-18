use std::mem::size_of;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, GetKeyState, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT,
    TrackMouseEvent, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{SM_CXDRAG, SM_CYDRAG};

pub(crate) fn capture_pointer(hwnd: HWND) {
    unsafe { SetCapture(hwnd) };
}

pub(crate) fn release_pointer(hwnd: HWND) {
    unsafe {
        if GetCapture() == hwnd {
            let _ = ReleaseCapture();
        }
    }
}

pub(crate) fn track_pointer_leave(hwnd: HWND) -> bool {
    let mut tracking = TRACKMOUSEEVENT {
        cbSize: track_mouse_event_size(),
        dwFlags: TME_LEAVE,
        hwndTrack: hwnd,
        ..TRACKMOUSEEVENT::default()
    };
    unsafe { TrackMouseEvent(&raw mut tracking) }.is_ok()
}

pub(crate) fn key_is_pressed(key: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(i32::from(key.0)) }.cast_unsigned() & 0x8000 != 0
}

pub(crate) fn drag_threshold(hwnd: HWND) -> (u32, u32) {
    let (horizontal, vertical) = unsafe {
        let dpi = GetDpiForWindow(hwnd).max(1);
        (
            GetSystemMetricsForDpi(SM_CXDRAG, dpi),
            GetSystemMetricsForDpi(SM_CYDRAG, dpi),
        )
    };
    (
        u32::try_from(horizontal).unwrap_or(1).max(1),
        u32::try_from(vertical).unwrap_or(1).max(1),
    )
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "TRACKMOUSEEVENT is a fixed Win32 ABI structure far smaller than u32::MAX"
)]
const fn track_mouse_event_size() -> u32 {
    size_of::<TRACKMOUSEEVENT>() as u32
}
