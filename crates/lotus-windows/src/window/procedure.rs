mod event_queue;
mod keyboard_text;
mod lifecycle;
mod pointer_capture;
mod timer_paint;

use std::cell::Cell;
use std::mem::size_of;
use std::panic::{AssertUnwindSafe, catch_unwind};

use event_queue::{EventQueue, QueuedEvent};
pub use lifecycle::apply_rounded_region;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, DefWindowProcW, GWLP_USERDATA,
    GetWindowLongPtrW, IDC_ARROW, LoadCursorW, RegisterClassExW, SetWindowLongPtrW,
    UnregisterClassW, WNDCLASSEXW,
};
use windows::core::w;

pub(crate) use super::events::{
    ContextMenuEvent, CursorMove, DockContextRequest, DockEvent, PointerEvent, SearchEdit,
    SearchEvent, SelectionDirection, SettingsEvent, SettingsKey, SignedPoint, StatusEvent,
    SwitcherEvent,
};
use crate::NativeError;
use crate::platform::windows::interaction::{PointerCursor, WindowTimer, request_exit};

type Result<T> = std::result::Result<T, NativeError>;

pub(super) use crate::messages::SEARCH_OUTSIDE_CLICK as SEARCH_OUTSIDE_CLICK_MESSAGE;
pub(super) const ANIMATION_TIMER: WindowTimer = WindowTimer::new(0x4C4F_5455, 16);
pub(super) const MASCOT_ANIMATION_TIMER: WindowTimer = WindowTimer::new(0x4C4F_544D, 1);
pub(super) const DOCK_STATUS_TIMER: WindowTimer = WindowTimer::new(0x4C4F_5453, 30_000);
pub(super) const SEARCH_CLOCK_TIMER: WindowTimer = WindowTimer::new(0x4C4F_5443, 30_000);
pub(super) const SEARCH_FOCUS_TIMER: WindowTimer = WindowTimer::new(0x4C4F_5446, 50);

const WNDPROC_PANIC_EXIT_CODE: i32 = 1;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum WindowKind {
    #[default]
    Dock,
    DockReplica,
    Status,
    Search,
    Settings,
    ContextMenu,
    Switcher,
}

pub struct WindowState {
    pending: EventQueue,
    corner_radius: Cell<u32>,
    pub(super) tracking_mouse_leave: Cell<bool>,
    pub(super) left_button_pressed: Cell<bool>,
    pub(super) animation_active: Cell<bool>,
    pub(super) mascot_animation_delay_ms: Cell<Option<u32>>,
    pub(super) pending_high_surrogate: Cell<Option<u16>>,
    pub(super) pointer_cursor: Cell<PointerCursor>,
    pub(super) settings_layout_dpi: Cell<u32>,
}

impl WindowState {
    fn with_pending(pending: EventQueue) -> Self {
        Self {
            pending,
            corner_radius: Cell::new(0),
            tracking_mouse_leave: Cell::new(false),
            left_button_pressed: Cell::new(false),
            animation_active: Cell::new(false),
            mascot_animation_delay_ms: Cell::new(None),
            pending_high_surrogate: Cell::new(None),
            pointer_cursor: Cell::new(PointerCursor::Arrow),
            settings_layout_dpi: Cell::new(0),
        }
    }
    pub fn search() -> Self {
        Self::with_pending(EventQueue::search())
    }
    pub fn status() -> Self {
        Self::with_pending(EventQueue::status())
    }
    pub fn dock_replica() -> Self {
        Self::with_pending(EventQueue::dock_replica())
    }
    pub fn settings() -> Self {
        Self::with_pending(EventQueue::settings())
    }
    pub fn context_menu() -> Self {
        Self::with_pending(EventQueue::context_menu())
    }
    pub fn switcher() -> Self {
        Self::with_pending(EventQueue::switcher())
    }

    pub(super) fn push_event<E: QueuedEvent>(&self, event: E) {
        self.pending.push(event);
    }

    pub(super) fn push_search_context_request(&self, request: DockContextRequest) {
        self.pending.push_search_context_request(request);
    }

