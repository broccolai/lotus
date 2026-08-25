use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use thiserror::Error;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_INVALID_PARAMETER, HANDLE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};
use windows::core::HRESULT;

use crate::NativeError;

const VALUE_NAME: &str = "Lotus";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RESTART_WAIT_MILLISECONDS: u32 = 5_000;

#[derive(Debug, Error)]
pub enum StartupRegistrationError {
    #[error("Lotus could not locate its executable for Windows startup: {0}")]
    CurrentExecutable(#[from] std::io::Error),
    #[error("Windows could not update the Lotus startup entry: {0}")]
    Registry(String),
}

impl From<windows_result::Error> for StartupRegistrationError {
    fn from(error: windows_result::Error) -> Self {
        Self::Registry(error.to_string())
    }
}

pub fn sync(enabled: bool) -> Result<(), StartupRegistrationError> {
    let key = windows_registry::CURRENT_USER.create(RUN_KEY)?;
    if enabled {
        let executable = std::env::current_exe()?;
        key.set_string(VALUE_NAME, startup_command(&executable))?;
    } else if let Err(error) = key.remove_value(VALUE_NAME) {
        const ERROR_FILE_NOT_FOUND: u32 = 2;
        if error.code() != HRESULT::from_win32(ERROR_FILE_NOT_FOUND) {
            return Err(error.into());
        }
    }
    Ok(())
}

fn startup_command(executable: &Path) -> String {
    format!(r#""{}""#, executable.display())
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StartupOptions {
    pub restart_after: Option<u32>,
    pub open_settings: bool,
    pub cleanup_update: Option<PathBuf>,
    pub post_install_health: bool,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum StartupArgsError {
    #[error("--restart-after requires a positive process identifier")]
    MissingRestartProcess,
    #[error("invalid --restart-after process identifier `{0}`")]
    InvalidRestartProcess(String),
    #[error("conflicting --restart-after process identifiers {first} and {second}")]
    ConflictingRestartProcesses { first: u32, second: u32 },
    #[error("--cleanup-update requires a staging directory")]
    MissingCleanupDirectory,
}

pub fn parse_startup_args<I, S>(arguments: I) -> Result<StartupOptions, StartupArgsError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let arguments = arguments.into_iter().collect::<Vec<_>>();
    let mut restart_after = None;
    let mut open_settings = false;
    let mut cleanup_update = None;
    let mut post_install_health = false;
    let mut index = 0;

    while index < arguments.len() {
        let argument = arguments[index].as_ref();
        if argument_eq(argument, "--restart-after") {
            index += 1;
            let value = arguments
                .get(index)
                .ok_or(StartupArgsError::MissingRestartProcess)?
                .as_ref();
            let process_id = parse_process_id(value)?;
            if let Some(first) = restart_after
                && first != process_id
            {
                return Err(StartupArgsError::ConflictingRestartProcesses {
                    first,
                    second: process_id,
                });
            }
            restart_after = Some(process_id);
        } else if argument_eq(argument, "--open-settings") {
            open_settings = true;
        } else if argument_eq(argument, "--cleanup-update") {
            index += 1;
            cleanup_update = Some(PathBuf::from(
                arguments
                    .get(index)
                    .ok_or(StartupArgsError::MissingCleanupDirectory)?
                    .as_ref(),
            ));
        } else if argument_eq(argument, "--post-install-health") {
            post_install_health = true;
        }
        index += 1;
    }

    Ok(StartupOptions {
        restart_after,
        open_settings,
        cleanup_update,
        post_install_health,
    })
}

fn argument_eq(argument: &OsStr, expected: &str) -> bool {
    argument
        .to_str()
        .is_some_and(|argument| argument.eq_ignore_ascii_case(expected))
}

fn parse_process_id(value: &OsStr) -> Result<u32, StartupArgsError> {
    let display = value.to_string_lossy().into_owned();
    match display.parse::<u32>() {
        Ok(process_id) if process_id > 0 => Ok(process_id),
        _ => Err(StartupArgsError::InvalidRestartProcess(display)),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestartWaitOutcome {
    NotRequested,
    CurrentProcessIgnored,
    AlreadyExited,
    Exited,
    TimedOut,
}

#[derive(Debug, Error)]
pub enum RestartWaitError {
    #[error("could not open restart source process {process_id}: {source}")]
    OpenProcess {
        process_id: u32,
        source: NativeError,
    },
    #[error("waiting for restart source process {process_id} failed: {source}")]
    Wait {
        process_id: u32,
        source: NativeError,
    },
    #[error(
        "restart source process {process_id} returned an unexpected wait status {status}"
    )]
    UnexpectedStatus { process_id: u32, status: u32 },
}

pub fn wait_for_restart_source(
    process_id: Option<u32>,
) -> Result<RestartWaitOutcome, RestartWaitError> {
    let Some(process_id) = process_id else {
        return Ok(RestartWaitOutcome::NotRequested);
    };
    if process_id == std::process::id() {
        return Ok(RestartWaitOutcome::CurrentProcessIgnored);
    }

    let process = match unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, process_id) } {
        Ok(handle) => OwnedProcess(handle),
        Err(source) if source.code() == HRESULT::from_win32(ERROR_INVALID_PARAMETER.0) => {
            return Ok(RestartWaitOutcome::AlreadyExited);
        }
        Err(source) => {
            return Err(RestartWaitError::OpenProcess {
                process_id,
                source: source.into(),
            });
        }
    };

    let status = unsafe { WaitForSingleObject(process.0, RESTART_WAIT_MILLISECONDS) };
    match status {
        WAIT_OBJECT_0 => Ok(RestartWaitOutcome::Exited),
        WAIT_TIMEOUT => Ok(RestartWaitOutcome::TimedOut),
        WAIT_FAILED => Err(RestartWaitError::Wait {
            process_id,
            source: windows::core::Error::from_thread().into(),
        }),
        status => Err(RestartWaitError::UnexpectedStatus {
            process_id,
            status: status.0,
        }),
    }
}

struct OwnedProcess(HANDLE);

impl Drop for OwnedProcess {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
