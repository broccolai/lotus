mod keyboard_text;
mod lifecycle;
mod pointer_capture;
mod timer_paint;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::mem::size_of;
use std::panic::{AssertUnwindSafe, catch_unwind};

pub use lifecycle::apply_rounded_region;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, DefWindowProcW, GWLP_USERDATA,
    GetWindowLongPtrW, IDC_ARROW, LoadCursorW, RegisterClassExW, SetWindowLongPtrW,
    UnregisterClassW, WNDCLASSEXW,
};
use windows::core::w;

pub(crate) use super::events::{
    ContextMenuEvent, CursorMove, DockContextRequest, PointerEvent, SearchEdit,
    SearchEvent, SelectionDirection, SettingsEvent, SettingsKey, SignedPoint,
    SwitcherEvent, WindowEvent,
};
use crate::NativeError;
use crate::platform::windows::interaction::{PointerCursor, WindowTimer, request_exit};

type Result<T> = std::result::Result<T, NativeError>;

pub(super) use crate::messages::SEARCH_OUTSIDE_CLICK as SEARCH_OUTSIDE_CLICK_MESSAGE;
pub(super) const ANIMATION_TIMER: WindowTimer = WindowTimer::new(0x4C4F_5455, 16);
pub(super) const DOCK_STATUS_TIMER: WindowTimer = WindowTimer::new(0x4C4F_5453, 30_000);
pub(super) const SEARCH_CLOCK_TIMER: WindowTimer = WindowTimer::new(0x4C4F_5443, 30_000);
pub(super) const SEARCH_FOCUS_TIMER: WindowTimer = WindowTimer::new(0x4C4F_5446, 50);

const WNDPROC_PANIC_EXIT_CODE: i32 = 1;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum WindowKind {
    #[default]
    Dock,
    Status,
    Search,
    Settings,
    ContextMenu,
    Switcher,
}

#[derive(Default)]
pub struct WindowState {
    pending: RefCell<VecDeque<WindowEvent>>,
    corner_radius: Cell<u32>,
    pub(super) tracking_mouse_leave: Cell<bool>,
    pub(super) left_button_pressed: Cell<bool>,
    pub(super) animation_active: Cell<bool>,
    pub(super) pending_high_surrogate: Cell<Option<u16>>,
    pub(super) pointer_cursor: Cell<PointerCursor>,
    pub(super) kind: WindowKind,
}

impl WindowState {
    pub fn search() -> Self {
        Self {
            kind: WindowKind::Search,
            ..Self::default()
        }
    }
    pub fn status() -> Self {
        Self {
            kind: WindowKind::Status,
            ..Self::default()
        }
    }
    pub fn settings() -> Self {
        Self {
            kind: WindowKind::Settings,
            ..Self::default()
        }
    }
    pub fn context_menu() -> Self {
        Self {
            kind: WindowKind::ContextMenu,
            ..Self::default()
        }
    }
    pub fn switcher() -> Self {
        Self {
            kind: WindowKind::Switcher,
            ..Self::default()
        }
    }

    pub(super) fn push(&self, event: WindowEvent) {
        self.pending.borrow_mut().push_back(event);
    }
    pub fn drain(&self) -> impl Iterator<Item = WindowEvent> {
        std::mem::take(&mut *self.pending.borrow_mut()).into_iter()
    }

    pub fn has_pending_events(&self) -> bool {
        !self.pending.borrow().is_empty()
    }
    pub fn set_corner_radius(&self, corner_radius: u32) {
        self.corner_radius.set(corner_radius);
    }
    pub fn clear_events(&self) {
        self.pending.borrow_mut().clear();
        self.pending_high_surrogate.set(None);
    }
    pub fn set_pointer_cursor(&self, cursor: PointerCursor) {
        self.pointer_cursor.set(cursor);
        let _ = cursor.apply();
    }

    pub fn set_animation_active(&self, hwnd: HWND, active: bool) -> Result<()> {
        if self.animation_active.get() == active {
            return Ok(());
        }

        if active {
            ANIMATION_TIMER.start(hwnd)?;
            self.animation_active.set(true);
        } else {
            self.animation_active.set(false);
            ANIMATION_TIMER.stop(hwnd);
        }
        Ok(())
    }
}

pub struct WindowClass {
    instance: HINSTANCE,
}

