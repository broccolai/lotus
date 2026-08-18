use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, SystemTime};

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};

use super::observer::TaskbarEventObserver;
use crate::exclusive_taskbar::ExclusiveTaskbarError;
use crate::taskbar_state::TaskbarStateGuard;

pub(super) const GUARDIAN_ARGUMENT: &str = "--lotus-taskbar-guardian";
pub(super) const READY_FILE: &str = "ready";
pub(super) const STOP_FILE: &str = "stop";
pub(super) const START_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL_MILLISECONDS: u32 = 100;

pub(super) fn spawn(
    parent_process_id: u32,
    control_directory: &Path,
) -> Result<Child, std::io::Error> {
    Command::new(std::env::current_exe()?)
        .arg(GUARDIAN_ARGUMENT)
        .arg(parent_process_id.to_string())
        .arg(control_directory)
        .spawn()
}

pub(super) fn request<I, S>(
    arguments: I,
) -> Result<Option<(u32, PathBuf)>, ExclusiveTaskbarError>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut arguments = arguments.into_iter().map(Into::into);
    let Some(first) = arguments.next() else {
        return Ok(None);
    };
    if !argument_eq(&first, GUARDIAN_ARGUMENT) {
        return Ok(None);
    }
    let process_id = arguments
        .next()
        .and_then(|value| value.to_str().and_then(|value| value.parse::<u32>().ok()))
        .filter(|value| *value != 0)
        .ok_or(ExclusiveTaskbarError::InvalidGuardianArguments)?;
    let directory = arguments
        .next()
        .map(PathBuf::from)
        .ok_or(ExclusiveTaskbarError::InvalidGuardianArguments)?;
    if arguments.next().is_some() {
        return Err(ExclusiveTaskbarError::InvalidGuardianArguments);
    }

    Ok(Some((process_id, directory)))
}

pub(super) fn run(
    parent_process_id: u32,
    control_directory: &Path,
) -> Result<(), ExclusiveTaskbarError> {
    let parent = ProcessHandle::open(parent_process_id)?;
    let mut taskbar_state = TaskbarStateGuard::enable_autohide()?;
    let event_observer = TaskbarEventObserver::start()?;
    fs::write(control_directory.join(READY_FILE), [])?;

    loop {
        // SAFETY: `parent` owns a live synchronization handle and the bounded timeout
        // keeps the guardian responsive to cancellation and taskbar recreation.
        match unsafe { WaitForSingleObject(parent.0, POLL_INTERVAL_MILLISECONDS) } {
            WAIT_OBJECT_0 => break,
            WAIT_TIMEOUT => {
                if control_directory.join(STOP_FILE).exists() {
                    break;
                }
                if event_observer.is_finished() {
                    return Err(ExclusiveTaskbarError::EventObserverStopped);
                }
            }
            _ => return Err(ExclusiveTaskbarError::ParentWait),
        }
    }

    drop(event_observer);
    let _ = taskbar_state.restore();
    cleanup_control_directory(control_directory);
    Ok(())
}

pub(super) fn control_directory() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    std::env::temp_dir().join(format!("lotus-taskbar-{}-{nonce}", std::process::id()))
}

pub(super) fn cleanup_control_directory(directory: &Path) {
    let _ = fs::remove_file(directory.join(READY_FILE));
    let _ = fs::remove_file(directory.join(STOP_FILE));
    let _ = fs::remove_dir(directory);
}

struct ProcessHandle(HANDLE);

impl ProcessHandle {
    fn open(process_id: u32) -> Result<Self, ExclusiveTaskbarError> {
        // SAFETY: The identifier is validated as nonzero and the requested right permits
        // waiting only; ownership of the returned handle transfers to this guard.
        unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, process_id) }
            .map(Self)
            .map_err(|error| ExclusiveTaskbarError::ParentProcess(error.into()))
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        // SAFETY: This guard owns the successful `OpenProcess` result exactly once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn argument_eq(argument: &OsStr, expected: &str) -> bool {
    argument
        .to_str()
        .is_some_and(|value| value.eq_ignore_ascii_case(expected))
}