    pub(super) fn drain_events<E: QueuedEvent>(&self) -> std::collections::VecDeque<E> {
        self.pending.drain()
    }
    pub(super) fn push_pointer(&self, event: PointerEvent) {
        match self.kind() {
            WindowKind::Dock | WindowKind::DockReplica => {
                self.push_event(DockEvent::Pointer(event));
            }
            WindowKind::Status => self.push_event(StatusEvent::Pointer(event)),
            WindowKind::Search => match event {
                PointerEvent::Moved { x, y } => {
                    self.push_event(SearchEvent::PointerMoved { x, y });
                }
                PointerEvent::Left => self.push_event(SearchEvent::PointerLeft),
                PointerEvent::LeftButtonReleased { x, y } => {
                    self.push_event(SearchEvent::PointerReleased { x, y });
                }
                PointerEvent::LeftButtonPressed { .. } | PointerEvent::Cancelled => {}
            },
            WindowKind::Settings => match event {
                PointerEvent::Moved { x, y } => {
                    self.push_event(SettingsEvent::PointerMoved { x, y });
                }
                PointerEvent::Left => self.push_event(SettingsEvent::PointerLeft),
                PointerEvent::LeftButtonPressed { x, y } => {
                    self.push_event(SettingsEvent::PointerPressed { x, y });
                }
                PointerEvent::LeftButtonReleased { x, y } => {
                    self.push_event(SettingsEvent::PointerReleased { x, y });
                }
                PointerEvent::Cancelled => {
                    self.push_event(SettingsEvent::PointerCancelled);
                }
            },
            WindowKind::ContextMenu => match event {
                PointerEvent::Moved { x, y } => {
                    self.push_event(ContextMenuEvent::PointerMoved { x, y });
                }
                PointerEvent::Left => self.push_event(ContextMenuEvent::PointerLeft),
                PointerEvent::LeftButtonReleased { x, y } => {
                    self.push_event(ContextMenuEvent::PointerReleased { x, y });
                }
                PointerEvent::Cancelled => {
                    self.push_event(ContextMenuEvent::DismissRequested);
                }
                PointerEvent::LeftButtonPressed { .. } => {}
            },
            WindowKind::Switcher => match event {
                PointerEvent::Moved { x, y } => {
                    self.push_event(SwitcherEvent::PointerMoved { x, y });
                }
                PointerEvent::Left => self.push_event(SwitcherEvent::PointerLeft),
                PointerEvent::LeftButtonReleased { x, y } => {
                    self.push_event(SwitcherEvent::PointerReleased { x, y });
                }
                PointerEvent::LeftButtonPressed { .. } | PointerEvent::Cancelled => {}
            },
        }
    }

    pub fn has_pending_events(&self) -> bool {
        !self.pending.is_empty()
    }
    pub fn set_corner_radius(&self, corner_radius: u32) {
        self.corner_radius.set(corner_radius);
    }
    pub fn clear_events(&self) {
        self.pending.clear();
        self.pending_high_surrogate.set(None);
    }

