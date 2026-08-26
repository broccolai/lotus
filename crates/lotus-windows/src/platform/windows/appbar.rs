use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

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
    ABN_POSCHANGED, APPBARDATA, DefSubclassProc, RemoveWindowSubclass, SHAppBarMessage,
    SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, PostMessageW, PostThreadMessageW,
    RegisterWindowMessageW, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, WINDOW_EX_STYLE,
    WM_NULL, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::w;

use crate::NativeError;
use crate::exclusive_taskbar::{ExclusiveTaskbarError, ExclusiveTaskbarGuard};
use crate::explorer_bridge::ExplorerBridgeLease;
use crate::messages::FULLSCREEN_NOTIFICATION as FULLSCREEN_NOTIFICATION_MESSAGE;
use crate::taskbar_state::{TaskbarStateError, TaskbarStateGuard};
use crate::window::{AppBarLayout, DockWindow};
const RESERVATION_SUBCLASS_ID: usize = 0x4C4F_5455;
static TASKBAR_CREATED_MESSAGE: AtomicU32 = AtomicU32::new(0);
static RECOVERY_QUEUED: AtomicBool = AtomicBool::new(false);
static RECOVERY_WAKE_FAILED: AtomicBool = AtomicBool::new(false);
static RECOVERY_SOURCE: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellIntegrationHealth {
    Disabled,
    Healthy,
    Degraded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellRecoverySource {
    Startup,
    TaskbarCreated,
    Settings,
    DisplayChange,
    SystemResume,
    SessionUnlock,
    AppBarPositionChanged,
}

impl ShellRecoverySource {
    const fn diagnostic_name(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::TaskbarCreated => "taskbar_created",
            Self::Settings => "settings",
            Self::DisplayChange => "display_change",
            Self::SystemResume => "system_resume",
            Self::SessionUnlock => "session_unlock",
            Self::AppBarPositionChanged => "appbar_position_changed",
        }
    }
}

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

impl ShellIntegrationError {
    fn recovery_stage(&self) -> &'static str {
        match self {
            Self::Taskbar(_) | Self::ExclusiveTaskbar(_) => "taskbar_ownership",
            Self::AppBarRejected("ABM_REMOVE") => "appbar_release",
            Self::Native(_)
            | Self::Geometry(_)
            | Self::AppBarRejected(_)
            | Self::CallbackRegistration => "appbar_setup",
        }
    }
}

pub struct ShellIntegration {
    taskbar_created_message: u32,
    recovery_enabled: bool,
    active: Option<ActiveShellIntegration>,
    health: ShellIntegrationHealth,
}

impl ShellIntegration {
    pub fn new(settings: &DockSettings, dock: &DockWindow, enabled: bool) -> Self {
        let taskbar_created_message = register_taskbar_created_message();
        let mut integration = Self {
            taskbar_created_message,
            recovery_enabled: enabled,
            active: None,
            health: ShellIntegrationHealth::Disabled,
        };
        if enabled {
            integration.recover(settings, dock, ShellRecoverySource::Startup);
        }
        integration
    }

    pub const fn health(&self) -> ShellIntegrationHealth {
        self.health
    }

    pub fn take_recovery_request(
        &self,
        is_thread_message: bool,
        message: u32,
    ) -> Option<ShellRecoverySource> {
        let private_wake =
            is_thread_message && message == crate::messages::SHELL_INTEGRATION_RECOVERY;
        let wake_failed = RECOVERY_WAKE_FAILED.swap(false, Ordering::AcqRel);
        if !private_wake && !wake_failed {
            return None;
        }
        if wake_failed {
            crate::diagnostics::record_diagnostic(
                "shell_integration.recovery_wake_failed",
                "the private thread wake failed; recovery resumed on the fallback window wake",
            );
        }

        if !RECOVERY_QUEUED.swap(false, Ordering::AcqRel) || !self.recovery_enabled {
            return None;
        }
        Some(match RECOVERY_SOURCE.swap(0, Ordering::AcqRel) {
            1 => ShellRecoverySource::AppBarPositionChanged,
            _ => ShellRecoverySource::TaskbarCreated,
        })
    }

