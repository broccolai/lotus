use lotus_ui::geometry::DpiScale;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateRoundRectRgn, DeleteObject, HGDIOBJ, ScreenToClient, SetWindowRgn,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_SHIFT;
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, GetClientRect, GetWindowRect, HTCAPTION, HTCLIENT, MA_NOACTIVATE,
    MINMAXINFO, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, WM_CLOSE, WM_CONTEXTMENU,
    WM_DESTROY, WM_DPICHANGED, WM_GETMINMAXINFO, WM_MOUSEACTIVATE, WM_NCCREATE,
    WM_NCDESTROY, WM_NCHITTEST, WM_SETCURSOR, WM_SIZE,
};

use super::{
    ContextMenuEvent, DockContextRequest, SearchEvent, SettingsEvent, SignedPoint,
    SwitcherEvent, WindowKind, clear_window_state, initialize_window_state,
    is_dock_context_window, is_dock_window, is_search_window, is_settings_window, low_word,
    push_context_request, push_dpi_event, push_event, push_resize_event, window_kind,
    with_window_state,
};
use crate::platform::windows::display::{nearest_display, nearest_display_to_point};
use crate::platform::windows::interaction::{key_is_pressed, request_exit};
use crate::window::settings::fit_size_within;

const SETTINGS_MIN_WIDTH_DIPS: u32 = 780;
const SETTINGS_MIN_HEIGHT_DIPS: u32 = 540;
const SETTINGS_SIDEBAR_WIDTH_DIPS: u32 = 209;
const SETTINGS_DRAG_REGION_HEIGHT_DIPS: u32 = 18;
const SETTINGS_WORK_AREA_MARGIN_DIPS: u32 = 16;

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
        WM_CONTEXTMENU if is_dock_context_window(hwnd) || is_search_window(hwnd) => {
            push_context_request(hwnd, context_request(hwnd, lparam));
            Some(LRESULT(0))
        }
        WM_GETMINMAXINFO if is_settings_window(hwnd) => {
            Some(apply_settings_minimum_size(hwnd, lparam))
        }
        WM_SIZE => {
            apply_configured_region(hwnd);
            let (width, height) = size_from_lparam(lparam);
            push_resize_event(hwnd, width, height);
            Some(LRESULT(0))
        }
        WM_DPICHANGED => Some(apply_dpi_change(hwnd, wparam, lparam)),
        message
            if is_dock_window(hwnd)
                && crate::appbar::queue_taskbar_created_recovery(hwnd, message) =>
        {
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
        Some(WindowKind::Dock | WindowKind::DockReplica | WindowKind::Status)
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
    match window_kind(hwnd) {
        Some(WindowKind::Search) => push_event(hwnd, SearchEvent::DismissRequested),
        Some(WindowKind::Settings) => {
            push_event(hwnd, SettingsEvent::CloseRequested);
        }
        Some(WindowKind::ContextMenu) => {
            push_event(hwnd, ContextMenuEvent::DismissRequested);
        }
        Some(WindowKind::Switcher) => {
            push_event(hwnd, SwitcherEvent::CloseRequested);
        }
        Some(WindowKind::Dock | WindowKind::DockReplica | WindowKind::Status) | None => {
            let _ = unsafe { DestroyWindow(hwnd) };
        }
    }
    LRESULT(0)
}

fn apply_settings_minimum_size(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let limits = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
    let dpi = DpiScale::from_system(unsafe { GetDpiForWindow(hwnd) });
    let Ok(display) = nearest_display(hwnd) else {
        return LRESULT(0);
    };
    let work_width = display
        .work_area
        .right
        .saturating_sub(display.work_area.left);
    let work_height = display
        .work_area
        .bottom
        .saturating_sub(display.work_area.top);
    let margin = dpi.physical_i32(SETTINGS_WORK_AREA_MARGIN_DIPS);
    let maximum_width = work_width.saturating_sub(margin.saturating_mul(2)).max(1);
    let maximum_height = work_height.saturating_sub(margin.saturating_mul(2)).max(1);
    let minimum = fit_size_within(
        dpi.physical_i32(SETTINGS_MIN_WIDTH_DIPS),
        dpi.physical_i32(SETTINGS_MIN_HEIGHT_DIPS),
        maximum_width,
        maximum_height,
    );
    limits.ptMinTrackSize.x = minimum.0;
    limits.ptMinTrackSize.y = minimum.1;
    limits.ptMaxTrackSize.x = maximum_width;
    limits.ptMaxTrackSize.y = maximum_height;
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
    let mut layout_dpi = 0;
    with_window_state(hwnd, |state| layout_dpi = state.settings_layout_dpi.get());
    let dpi = DpiScale::from_system(if layout_dpi == 0 {
        unsafe { GetDpiForWindow(hwnd) }
    } else {
        layout_dpi
    });
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
    let bounds = settings_dpi_bounds(hwnd, suggested, wparam);
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            None,
            bounds.left,
            bounds.top,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            SWP_NOACTIVATE | SWP_NOZORDER,
        )
    };
    apply_configured_region(hwnd);
    push_dpi_event(hwnd, u32::try_from(wparam.0 & 0xFFFF).unwrap_or_default());
    LRESULT(0)
}

fn settings_dpi_bounds(hwnd: HWND, suggested: &RECT, wparam: WPARAM) -> RECT {
    if !is_settings_window(hwnd) {
        return *suggested;
    }
    let center_x = suggested
        .left
        .saturating_add((suggested.right - suggested.left) / 2);
    let center_y = suggested
        .top
        .saturating_add((suggested.bottom - suggested.top) / 2);
    let Ok(display) = nearest_display_to_point(center_x, center_y) else {
        return *suggested;
    };
    let dpi = DpiScale::from_system(u32::try_from(wparam.0 & 0xFFFF).unwrap_or_default());
    let margin = dpi.physical_i32(SETTINGS_WORK_AREA_MARGIN_DIPS);
    let maximum_width = display
        .work_area
        .right
        .saturating_sub(display.work_area.left)
        .saturating_sub(margin.saturating_mul(2))
        .max(1);
    let maximum_height = display
        .work_area
        .bottom
        .saturating_sub(display.work_area.top)
        .saturating_sub(margin.saturating_mul(2))
        .max(1);
    let (width, height) = fit_size_within(
        suggested.right.saturating_sub(suggested.left),
        suggested.bottom.saturating_sub(suggested.top),
        maximum_width,
        maximum_height,
    );
    let left = suggested.left.clamp(
        display.work_area.left.saturating_add(margin),
        display
            .work_area
            .right
            .saturating_sub(margin)
            .saturating_sub(width),
    );
    let top = suggested.top.clamp(
        display.work_area.top.saturating_add(margin),
        display
            .work_area
            .bottom
            .saturating_sub(margin)
            .saturating_sub(height),
    );
    RECT {
        left,
        top,
        right: left.saturating_add(width),
        bottom: top.saturating_add(height),
    }
}

fn apply_configured_region(hwnd: HWND) {
    with_window_state(hwnd, |state| {
        apply_rounded_region(hwnd, state.corner_radius.get());
    });
}
fn stop_animation_timer(hwnd: HWND) {
    with_window_state(hwnd, |state| {
        let _ = state.set_animation_active(hwnd, false);
        let _ = state.set_mascot_animation_delay(hwnd, None);
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
    let shift_held = key_is_pressed(VK_SHIFT);
    if lparam.0 == -1 {
        return DockContextRequest::Keyboard { shift_held };
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
    DockContextRequest::Pointer {
        screen,
        client,
        shift_held,
    }
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
