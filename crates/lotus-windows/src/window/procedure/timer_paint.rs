use windows::Win32::Foundation::{HWND, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows::Win32::UI::WindowsAndMessaging::{WM_PAINT, WM_TIMER};

use super::{
    ANIMATION_TIMER, DOCK_STATUS_TIMER, DockEvent, MASCOT_ANIMATION_TIMER,
    SEARCH_CLOCK_TIMER, SEARCH_FOCUS_TIMER, is_dock_window, is_search_window, push_event,
    push_render_event, with_window_state,
};

pub(super) fn dispatch_timer(hwnd: HWND, message: u32, wparam: WPARAM) -> Option<LRESULT> {
    if message != WM_TIMER {
        return None;
    }
    if ANIMATION_TIMER.matches(wparam.0) {
        let mut active = false;
        with_window_state(hwnd, |state| active = state.animation_active.get());
        if active {
            push_event(hwnd, DockEvent::AnimationFrame);
        }
        return Some(LRESULT(0));
    }
    if MASCOT_ANIMATION_TIMER.matches(wparam.0) && is_dock_window(hwnd) {
        with_window_state(hwnd, |state| state.mascot_animation_delay_ms.set(None));
        MASCOT_ANIMATION_TIMER.stop(hwnd);
        push_event(hwnd, DockEvent::MascotAnimationDeadline);
        return Some(LRESULT(0));
    }
    if DOCK_STATUS_TIMER.matches(wparam.0) && is_dock_window(hwnd) {
        push_event(hwnd, DockEvent::StatusRefreshRequested);
        return Some(LRESULT(0));
    }
    if SEARCH_CLOCK_TIMER.matches(wparam.0) && is_search_window(hwnd) {
        push_event(hwnd, super::SearchEvent::ClockRefreshRequested);
        return Some(LRESULT(0));
    }
    if SEARCH_FOCUS_TIMER.matches(wparam.0) && is_search_window(hwnd) {
        push_event(hwnd, super::SearchEvent::FocusRefreshRequested);
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
    push_render_event(hwnd);
    Some(LRESULT(0))
}