    pub fn recover(
        &mut self,
        settings: &DockSettings,
        dock: &DockWindow,
        source: ShellRecoverySource,
    ) {
        if !self.recovery_enabled {
            self.release_to_normal_placement(settings, dock, "disabled");
            self.health = ShellIntegrationHealth::Disabled;
            return;
        }
        if self.taskbar_created_message == 0 {
            self.taskbar_created_message = register_taskbar_created_message();
        }
        crate::diagnostics::record_diagnostic(
            "shell_integration.recovery_requested",
            &format!("source={}", source.diagnostic_name()),
        );

        if !settings.replace_windows_taskbar {
            self.release_to_normal_placement(settings, dock, "disabled_by_settings");
            self.health = ShellIntegrationHealth::Disabled;
            crate::diagnostics::record_diagnostic(
                "shell_integration.recovery_succeeded",
                &format!("source={} health=disabled", source.diagnostic_name()),
            );
            return;
        }

        if self.active.is_none() {
            match ActiveShellIntegration::start(settings, dock) {
                Ok(active) => self.active = Some(active),
                Err(error) => {
                    self.record_recovery_failure(settings, dock, source, &error);
                    return;
                }
            }
        }
        let outcome = self
            .active
            .as_mut()
            .expect("active shell integration was initialized above")
            .recover(settings, dock, source);
        match outcome {
            Ok(active_healthy) => {
                self.health = if self.taskbar_created_message != 0 && active_healthy {
                    ShellIntegrationHealth::Healthy
                } else {
                    ShellIntegrationHealth::Degraded
                };
                crate::diagnostics::record_diagnostic(
                    "shell_integration.recovery_succeeded",
                    &format!(
                        "source={} health={}",
                        source.diagnostic_name(),
                        health_name(self.health)
                    ),
                );
                if self.health == ShellIntegrationHealth::Degraded {
                    crate::diagnostics::record_diagnostic(
                        "shell_integration.recovery_degraded",
                        &format!("source={} health=degraded", source.diagnostic_name()),
                    );
                }
            }
            Err(error) => {
                self.record_recovery_failure(settings, dock, source, &error);
            }
        }
    }

    fn record_recovery_failure(
        &mut self,
        settings: &DockSettings,
        dock: &DockWindow,
        source: ShellRecoverySource,
        error: &ShellIntegrationError,
    ) {
        self.release_to_normal_placement(settings, dock, "recovery_failed");
        self.health = ShellIntegrationHealth::Degraded;
        crate::diagnostics::record_error("shell_integration.recovery_degraded", error);
        crate::diagnostics::record_diagnostic(
            "shell_integration.recovery_failed",
            &format!(
                "source={} stage={} fail_open=native_taskbar",
                source.diagnostic_name(),
                error.recovery_stage()
            ),
        );
    }

    fn release_to_normal_placement(
        &mut self,
        settings: &DockSettings,
        dock: &DockWindow,
        reason: &str,
    ) {
        drop(self.active.take());
        dock.clear_appbar_ownership();
        if let Err(error) = dock.refresh_placement(settings) {
            crate::diagnostics::record_error(
                "shell_integration.fail_open_placement_failed",
                &error,
            );
            crate::diagnostics::record_diagnostic(
                "shell_integration.fail_open_placement_failed",
                &format!("reason={reason}"),
            );
        }
    }
}

fn register_taskbar_created_message() -> u32 {
    let registered = TASKBAR_CREATED_MESSAGE.load(Ordering::Acquire);
    if registered != 0 {
        return registered;
    }

    let registered = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    if registered != 0 {
        let _ = TASKBAR_CREATED_MESSAGE.compare_exchange(
            0,
            registered,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
    TASKBAR_CREATED_MESSAGE.load(Ordering::Acquire)
}

pub(crate) fn queue_taskbar_created_recovery(hwnd: HWND, message: u32) -> bool {
    let taskbar_created = TASKBAR_CREATED_MESSAGE.load(Ordering::Acquire);
    if taskbar_created == 0 || message != taskbar_created {
        return false;
    }
    queue_recovery(hwnd, 0);
    true
}

fn queue_appbar_position_recovery(hwnd: HWND) {
    queue_recovery(hwnd, 1);
}

fn queue_recovery(hwnd: HWND, source: u32) {
    if RECOVERY_QUEUED.swap(true, Ordering::AcqRel) {
        if source == 0 {
            RECOVERY_SOURCE.store(0, Ordering::Release);
        }
        return;
    }
    RECOVERY_SOURCE.store(source, Ordering::Release);

    let thread_id = unsafe { GetCurrentThreadId() };
    if unsafe {
        PostThreadMessageW(
            thread_id,
            crate::messages::SHELL_INTEGRATION_RECOVERY,
            WPARAM(0),
            LPARAM(0),
        )
    }
    .is_err()
    {
        RECOVERY_WAKE_FAILED.store(true, Ordering::Release);
        let _ = unsafe { PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0)) };
    }
}