    fn kind(&self) -> WindowKind {
        self.pending.kind()
    }
    pub fn set_pointer_cursor(&self, cursor: PointerCursor) {
        self.pointer_cursor.set(cursor);
        let _ = cursor.apply();
    }
    pub fn set_settings_layout_dpi(&self, dpi: u32) {
        self.settings_layout_dpi.set(dpi);
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

    pub fn set_mascot_animation_delay(
        &self,
        hwnd: HWND,
        delay: Option<std::time::Duration>,
    ) -> Result<()> {
        let delay =
            delay.map(|delay| u32::try_from(delay.as_millis()).unwrap_or(u32::MAX).max(1));
        if self.mascot_animation_delay_ms.get() == delay {
            return Ok(());
        }
        if let Some(delay) = delay {
            MASCOT_ANIMATION_TIMER.start_with_interval(hwnd, delay)?;
        } else {
            MASCOT_ANIMATION_TIMER.stop(hwnd);
        }
        self.mascot_animation_delay_ms.set(delay);
        Ok(())
    }
}
impl Default for WindowState {
    fn default() -> Self {
        Self::with_pending(EventQueue::dock())
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

pub(super) fn with_window_state(hwnd: HWND, action: impl FnOnce(&WindowState)) {
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut WindowState;
    if !pointer.is_null() {
        action(unsafe { &*pointer });
    }
}
pub(super) fn push_pointer_event(hwnd: HWND, event: PointerEvent) {
    with_window_state(hwnd, |state| state.push_pointer(event));
}
pub(super) fn push_resize_event(hwnd: HWND, width: u32, height: u32) {
    with_window_state(hwnd, |state| match state.kind() {
        WindowKind::Dock | WindowKind::DockReplica => {
            state.push_event(DockEvent::Resized { width, height });
        }
        WindowKind::Status => state.push_event(StatusEvent::Resized { width, height }),
        WindowKind::Search => state.push_event(SearchEvent::Resized { width, height }),
        WindowKind::Settings => {
            state.push_event(SettingsEvent::Resized { width, height });
        }
        WindowKind::ContextMenu => {
            state.push_event(ContextMenuEvent::Resized { width, height });
        }
        WindowKind::Switcher => {
            state.push_event(SwitcherEvent::Resized { width, height });
        }
    });
}
pub(super) fn push_dpi_event(hwnd: HWND, dpi: u32) {
    with_window_state(hwnd, |state| match state.kind() {
        WindowKind::Dock | WindowKind::DockReplica => {
            state.push_event(DockEvent::DpiChanged { dpi });
        }
        WindowKind::Status => state.push_event(StatusEvent::DpiChanged { dpi }),
        WindowKind::Search => state.push_event(SearchEvent::DpiChanged { dpi }),
        WindowKind::Settings => state.push_event(SettingsEvent::DpiChanged { dpi }),
        WindowKind::ContextMenu => {
            state.push_event(ContextMenuEvent::DpiChanged { dpi });
        }
        WindowKind::Switcher => state.push_event(SwitcherEvent::DpiChanged { dpi }),
    });
}
pub(super) fn push_render_event(hwnd: HWND) {
    with_window_state(hwnd, |state| match state.kind() {
        WindowKind::Dock | WindowKind::DockReplica => {
            state.push_event(DockEvent::RenderRequested);
        }
        WindowKind::Status => state.push_event(StatusEvent::RenderRequested),
        WindowKind::Search => state.push_event(SearchEvent::RenderRequested),
        WindowKind::Settings => state.push_event(SettingsEvent::RenderRequested),
        WindowKind::ContextMenu => {
            state.push_event(ContextMenuEvent::RenderRequested);
        }
        WindowKind::Switcher => state.push_event(SwitcherEvent::RenderRequested),
    });
}
pub(super) fn push_context_request(hwnd: HWND, request: DockContextRequest) {
    with_window_state(hwnd, |state| match state.kind() {
        WindowKind::Dock | WindowKind::DockReplica => {
            state.push_event(DockEvent::ContextMenuRequested(request));
        }
        WindowKind::Search => state.push_search_context_request(request),
        WindowKind::Status
        | WindowKind::Settings
        | WindowKind::ContextMenu
        | WindowKind::Switcher => {}
    });
}
pub(super) fn push_event<E: QueuedEvent>(hwnd: HWND, event: E) {
    with_window_state(hwnd, |state| state.push_event(event));
}
pub(super) fn window_kind(hwnd: HWND) -> Option<WindowKind> {
    let mut kind = None;
    with_window_state(hwnd, |state| kind = Some(state.kind()));
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
pub(super) fn is_dock_context_window(hwnd: HWND) -> bool {
    matches!(
        window_kind(hwnd),
        Some(WindowKind::Dock | WindowKind::DockReplica)
    )
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
pub fn set_dock_mascot_animation_delay(
    hwnd: HWND,
    delay: Option<std::time::Duration>,
) -> Result<()> {
    let mut result = Ok(());
    with_window_state(hwnd, |state| {
        result = state.set_mascot_animation_delay(hwnd, delay);
    });
    result
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
