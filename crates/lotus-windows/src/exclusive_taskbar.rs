//! Optional, fail-open replacement of only the native taskbar windows.

mod guardian;
mod observer;
mod taskbar_windows;
mod visibility_transaction;

use std::path::PathBuf;
use std::process::Child;
use std::sync::mpsc::{self, TryRecvError};
use std::time::{Duration, Instant};
use std::{fs, thread};

use guardian::{READY_FILE, REFRESH_FILE, START_TIMEOUT};
use thiserror::Error;

use super::taskbar_state::TaskbarStateError;
use crate::NativeError;

#[derive(Debug, Error)]
pub enum ExclusiveTaskbarError {
    #[error("exclusive taskbar mode could not access its recovery directory: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    TaskbarState(#[from] TaskbarStateError),
    #[error("invalid exclusive-taskbar guardian arguments")]
    InvalidGuardianArguments,
    #[error("the exclusive-taskbar guardian stopped before it became ready")]
    GuardianStopped,
    #[error("the exclusive-taskbar guardian did not become ready within five seconds")]
    GuardianTimedOut,
    #[error("the exclusive-taskbar guardian could not monitor Lotus: {0}")]
    ParentProcess(NativeError),
    #[error("the exclusive-taskbar guardian wait failed")]
    ParentWait,
    #[error("the exclusive-taskbar event observer failed: {0}")]
    EventObserver(NativeError),
    #[error("the exclusive-taskbar event observer stopped unexpectedly")]
    EventObserverStopped,
}

/// Owns the guardian that restores the taskbar if Lotus exits unexpectedly.
pub struct ExclusiveTaskbarGuard {
    child: Child,
    control_directory: PathBuf,
    cancellation: guardian::CancellationEvent,
    progress: guardian::ProgressEvent,
}

// SAFETY: this guard contains process/event handles and value snapshots only. Its methods never
// access a Lotus HWND; UI-affine AppBar and placement work remains in the caller.
unsafe impl Send for ExclusiveTaskbarGuard {}

/// External guardian launch/readiness coordination. Poll it from the UI without waiting; the
/// worker owns filesystem polling and child-process waits until it can return a guard.
pub struct ExclusiveTaskbarStart {
    completion: mpsc::Receiver<Result<ExclusiveTaskbarGuard, ExclusiveTaskbarError>>,
}

impl ExclusiveTaskbarGuard {
    pub fn start_async() -> Result<ExclusiveTaskbarStart, ExclusiveTaskbarError> {
        let (sender, completion) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("lotus-taskbar-guardian-start".into())
            .spawn(move || {
                let _ = sender.send(Self::start());
            })
            .map_err(ExclusiveTaskbarError::Io)?;
        Ok(ExclusiveTaskbarStart { completion })
    }

    fn start() -> Result<Self, ExclusiveTaskbarError> {
        let control_directory = guardian::control_directory();
        fs::create_dir(&control_directory)?;
        let cancellation = match guardian::CancellationEvent::create() {
            Ok(cancellation) => cancellation,
            Err(error) => {
                guardian::cleanup_control_directory(&control_directory);
                return Err(error);
            }
        };
        let progress = match guardian::ProgressEvent::create() {
            Ok(progress) => progress,
            Err(error) => {
                guardian::cleanup_control_directory(&control_directory);
                return Err(error);
            }
        };
        let mut child = match guardian::spawn(
            std::process::id(),
            &control_directory,
            cancellation.name(),
            progress.name(),
        ) {
            Ok(child) => child,
            Err(error) => {
                guardian::cleanup_control_directory(&control_directory);
                return Err(error.into());
            }
        };

        let started = Instant::now();
        loop {
            if child.try_wait()?.is_some() {
                guardian::cleanup_control_directory(&control_directory);
                return Err(ExclusiveTaskbarError::GuardianStopped);
            }
            if control_directory.join(READY_FILE).is_file() {
                return Ok(Self {
                    child,
                    control_directory,
                    cancellation,
                    progress,
                });
            }
            if started.elapsed() >= START_TIMEOUT {
                if stop_guardian(&mut child, &cancellation) {
                    guardian::cleanup_control_directory(&control_directory);
                }
                return Err(ExclusiveTaskbarError::GuardianTimedOut);
            }

            thread::sleep(Duration::from_millis(25));
        }
    }

    pub fn reassert_hidden(&mut self) -> Result<(), ExclusiveTaskbarError> {
        if self.child.try_wait()?.is_some() {
            return Err(ExclusiveTaskbarError::GuardianStopped);
        }

        fs::write(self.control_directory.join(REFRESH_FILE), [])?;
        Ok(())
    }

    /// Records actual UI-owned progress. The guardian will not hide taskbars until this
    /// permission is observed, and it restores them when fresh UI progress expires.
    pub fn report_ui_progress(&self) {
        self.progress.report();
    }

    /// Revokes takeover before any native restoration path. This is intentionally the same
    /// cooperative signal used for shutdown: a stale session must not resume hiding taskbars.
    pub fn revoke_takeover(&self) {
        self.cancellation.signal();
    }

    pub fn is_alive(&mut self) -> Result<bool, ExclusiveTaskbarError> {
        Ok(self.child.try_wait()?.is_none())
    }
}

impl ExclusiveTaskbarStart {
    /// Returns a completed launch without sleeping or joining. A disconnected worker is a
    /// fail-open error rather than a reason to wait on the UI thread.
    pub fn try_complete(
        &self,
    ) -> Option<Result<ExclusiveTaskbarGuard, ExclusiveTaskbarError>> {
        match self.completion.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err(ExclusiveTaskbarError::GuardianStopped))
            }
        }
    }
}

impl Drop for ExclusiveTaskbarGuard {
    fn drop(&mut self) {
        self.revoke_takeover();
        crate::diagnostics::record_state(
            "exclusive_taskbar.guardian_owner_shutdown",
            &[("guardian_stop_requested", 1)],
        );
    }
}

/// Restores only taskbar HWNDs that still prove their Explorer ownership and class identity.
pub fn restore_verified_taskbars() {
    taskbar_windows::restore_verified_taskbars();
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> std::io::Result<bool> {
    let started = Instant::now();
    while child.try_wait()?.is_none() {
        if started.elapsed() >= timeout {
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(25));
    }
    Ok(true)
}

fn stop_guardian(child: &mut Child, cancellation: &guardian::CancellationEvent) -> bool {
    cancellation.signal();
    if wait_for_exit(child, START_TIMEOUT).unwrap_or(false) {
        return true;
    }

    // Cooperative shutdown exhausted its deadline. Stop only this owned child before
    // restoring taskbars; a surviving observer could otherwise hide them again.
    if let Err(error) = child.kill() {
        crate::diagnostics::record_error("exclusive_taskbar.guardian_stop_failed", &error);
    }
    let stopped = wait_for_exit(child, Duration::from_secs(1)).unwrap_or(false);
    crate::diagnostics::record_state(
        "exclusive_taskbar.guardian_forced_stop",
        &[("stopped", u64::from(stopped))],
    );
    stopped
}

/// Runs the recovery guardian instead of the normal application when requested.
pub fn run_guardian_if_requested() -> Result<bool, ExclusiveTaskbarError> {
    let request = guardian::request(std::env::args_os().skip(1))?;
    let Some((parent_process_id, control_directory, cancellation_event, progress_event)) =
        request
    else {
        return Ok(false);
    };

    guardian::run(
        parent_process_id,
        &control_directory,
        &cancellation_event,
        &progress_event,
    )?;
    Ok(true)
}
