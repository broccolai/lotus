use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::mem::size_of;
use std::panic::{AssertUnwindSafe, catch_unwind};

use lotus_ui::geometry::DpiScale;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateRoundRectRgn, DeleteObject, HGDIOBJ, ScreenToClient, SetWindowRgn, ValidateRect,
};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT,
    VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, DefWindowProcW, DestroyWindow, GWLP_USERDATA,
    GetClientRect, GetWindowLongPtrW, HTCAPTION, HTCLIENT, IDC_ARROW, LoadCursorW,
    MA_NOACTIVATE, MINMAXINFO, RegisterClassExW, SPI_SETWORKAREA, SWP_NOACTIVATE,
    SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, UnregisterClassW, WA_INACTIVE,
    WM_ACTIVATE, WM_CANCELMODE, WM_CAPTURECHANGED, WM_CHAR, WM_CLOSE, WM_CONTEXTMENU,
    WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_GETMINMAXINFO, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_NCCREATE, WM_NCDESTROY, WM_NCHITTEST, WM_PAINT, WM_SETCURSOR, WM_SETTINGCHANGE,
    WM_SIZE, WM_TIMER, WNDCLASSEXW,
};
use windows::core::w;

pub(crate) use super::events::{
    ContextMenuEvent, CursorMove, DockContextRequest, PointerEvent, SearchEdit,
    SearchEvent, SelectionDirection, SettingsEvent, SettingsKey, SignedPoint,
    SwitcherEvent, WindowEvent,
};
use crate::NativeError;

type Result<T> = std::result::Result<T, NativeError>;

