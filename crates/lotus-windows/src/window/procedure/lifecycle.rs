use lotus_ui::geometry::DpiScale;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateRoundRectRgn, DeleteObject, HGDIOBJ, ScreenToClient, SetWindowRgn,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, GetClientRect, GetWindowRect, HTCAPTION, HTCLIENT, MA_NOACTIVATE,
    MINMAXINFO, SPI_SETWORKAREA, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, WM_CLOSE,
    WM_CONTEXTMENU, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_GETMINMAXINFO,
    WM_MOUSEACTIVATE, WM_NCCREATE, WM_NCDESTROY, WM_NCHITTEST, WM_SETCURSOR,
    WM_SETTINGCHANGE, WM_SIZE,
};

use super::{
    ContextMenuEvent, DockContextRequest, SearchEvent, SettingsEvent, SignedPoint,
    SwitcherEvent, WindowEvent, WindowKind, clear_window_state, initialize_window_state,
    is_dock_window, is_settings_window, low_word, push_window_event, window_kind,
    with_window_state,
};
use crate::platform::windows::interaction::request_exit;

const SETTINGS_MIN_WIDTH_DIPS: u32 = 780;
const SETTINGS_MIN_HEIGHT_DIPS: u32 = 540;
const SETTINGS_SIDEBAR_WIDTH_DIPS: u32 = 209;
const SETTINGS_DRAG_REGION_HEIGHT_DIPS: u32 = 18;

pub(super) fn dispatch(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    match message {
        WM_NCCREATE => {
            initialize_window_state(hwnd, lparam);
            None
        }
        WM_NCDESTROY => {
            clear_window_state(hwnd);
            None
        }
        WM_MOUSEACTIVATE if is_nonactivating_window(hwnd) => {
            Some(LRESULT(isize::try_from(MA_NOACTIVATE).unwrap_or_default()))
        }
        WM_SETCURSOR
            if low_word(lparam.0.cast_unsigned()) == HTCLIENT
                && apply_pointer_cursor(hwnd) =>
        {
            Some(LRESULT(1))
        }
        WM_NCHITTEST if is_settings_window(hwnd) => {
            Some(settings_header_hit_test(hwnd, lparam))
        }
        WM_CONTEXTMENU if is_dock_window(hwnd) => {
            push_window_event(
                hwnd,
                WindowEvent::ContextMenuRequested(context_request(hwnd, lparam)),
            );
            Some(LRESULT(0))
        }
        WM_GETMINMAXINFO if is_settings_window(hwnd) => {
            Some(apply_settings_minimum_size(hwnd, lparam))
        }
        WM_SIZE => {
            apply_configured_region(hwnd);
            let (width, height) = size_from_lparam(lparam);
            push_window_event(hwnd, WindowEvent::Resized { width, height });
            Some(LRESULT(0))
        }
        WM_DPICHANGED => Some(apply_dpi_change(hwnd, wparam, lparam)),
        message if is_dock_window(hwnd) && requests_placement_refresh(message, wparam) => {
            push_window_event(hwnd, WindowEvent::PlacementRefreshRequested);
            Some(LRESULT(0))
        }
        WM_CLOSE => Some(dispatch_close_message(hwnd)),
        WM_DESTROY => {
            stop_animation_timer(hwnd);
            if is_dock_window(hwnd) {
                request_exit(0);
            }
            Some(LRESULT(0))
        }
        _ => None,
    }
}

fn is_nonactivating_window(hwnd: HWND) -> bool {
    matches!(
        window_kind(hwnd),
        Some(WindowKind::Dock | WindowKind::Status)
    )
}
fn apply_pointer_cursor(hwnd: HWND) -> bool {
    let mut applied = false;
    with_window_state(hwnd, |state| {
        applied = state.pointer_cursor.get().apply().is_ok();
    });
    applied
}

fn dispatch_close_message(hwnd: HWND) -> LRESULT {
    let event = match window_kind(hwnd) {
        Some(WindowKind::Search) => {
            Some(WindowEvent::Search(SearchEvent::DismissRequested))
        }
        Some(WindowKind::Settings) => {
            Some(WindowEvent::Settings(SettingsEvent::CloseRequested))
        }
        Some(WindowKind::ContextMenu) => {
            Some(WindowEvent::ContextMenu(ContextMenuEvent::DismissRequested))
        }
        Some(WindowKind::Switcher) => {
            Some(WindowEvent::Switcher(SwitcherEvent::CloseRequested))
        }
        Some(WindowKind::Dock | WindowKind::Status) | None => None,
    };
    if let Some(event) = event {
        push_window_event(hwnd, event);
    } else {
        let _ = unsafe { DestroyWindow(hwnd) };
    }
    LRESULT(0)
}

