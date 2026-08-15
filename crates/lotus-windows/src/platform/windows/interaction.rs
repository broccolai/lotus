use std::fmt;
use std::mem::size_of;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetActiveWindow, GetCapture, GetFocus, GetKeyState, ReleaseCapture, SetActiveWindow,
    SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, CallNextHookEx, DispatchMessageW, GA_ROOT, GetAncestor,
    GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, HHOOK, IDC_ARROW, IDC_HAND,
    IDC_SIZEWE, KillTimer, LoadCursorW, MSG, MSLLHOOKSTRUCT, PostMessageW, PostQuitMessage,
    SM_CXDRAG, SM_CYDRAG, SetCursor, SetForegroundWindow, SetTimer, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, WH_MOUSE_LL, WM_LBUTTONDOWN, WM_MBUTTONDOWN,
    WM_RBUTTONDOWN, WM_XBUTTONDOWN, WindowFromPoint,
};

use crate::NativeError;

static OUTSIDE_CLICK_TARGET: AtomicIsize = AtomicIsize::new(0);
static OUTSIDE_CLICK_MESSAGE: AtomicIsize = AtomicIsize::new(0);

pub(crate) struct OutsideClickObserver {
    hook: HHOOK,
    target: HWND,
}

impl OutsideClickObserver {
    pub(crate) fn start(target: HWND, message: u32) -> Result<Self, NativeError> {
        // SAFETY: A null module name requests this process module, which remains loaded while the
        // callback and its hook are active.
        let module = unsafe { GetModuleHandleW(None) }?;
        OUTSIDE_CLICK_TARGET.store(target.0.addr().cast_signed(), Ordering::Release);
        OUTSIDE_CLICK_MESSAGE.store(
            isize::try_from(message).unwrap_or_default(),
            Ordering::Release,
        );

        // SAFETY: The callback has the required ABI and static lifetime. A zero thread ID installs
        // the documented low-level mouse observer without intercepting input.
        let hook = unsafe {
            SetWindowsHookExW(
                WH_MOUSE_LL,
                Some(outside_click_hook),
                Some(HINSTANCE(module.0)),
                0,
            )
        }
        .inspect_err(|_| clear_outside_click_target(target))?;
        Ok(Self { hook, target })
    }
}

impl Drop for OutsideClickObserver {
    fn drop(&mut self) {
        clear_outside_click_target(self.target);
        // SAFETY: This guard owns the successful hook installation and releases it once.
        let _ = unsafe { UnhookWindowsHookEx(self.hook) };
    }
}