use crate::platform::windows::interaction::{
    PointerCursor, WindowTimer, capture_pointer, claim_keyboard_focus, key_is_pressed,
    release_pointer, request_exit, track_pointer_leave,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum WindowKind {
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
    tracking_mouse_leave: Cell<bool>,
    left_button_pressed: Cell<bool>,
    animation_active: Cell<bool>,
    pending_high_surrogate: Cell<Option<u16>>,
    pointer_cursor: Cell<PointerCursor>,
    kind: WindowKind,
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

    fn push(&self, event: WindowEvent) {
        self.pending.borrow_mut().push_back(event);
    }

    pub fn drain(&self) -> impl Iterator<Item = WindowEvent> {
        std::mem::take(&mut *self.pending.borrow_mut()).into_iter()
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

const ANIMATION_TIMER: WindowTimer = WindowTimer::new(0x4C4F_5455, 16);
const DOCK_STATUS_TIMER: WindowTimer = WindowTimer::new(0x4C4F_5453, 30_000);
const SEARCH_CLOCK_TIMER: WindowTimer = WindowTimer::new(0x4C4F_5443, 30_000);
const SEARCH_FOCUS_TIMER: WindowTimer = WindowTimer::new(0x4C4F_5446, 50);
const WNDPROC_PANIC_EXIT_CODE: i32 = 1;
const SETTINGS_MIN_WIDTH_DIPS: u32 = 780;
const SETTINGS_MIN_HEIGHT_DIPS: u32 = 540;

pub struct WindowClass {
    instance: HINSTANCE,
}

impl WindowClass {
    pub const NAME: windows::core::PCWSTR = w!("Lotus.NativeWindow");

    pub fn register(instance: HINSTANCE) -> Result<Self> {
        // SAFETY: IDC_ARROW is a predefined system cursor and needs no instance handle.
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

        // SAFETY: Every pointer in `class` is either null or valid static/process-owned data.
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
        // SAFETY: This guard owns the registration and drops after its window is destroyed.
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
            // SAFETY: Posting a nonzero quit code asks the owning UI loop to unwind normally, so
            // its shell-state guards run. No Rust unwind is permitted across this ABI boundary.
            request_exit(WNDPROC_PANIC_EXIT_CODE);
            // SAFETY: The original message still receives valid default processing.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        })
}

fn dispatch(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if let Some(result) = dispatch_pointer_message(hwnd, message, lparam) {
        return result;
    }
    if message == WM_TIMER
        && let Some(result) = dispatch_timer_message(hwnd, wparam)
    {
        return result;
    }
    if message == WM_MOUSEWHEEL && is_search_window(hwnd) {
        return dispatch_search_wheel(hwnd, wparam);
    }

    match message {
        WM_NCCREATE => {
            initialize_window_state(hwnd, lparam);
            // SAFETY: Default non-client creation behavior must still run with original arguments.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_NCDESTROY => {
            clear_window_state(hwnd);
            // SAFETY: Default non-client destruction behavior must receive original arguments.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_MOUSEACTIVATE if is_nonactivating_window(hwnd) => {
            LRESULT(isize::try_from(MA_NOACTIVATE).unwrap_or_default())
        }
        WM_SETCURSOR
            if low_word(lparam.0.cast_unsigned()) == HTCLIENT
                && apply_pointer_cursor(hwnd) =>
        {
            LRESULT(1)
        }
        WM_NCHITTEST if is_settings_window(hwnd) => settings_header_hit_test(hwnd, lparam),
        WM_ACTIVATE if is_search_window(hwnd) => {
            dispatch_search_activation(hwnd, wparam, lparam)
        }
        WM_ACTIVATE if is_context_menu_window(hwnd) => {
            dispatch_context_menu_activation(hwnd, wparam, lparam)
        }
        WM_KEYDOWN if is_search_window(hwnd) => {
            dispatch_search_key(hwnd, message, wparam, lparam)
        }
        WM_KEYDOWN if is_settings_window(hwnd) => {
            dispatch_settings_key(hwnd, message, wparam, lparam)
        }
        WM_KEYDOWN if is_context_menu_window(hwnd) => {
            dispatch_context_menu_key(hwnd, message, wparam, lparam)
        }
        WM_CHAR if is_search_window(hwnd) => {
            push_search_text_unit(hwnd, wparam);
            LRESULT(0)
        }
        WM_CONTEXTMENU if is_dock_window(hwnd) => {
            push_window_event(
                hwnd,
                WindowEvent::ContextMenuRequested(context_request(hwnd, lparam)),
            );
            LRESULT(0)
        }
        WM_GETMINMAXINFO if is_settings_window(hwnd) => {
            apply_settings_minimum_size(hwnd, lparam)
        }
        WM_SIZE => {
            apply_configured_region(hwnd);
            let (width, height) = size_from_lparam(lparam);
            push_window_event(hwnd, WindowEvent::Resized { width, height });
            LRESULT(0)
        }
        WM_PAINT => {
            push_window_event(hwnd, WindowEvent::RenderRequested);
            // SAFETY: Validating the complete update region is permitted for this live HWND and
            // prevents Windows from continuously reposting WM_PAINT before app-side rendering.
            let _ = unsafe { ValidateRect(Some(hwnd), None) };
            LRESULT(0)
        }
        WM_DPICHANGED => {
            // SAFETY: For WM_DPICHANGED, lParam points to a RECT valid for this callback.
            let suggested = unsafe { &*(lparam.0 as *const RECT) };
            // SAFETY: The suggested rectangle is supplied by Windows for this live HWND.
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
                    dpi: dpi_from_wparam(wparam),
                },
            );
            LRESULT(0)
        }
        message if is_dock_window(hwnd) && requests_placement_refresh(message, wparam) => {
            push_window_event(hwnd, WindowEvent::PlacementRefreshRequested);
            LRESULT(0)
        }
        WM_CLOSE => dispatch_close_message(hwnd),
        WM_DESTROY => {
            stop_animation_timer(hwnd);
            if is_dock_window(hwnd) {
                // SAFETY: Posting WM_QUIT when the dock dies cleanly ends the UI message loop.
                request_exit(0);
            }
            LRESULT(0)
        }
        _ => {
            // SAFETY: Unhandled messages must be delegated with their original arguments.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
    }
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
        // SAFETY: The close message belongs to this live dock HWND on its owning thread.
        let _ = unsafe { DestroyWindow(hwnd) };
    }
    LRESULT(0)
}

fn initialize_window_state(hwnd: HWND, lparam: LPARAM) {
    // SAFETY: WM_NCCREATE supplies a valid CREATESTRUCTW. This stores, but does not dereference,
    // its stable same-process WindowState pointer until WM_NCDESTROY clears the window data.
    let events = unsafe { (*(lparam.0 as *const CREATESTRUCTW)).lpCreateParams };
    // SAFETY: The pointer remains owned by the matching window wrapper for the HWND lifetime.
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, events.addr().cast_signed()) };
}

