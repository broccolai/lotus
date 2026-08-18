//! Optional, fail-open replacement of only the native taskbar windows.

mod guardian;
mod observer;
mod taskbar_windows;
mod visibility_transaction;

use std::path::PathBuf;
use std::process::Child;
use std::time::{Duration, Instant};
use std::{fs, thread};

use guardian::{READY_FILE, START_TIMEOUT, STOP_FILE};
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
}

impl ExclusiveTaskbarGuard {
    pub fn start() -> Result<Self, ExclusiveTaskbarError> {
        let control_directory = guardian::control_directory();
        fs::create_dir(&control_directory)?;
        let mut child = guardian::spawn(std::process::id(), &control_directory)?;

        let started = Instant::now();
        loop {
            if control_directory.join(READY_FILE).is_file() {
                return Ok(Self {
                    child,
                    control_directory,
                });
            }
            if child.try_wait()?.is_some() {
                guardian::cleanup_control_directory(&control_directory);
                return Err(ExclusiveTaskbarError::GuardianStopped);
            }
            if started.elapsed() >= START_TIMEOUT {
                let _ = fs::write(control_directory.join(STOP_FILE), []);
                let _ = child.wait();
                guardian::cleanup_control_directory(&control_directory);
                return Err(ExclusiveTaskbarError::GuardianTimedOut);
            }

            thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for ExclusiveTaskbarGuard {
    fn drop(&mut self) {
        let _ = fs::write(self.control_directory.join(STOP_FILE), []);
        let _ = self.child.wait();
        guardian::cleanup_control_directory(&self.control_directory);
    }
}

/// Runs the recovery guardian instead of the normal application when requested.
pub fn run_guardian_if_requested() -> bool {
    let Ok(request) = guardian::request(std::env::args_os().skip(1)) else {
        return true;
    };
    let Some((parent_process_id, control_directory)) = request else {
        return false;
    };

    let _ = guardian::run(parent_process_id, &control_directory);
    true
}
