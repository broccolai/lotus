use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryIter};

pub use lotus_update::{Release, StagedUpdate, UpdateError, UpdateStatus};
use thiserror::Error;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_APP};
use windows::core::PCWSTR;

use crate::startup::{parse_startup_args, wait_for_restart_source};

const UPDATE_WAKE_MESSAGE: u32 = WM_APP + 0x4C9;
const APPLY_UPDATE_ARGUMENT: &str = "--apply-update";
const CLEANUP_UPDATE_ARGUMENT: &str = "--cleanup-update";

pub enum UpdateResult {
    Checked(Result<UpdateStatus, UpdateError>),
    Staged(Result<StagedUpdate, UpdateError>),
}

pub struct UpdateChecker {
    owner_thread: u32,
    working: Arc<AtomicBool>,
    results: Receiver<UpdateResult>,
    sender: mpsc::Sender<UpdateResult>,
}

impl UpdateChecker {
    pub fn new() -> Self {
        let (sender, results) = mpsc::channel();
        // SAFETY: GetCurrentThreadId has no preconditions and captures the message-loop owner.
        let owner_thread = unsafe { GetCurrentThreadId() };
        Self { owner_thread, working: Arc::new(AtomicBool::new(false)), results, sender }
    }

    pub fn start_check(&self, current_version: &'static str) -> Result<bool, UpdateStartError> {
        self.spawn("lotus-update-check", move || {
            UpdateResult::Checked(lotus_update::check(current_version))
        })
    }

    pub fn start_download(&self, release: Release) -> Result<bool, UpdateStartError> {
        self.spawn("lotus-update-download", move || {
            UpdateResult::Staged(lotus_update::stage(&release))
        })
    }

    pub fn drain(&self) -> TryIter<'_, UpdateResult> {
        self.results.try_iter()
    }

    fn spawn(
        &self,
        name: &'static str,
        work: impl FnOnce() -> UpdateResult + Send + 'static,
    ) -> Result<bool, UpdateStartError> {
        if self.working.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err()
        {
            return Ok(false);
        }
        let sender = self.sender.clone();
        let working = Arc::clone(&self.working);
        let owner_thread = self.owner_thread;
        let spawn = std::thread::Builder::new().name(name.into()).spawn(move || {
            let result = work();
            working.store(false, Ordering::Release);
            if sender.send(result).is_ok() {
                // SAFETY: This private message carries no pointers and targets the captured UI thread.
                let _ = unsafe {
                    PostThreadMessageW(owner_thread, UPDATE_WAKE_MESSAGE, WPARAM(0), LPARAM(0))
                };
            }
        });
        if let Err(source) = spawn {
            self.working.store(false, Ordering::Release);
            return Err(UpdateStartError::Thread(source));
        }
        Ok(true)
    }
}

impl Default for UpdateChecker {
    fn default() -> Self {
        Self::new()
    }
}

pub fn installed_executable() -> Result<PathBuf, UpdateInstallError> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("Programs").join("Lotus").join("lotus.exe"))
        .ok_or(UpdateInstallError::MissingLocalAppData)
}

pub fn is_installed() -> Result<bool, UpdateInstallError> {
    let current = std::env::current_exe().map_err(UpdateInstallError::CurrentExecutable)?;
    Ok(paths_equal(&current, &installed_executable()?))
}

pub fn launch_installer(staged: &StagedUpdate) -> Result<(), UpdateInstallError> {
    launch_helper(&staged.executable)
}

pub fn launch_current_installer() -> Result<(), UpdateInstallError> {
    let current = std::env::current_exe().map_err(UpdateInstallError::CurrentExecutable)?;
    launch_helper(&current)
}

fn launch_helper(executable: &Path) -> Result<(), UpdateInstallError> {
    let target = installed_executable()?;
    Command::new(executable)
        .arg(APPLY_UPDATE_ARGUMENT)
        .arg(&target)
        .arg("--restart-after")
        .arg(std::process::id().to_string())
        .spawn()
        .map_err(UpdateInstallError::LaunchHelper)?;
    Ok(())
}

