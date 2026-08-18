use std::ffi::c_void;
use std::mem::size_of;

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowRect, SET_WINDOW_POS_FLAGS, SWP_NOMOVE, SWP_NOSIZE,
};
use windows::core::{PCSTR, w};

const EDGE_INSET_DIP: i32 = 12;

pub(crate) fn set_window_pos_address() -> Option<*mut c_void> {
    let module = unsafe { GetModuleHandleW(w!("user32.dll")) }.ok()?;
    let procedure =
        unsafe { GetProcAddress(module, PCSTR(c"SetWindowPos".as_ptr().cast())) }?;
    Some(procedure as *mut c_void)
}

pub(crate) fn is_control_center_window(window: HWND) -> bool {
    let mut class_name = [0_u16; 64];
    let length = unsafe { GetClassNameW(window, &mut class_name) };
    let Ok(length) = usize::try_from(length) else {
        return false;
    };
    let expected_name = w!("ControlCenterWindow");
    let expected = unsafe { expected_name.as_wide() };
    class_name.get(..length) == Some(expected)
}

pub(crate) fn desired_position(
    window: HWND,
    width: i32,
    height: i32,
    flags: SET_WINDOW_POS_FLAGS,
    anchor: (i32, i32),
) -> Option<(i32, i32)> {
    if flags.0 & (SWP_NOMOVE.0 | SWP_NOSIZE.0) == SWP_NOMOVE.0 | SWP_NOSIZE.0 {
        return None;
    }

    let mut current = RECT::default();
    unsafe { GetWindowRect(window, &raw mut current) }.ok()?;
    let actual_width = if flags.0 & SWP_NOSIZE.0 != 0 {
        current.right.saturating_sub(current.left)
    } else {
        width
    };
    let actual_height = if flags.0 & SWP_NOSIZE.0 != 0 {
        current.bottom.saturating_sub(current.top)
    } else {
        height
    };
    if actual_width <= 0 || actual_height <= 0 {
        return None;
    }

    let (anchor_x, anchor_y) = anchor;
    let monitor = unsafe {
        MonitorFromPoint(
            POINT {
                x: anchor_x,
                y: anchor_y,
            },
            MONITOR_DEFAULTTONEAREST,
        )
    };
    let mut monitor_info = MONITORINFO {
        cbSize: u32::try_from(size_of::<MONITORINFO>()).ok()?,
        ..MONITORINFO::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &raw mut monitor_info) }.as_bool() {
        return None;
    }

    let dpi = unsafe { GetDpiForWindow(window) }.max(96);
    let inset = EDGE_INSET_DIP.saturating_mul(i32::try_from(dpi).unwrap_or(96)) / 96;
    let minimum_x = monitor_info.rcWork.left.saturating_add(inset);
    let maximum_x = monitor_info
        .rcWork
        .right
        .saturating_sub(actual_width)
        .saturating_sub(inset)
        .max(minimum_x);
    let minimum_y = monitor_info.rcWork.top;
    let maximum_y = monitor_info
        .rcWork
        .bottom
        .saturating_sub(actual_height)
        .max(minimum_y);
    let x = anchor_x
        .saturating_sub(actual_width / 2)
        .clamp(minimum_x, maximum_x);
    let y = anchor_y
        .saturating_sub(actual_height)
        .clamp(minimum_y, maximum_y);
    Some((x, y))
}