struct ActiveShellIntegration {
    appbar: Option<AppBarController>,
    taskbar: Option<TaskbarOwnership>,
    taskbar_needs_refresh: bool,
}

impl ActiveShellIntegration {
    fn start(
        settings: &DockSettings,
        dock: &DockWindow,
    ) -> Result<Self, ShellIntegrationError> {
        let taskbar = if settings.exclusive_taskbar_replacement {
            let bridge = ExplorerBridgeLease::attach(dock.hwnd());
            if bridge.is_none() {
                crate::diagnostics::record_diagnostic(
                    "shell_integration.bridge_attachment_failed",
                    "mode=exclusive",
                );
            }
            TaskbarOwnership::Exclusive {
                bridge,
                guard: ExclusiveTaskbarGuard::start()?,
            }
        } else {
            TaskbarOwnership::Autohide {
                guard: TaskbarStateGuard::enable_autohide()?,
            }
        };
        let taskbar_needs_refresh = !taskbar.is_healthy();
        Ok(Self {
            appbar: None,
            taskbar: Some(taskbar),
            taskbar_needs_refresh,
        })
    }

    fn is_healthy(&self) -> bool {
        self.taskbar
            .as_ref()
            .is_some_and(TaskbarOwnership::is_healthy)
    }

    fn recover(
        &mut self,
        settings: &DockSettings,
        dock: &DockWindow,
        source: ShellRecoverySource,
    ) -> Result<bool, ShellIntegrationError> {
        if source == ShellRecoverySource::AppBarPositionChanged
            && let Some(appbar) = self.appbar.as_ref()
        {
            appbar.refresh_position(dock, settings)?;
            let healthy = self
                .taskbar
                .as_mut()
                .map_or(Ok(false), |taskbar| taskbar.reassert(dock))?;
            self.taskbar_needs_refresh = !healthy;
            return Ok(healthy);
        }
        if let Some(appbar) = self.appbar.as_mut() {
            appbar.release_for_recovery(source)?;
            drop(self.appbar.take());
            dock.clear_appbar_ownership();
            self.taskbar_needs_refresh = true;
        }
        let ownership_healthy = if self.taskbar_needs_refresh {
            let healthy = self
                .taskbar
                .as_mut()
                .map_or(Ok(false), |taskbar| taskbar.refresh(dock))?;
            self.taskbar_needs_refresh = !healthy;
            healthy
        } else {
            self.is_healthy()
        };
        self.appbar = Some(AppBarController::register(dock, settings)?);
        Ok(ownership_healthy)
    }
}

enum TaskbarOwnership {
    Autohide {
        guard: TaskbarStateGuard,
    },
    Exclusive {
        bridge: Option<ExplorerBridgeLease>,
        guard: ExclusiveTaskbarGuard,
    },
}

impl TaskbarOwnership {
    const fn is_healthy(&self) -> bool {
        match self {
            Self::Autohide { .. } => true,
            Self::Exclusive { bridge, .. } => bridge.is_some(),
        }
    }

    fn refresh(&mut self, dock: &DockWindow) -> Result<bool, ShellIntegrationError> {
        match self {
            Self::Autohide { guard } => {
                guard.ensure_autohide()?;
                Ok(true)
            }
            Self::Exclusive { bridge, guard } => {
                drop(bridge.take());
                *bridge = ExplorerBridgeLease::attach(dock.hwnd());
                guard.reassert_hidden()?;
                if bridge.is_none() {
                    crate::diagnostics::record_diagnostic(
                        "shell_integration.bridge_attachment_failed",
                        "mode=exclusive",
                    );
                }
                Ok(bridge.is_some())
            }
        }
    }

    fn reassert(&mut self, dock: &DockWindow) -> Result<bool, ShellIntegrationError> {
        match self {
            Self::Autohide { guard } => {
                guard.ensure_autohide()?;
                Ok(true)
            }
            Self::Exclusive { bridge, guard } => {
                guard.reassert_hidden()?;
                if bridge.is_none() {
                    *bridge = ExplorerBridgeLease::attach(dock.hwnd());
                }
                Ok(bridge.is_some())
            }
        }
    }
}