impl WindowClass {
    pub const NAME: windows::core::PCWSTR = w!("Lotus.NativeWindow");

    pub fn register(instance: HINSTANCE) -> Result<Self> {
        let cursor = unsafe { LoadCursorW(None, IDC_ARROW)? };
        let class = WNDCLASSEXW {
            cbSize: window_class_size(),
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_procedure),
            hInstance: instance,
            hCursor: cursor,
            lpszClassName: Self::NAME,
            ..WNDCLASSEXW::default()
        };
        if unsafe { RegisterClassExW(&raw const class) } == 0 {
            return Err(windows::core::Error::from_thread().into());
        }
        Ok(Self { instance })
    }

    pub const fn instance(&self) -> HINSTANCE {
        self.instance
    }
}

impl Drop for WindowClass {
    fn drop(&mut self) {
        let _ = unsafe { UnregisterClassW(Self::NAME, Some(self.instance)) };
    }
}

unsafe extern "system" fn window_procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    catch_unwind(AssertUnwindSafe(|| dispatch(hwnd, message, wparam, lparam)))
        .unwrap_or_else(|_| {
            request_exit(WNDPROC_PANIC_EXIT_CODE);
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        })
}

fn dispatch(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    pointer_capture::dispatch(hwnd, message, lparam)
        .or_else(|| timer_paint::dispatch_timer(hwnd, message, wparam))
        .or_else(|| keyboard_text::dispatch(hwnd, message, wparam, lparam))
        .or_else(|| lifecycle::dispatch(hwnd, message, wparam, lparam))
        .or_else(|| timer_paint::dispatch_paint(hwnd, message))
        .unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, message, wparam, lparam) })
}

pub(super) fn push_window_event(hwnd: HWND, event: WindowEvent) {
    with_window_state(hwnd, |state| state.push(event));
}
pub(super) fn with_window_state(hwnd: HWND, action: impl FnOnce(&WindowState)) {
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut WindowState;
    if !pointer.is_null() {
        action(unsafe { &*pointer });
    }
}
pub(super) fn window_kind(hwnd: HWND) -> Option<WindowKind> {
    let mut kind = None;
    with_window_state(hwnd, |state| kind = Some(state.kind));
    kind
}
pub(super) fn is_search_window(hwnd: HWND) -> bool {
    window_kind(hwnd) == Some(WindowKind::Search)
}
pub(super) fn is_settings_window(hwnd: HWND) -> bool {
    window_kind(hwnd) == Some(WindowKind::Settings)
}
pub(super) fn is_context_menu_window(hwnd: HWND) -> bool {
    window_kind(hwnd) == Some(WindowKind::ContextMenu)
}
pub(super) fn is_dock_window(hwnd: HWND) -> bool {
    window_kind(hwnd) == Some(WindowKind::Dock)
}
pub(super) fn initialize_window_state(hwnd: HWND, lparam: LPARAM) {
    let events = unsafe { (*(lparam.0 as *const CREATESTRUCTW)).lpCreateParams };
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, events.addr().cast_signed()) };
}
pub(super) fn clear_window_state(hwnd: HWND) {
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
}
pub(super) fn low_word(value: usize) -> u32 {
    u32::try_from(value & 0xFFFF).unwrap_or_default()
}

pub fn start_search_clock_timer(hwnd: HWND) -> Result<()> {
    SEARCH_CLOCK_TIMER.start(hwnd)
}
pub fn set_dock_status_timer(hwnd: HWND, active: bool) -> Result<()> {
    if active {
        DOCK_STATUS_TIMER.start(hwnd)
    } else {
        DOCK_STATUS_TIMER.stop(hwnd);
        Ok(())
    }
}
pub fn stop_search_clock_timer(hwnd: HWND) {
    SEARCH_CLOCK_TIMER.stop(hwnd);
}
pub fn start_search_focus_timer(hwnd: HWND) -> Result<()> {
    SEARCH_FOCUS_TIMER.start(hwnd)
}
pub fn stop_search_focus_timer(hwnd: HWND) {
    SEARCH_FOCUS_TIMER.stop(hwnd);
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "WNDCLASSEXW is a fixed Win32 ABI structure far smaller than u32::MAX"
)]
const fn window_class_size() -> u32 {
    size_of::<WNDCLASSEXW>() as u32
}