fn clear_outside_click_target(target: HWND) {
    let target = target.0.addr().cast_signed();
    let _ = OUTSIDE_CLICK_TARGET.compare_exchange(
        target,
        0,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
    OUTSIDE_CLICK_MESSAGE.store(0, Ordering::Release);
}

unsafe extern "system" fn outside_click_hook(
    code: i32,
    message: WPARAM,
    data: LPARAM,
) -> LRESULT {
    if code >= 0 && is_pointer_press(message) && data.0 != 0 {
        let raw_target = OUTSIDE_CLICK_TARGET.load(Ordering::Acquire);
        let raw_message = OUTSIDE_CLICK_MESSAGE.load(Ordering::Acquire);
        if raw_target != 0 && raw_message > 0 {
            let target = HWND(raw_target.cast_unsigned() as *mut _);
            // SAFETY: Windows supplies a valid MSLLHOOKSTRUCT pointer for a nonnegative
            // WH_MOUSE_LL callback.
            let pointer = unsafe { &*(data.0 as *const MSLLHOOKSTRUCT) };
            // SAFETY: Both calls only inspect the window directly beneath the supplied screen
            // coordinate and its root owner; neither transfers ownership.
            let clicked_root = unsafe { GetAncestor(WindowFromPoint(pointer.pt), GA_ROOT) };
            let inside = clicked_root == target;
            if !inside {
                // SAFETY: Posting an application-owned message does not retain borrowed data.
                let _ = unsafe {
                    PostMessageW(
                        Some(target),
                        u32::try_from(raw_message).unwrap_or_default(),
                        WPARAM(0),
                        LPARAM(0),
                    )
                };
            }
        }
    }

    // SAFETY: Lotus observes but never consumes mouse input, so every event is forwarded intact.
    unsafe { CallNextHookEx(None, code, message, data) }
}

fn is_pointer_press(message: WPARAM) -> bool {
    u32::try_from(message.0).is_ok_and(|message| {
        matches!(
            message,
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
        )
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FocusClaim {
    Owned,
    Denied,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PointerCursor {
    #[default]
    Arrow,
    Hand,
    HorizontalResize,
}

impl PointerCursor {
    pub(crate) fn apply(self) -> Result<(), NativeError> {
        let resource = match self {
            Self::Arrow => IDC_ARROW,
            Self::Hand => IDC_HAND,
            Self::HorizontalResize => IDC_SIZEWE,
        };
        // SAFETY: Each resource is a predefined system cursor and requires no instance handle.
        let cursor = unsafe { LoadCursorW(None, resource)? };
        // SAFETY: The shared system cursor remains owned by Windows and may be selected directly.
        unsafe { SetCursor(Some(cursor)) };
        Ok(())
    }
}

impl FocusClaim {
    pub(crate) const fn is_owned(self) -> bool {
        matches!(self, Self::Owned)
    }
}

pub(crate) fn claim_keyboard_focus(hwnd: HWND) -> FocusClaim {
    if owns_keyboard_focus(hwnd) || focus_once(hwnd) {
        return FocusClaim::Owned;
    }

    // SAFETY: Reading the current foreground HWND has no preconditions or ownership transfer.
    let foreground = unsafe { GetForegroundWindow() };
    let foreground_thread = if foreground.is_invalid() {
        0
    } else {
        // SAFETY: The observed foreground HWND is used only for this immediate thread-id query.
        unsafe { GetWindowThreadProcessId(foreground, None) }
    };
    // SAFETY: Reading the caller's thread ID has no preconditions.
    let current_thread = unsafe { GetCurrentThreadId() };
    let _attachment = InputQueueAttachment::new(current_thread, foreground_thread);

    if focus_once(hwnd) {
        FocusClaim::Owned
    } else {
        FocusClaim::Denied
    }
}

pub(crate) fn activate_window(hwnd: HWND) -> FocusClaim {
    if owns_foreground_application(hwnd) {
        return FocusClaim::Owned;
    }

    // SAFETY: The queried HWND and thread identifiers are borrowed only for this activation.
    let foreground = unsafe { GetForegroundWindow() };
    let foreground_thread = window_thread(foreground);
    let target_thread = window_thread(hwnd);
    // SAFETY: Reading the caller's thread ID has no preconditions.
    let current_thread = unsafe { GetCurrentThreadId() };
    let _foreground_attachment =
        InputQueueAttachment::new(current_thread, foreground_thread);
    let _target_attachment = InputQueueAttachment::new(current_thread, target_thread);

    if request_activation(hwnd) || owns_foreground_application(hwnd) {
        FocusClaim::Owned
    } else {
        FocusClaim::Denied
    }
}

fn window_thread(hwnd: HWND) -> u32 {
    if hwnd.is_invalid() {
        return 0;
    }
    // SAFETY: The borrowed HWND is used only for this immediate thread-id query.
    unsafe { GetWindowThreadProcessId(hwnd, None) }
}

fn focus_once(hwnd: HWND) -> bool {
    // SAFETY: Each operation targets the same live top-level HWND on its owning UI thread.
    unsafe {
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetActiveWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
    }
    owns_keyboard_focus(hwnd)
}

fn request_activation(hwnd: HWND) -> bool {
    // SAFETY: Each operation targets the same live top-level HWND on its owning UI thread.
    unsafe {
        let _ = BringWindowToTop(hwnd);
        let requested = SetForegroundWindow(hwnd).as_bool();
        let _ = SetActiveWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        requested
    }
}

fn owns_foreground_application(hwnd: HWND) -> bool {
    // SAFETY: Reading the current foreground HWND has no preconditions or ownership transfer.
    let foreground = unsafe { GetForegroundWindow() };
    if foreground == hwnd {
        return true;
    }
    let target_process = window_process(hwnd);
    target_process != 0 && target_process == window_process(foreground)
}

fn window_process(hwnd: HWND) -> u32 {
    if hwnd.is_invalid() {
        return 0;
    }
    let mut process_id = 0;
    // SAFETY: The borrowed HWND is used only for this immediate process-id query.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut process_id)) };
    process_id
}

fn owns_keyboard_focus(hwnd: HWND) -> bool {
    // SAFETY: These calls only inspect foreground and calling-thread input state.
    unsafe {
        GetForegroundWindow() == hwnd && GetActiveWindow() == hwnd && GetFocus() == hwnd
    }
}

struct InputQueueAttachment {
    source: u32,
    target: u32,
    attached: bool,
}

impl InputQueueAttachment {
    fn new(source: u32, target: u32) -> Self {
        let attached = source != 0
            && target != 0
            && source != target
            // SAFETY: Both IDs identify live GUI threads observed immediately before this call.
            && unsafe { AttachThreadInput(source, target, true) }.as_bool();
        Self {
            source,
            target,
            attached,
        }
    }
}

impl Drop for InputQueueAttachment {
    fn drop(&mut self) {
        if self.attached {
            // SAFETY: This exactly balances the successful attachment owned by this guard.
            let _ = unsafe { AttachThreadInput(self.source, self.target, false) };
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WindowTimer {
    id: usize,
    interval_ms: u32,
}

impl WindowTimer {
    pub(crate) const fn new(id: usize, interval_ms: u32) -> Self {
        Self { id, interval_ms }
    }

    pub(crate) const fn matches(self, id: usize) -> bool {
        self.id == id
    }

    pub(crate) fn start(self, hwnd: HWND) -> Result<(), NativeError> {
        // SAFETY: The timer belongs to the supplied HWND and posts WM_TIMER without retaining an
        // unmanaged callback. Reusing the typed ID refreshes that exact window-owned timer.
        if unsafe { SetTimer(Some(hwnd), self.id, self.interval_ms, None) } == 0 {
            return Err(windows::core::Error::from_thread().into());
        }
        Ok(())
    }

    pub(crate) fn stop(self, hwnd: HWND) {
        // SAFETY: Killing a missing timer is harmless; a present timer is identified by HWND+ID.
        let _ = unsafe { KillTimer(Some(hwnd), self.id) };
    }
}

pub struct NativeMessage(MSG);

impl NativeMessage {
    pub const fn id(&self) -> u32 {
        self.0.message
    }

    pub const fn parameter(&self) -> usize {
        self.0.wParam.0
    }

    pub const fn is_thread_message(&self) -> bool {
        self.0.hwnd.0.is_null()
    }

    pub fn dispatch(&self) {
        // SAFETY: `self.0` was initialized by a successful GetMessageW call and remains live for
        // both synchronous translation and dispatch operations.
        unsafe {
            let _ = TranslateMessage(&raw const self.0);
            DispatchMessageW(&raw const self.0);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MessagePumpError;

impl fmt::Display for MessagePumpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GetMessageW failed")
    }
}

pub fn next_message() -> Result<Option<NativeMessage>, MessagePumpError> {
    let mut message = MSG::default();
    // SAFETY: `message` is valid writable storage for the duration of GetMessageW.
    match unsafe { GetMessageW(&raw mut message, None, 0, 0) }.0 {
        -1 => Err(MessagePumpError),
        0 => Ok(None),
        _ => Ok(Some(NativeMessage(message))),
    }
}

pub fn request_exit(exit_code: i32) {
    // SAFETY: Posting WM_QUIT to the current thread transfers no pointers or ownership.
    unsafe { PostQuitMessage(exit_code) };
}

pub(crate) fn capture_pointer(hwnd: HWND) {
    // SAFETY: Capturing to this live UI-thread HWND transfers no ownership and does not activate it.
    unsafe { SetCapture(hwnd) };
}

pub(crate) fn release_pointer(hwnd: HWND) {
    // SAFETY: The ownership check prevents Lotus from disturbing capture held by another HWND.
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
    // SAFETY: The request declares its ABI size and Windows retains no pointer to it.
    unsafe { TrackMouseEvent(&raw mut tracking) }.is_ok()
}

pub(crate) fn key_is_pressed(key: VIRTUAL_KEY) -> bool {
    // SAFETY: GetKeyState reads the calling UI thread's state for a documented virtual key.
    unsafe { GetKeyState(i32::from(key.0)) }.cast_unsigned() & 0x8000 != 0
}

pub(crate) fn drag_threshold(hwnd: HWND) -> (u32, u32) {
    // SAFETY: All three calls are read-only queries against a live HWND and its effective DPI.
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
