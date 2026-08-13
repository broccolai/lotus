use std::ffi::c_void;
use std::mem::size_of;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use lotus_core::fullscreen::{ScreenRect, is_fullscreen_foreground};
use lotus_core::window::{WindowId, WindowInfo};
use windows::Win32::Foundation::{CloseHandle, E_FAIL, HANDLE, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, GetCurrentThreadId, OpenProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE, EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_NAMECHANGE,
    EVENT_OBJECT_SHOW, EVENT_SYSTEM_FOREGROUND, EnumWindows, GA_ROOT, GW_OWNER, GWL_EXSTYLE,
    GetAncestor, GetClassNameW, GetForegroundWindow, GetWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, IsZoomed,
    KillTimer, OBJID_WINDOW, PostThreadMessageW, SetTimer, WINEVENT_OUTOFCONTEXT,
    WINEVENT_SKIPOWNPROCESS, WM_APP, WM_TIMER, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, Error, Result as WindowsResult};

use crate::NativeError;

const REFRESH_MESSAGE: u32 = WM_APP + 1;
const REFRESH_DELAY_MS: u32 = 120;
const RECONCILE_INTERVAL_MS: u32 = 1_000;
const IMAGE_PATH_CAPACITY: usize = 32_768;
const CLASS_NAME_CAPACITY: usize = 128;
const FULLSCREEN_EDGE_TOLERANCE: i32 = 2;

static CALLBACK_THREAD: AtomicU32 = AtomicU32::new(0);
static NOTIFICATION_QUEUED: AtomicBool = AtomicBool::new(false);

pub struct WindowTracker {
    hooks: Vec<OwnedWinEventHook>,
    windows: Vec<WindowInfo>,
    own_process_id: u32,
    timer_id: Option<usize>,
    reconcile_timer_id: usize,
    fullscreen_window: Option<WindowId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowTrackerEvent {
    SnapshotRefreshed,
}

impl WindowTracker {
    pub fn start() -> Result<Self, NativeError> {
        // SAFETY: These identifiers are immutable properties of the calling process and thread.
        let (own_process_id, thread_id) = unsafe { (GetCurrentProcessId(), GetCurrentThreadId()) };
        if CALLBACK_THREAD
            .compare_exchange(0, thread_id, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(Error::new(E_FAIL, "a Lotus window tracker is already active").into());
        }

        let mut tracker = Self {
            hooks: Vec::with_capacity(TRACKED_EVENTS.len()),
            windows: Vec::new(),
            own_process_id,
            timer_id: None,
            reconcile_timer_id: 0,
            fullscreen_window: None,
        };
        for event in TRACKED_EVENTS {
            tracker.hooks.push(OwnedWinEventHook::install(event)?);
        }
        tracker.refresh()?;
        tracker.reconcile_timer_id = create_thread_timer(RECONCILE_INTERVAL_MS)?;
        Ok(tracker)
    }

    pub fn current_windows(&self) -> &[WindowInfo] {
        &self.windows
    }

    pub const fn fullscreen_window(&self) -> Option<WindowId> {
        self.fullscreen_window
    }

    pub fn handle_message(
        &mut self,
        is_thread_message: bool,
        message_id: u32,
        parameter: usize,
    ) -> Result<Option<WindowTrackerEvent>, NativeError> {
        if !is_thread_message {
            return Ok(None);
        }

        match message_id {
            REFRESH_MESSAGE => {
                NOTIFICATION_QUEUED.store(false, Ordering::Release);
                self.restart_timer()?;
                Ok(None)
            }
            WM_TIMER if self.timer_id == Some(parameter) => {
                self.cancel_timer();
                self.refresh()?;
                Ok(Some(WindowTrackerEvent::SnapshotRefreshed))
            }
            WM_TIMER if self.reconcile_timer_id == parameter => {
                Ok(self.refresh_if_changed()?.then_some(WindowTrackerEvent::SnapshotRefreshed))
            }
            _ => Ok(None),
        }
    }

