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
    #[error(
        "Lotus is running from the temporary path `{path}` and will not register it for Windows startup"
    )]
    TransientExecutable { path: PathBuf },
    #[error("Windows could not update the Lotus startup entry: {0}")]
    Registry(String),
}

impl From<windows_result::Error> for StartupRegistrationError {
    fn from(error: windows_result::Error) -> Self {
        Self::Registry(error.to_string())
    }
}

pub fn sync(enabled: bool) -> Result<(), StartupRegistrationError> {
    if enabled {
        let executable = match stable_startup_executable() {
            Ok(executable) => executable,
            Err(error @ StartupRegistrationError::TransientExecutable { .. }) => {
                remove_transient_startup_registration()?;
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        let key = windows_registry::CURRENT_USER.create(RUN_KEY)?;
        key.set_string(VALUE_NAME, startup_command(&executable))?;
    } else {
        let key = windows_registry::CURRENT_USER.create(RUN_KEY)?;
        if let Err(error) = key.remove_value(VALUE_NAME) {
            const ERROR_FILE_NOT_FOUND: u32 = 2;
            if error.code() != HRESULT::from_win32(ERROR_FILE_NOT_FOUND) {
                return Err(error.into());
            }
        }
    }
    Ok(())
}

fn stable_startup_executable() -> Result<PathBuf, StartupRegistrationError> {
    let current = std::env::current_exe()?;
    if !is_temporary_path(&current) {
        return Ok(current);
    }

    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        let installed = PathBuf::from(local_app_data)
            .join("Programs")
            .join("Lotus")
            .join("lotus.exe");
        if installed.is_file() && !is_temporary_path(&installed) {
            return Ok(installed);
        }
    }

    if let Some(registered) = registered_startup_executable()
        && registered.is_file()
        && !is_temporary_path(&registered)
    {
        return Ok(registered);
    }

    Err(StartupRegistrationError::TransientExecutable { path: current })
}

fn remove_transient_startup_registration() -> Result<(), StartupRegistrationError> {
    let Ok(key) = windows_registry::CURRENT_USER.open(RUN_KEY) else {
        return Ok(());
    };
    if registered_startup_executable_from(&key).is_some_and(|path| is_temporary_path(&path))
    {
        key.remove_value(VALUE_NAME)?;
    }
    Ok(())
}

fn registered_startup_executable() -> Option<PathBuf> {
    let key = windows_registry::CURRENT_USER.open(RUN_KEY).ok()?;
    registered_startup_executable_from(&key)
}

fn registered_startup_executable_from(key: &windows_registry::Key) -> Option<PathBuf> {
    let command = key.get_string(VALUE_NAME).ok()?;
    let command = command.trim();
    let path = command
        .strip_prefix('"')
        .and_then(|command| command.strip_suffix('"'))
        .unwrap_or(command);
    let path = PathBuf::from(path);
    path.file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("lotus.exe"))
        .then_some(path)
}

fn is_temporary_path(path: &Path) -> bool {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_owned());
    let temporary = std::env::temp_dir();
    let temporary = temporary.canonicalize().unwrap_or(temporary);
    path.starts_with(temporary)
}

fn startup_command(executable: &Path) -> String {
    format!(r#""{}""#, executable.display())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum StartupMode {
    #[default]
    Standard,
    Development,
}

impl StartupMode {
    pub const fn is_development(self) -> bool {
        matches!(self, Self::Development)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StartupOptions {
    pub mode: StartupMode,
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
    #[error("--development cannot be used with {argument}")]
    DevelopmentWithInstalledUpdateArgument { argument: &'static str },
}

pub fn parse_startup_args<I, S>(arguments: I) -> Result<StartupOptions, StartupArgsError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let arguments = arguments.into_iter().collect::<Vec<_>>();
    let mut restart_after = None;
    let mut mode = StartupMode::Standard;
    let mut open_settings = false;
    let mut cleanup_update = None;
    let mut post_install_health = false;
    let mut index = 0;

    while index < arguments.len() {
        let argument = arguments[index].as_ref();
        if argument_eq(argument, "--development") {
            mode = StartupMode::Development;
        } else if argument_eq(argument, "--restart-after") {
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

    if mode == StartupMode::Development {
        if cleanup_update.is_some() {
            return Err(StartupArgsError::DevelopmentWithInstalledUpdateArgument {
                argument: "--cleanup-update",
            });
        }
        if post_install_health {
            return Err(StartupArgsError::DevelopmentWithInstalledUpdateArgument {
                argument: "--post-install-health",
            });
        }
    }

    Ok(StartupOptions {
        mode,
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
