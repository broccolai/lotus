//! Optional, fail-open replacement of only the native taskbar windows.

mod guardian;
mod observer;
mod taskbar_windows;
mod visibility_transaction;

use std::path::PathBuf;
use std::process::Child;
use std::time::{Duration, Instant};
use std::{fs, thread};

use guardian::{READY_FILE, REFRESH_FILE, START_TIMEOUT};
use thiserror::Error;

use super::taskbar_state::{TaskbarStateError, TaskbarStateSnapshot};
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
    taskbar_baseline: TaskbarStateSnapshot,
}

impl ExclusiveTaskbarGuard {
    pub fn start() -> Result<Self, ExclusiveTaskbarError> {
        let control_directory = guardian::control_directory();
        fs::create_dir(&control_directory)?;
        let taskbar_baseline = TaskbarStateSnapshot::capture()?;
        let cancellation = match guardian::CancellationEvent::create() {
            Ok(cancellation) => cancellation,
            Err(error) => {
                guardian::cleanup_control_directory(&control_directory);
                return Err(error);
            }
        };
        let mut child = match guardian::spawn(
            std::process::id(),
            &control_directory,
            cancellation.name(),
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
                restore_verified_taskbars();
                let _ = taskbar_baseline.restore_exclusive_fallback();
                return Err(ExclusiveTaskbarError::GuardianStopped);
            }
            if control_directory.join(READY_FILE).is_file() {
                return Ok(Self {
                    child,
                    control_directory,
                    cancellation,
                    taskbar_baseline,
                });
            }
            if started.elapsed() >= START_TIMEOUT {
                if stop_guardian(&mut child, &cancellation) {
                    guardian::cleanup_control_directory(&control_directory);
                }
                restore_verified_taskbars();
                let _ = taskbar_baseline.restore_exclusive_fallback();
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

    pub fn is_alive(&mut self) -> Result<bool, ExclusiveTaskbarError> {
        Ok(self.child.try_wait()?.is_none())
    }
}

impl Drop for ExclusiveTaskbarGuard {
    fn drop(&mut self) {
        let stopped = stop_guardian(&mut self.child, &self.cancellation);
        restore_verified_taskbars();
        let fallback_restored = self
            .taskbar_baseline
            .restore_exclusive_fallback()
            .unwrap_or(false);
        crate::diagnostics::record_state(
            "exclusive_taskbar.guardian_owner_shutdown",
            &[
                ("guardian_stopped", u64::from(stopped)),
                (
                    "taskbar_state_fallback_restored",
                    u64::from(fallback_restored),
                ),
            ],
        );
        if stopped {
            guardian::cleanup_control_directory(&self.control_directory);
        }
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
    let Some((parent_process_id, control_directory, cancellation_event)) = request else {
        return Ok(false);
    };

    guardian::run(parent_process_id, &control_directory, &cancellation_event)?;
    Ok(true)
}