    fn restart_timer(&mut self) -> Result<(), NativeError> {
        self.cancel_timer();
        self.timer_id = Some(create_thread_timer(REFRESH_DELAY_MS)?);
        Ok(())
    }

    fn cancel_timer(&mut self) {
        if let Some(timer_id) = self.timer_id.take() {
            // SAFETY: `timer_id` was returned for a thread timer created by this tracker.
            let _ = unsafe { KillTimer(None, timer_id) };
        }
    }

    fn refresh(&mut self) -> Result<(), NativeError> {
        self.windows = enumerate_windows(self.own_process_id)?;
        self.fullscreen_window = observe_fullscreen_window(self.own_process_id);
        Ok(())
    }

    fn refresh_if_changed(&mut self) -> Result<bool, NativeError> {
        let windows = enumerate_windows(self.own_process_id)?;
        let fullscreen_window = observe_fullscreen_window(self.own_process_id);
        if self.windows == windows && self.fullscreen_window == fullscreen_window {
            return Ok(false);
        }
        self.windows = windows;
        self.fullscreen_window = fullscreen_window;
        Ok(true)
    }
}

impl Drop for WindowTracker {
    fn drop(&mut self) {
        CALLBACK_THREAD.store(0, Ordering::Release);
        NOTIFICATION_QUEUED.store(false, Ordering::Release);
        self.cancel_timer();
        if self.reconcile_timer_id != 0 {
            // SAFETY: This tracker owns the thread timer created during startup.
            let _ = unsafe { KillTimer(None, self.reconcile_timer_id) };
        }
        self.hooks.clear();
    }
}

fn create_thread_timer(interval_ms: u32) -> Result<usize, NativeError> {
    // SAFETY: A null HWND creates a timer for this thread's existing message queue. Lotus consumes
    // its WM_TIMER message directly and does not install an unmanaged callback.
    let timer_id = unsafe { SetTimer(None, 0, interval_ms, None) };
    if timer_id == 0 { Err(Error::from_thread().into()) } else { Ok(timer_id) }
}

const TRACKED_EVENTS: [u32; 6] = [
    EVENT_SYSTEM_FOREGROUND,
    EVENT_OBJECT_DESTROY,
    EVENT_OBJECT_SHOW,
    EVENT_OBJECT_HIDE,
    EVENT_OBJECT_LOCATIONCHANGE,
    EVENT_OBJECT_NAMECHANGE,
];

struct OwnedWinEventHook(HWINEVENTHOOK);

impl OwnedWinEventHook {
    fn install(event: u32) -> Result<Self, NativeError> {
        // SAFETY: The callback is a static function and out-of-context delivery requires no DLL
        // module. Skipping this process keeps window tracking independent of Lotus itself.
        let hook = unsafe {
            SetWinEventHook(
                event,
                event,
                None,
                Some(win_event_callback),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        };
        if hook.is_invalid() { Err(Error::from_thread().into()) } else { Ok(Self(hook)) }
    }
}

impl Drop for OwnedWinEventHook {
    fn drop(&mut self) {
        // SAFETY: This guard owns the successful SetWinEventHook registration.
        let _ = unsafe { UnhookWinEvent(self.0) };
    }
}

unsafe extern "system" fn win_event_callback(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object_id: i32,
    _child_id: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if hwnd.0.is_null()
        || (event != EVENT_SYSTEM_FOREGROUND && object_id != OBJID_WINDOW.0)
        || NOTIFICATION_QUEUED.swap(true, Ordering::AcqRel)
    {
        return;
    }

    let thread_id = CALLBACK_THREAD.load(Ordering::Acquire);
    if thread_id == 0 {
        NOTIFICATION_QUEUED.store(false, Ordering::Release);
        return;
    }

    // SAFETY: `thread_id` belongs to the tracker that installed this callback and owns an active
    // GUI message queue. The custom message contains no borrowed pointers.
    if unsafe { PostThreadMessageW(thread_id, REFRESH_MESSAGE, WPARAM(0), LPARAM(0)) }.is_err() {
        NOTIFICATION_QUEUED.store(false, Ordering::Release);
    }
}

fn enumerate_windows(own_process_id: u32) -> WindowsResult<Vec<WindowInfo>> {
    let mut state = EnumerationState { own_process_id, windows: Vec::new() };
    // SAFETY: EnumWindows invokes the callback synchronously while `state` remains live.
    unsafe { EnumWindows(Some(visit_window), pointer_lparam(&raw mut state))? };
    Ok(state.windows)
}

struct EnumerationState {
    own_process_id: u32,
    windows: Vec<WindowInfo>,
}

unsafe extern "system" fn visit_window(hwnd: HWND, state: LPARAM) -> BOOL {
    // SAFETY: `state` points to the live EnumerationState supplied to synchronous EnumWindows.
    let state = unsafe { &mut *(state.0 as *mut EnumerationState) };
    if let Some(window) = window_info(hwnd, state.own_process_id) {
        state.windows.push(window);
    }
    BOOL(1)
}

fn window_info(hwnd: HWND, own_process_id: u32) -> Option<WindowInfo> {
    if !should_include_window(hwnd) {
        return None;
    }

    let mut process_id = 0;
    // SAFETY: `process_id` is valid writable storage and querying does not mutate the window.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut process_id)) };
    if process_id == 0 || process_id == own_process_id {
        return None;
    }

    let title = window_title(hwnd);
    let executable_path = window_icon_identity(&title, process_image_path(process_id)?);
    Some(WindowInfo { id: window_id(hwnd)?, process_id, title, executable_path })
}

fn window_icon_identity(title: &str, executable_path: PathBuf) -> PathBuf {
    let executable = executable_path.file_name().and_then(|name| name.to_str());
    if title.eq_ignore_ascii_case("Settings")
        && executable.is_some_and(|name| {
            name.eq_ignore_ascii_case("ApplicationFrameHost.exe")
                || name.eq_ignore_ascii_case("SystemSettings.exe")
        })
    {
        return PathBuf::from(
            r"shell:AppsFolder\windows.immersivecontrolpanel_cw5n1h2txyewy!microsoft.windows.immersivecontrolpanel",
        );
    }
    executable_path
}

fn observe_fullscreen_window(own_process_id: u32) -> Option<WindowId> {
    // SAFETY: Foreground lookup is read-only and may validly return null.
    let hwnd = unsafe { GetForegroundWindow() };
    let id = window_id(hwnd)?;
    let mut process_id = 0;
    // SAFETY: `process_id` is valid writable storage and querying does not mutate the window.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut process_id)) };
    let eligible = process_id != 0 && process_id != own_process_id && should_include_window(hwnd);
    let window = window_bounds(hwnd)?;
    let monitor = monitor_bounds(hwnd)?;
    // SAFETY: This read-only query accepts the live foreground HWND.
    let maximized = unsafe { IsZoomed(hwnd).as_bool() };
    is_fullscreen_foreground(
        eligible,
        maximized,
        screen_rect(window),
        screen_rect(monitor),
        FULLSCREEN_EDGE_TOLERANCE,
    )
    .then_some(id)
}

fn window_bounds(hwnd: HWND) -> Option<RECT> {
    let mut bounds = RECT::default();
    // SAFETY: `bounds` is valid writable storage and the foreground HWND is live.
    unsafe { GetWindowRect(hwnd, &raw mut bounds) }.ok()?;
    Some(bounds)
}

fn monitor_bounds(hwnd: HWND) -> Option<RECT> {
    // SAFETY: The foreground HWND is live; nearest-monitor fallback is read-only.
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_invalid() {
        return None;
    }
    let mut info = MONITORINFO { cbSize: u32_size::<MONITORINFO>(), ..MONITORINFO::default() };
    // SAFETY: The monitor handle is valid and `info` has the required size initialized.
    unsafe { GetMonitorInfoW(monitor, &raw mut info) }.as_bool().then_some(info.rcMonitor)
}

const fn screen_rect(rect: RECT) -> ScreenRect {
    ScreenRect { left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom }
}

fn window_id(hwnd: HWND) -> Option<WindowId> {
    (!hwnd.0.is_null()).then(|| u64::try_from(hwnd.0.addr()).ok()).flatten().map(WindowId::new)
}

fn should_include_window(hwnd: HWND) -> bool {
    // SAFETY: These calls only inspect a candidate HWND supplied by EnumWindows.
    let (is_visible, root, has_owner, extended_style) = unsafe {
        (
            IsWindowVisible(hwnd).as_bool(),
            GetAncestor(hwnd, GA_ROOT),
            GetWindow(hwnd, GW_OWNER).is_ok(),
            GetWindowLongPtrW(hwnd, GWL_EXSTYLE).cast_unsigned(),
        )
    };
    let tool_window = usize::try_from(WS_EX_TOOLWINDOW.0).unwrap_or_default();
    let app_window = usize::try_from(WS_EX_APPWINDOW.0).unwrap_or_default();
    let is_tool_window = extended_style & tool_window != 0;
    let is_app_window = extended_style & app_window != 0;
    if !is_visible
        || root != hwnd
        || (has_owner && !is_app_window)
        || (is_tool_window && !is_app_window)
    {
        return false;
    }

    if matches!(
        window_class(hwnd).as_str(),
        "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
    ) {
        return false;
    }

    let mut cloaked = 0_u32;
    // SAFETY: `cloaked` has the exact type and size required by DWMWA_CLOAKED.
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast::<c_void>(),
            u32_size::<u32>(),
        )
    }
    .is_err()
        || cloaked == 0
}

