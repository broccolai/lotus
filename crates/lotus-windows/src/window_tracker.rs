mod enumeration;
mod events;
mod foreground;

use lotus_core::window::{WindowId, WindowInfo};
use windows::Win32::Foundation::E_FAIL;
use windows::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::{EVENT_SYSTEM_FOREGROUND, WM_TIMER};
use windows::core::Error;

pub(crate) use self::enumeration::process_image_path;
use self::events::{
    OwnedWinEventHook, RECONCILE_INTERVAL_MS, REFRESH_DELAY_MS, REFRESH_MESSAGE,
};
use crate::{NativeError, WindowHandle};

pub struct WindowTracker {
    hooks: Vec<OwnedWinEventHook>,
    windows: Vec<WindowInfo>,
    own_process_id: u32,
    timer_id: Option<usize>,
    reconcile_timer_id: usize,
    fullscreen_window: Option<WindowId>,
    shell_fullscreen_window: Option<WindowId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowTrackerEvent {
    SnapshotRefreshed,
    FullscreenRefreshed,
}

impl WindowTracker {
    pub fn start() -> Result<Self, NativeError> {
        let (own_process_id, thread_id) =
            unsafe { (GetCurrentProcessId(), GetCurrentThreadId()) };
        if !events::claim_callback_thread(thread_id) {
            return Err(
                Error::new(E_FAIL, "a Lotus window tracker is already active").into(),
            );
        }
        let hooks = match events::install_hooks() {
            Ok(hooks) => hooks,
            Err(error) => {
                events::release_callback_thread();
                return Err(error);
            }
        };

        let mut tracker = Self {
            hooks,
            windows: Vec::new(),
            own_process_id,
            timer_id: None,
            reconcile_timer_id: 0,
            fullscreen_window: None,
            shell_fullscreen_window: None,
        };
        tracker.refresh()?;
        tracker.reconcile_timer_id = events::create_thread_timer(RECONCILE_INTERVAL_MS)?;
        Ok(tracker)
    }

    pub fn current_windows(&self) -> &[WindowInfo] {
        &self.windows
    }

    pub const fn fullscreen_window(&self) -> Option<WindowId> {
        match self.shell_fullscreen_window {
            Some(window) => Some(window),
            None => self.fullscreen_window,
        }
    }

    pub fn fullscreen_on_same_monitor(&self, window: WindowHandle) -> bool {
        [self.shell_fullscreen_window, self.fullscreen_window]
            .into_iter()
            .flatten()
            .filter_map(foreground::hwnd_from_window_id)
            .any(|fullscreen| foreground::same_monitor(window.raw(), fullscreen))
    }

    pub fn set_shell_fullscreen(&mut self, fullscreen: bool) {
        if fullscreen {
            self.shell_fullscreen_window =
                foreground::observe_foreground_window(self.own_process_id);
            if self.shell_fullscreen_window.is_none() {
                self.fullscreen_window =
                    foreground::observe_fullscreen_window(self.own_process_id);
            }
            return;
        }
        let ended_window = self.shell_fullscreen_window.take();
        let foreground = foreground::observe_foreground_window(self.own_process_id);
        self.fullscreen_window = if foreground == ended_window {
            None
        } else {
            foreground::observe_fullscreen_window(self.own_process_id)
        };
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
                let foreground = parameter == EVENT_SYSTEM_FOREGROUND as usize;
                if !foreground {
                    events::clear_deferred_notification();
                }
                self.restart_timer()?;
                if foreground {
                    self.refresh_fullscreen();
                    Ok(Some(WindowTrackerEvent::FullscreenRefreshed))
                } else {
                    Ok(None)
                }
            }
            WM_TIMER if self.timer_id == Some(parameter) => {
                self.cancel_timer();
                Ok(self
                    .refresh_if_changed()?
                    .then_some(WindowTrackerEvent::SnapshotRefreshed))
            }
            WM_TIMER if self.reconcile_timer_id == parameter => Ok(self
                .refresh_if_changed()?
                .then_some(WindowTrackerEvent::SnapshotRefreshed)),
            _ => Ok(None),
        }
    }

    fn restart_timer(&mut self) -> Result<(), NativeError> {
        self.cancel_timer();
        self.timer_id = Some(events::create_thread_timer(REFRESH_DELAY_MS)?);
        Ok(())
    }

    fn cancel_timer(&mut self) {
        if let Some(timer_id) = self.timer_id.take() {
            events::cancel_thread_timer(timer_id);
        }
    }

    fn refresh(&mut self) -> Result<(), NativeError> {
        self.windows = enumeration::enumerate_windows(self.own_process_id)?;
        self.refresh_fullscreen();
        Ok(())
    }

    pub fn refresh_fullscreen(&mut self) {
        self.validate_shell_fullscreen();
        self.fullscreen_window = foreground::observe_fullscreen_window(self.own_process_id);
    }

    fn refresh_if_changed(&mut self) -> Result<bool, NativeError> {
        let windows = enumeration::enumerate_windows(self.own_process_id)?;
        let fullscreen_window = foreground::observe_fullscreen_window(self.own_process_id);
        let previous_shell_fullscreen = self.shell_fullscreen_window;
        self.validate_shell_fullscreen();
        if same_window_snapshot(&self.windows, &windows)
            && self.fullscreen_window == fullscreen_window
            && self.shell_fullscreen_window == previous_shell_fullscreen
        {
            return Ok(false);
        }
        self.windows = windows;
        self.fullscreen_window = fullscreen_window;
        Ok(true)
    }

    fn validate_shell_fullscreen(&mut self) {
        if self
            .shell_fullscreen_window
            .is_some_and(|window| !foreground::is_fullscreen_window(window))
        {
            self.shell_fullscreen_window = None;
        }
    }
}

fn same_window_snapshot(previous: &[WindowInfo], current: &[WindowInfo]) -> bool {
    previous.len() == current.len()
        && previous
            .iter()
            .all(|window| current.iter().any(|candidate| candidate == window))
}

impl Drop for WindowTracker {
    fn drop(&mut self) {
        events::release_callback_thread();
        self.cancel_timer();
        if self.reconcile_timer_id != 0 {
            events::cancel_thread_timer(self.reconcile_timer_id);
        }
        self.hooks.clear();
    }
}
