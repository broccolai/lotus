use windows::Win32::Foundation::{HWND, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows::Win32::UI::WindowsAndMessaging::{WM_PAINT, WM_TIMER};

use super::{
    ANIMATION_TIMER, DOCK_STATUS_TIMER, SEARCH_CLOCK_TIMER, SEARCH_FOCUS_TIMER,
    WindowEvent, is_dock_window, is_search_window, push_window_event, with_window_state,
};

pub(super) fn dispatch_timer(hwnd: HWND, message: u32, wparam: WPARAM) -> Option<LRESULT> {
    if message != WM_TIMER {
        return None;
    }
    if ANIMATION_TIMER.matches(wparam.0) {
        let mut active = false;
        with_window_state(hwnd, |state| active = state.animation_active.get());
        if active {
            push_window_event(hwnd, WindowEvent::AnimationFrame);
        }
        return Some(LRESULT(0));
    }
    if DOCK_STATUS_TIMER.matches(wparam.0) && is_dock_window(hwnd) {
        push_window_event(hwnd, WindowEvent::StatusRefreshRequested);
        return Some(LRESULT(0));
    }
    if SEARCH_CLOCK_TIMER.matches(wparam.0) && is_search_window(hwnd) {
        push_window_event(
            hwnd,
            WindowEvent::Search(super::SearchEvent::ClockRefreshRequested),
        );
        return Some(LRESULT(0));
    }
    if SEARCH_FOCUS_TIMER.matches(wparam.0) && is_search_window(hwnd) {
        push_window_event(
            hwnd,
            WindowEvent::Search(super::SearchEvent::FocusRefreshRequested),
        );
        return Some(LRESULT(0));
    }
    None
}

pub(super) fn dispatch_paint(hwnd: HWND, message: u32) -> Option<LRESULT> {
    if message != WM_PAINT {
        return None;
    }

    let mut paint = PAINTSTRUCT::default();
    // SAFETY: WM_PAINT grants this procedure the update region until the matching EndPaint.
    unsafe {
        let _ = BeginPaint(hwnd, &raw mut paint);
        let _ = EndPaint(hwnd, &raw const paint);
    }
    push_window_event(hwnd, WindowEvent::RenderRequested);
    Some(LRESULT(0))
}