fn window_title(hwnd: HWND) -> String {
    // SAFETY: Querying the title length does not mutate the candidate HWND.
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    let capacity = usize::try_from(length.max(0)).unwrap_or_default().saturating_add(1);
    let mut buffer = vec![0_u16; capacity];
    // SAFETY: `buffer` is writable and includes room for the terminating null character.
    let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..usize::try_from(copied.max(0)).unwrap_or_default()])
}

fn window_class(hwnd: HWND) -> String {
    let mut buffer = [0_u16; CLASS_NAME_CAPACITY];
    // SAFETY: `buffer` is valid writable storage and the call only inspects the HWND.
    let copied = unsafe { GetClassNameW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..usize::try_from(copied.max(0)).unwrap_or_default()])
}

pub(crate) fn process_image_path(process_id: u32) -> Option<PathBuf> {
    // SAFETY: The requested access is read-only and the PID came from a current top-level HWND.
    let process =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }.ok()?;
    let process = OwnedHandle(process);
    let mut buffer = vec![0_u16; IMAGE_PATH_CAPACITY];
    let mut length = u32::try_from(buffer.len()).ok()?;
    // SAFETY: The process handle is live, and `buffer`/`length` describe valid writable storage.
    unsafe {
        QueryFullProcessImageNameW(
            process.get(),
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &raw mut length,
        )
    }
    .ok()?;
    buffer.truncate(usize::try_from(length).ok()?);
    Some(PathBuf::from(String::from_utf16_lossy(&buffer)))
}

struct OwnedHandle(HANDLE);

impl OwnedHandle {
    fn get(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: This guard owns a successful OpenProcess handle.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

#[allow(
    clippy::cast_possible_wrap,
    reason = "Win32 LPARAM intentionally transports an in-process pointer-sized value"
)]
fn pointer_lparam<T>(pointer: *mut T) -> LPARAM {
    LPARAM(pointer.addr() as isize)
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "Win32 ABI scalar sizes are fixed and far below u32::MAX"
)]
const fn u32_size<T>() -> u32 {
    size_of::<T>() as u32
}