fn apply_settings_minimum_size(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let limits = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
    let dpi = DpiScale::from_system(unsafe { GetDpiForWindow(hwnd) });
    limits.ptMinTrackSize.x = dpi.physical_i32(SETTINGS_MIN_WIDTH_DIPS);
    limits.ptMinTrackSize.y = dpi.physical_i32(SETTINGS_MIN_HEIGHT_DIPS);
    LRESULT(0)
}

fn settings_header_hit_test(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let screen = signed_point_from_lparam(lparam);
    let mut client = POINT {
        x: screen.x,
        y: screen.y,
    };
    let mut bounds = RECT::default();
    let converted = unsafe { ScreenToClient(hwnd, &raw mut client) }.as_bool();
    let bounds_read = unsafe { GetClientRect(hwnd, &raw mut bounds) }.is_ok();
    if !converted || !bounds_read {
        return LRESULT(isize::try_from(HTCLIENT).unwrap_or_default());
    }
    let dpi = DpiScale::from_system(unsafe { GetDpiForWindow(hwnd) });
    let draggable = client.x >= 0
        && client.x
            < dpi
                .physical_i32(SETTINGS_SIDEBAR_WIDTH_DIPS)
                .min(bounds.right)
        && client.y >= 0
        && client.y < dpi.physical_i32(SETTINGS_DRAG_REGION_HEIGHT_DIPS);
    LRESULT(
        isize::try_from(if draggable {
            HTCAPTION
        } else {
            HTCLIENT
        })
        .unwrap_or_default(),
    )
}

fn apply_dpi_change(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let suggested = unsafe { &*(lparam.0 as *const RECT) };
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            None,
            suggested.left,
            suggested.top,
            suggested.right - suggested.left,
            suggested.bottom - suggested.top,
            SWP_NOACTIVATE | SWP_NOZORDER,
        )
    };
    apply_configured_region(hwnd);
    push_window_event(
        hwnd,
        WindowEvent::DpiChanged {
            dpi: u32::try_from(wparam.0 & 0xFFFF).unwrap_or_default(),
        },
    );
    LRESULT(0)
}

fn apply_configured_region(hwnd: HWND) {
    with_window_state(hwnd, |state| {
        apply_rounded_region(hwnd, state.corner_radius.get());
    });
}
fn stop_animation_timer(hwnd: HWND) {
    with_window_state(hwnd, |state| {
        let _ = state.set_animation_active(hwnd, false);
    });
}
fn size_from_lparam(lparam: LPARAM) -> (u32, u32) {
    let packed = lparam.0.cast_unsigned();
    (
        u32::try_from(packed & 0xFFFF).unwrap_or_default(),
        u32::try_from((packed >> 16) & 0xFFFF).unwrap_or_default(),
    )
}
fn signed_point_from_lparam(lparam: LPARAM) -> SignedPoint {
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
    SignedPoint {
        x: i32::from(x),
        y: i32::from(y),
    }
}

fn context_request(hwnd: HWND, lparam: LPARAM) -> DockContextRequest {
    if lparam.0 == -1 {
        return DockContextRequest::Keyboard;
    }
    let screen = signed_point_from_lparam(lparam);
    let mut client = POINT {
        x: screen.x,
        y: screen.y,
    };
    let client = if unsafe { ScreenToClient(hwnd, &raw mut client) }.as_bool() {
        SignedPoint {
            x: client.x,
            y: client.y,
        }
    } else {
        screen
    };
    DockContextRequest::Pointer { screen, client }
}

fn requests_placement_refresh(message: u32, wparam: WPARAM) -> bool {
    message == WM_DISPLAYCHANGE
        || (message == WM_SETTINGCHANGE
            && u32::try_from(wparam.0).ok() == Some(SPI_SETWORKAREA.0))
}

pub fn apply_rounded_region(hwnd: HWND, radius_dips: u32) {
    if radius_dips == 0 {
        let _ = unsafe { SetWindowRgn(hwnd, None, true) };
        return;
    }
    let mut bounds = RECT::default();
    if unsafe { GetWindowRect(hwnd, &raw mut bounds) }.is_err() {
        return;
    }
    let diameter = DpiScale::from_system(unsafe { GetDpiForWindow(hwnd) })
        .physical_i32(radius_dips)
        .max(1)
        * 2;
    let region = unsafe {
        CreateRoundRectRgn(
            0,
            0,
            bounds.right - bounds.left + 1,
            bounds.bottom - bounds.top + 1,
            diameter,
            diameter,
        )
    };
    if region.is_invalid() {
        return;
    }
    if unsafe { SetWindowRgn(hwnd, Some(region), true) } == 0 {
        let _ = unsafe { DeleteObject(HGDIOBJ::from(region)) };
    }
}