pub fn run_helper_if_requested() -> Result<bool, UpdateInstallError> {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let Some(index) =
        arguments.iter().position(|argument| argument_eq(argument, APPLY_UPDATE_ARGUMENT))
    else {
        return Ok(false);
    };
    let target =
        arguments.get(index + 1).map(PathBuf::from).ok_or(UpdateInstallError::MissingTarget)?;
    let startup = parse_startup_args(&arguments).map_err(UpdateInstallError::StartupArguments)?;
    wait_for_restart_source(startup.restart_after).map_err(UpdateInstallError::RestartWait)?;
    let source = std::env::current_exe().map_err(UpdateInstallError::CurrentExecutable)?;
    let directory = target.parent().ok_or(UpdateInstallError::InvalidTarget)?.to_owned();
    fs::create_dir_all(&directory).map_err(UpdateInstallError::InstallDirectory)?;
    let temporary = directory.join("lotus.update.exe");
    fs::copy(&source, &temporary).map_err(UpdateInstallError::CopyExecutable)?;
    replace_file(&temporary, &target)?;
    Command::new(&target)
        .arg("--restart-after")
        .arg(std::process::id().to_string())
        .arg(CLEANUP_UPDATE_ARGUMENT)
        .arg(source.parent().ok_or(UpdateInstallError::InvalidSource)?)
        .arg("--open-settings")
        .spawn()
        .map_err(UpdateInstallError::LaunchInstalled)?;
    Ok(true)
}

pub fn cleanup_staging_directory(path: &Path) -> Result<(), UpdateInstallError> {
    let expected_parent = std::env::temp_dir();
    let valid = path.parent().is_some_and(|parent| paths_equal(parent, &expected_parent))
        && path
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with("lotus-update-"));
    if !valid {
        return Err(UpdateInstallError::InvalidCleanupPath(path.to_owned()));
    }
    if path.exists() {
        fs::remove_dir_all(path).map_err(UpdateInstallError::Cleanup)?;
    }
    Ok(())
}

pub const fn is_update_wake(message: u32) -> bool {
    message == UPDATE_WAKE_MESSAGE
}

fn replace_file(source: &Path, target: &Path) -> Result<(), UpdateInstallError> {
    let source = wide_null(source.as_os_str());
    let target = wide_null(target.as_os_str());
    // SAFETY: Both paths are NUL-terminated and remain alive through this synchronous move.
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(target.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|source| UpdateInstallError::ReplaceExecutable(source.into()))
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    left.to_string_lossy().eq_ignore_ascii_case(&right.to_string_lossy())
}

fn argument_eq(argument: &OsStr, expected: &str) -> bool {
    argument.to_str().is_some_and(|argument| argument.eq_ignore_ascii_case(expected))
}

fn wide_null(value: &OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().chain([0]).collect()
}

#[derive(Debug, Error)]
pub enum UpdateStartError {
    #[error("Lotus could not start its update work: {0}")]
    Thread(#[source] std::io::Error),
}

#[derive(Debug, Error)]
pub enum UpdateInstallError {
    #[error("LOCALAPPDATA is unavailable")]
    MissingLocalAppData,
    #[error("Lotus could not locate its executable: {0}")]
    CurrentExecutable(#[source] std::io::Error),
    #[error("the update helper is missing its install target")]
    MissingTarget,
    #[error("the update install target is invalid")]
    InvalidTarget,
    #[error("the staged update path is invalid")]
    InvalidSource,
    #[error("invalid update helper arguments: {0}")]
    StartupArguments(#[source] crate::startup::StartupArgsError),
    #[error("the update helper could not wait for Lotus: {0}")]
    RestartWait(#[source] crate::startup::RestartWaitError),
    #[error("Lotus could not create its install directory: {0}")]
    InstallDirectory(#[source] std::io::Error),
    #[error("Lotus could not stage its installed executable: {0}")]
    CopyExecutable(#[source] std::io::Error),
    #[error("Windows could not replace the installed Lotus executable: {0}")]
    ReplaceExecutable(#[source] crate::NativeError),
    #[error("Lotus could not launch its update helper: {0}")]
    LaunchHelper(#[source] std::io::Error),
    #[error("Lotus could not launch the installed update: {0}")]
    LaunchInstalled(#[source] std::io::Error),
    #[error("refusing to clean an invalid update path: {0}")]
    InvalidCleanupPath(PathBuf),
    #[error("Lotus could not remove its staged update: {0}")]
    Cleanup(#[source] std::io::Error),
}
