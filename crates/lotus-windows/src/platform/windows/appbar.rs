use std::mem::size_of;

use lotus_core::fullscreen::ScreenRect;
use lotus_core::settings::DockSettings;
use thiserror::Error;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Shell::{
    ABE_BOTTOM, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, ABN_FULLSCREENAPP,
    APPBARDATA, DefSubclassProc, RemoveWindowSubclass, SHAppBarMessage, SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, PostThreadMessageW, RegisterWindowMessageW,
    SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, WINDOW_EX_STYLE, WM_APP, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::w;

use crate::NativeError;
use crate::exclusive_taskbar::{ExclusiveTaskbarError, ExclusiveTaskbarGuard};
use crate::explorer_bridge::ExplorerBridgeLease;
use crate::taskbar_state::{TaskbarStateError, TaskbarStateGuard};
use crate::window::{AppBarLayout, DockWindow};

const FULLSCREEN_NOTIFICATION_MESSAGE: u32 = WM_APP + 0x4C8;
const RESERVATION_SUBCLASS_ID: usize = 0x4C4F_5455;

pub fn fullscreen_notification(
    is_thread_message: bool,
    message_id: u32,
    parameter: usize,
) -> Option<bool> {
    (is_thread_message && message_id == FULLSCREEN_NOTIFICATION_MESSAGE)
        .then_some(parameter != 0)
}

#[derive(Debug, Error)]
pub enum ShellIntegrationError {
    #[error(transparent)]
    Native(#[from] NativeError),
    #[error(transparent)]
    Taskbar(#[from] TaskbarStateError),
    #[error(transparent)]
    ExclusiveTaskbar(#[from] ExclusiveTaskbarError),
    #[error(transparent)]
    Geometry(#[from] lotus_dock::appbar::AppBarGeometryError),
    #[error("the Windows shell rejected AppBar message {0}")]
    AppBarRejected(&'static str),
    #[error("RegisterWindowMessageW could not allocate the Lotus AppBar callback")]
    CallbackRegistration,
}

impl From<windows::core::Error> for ShellIntegrationError {
    fn from(error: windows::core::Error) -> Self {
        Self::Native(error.into())
    }
}

pub struct ShellIntegration {
    appbar: AppBarController,
    taskbar: Option<TaskbarOwnership>,
}

impl ShellIntegration {
    pub fn setup(
        settings: &DockSettings,
        dock: &DockWindow,
    ) -> Result<Option<Self>, ShellIntegrationError> {
        if !settings.replace_windows_taskbar {
            return Ok(None);
        }

        let taskbar = if settings.exclusive_taskbar_replacement {
            TaskbarOwnership::Exclusive {
                _bridge: ExplorerBridgeLease::attach(dock.hwnd()),
                _guard: ExclusiveTaskbarGuard::start()?,
            }
        } else {
            TaskbarOwnership::Autohide {
                _guard: TaskbarStateGuard::enable_autohide()?,
            }
        };
        let appbar = AppBarController::register(dock, settings)?;
        Ok(Some(Self {
            appbar,
            taskbar: Some(taskbar),
        }))
    }
}

impl Drop for ShellIntegration {
    fn drop(&mut self) {
        self.appbar.remove();
        let _ = self.taskbar.take();
    }
}

enum TaskbarOwnership {
    Autohide {
        _guard: TaskbarStateGuard,
    },
    Exclusive {
        _bridge: Option<ExplorerBridgeLease>,
        _guard: ExclusiveTaskbarGuard,
    },
}

struct AppBarController {
    reservation: ReservationWindow,
    registered: bool,
}

impl AppBarController {
    fn register(
        dock: &DockWindow,
        settings: &DockSettings,
    ) -> Result<Self, ShellIntegrationError> {
        // SAFETY: The static message name is NUL-terminated and process lifetime stable.
        let callback_message =
            unsafe { RegisterWindowMessageW(w!("Lotus.AppBar.Callback")) };
        if callback_message == 0 {
            return Err(ShellIntegrationError::CallbackRegistration);
        }

        let mut controller = Self {
            reservation: ReservationWindow::create(callback_message)?,
            registered: false,
        };
        let mut data = appbar_data(controller.reservation.hwnd());
        data.uCallbackMessage = callback_message;
        // SAFETY: APPBARDATA has the correct ABI size and refers to Lotus's live dock HWND.
        if unsafe { SHAppBarMessage(ABM_NEW, &raw mut data) } == 0 {
            return Err(ShellIntegrationError::AppBarRejected("ABM_NEW"));
        }
        controller.registered = true;

        let layout = requested_layout(dock, settings)?;
        let negotiated = controller.reserve(layout)?;
        dock.apply_appbar_layout(negotiated, settings)?;
        Ok(controller)
    }

    fn reserve(&self, layout: AppBarLayout) -> Result<AppBarLayout, ShellIntegrationError> {
        let mut data = appbar_data(self.reservation.hwnd());
        data.uEdge = ABE_BOTTOM;
        data.rc = to_rect(layout.reserved_rect());
        // SAFETY: The shell synchronously reads and updates this initialized APPBARDATA.
        unsafe { SHAppBarMessage(ABM_QUERYPOS, &raw mut data) };
        let queried = layout.with_shell_bounds(to_screen_rect(data.rc))?;
        data.rc = to_rect(queried.reserved_rect());
        // SAFETY: The registered AppBar HWND and negotiated rectangle remain valid.
        if unsafe { SHAppBarMessage(ABM_SETPOS, &raw mut data) } == 0 {
            return Err(ShellIntegrationError::AppBarRejected("ABM_SETPOS"));
        }
        let negotiated = queried.with_shell_bounds(to_screen_rect(data.rc))?;
        self.reservation.move_to(negotiated.reserved_rect())?;
        Ok(negotiated)
    }

    fn remove(&mut self) {
        if !self.registered {
            return;
        }
        let mut data = appbar_data(self.reservation.hwnd());
        // SAFETY: This controller owns the matching successful ABM_NEW registration.
        let _ = unsafe { SHAppBarMessage(ABM_REMOVE, &raw mut data) };
        self.registered = false;
    }
}

struct ReservationCallback {
    message: u32,
    thread_id: u32,
}

struct ReservationWindow {
    hwnd: HWND,
    _callback: Box<ReservationCallback>,
}

impl ReservationWindow {
    fn create(callback_message: u32) -> Result<Self, windows::core::Error> {
        let extended_style = WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0);
        // SAFETY: STATIC is a process-independent system class. The reservation is a hidden,
        // ownerless top-level popup with no application state or custom window procedure.
        let hwnd = unsafe {
            CreateWindowExW(
                extended_style,
                w!("STATIC"),
                w!("Lotus AppBar Reservation"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                None,
                None,
            )?
        };
        // SAFETY: Reading the creating UI thread identifier has no preconditions.
        let thread_id = unsafe { GetCurrentThreadId() };
        let callback = Box::new(ReservationCallback {
            message: callback_message,
            thread_id,
        });
        let callback_pointer = std::ptr::from_ref(callback.as_ref()).addr();
        // SAFETY: The boxed callback state stays at a stable address until this owned HWND removes
        // the subclass during Drop. The callback itself never unwinds or retains message pointers.
        if !unsafe {
            SetWindowSubclass(
                hwnd,
                Some(reservation_subclass_proc),
                RESERVATION_SUBCLASS_ID,
                callback_pointer,
            )
        }
        .as_bool()
        {
            // SAFETY: The HWND was created above and no subclass ownership was established.
            let _ = unsafe { DestroyWindow(hwnd) };
            return Err(windows::core::Error::from_thread());
        }
        Ok(Self {
            hwnd,
            _callback: callback,
        })
    }

    const fn hwnd(&self) -> HWND {
        self.hwnd
    }

    fn move_to(&self, rect: ScreenRect) -> Result<(), windows::core::Error> {
        // SAFETY: AppBar geometry validates a positive Win32-sized rectangle, and this guard
        // owns the live reservation HWND for the duration of the registration.
        unsafe {
            SetWindowPos(
                self.hwnd,
                None,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOACTIVATE | SWP_NOZORDER,
            )?;
        }
        Ok(())
    }
}

impl Drop for ReservationWindow {
    fn drop(&mut self) {
        // SAFETY: This exactly removes the subclass installed by create while its callback state
        // is still live.
        let _ = unsafe {
            RemoveWindowSubclass(
                self.hwnd,
                Some(reservation_subclass_proc),
                RESERVATION_SUBCLASS_ID,
            )
        };
        // SAFETY: This guard uniquely owns the hidden HWND and drops only after AppBar removal.
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}

unsafe extern "system" fn reservation_subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    callback_pointer: usize,
) -> LRESULT {
    let callback =
        std::ptr::with_exposed_provenance::<ReservationCallback>(callback_pointer);
    // SAFETY: SetWindowSubclass receives the stable boxed pointer and RemoveWindowSubclass runs
    // before that box is dropped.
    let callback = unsafe { &*callback };
    if message == callback.message
        && wparam.0 == usize::try_from(ABN_FULLSCREENAPP).unwrap_or(2)
    {
        let fullscreen = usize::from(lparam.0 != 0);
        // SAFETY: This posts a pointer-free private message to the creating UI thread.
        let _ = unsafe {
            PostThreadMessageW(
                callback.thread_id,
                FULLSCREEN_NOTIFICATION_MESSAGE,
                WPARAM(fullscreen),
                LPARAM(0),
            )
        };
        return LRESULT(0);
    }
    // SAFETY: Unhandled messages must continue through the subclass chain unchanged.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

impl Drop for AppBarController {
    fn drop(&mut self) {
        self.remove();
    }
}

fn requested_layout(
    dock: &DockWindow,
    settings: &DockSettings,
) -> Result<AppBarLayout, ShellIntegrationError> {
    let (width, height) = dock.client_size()?;
    let monitor = monitor_rect(dock.hwnd())?;
    AppBarLayout::new(monitor, width, height, settings.bottom_offset, dock.dpi())
        .map_err(Into::into)
}

fn monitor_rect(hwnd: HWND) -> Result<ScreenRect, windows::core::Error> {
    // SAFETY: The dock HWND is live; primary fallback guarantees a monitor handle.
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY) };
    let mut info = MONITORINFO {
        cbSize: monitor_info_size(),
        ..MONITORINFO::default()
    };
    // SAFETY: `info` has the required ABI size and valid writable storage.
    unsafe { GetMonitorInfoW(monitor, &raw mut info).ok()? };
    Ok(to_screen_rect(info.rcMonitor))
}

fn appbar_data(hwnd: HWND) -> APPBARDATA {
    APPBARDATA {
        cbSize: appbar_data_size(),
        hWnd: hwnd,
        lParam: LPARAM(0),
        ..APPBARDATA::default()
    }
}

const fn to_rect(rect: ScreenRect) -> RECT {
    RECT {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

const fn to_screen_rect(rect: RECT) -> ScreenRect {
    ScreenRect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "APPBARDATA is a fixed Win32 ABI structure"
)]
const fn appbar_data_size() -> u32 {
    size_of::<APPBARDATA>() as u32
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "MONITORINFO is a fixed Win32 ABI structure"
)]
const fn monitor_info_size() -> u32 {
    size_of::<MONITORINFO>() as u32
}