fn clear_window_state(hwnd: HWND) {
    // SAFETY: Clearing pointer-sized window data does not dereference the queue and prevents later
    // messages from observing it after the HWND's non-client destruction completes.
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
}

fn dispatch_timer_message(hwnd: HWND, wparam: WPARAM) -> Option<LRESULT> {
    if ANIMATION_TIMER.matches(wparam.0) {
        if animation_is_active(hwnd) {
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
            WindowEvent::Search(SearchEvent::ClockRefreshRequested),
        );
        return Some(LRESULT(0));
    }
    if SEARCH_FOCUS_TIMER.matches(wparam.0) && is_search_window(hwnd) {
        push_window_event(
            hwnd,
            WindowEvent::Search(SearchEvent::FocusRefreshRequested),
        );
        return Some(LRESULT(0));
    }
    None
}

fn dispatch_pointer_message(hwnd: HWND, message: u32, lparam: LPARAM) -> Option<LRESULT> {
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
            release_capture_if_owned(hwnd);
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

fn push_window_event(hwnd: HWND, event: WindowEvent) {
    with_window_state(hwnd, |state| state.push(event));
}

fn apply_configured_region(hwnd: HWND) {
    with_window_state(hwnd, |state| {
        apply_rounded_region(hwnd, state.corner_radius.get());
    });
}

fn with_window_state(hwnd: HWND, action: impl FnOnce(&WindowState)) {
    // SAFETY: GWLP_USERDATA is written only from WM_NCCREATE and cleared by WM_NCDESTROY on this
    // thread. A nonzero value therefore points to DockWindow's live boxed WindowState.
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut WindowState;
    if !pointer.is_null() {
        // SAFETY: The Box remains stable for the HWND lifetime. WindowState uses interior
        // mutability for callback state, so synchronous Win32 reentrancy never aliases `&mut`.
        action(unsafe { &*pointer });
    }
}

fn dispatch_settings_key(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if let Some(key) = settings_key(wparam) {
        push_window_event(hwnd, WindowEvent::Settings(SettingsEvent::KeyPressed(key)));
        LRESULT(0)
    } else {
        // SAFETY: Unhandled keys retain normal top-level window keyboard processing.
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }
}

fn dispatch_search_key(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if let Some(event) = search_key_event(wparam) {
        push_window_event(hwnd, WindowEvent::Search(event));
        LRESULT(0)
    } else {
        // SAFETY: Unhandled keys retain normal popup keyboard processing.
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }
}

fn dispatch_search_wheel(hwnd: HWND, wparam: WPARAM) -> LRESULT {
    if let Some(direction) = wheel_selection_direction(wparam) {
        push_window_event(
            hwnd,
            WindowEvent::Search(SearchEvent::MoveSelection(direction)),
        );
    }
    LRESULT(0)
}

fn dispatch_search_activation(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let inactive = low_word(wparam.0) == WA_INACTIVE;
    if inactive {
        push_window_event(hwnd, WindowEvent::Search(SearchEvent::DismissRequested));
    }
    // SAFETY: Search is a normal activating popup; default activation processing must run.
    let result = unsafe { DefWindowProcW(hwnd, WM_ACTIVATE, wparam, lparam) };
    if !inactive {
        let _ = claim_keyboard_focus(hwnd);
    }
    result
}

fn dispatch_context_menu_activation(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if low_word(wparam.0) == WA_INACTIVE {
        push_window_event(
            hwnd,
            WindowEvent::ContextMenu(ContextMenuEvent::DismissRequested),
        );
    }
    // SAFETY: The popup retains standard top-level activation processing. `show` clears any
    // inactive event produced while the hidden window is positioned before revealing it.
    unsafe { DefWindowProcW(hwnd, WM_ACTIVATE, wparam, lparam) }
}

fn dispatch_context_menu_key(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let Ok(key) = u16::try_from(wparam.0) else {
        // SAFETY: Unhandled keys retain normal popup processing.
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    };
    let event = match key {
        key if key == VK_ESCAPE.0 => ContextMenuEvent::DismissRequested,
        key if key == VK_RETURN.0 || key == VK_SPACE.0 => {
            ContextMenuEvent::SelectionRequested
        }
        key if key == VK_LEFT.0 || key == VK_UP.0 => {
            ContextMenuEvent::MoveSelection(SelectionDirection::Previous)
        }
        key if key == VK_RIGHT.0 || key == VK_DOWN.0 => {
            ContextMenuEvent::MoveSelection(SelectionDirection::Next)
        }
        _ => {
            // SAFETY: Unhandled keys retain normal popup processing.
            return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
        }
    };
    push_window_event(hwnd, WindowEvent::ContextMenu(event));
    LRESULT(0)
}

fn apply_settings_minimum_size(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    // SAFETY: For WM_GETMINMAXINFO, lParam points to writable MINMAXINFO storage valid only for
    // this synchronous callback.
    let limits = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
    // SAFETY: Reading the DPI of this live settings HWND has no side effects.
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
    // SAFETY: Both output structures are writable, and `hwnd` is the live settings window that
    // received WM_NCHITTEST. Failure conservatively leaves the point in the client area.
    let converted = unsafe { ScreenToClient(hwnd, &raw mut client) }.as_bool();
    // SAFETY: Reading the client bounds of the live settings HWND does not mutate it.
    let bounds_read = unsafe { GetClientRect(hwnd, &raw mut bounds) }.is_ok();
    if !converted || !bounds_read {
        return LRESULT(isize::try_from(HTCLIENT).unwrap_or_default());
    }

    // SAFETY: Reading per-window DPI has no side effects on this live HWND.
    let dpi = DpiScale::from_system(unsafe { GetDpiForWindow(hwnd) });
    let header_bottom = dpi.physical_i32(64);
    let close_left = bounds.right.saturating_sub(dpi.physical_i32(52));
    let draggable =
        client.x >= 0 && client.x < close_left && client.y >= 0 && client.y < header_bottom;
    LRESULT(
        isize::try_from(if draggable {
            HTCAPTION
        } else {
            HTCLIENT
        })
        .unwrap_or_default(),
    )
}

fn is_search_window(hwnd: HWND) -> bool {
    window_kind(hwnd) == Some(WindowKind::Search)
}

fn is_settings_window(hwnd: HWND) -> bool {
    window_kind(hwnd) == Some(WindowKind::Settings)
}

fn is_context_menu_window(hwnd: HWND) -> bool {
    window_kind(hwnd) == Some(WindowKind::ContextMenu)
}

fn is_dock_window(hwnd: HWND) -> bool {
    window_kind(hwnd) == Some(WindowKind::Dock)
}

fn is_nonactivating_window(hwnd: HWND) -> bool {
    matches!(
        window_kind(hwnd),
        Some(WindowKind::Dock | WindowKind::Status)
    )
}

fn window_kind(hwnd: HWND) -> Option<WindowKind> {
    let mut kind = None;
    with_window_state(hwnd, |state| kind = Some(state.kind));
    kind
}

fn search_key_event(wparam: WPARAM) -> Option<SearchEvent> {
    let key = u16::try_from(wparam.0).ok()?;
    search_key_event_for(key, control_is_pressed())
}

fn wheel_selection_direction(wparam: WPARAM) -> Option<SelectionDirection> {
    let bits = u16::try_from((wparam.0 >> 16) & 0xFFFF).ok()?;
    match i16::from_ne_bytes(bits.to_ne_bytes()).cmp(&0) {
        std::cmp::Ordering::Greater => Some(SelectionDirection::Previous),
        std::cmp::Ordering::Less => Some(SelectionDirection::Next),
        std::cmp::Ordering::Equal => None,
    }
}

fn search_key_event_for(key: u16, control_pressed: bool) -> Option<SearchEvent> {
    if control_pressed {
        return match key {
            0x41 => Some(SearchEvent::Edit(SearchEdit::SelectAll)),
            0x56 => Some(SearchEvent::PasteRequested),
            _ => None,
        };
    }
    match key {
        key if key == VK_BACK.0 => Some(SearchEvent::Edit(SearchEdit::DeleteBackward)),
        key if key == VK_DELETE.0 => Some(SearchEvent::Edit(SearchEdit::DeleteForward)),
        key if key == VK_HOME.0 => {
            Some(SearchEvent::Edit(SearchEdit::MoveCursor(CursorMove::Home)))
        }
        key if key == VK_END.0 => {
            Some(SearchEvent::Edit(SearchEdit::MoveCursor(CursorMove::End)))
        }
        key if key == VK_LEFT.0 => Some(SearchEvent::Edit(SearchEdit::MoveCursor(
            CursorMove::Previous,
        ))),
        key if key == VK_RIGHT.0 => {
            Some(SearchEvent::Edit(SearchEdit::MoveCursor(CursorMove::Next)))
        }
        key if key == VK_UP.0 => {
            Some(SearchEvent::MoveSelection(SelectionDirection::Previous))
        }
        key if key == VK_DOWN.0 => {
            Some(SearchEvent::MoveSelection(SelectionDirection::Next))
        }
        key if key == VK_ESCAPE.0 => Some(SearchEvent::DismissRequested),
        key if key == VK_RETURN.0 => Some(SearchEvent::SubmitRequested),
        _ => None,
    }
}

fn settings_key(wparam: WPARAM) -> Option<SettingsKey> {
    let key = u16::try_from(wparam.0).ok()?;
    settings_key_for(key, control_is_pressed(), shift_is_pressed())
}

fn settings_key_for(
    key: u16,
    control_pressed: bool,
    shift_pressed: bool,
) -> Option<SettingsKey> {
    if control_pressed && key == 0x53 {
        return Some(SettingsKey::Save);
    }
    match key {
        key if key == VK_ESCAPE.0 => Some(SettingsKey::Escape),
        key if key == VK_RETURN.0 => Some(SettingsKey::Enter),
        key if key == VK_TAB.0 => Some(SettingsKey::Tab {
            reverse: shift_pressed,
        }),
        key if key == VK_LEFT.0 => Some(SettingsKey::Left),
        key if key == VK_RIGHT.0 => Some(SettingsKey::Right),
        key if key == VK_UP.0 => Some(SettingsKey::Up),
        key if key == VK_DOWN.0 => Some(SettingsKey::Down),
        key if key == VK_SPACE.0 => Some(SettingsKey::Space),
        _ => None,
    }
}

fn control_is_pressed() -> bool {
    key_is_pressed(VK_CONTROL)
}

fn shift_is_pressed() -> bool {
    key_is_pressed(VK_SHIFT)
}

fn push_search_text_unit(hwnd: HWND, wparam: WPARAM) {
    let Ok(unit) = u16::try_from(wparam.0) else {
        return;
    };
    with_window_state(hwnd, |state| {
        let character = decode_text_unit(&state.pending_high_surrogate, unit);
        if let Some(character) = character {
            state.push(WindowEvent::Search(SearchEvent::TextInput(character)));
        }
    });
}

fn decode_text_unit(pending_high_surrogate: &Cell<Option<u16>>, unit: u16) -> Option<char> {
    if (0xD800..=0xDBFF).contains(&unit) {
        pending_high_surrogate.set(Some(unit));
        None
    } else if (0xDC00..=0xDFFF).contains(&unit) {
        pending_high_surrogate.replace(None).and_then(|high| {
            char::decode_utf16([high, unit])
                .next()
                .and_then(std::result::Result::ok)
        })
    } else {
        pending_high_surrogate.set(None);
        char::from_u32(u32::from(unit)).filter(|character| !character.is_control())
    }
}

fn low_word(value: usize) -> u32 {
    u32::try_from(value & 0xFFFF).unwrap_or_default()
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
    release_capture_if_owned(hwnd);
}

fn release_capture_if_owned(hwnd: HWND) {
    release_pointer(hwnd);
}

fn animation_is_active(hwnd: HWND) -> bool {
    let mut active = false;
    with_window_state(hwnd, |state| active = state.animation_active.get());
    active
}

fn stop_animation_timer(hwnd: HWND) {
    with_window_state(hwnd, |state| {
        let _ = state.set_animation_active(hwnd, false);
    });
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

fn size_from_lparam(lparam: LPARAM) -> (u32, u32) {
    let packed = lparam.0.cast_unsigned();
    let width = u32::try_from(packed & 0xFFFF).unwrap_or_default();
    let height = u32::try_from((packed >> 16) & 0xFFFF).unwrap_or_default();
    (width, height)
}

fn client_point_from_lparam(lparam: LPARAM) -> (i32, i32) {
    let point = signed_point_from_lparam(lparam);
    (point.x, point.y)
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
    // SAFETY: `client` is live writable storage and `hwnd` is the window that
    // received WM_CONTEXTMENU. Failure leaves the original screen point, which
    // is still a deterministic fallback for event delivery.
    let converted = unsafe { ScreenToClient(hwnd, &raw mut client) }.as_bool();
    let client = if converted {
        SignedPoint {
            x: client.x,
            y: client.y,
        }
    } else {
        screen
    };
    DockContextRequest::Pointer { screen, client }
}

fn dpi_from_wparam(wparam: WPARAM) -> u32 {
    u32::try_from(wparam.0 & 0xFFFF).unwrap_or_default()
}

fn requests_placement_refresh(message: u32, wparam: WPARAM) -> bool {
    message == WM_DISPLAYCHANGE
        || (message == WM_SETTINGCHANGE
            && u32::try_from(wparam.0).ok() == Some(SPI_SETWORKAREA.0))
}

pub fn apply_rounded_region(hwnd: HWND, radius_dips: u32) {
    if radius_dips == 0 {
        // SAFETY: A null region removes any previous manual clipping from this live HWND and
        // returns ownership of its visible curve to DWM.
        let _ = unsafe { SetWindowRgn(hwnd, None, true) };
        return;
    }

    let mut bounds = RECT::default();
    // SAFETY: `bounds` is valid writable storage and HWND is live while processing UI work.
    if unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &raw mut bounds)
    }
    .is_err()
    {
        return;
    }

    // SAFETY: Reading per-window DPI does not mutate the HWND.
    let dpi = DpiScale::from_system(unsafe { GetDpiForWindow(hwnd) });
    let diameter = dpi.physical_i32(radius_dips).max(1) * 2;
    // SAFETY: Width, height, and ellipse dimensions are finite physical-pixel values.
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

    // SAFETY: On success SetWindowRgn takes ownership; on failure we release the region below.
    if unsafe { SetWindowRgn(hwnd, Some(region), true) } == 0 {
        // SAFETY: SetWindowRgn failed, so Lotus still owns this valid GDI region handle.
        let _ = unsafe { DeleteObject(HGDIOBJ::from(region)) };
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "WNDCLASSEXW is a fixed Win32 ABI structure far smaller than u32::MAX"
)]
const fn window_class_size() -> u32 {
    size_of::<WNDCLASSEXW>() as u32
}
