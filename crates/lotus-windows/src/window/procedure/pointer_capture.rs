use windows::Win32::Foundation::{HWND, LPARAM, LRESULT};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::WindowsAndMessaging::{
    WM_CANCELMODE, WM_CAPTURECHANGED, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
};

use super::{PointerEvent, WindowEvent, push_window_event, with_window_state};
use crate::platform::windows::interaction::{
    capture_pointer, release_pointer, track_pointer_leave,
};

pub(super) fn dispatch(hwnd: HWND, message: u32, lparam: LPARAM) -> Option<LRESULT> {
    let event = match message {
        WM_MOUSEMOVE => {
            begin_mouse_leave_tracking(hwnd);
            let (x, y) = client_point_from_lparam(lparam);
            PointerEvent::Moved { x, y }
        }
        WM_MOUSELEAVE => {
            with_window_state(hwnd, |state| state.tracking_mouse_leave.set(false));
            PointerEvent::Left
        }
        WM_LBUTTONDOWN => {
            with_window_state(hwnd, |state| state.left_button_pressed.set(true));
            capture_pointer(hwnd);
            let (x, y) = client_point_from_lparam(lparam);
            PointerEvent::LeftButtonPressed { x, y }
        }
        WM_LBUTTONUP => {
            with_window_state(hwnd, |state| state.left_button_pressed.set(false));
            release_pointer(hwnd);
            let (x, y) = client_point_from_lparam(lparam);
            PointerEvent::LeftButtonReleased { x, y }
        }
        WM_CANCELMODE | WM_CAPTURECHANGED => {
            cancel_pointer_if_pressed(hwnd);
            return Some(LRESULT(0));
        }
        _ => return None,
    };
    push_window_event(hwnd, WindowEvent::Pointer(event));
    Some(LRESULT(0))
}

fn begin_mouse_leave_tracking(hwnd: HWND) {
    with_window_state(hwnd, |state| {
        if state.tracking_mouse_leave.get() {
            return;
        }
        if track_pointer_leave(hwnd) {
            state.tracking_mouse_leave.set(true);
        }
    });
}

fn cancel_pointer_if_pressed(hwnd: HWND) {
    let mut was_pressed = false;
    with_window_state(hwnd, |state| {
        was_pressed = state.left_button_pressed.replace(false);
    });
    if was_pressed {
        push_window_event(hwnd, WindowEvent::Pointer(PointerEvent::Cancelled));
    }
    release_pointer(hwnd);
}

fn client_point_from_lparam(lparam: LPARAM) -> (i32, i32) {
    let packed = lparam.0.cast_unsigned();
    let x = i16::from_ne_bytes(
        u16::try_from(packed & 0xFFFF)
            .unwrap_or_default()
            .to_ne_bytes(),
    );
    let y = i16::from_ne_bytes(
        u16::try_from((packed >> 16) & 0xFFFF)
            .unwrap_or_default()
            .to_ne_bytes(),
    );
    (i32::from(x), i32::from(y))
}
