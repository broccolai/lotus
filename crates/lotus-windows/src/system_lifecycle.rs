use windows::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification,
    WTSUnRegisterSessionNotification,
};
use windows::Win32::UI::WindowsAndMessaging::{
    PBT_APMRESUMEAUTOMATIC, PBT_APMSUSPEND, SPI_SETWORKAREA, WM_DISPLAYCHANGE,
    WM_DPICHANGED, WM_POWERBROADCAST, WM_SETTINGCHANGE, WM_WTSSESSION_CHANGE,
    WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
};

use crate::WindowHandle;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemLifecycleHealth {
    Healthy,
    Degraded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemLifecycleEvent {
    DisplayChanged,
    DpiChanged,
    WorkAreaChanged,
    Suspending,
    Resumed,
    SessionLocked,
    SessionUnlocked,
}

impl SystemLifecycleEvent {
    pub const fn diagnostic_name(self) -> &'static str {
        match self {
            Self::DisplayChanged => "display_changed",
            Self::DpiChanged => "dpi_changed",
            Self::WorkAreaChanged => "work_area_changed",
            Self::Suspending => "suspending",
            Self::Resumed => "resumed",
            Self::SessionLocked => "session_locked",
            Self::SessionUnlocked => "session_unlocked",
        }
    }
}

pub struct SystemLifecycleObserver {
    window: WindowHandle,
    session_registered: bool,
}

impl SystemLifecycleObserver {
    pub fn register(window: WindowHandle) -> Self {
        let session_registered = unsafe {
            WTSRegisterSessionNotification(window.raw(), NOTIFY_FOR_THIS_SESSION)
        }
        .is_ok();
        if session_registered {
            crate::diagnostics::record_diagnostic(
                "system_lifecycle.registration",
                "session_notifications=healthy",
            );
        } else {
            crate::diagnostics::record_diagnostic(
                "system_lifecycle.registration",
                "session_notifications=degraded",
            );
        }

        Self {
            window,
            session_registered,
        }
    }

    pub const fn health(&self) -> SystemLifecycleHealth {
        if self.session_registered {
            SystemLifecycleHealth::Healthy
        } else {
            SystemLifecycleHealth::Degraded
        }
    }

    pub fn recover_registration(&mut self) {
        if self.session_registered {
            return;
        }
        self.session_registered = unsafe {
            WTSRegisterSessionNotification(self.window.raw(), NOTIFY_FOR_THIS_SESSION)
        }
        .is_ok();
        crate::diagnostics::record_diagnostic(
            "system_lifecycle.registration_recovery",
            if self.session_registered {
                "session_notifications=healthy"
            } else {
                "session_notifications=degraded"
            },
        );
    }

    pub const fn classify(
        &self,
        message: u32,
        parameter: usize,
    ) -> Option<SystemLifecycleEvent> {
        match message {
            WM_DISPLAYCHANGE => Some(SystemLifecycleEvent::DisplayChanged),
            WM_DPICHANGED => Some(SystemLifecycleEvent::DpiChanged),
            WM_SETTINGCHANGE if parameter == SPI_SETWORKAREA.0 as usize => {
                Some(SystemLifecycleEvent::WorkAreaChanged)
            }
            WM_POWERBROADCAST if parameter == PBT_APMSUSPEND as usize => {
                Some(SystemLifecycleEvent::Suspending)
            }
            WM_POWERBROADCAST if parameter == PBT_APMRESUMEAUTOMATIC as usize => {
                Some(SystemLifecycleEvent::Resumed)
            }
            WM_WTSSESSION_CHANGE if parameter == WTS_SESSION_LOCK as usize => {
                Some(SystemLifecycleEvent::SessionLocked)
            }
            WM_WTSSESSION_CHANGE if parameter == WTS_SESSION_UNLOCK as usize => {
                Some(SystemLifecycleEvent::SessionUnlocked)
            }
            _ => None,
        }
    }
}

impl Drop for SystemLifecycleObserver {
    fn drop(&mut self) {
        if self.session_registered {
            let _ = unsafe { WTSUnRegisterSessionNotification(self.window.raw()) };
        }
    }
}