const fn health_name(health: ShellIntegrationHealth) -> &'static str {
    match health {
        ShellIntegrationHealth::Disabled => "disabled",
        ShellIntegrationHealth::Healthy => "healthy",
        ShellIntegrationHealth::Degraded => "degraded",
    }
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
        dock.clear_appbar_ownership();
        dock.refresh_placement(settings)?;

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
        unsafe { SHAppBarMessage(ABM_QUERYPOS, &raw mut data) };
        let queried = layout.with_shell_bounds(to_screen_rect(data.rc))?;
        data.rc = to_rect(queried.reserved_rect());
        self.reservation.ignore_next_position_change();
        if unsafe { SHAppBarMessage(ABM_SETPOS, &raw mut data) } == 0 {
            self.reservation.cancel_ignored_position_change();
            return Err(ShellIntegrationError::AppBarRejected("ABM_SETPOS"));
        }
        let negotiated = queried.with_shell_bounds(to_screen_rect(data.rc))?;
        self.reservation.move_to(negotiated.reserved_rect())?;
        Ok(negotiated)
    }

    fn refresh_position(
        &self,
        dock: &DockWindow,
        settings: &DockSettings,
    ) -> Result<(), ShellIntegrationError> {
        let layout = requested_layout(dock, settings)?;
        let negotiated = self.reserve(layout)?;
        dock.apply_appbar_layout(negotiated, settings)?;
        Ok(())
    }

    fn release_for_recovery(
        &mut self,
        source: ShellRecoverySource,
    ) -> Result<(), ShellIntegrationError> {
        if source == ShellRecoverySource::TaskbarCreated {
            self.registered = false;
            return Ok(());
        }
        self.remove()
    }

    fn remove(&mut self) -> Result<(), ShellIntegrationError> {
        if !self.registered {
            return Ok(());
        }
        let mut data = appbar_data(self.reservation.hwnd());
        if unsafe { SHAppBarMessage(ABM_REMOVE, &raw mut data) } == 0 {
            return Err(ShellIntegrationError::AppBarRejected("ABM_REMOVE"));
        }
        self.registered = false;
        Ok(())
    }
}

struct ReservationCallback {
    message: u32,
    thread_id: u32,
    ignore_position_change: AtomicBool,
}

struct ReservationWindow {
    hwnd: HWND,
    callback: Box<ReservationCallback>,
}

impl ReservationWindow {
    fn create(callback_message: u32) -> Result<Self, windows::core::Error> {
        let extended_style = WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0);
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
        let thread_id = unsafe { GetCurrentThreadId() };
        let callback = Box::new(ReservationCallback {
            message: callback_message,
            thread_id,
            ignore_position_change: AtomicBool::new(false),
        });
        let callback_pointer = std::ptr::from_ref(callback.as_ref()).addr();
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
            let _ = unsafe { DestroyWindow(hwnd) };
            return Err(windows::core::Error::from_thread());
        }
        Ok(Self { hwnd, callback })
    }

    const fn hwnd(&self) -> HWND {
        self.hwnd
    }

    fn ignore_next_position_change(&self) {
        self.callback
            .ignore_position_change
            .store(true, Ordering::Release);
    }

    fn cancel_ignored_position_change(&self) {
        self.callback
            .ignore_position_change
            .store(false, Ordering::Release);
    }

    fn move_to(&self, rect: ScreenRect) -> Result<(), windows::core::Error> {
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
        let _ = unsafe {
            RemoveWindowSubclass(
                self.hwnd,
                Some(reservation_subclass_proc),
                RESERVATION_SUBCLASS_ID,
            )
        };
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
    let callback = unsafe { &*callback };
    if message != callback.message {
        return unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    }
    if wparam.0 == usize::try_from(ABN_FULLSCREENAPP).unwrap_or(2) {
        let fullscreen = usize::from(lparam.0 != 0);
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
    if wparam.0 == usize::try_from(ABN_POSCHANGED).unwrap_or(1) {
        if callback
            .ignore_position_change
            .swap(false, Ordering::AcqRel)
        {
            return LRESULT(0);
        }
        queue_appbar_position_recovery(hwnd);
        return LRESULT(0);
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

impl Drop for AppBarController {
    fn drop(&mut self) {
        let _ = self.remove();
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
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY) };
    let mut info = MONITORINFO {
        cbSize: monitor_info_size(),
        ..MONITORINFO::default()
    };
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
