use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryIter};
use std::time::{Duration, SystemTime};

use atomic_write_file::AtomicWriteFile;
pub use lotus_core::settings::UpdateChannel;
pub use lotus_update::{Release, StagedUpdate, UpdateError, UpdateStatus};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;

use crate::messages::UPDATE_WAKE as UPDATE_WAKE_MESSAGE;
use crate::startup::{RestartWaitOutcome, parse_startup_args, wait_for_restart_source};

const INSTALL_UPDATE_ARGUMENT: &str = "--install-update";
const UPDATE_HELPER_NAME: &str = "lotus-update-helper.exe";
const UPDATE_STATE_NAME: &str = "update-state.json";
const STAGING_MARKER_NAME: &str = "lotus-update.staged";
const POST_INSTALL_HEALTH_MARKER_NAME: &str = "lotus-health.pending";
const STALE_STAGING_AGE: Duration = Duration::from_hours(24);

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
        Self {
            owner_thread: unsafe { GetCurrentThreadId() },
            working: Arc::new(AtomicBool::new(false)),
            results,
            sender,
        }
    }
    pub fn start_check(
        &self,
        current_version: &'static str,
        channel: UpdateChannel,
    ) -> Result<bool, UpdateStartError> {
        self.spawn("lotus-update-check", move || {
            UpdateResult::Checked(lotus_update::check(current_version, channel))
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
        if self
            .working
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(false);
        }
        let sender = self.sender.clone();
        let working = Arc::clone(&self.working);
        let owner_thread = self.owner_thread;
        let spawned = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let result = work();
                working.store(false, Ordering::Release);
                if sender.send(result).is_ok() {
                    let _ = unsafe {
                        PostThreadMessageW(
                            owner_thread,
                            UPDATE_WAKE_MESSAGE,
                            WPARAM(0),
                            LPARAM(0),
                        )
                    };
                }
            });
        if let Err(source) = spawned {
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

#[derive(Clone, Debug, Deserialize, Serialize)]
struct UpdateJournal {
    target_version: String,
    source_executable: PathBuf,
    staging_directory: PathBuf,
    phase: UpdatePhase,
    diagnostic: Option<String>,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
enum UpdatePhase {
    Prepared,
    InstallerRunning,
    Failed,
}

pub fn is_installed() -> Result<bool, UpdateInstallError> {
    let current = std::env::current_exe().map_err(UpdateInstallError::CurrentExecutable)?;
    Ok(current
        .parent()
        .is_some_and(|directory| directory.join("unins000.exe").is_file()))
}

pub fn post_install_health_pending() -> Result<bool, UpdateInstallError> {
    Ok(post_install_health_marker()?.is_file())
}

pub fn interrupted_install_health_pending() -> Result<bool, UpdateInstallError> {
    let Some(journal) = read_journal()? else {
        return Ok(false);
    };
    validate_journal(&journal)?;
    if !matches!(journal.phase, UpdatePhase::InstallerRunning) {
        return Ok(false);
    }

    let current = std::env::current_exe().map_err(UpdateInstallError::CurrentExecutable)?;
    Ok(!paths_equal(&current, &journal.source_executable) || is_installed()?)
}

pub fn verify_post_install_target() -> Result<(), UpdateInstallError> {
    let Some(journal) = read_journal()? else {
        return Ok(());
    };
    validate_journal(&journal)?;
    if matches!(journal.phase, UpdatePhase::Failed) {
        return Err(UpdateInstallError::FailedJournal);
    }
    if journal.target_version != env!("CARGO_PKG_VERSION") {
        return Err(UpdateInstallError::TargetVersionMismatch {
            expected: journal.target_version,
            actual: env!("CARGO_PKG_VERSION").to_owned(),
        });
    }
    Ok(())
}

pub fn launch_installer(staged: &StagedUpdate) -> Result<(), UpdateInstallError> {
    validate_staging_directory(&staged.directory)?;
    let source = std::env::current_exe().map_err(UpdateInstallError::CurrentExecutable)?;
    let journal = UpdateJournal {
        target_version: staged.version.clone(),
        source_executable: source.clone(),
        staging_directory: staged.directory.clone(),
        phase: UpdatePhase::Prepared,
        diagnostic: None,
    };
    write_journal(&journal)?;
    let helper = staged.directory.join(UPDATE_HELPER_NAME);
    if let Err(error) = fs::copy(&source, &helper) {
        abandon_prepared_update(&journal, "Lotus could not stage its update helper.");
        return Err(UpdateInstallError::CopyHelper(error));
    }
    if let Err(error) = Command::new(&helper)
        .arg(INSTALL_UPDATE_ARGUMENT)
        .arg(&staged.executable)
        .arg("--restart-after")
        .arg(std::process::id().to_string())
        .spawn()
    {
        abandon_prepared_update(&journal, "Lotus could not launch its update helper.");
        return Err(UpdateInstallError::LaunchHelper(error));
    }
    Ok(())
}

pub fn run_helper_if_requested() -> Result<bool, UpdateInstallError> {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let Some(installer) = helper_target(&arguments, INSTALL_UPDATE_ARGUMENT)? else {
        return Ok(false);
    };
    let startup =
        parse_startup_args(&arguments).map_err(UpdateInstallError::StartupArguments)?;
    let mut journal = read_journal()?.ok_or(UpdateInstallError::MissingJournal)?;
    validate_journal(&journal)?;
    wait_for_update_source(startup.restart_after)?;
    journal.phase = UpdatePhase::InstallerRunning;
    if let Err(error) = write_journal(&journal) {
        let _ = relaunch_source(&journal);
        return Err(error);
    }
    match run_installer(&installer) {
        Ok(()) => Ok(true),
        Err(error) => {
            recover_failed_update(&journal, &error.to_string())?;
            Ok(true)
        }
    }
}

pub fn recover_startup(
    post_install_health: bool,
) -> Result<Option<String>, UpdateInstallError> {
    if post_install_health {
        return Ok(None);
    }
    let Some(mut journal) = read_journal()? else {
        return Ok(None);
    };
    validate_journal(&journal)?;
    if !matches!(journal.phase, UpdatePhase::Failed) {
        journal.phase = UpdatePhase::Failed;
        journal.diagnostic = Some(format!(
            "Lotus did not finish installing version {}. Please re-run the Lotus installer to repair the installation.",
            journal.target_version
        ));
        write_journal(&journal)?;
    }
    cleanup_staging_directory(&journal.staging_directory)?;
    let diagnostic = journal.diagnostic;
    clear_journal()?;
    Ok(diagnostic)
}

pub fn complete_post_install_health(
    success: bool,
    diagnostic: &str,
) -> Result<(), UpdateInstallError> {
    if success {
        verify_post_install_target()?;
    }
    if let Some(mut journal) = read_journal()? {
        validate_journal(&journal)?;
        if success {
            let installed =
                std::env::current_exe().map_err(UpdateInstallError::CurrentExecutable)?;
            if !paths_equal(&journal.source_executable, &installed) {
                crate::diagnostics::record_message(
                    "update.portable_migration",
                    &format!(
                        "Lotus installed version {} while preserving settings from {}.",
                        journal.target_version,
                        journal.source_executable.display()
                    ),
                );
            }
            cleanup_staging_directory(&journal.staging_directory)?;
            clear_journal()?;
        } else {
            journal.phase = UpdatePhase::Failed;
            journal.diagnostic = Some(diagnostic.to_owned());
            write_journal(&journal)?;
        }
    }
    if success {
        let marker = post_install_health_marker()?;
        if marker.exists() {
            fs::remove_file(marker).map_err(UpdateInstallError::HealthMarkerRemove)?;
        }
    }
    Ok(())
}

pub fn cleanup_stale_staging() -> Vec<UpdateInstallError> {
    let entries = match fs::read_dir(std::env::temp_dir()) {
        Ok(entries) => entries,
        Err(error) => return vec![UpdateInstallError::StaleEnumeration(error)],
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_staging_directory(path))
        .filter(|path| is_stale(path))
        .filter_map(|path| cleanup_staging_directory(&path).err())
        .collect()
}

pub fn cleanup_staging_directory(path: &Path) -> Result<(), UpdateInstallError> {
    let directory = staging_directory_from_path(path)?;
    if directory.exists() {
        fs::remove_dir_all(directory).map_err(UpdateInstallError::Cleanup)?;
    }
    Ok(())
}
pub const fn is_update_wake(message: u32) -> bool {
    message == UPDATE_WAKE_MESSAGE
}

fn run_installer(installer: &Path) -> Result<(), UpdateInstallError> {
    let status = Command::new(installer)
        .arg("/VERYSILENT")
        .arg("/SUPPRESSMSGBOXES")
        .arg("/NORESTART")
        .arg("/RESTARTLOTUS=1")
        .status()
        .map_err(UpdateInstallError::LaunchInstaller)?;
    if status.success() {
        Ok(())
    } else {
        Err(UpdateInstallError::InstallerExit(status.code()))
    }
}
fn recover_failed_update(
    journal: &UpdateJournal,
    detail: &str,
) -> Result<(), UpdateInstallError> {
    let diagnostic = format!(
        "Lotus could not install version {}. The installer failed: {detail} Please re-run the Lotus installer to repair the installation.",
        journal.target_version
    );
    let journal_result = fail_journal(journal, &diagnostic);
    relaunch_source(journal)?;
    journal_result
}
fn relaunch_source(journal: &UpdateJournal) -> Result<(), UpdateInstallError> {
    Command::new(&journal.source_executable)
        .arg("--restart-after")
        .arg(std::process::id().to_string())
        .arg("--cleanup-update")
        .arg(&journal.staging_directory)
        .arg("--open-settings")
        .spawn()
        .map_err(UpdateInstallError::LaunchSource)?;
    Ok(())
}
fn abandon_prepared_update(journal: &UpdateJournal, diagnostic: &str) {
    if let Err(error) = fail_journal(journal, diagnostic) {
        crate::diagnostics::record_error("update.rollback_journal", &error);
        return;
    }
    if let Err(error) = cleanup_staging_directory(&journal.staging_directory) {
        crate::diagnostics::record_error("update.rollback_cleanup", &error);
        return;
    }
    if let Err(error) = clear_journal() {
        crate::diagnostics::record_error("update.rollback_journal", &error);
    }
}
fn fail_journal(
    journal: &UpdateJournal,
    diagnostic: &str,
) -> Result<(), UpdateInstallError> {
    let mut failed = journal.clone();
    failed.phase = UpdatePhase::Failed;
    failed.diagnostic = Some(diagnostic.to_owned());
    write_journal(&failed)
}
fn wait_for_update_source(restart_after: Option<u32>) -> Result<(), UpdateInstallError> {
    let Some(process_id) = restart_after else {
        return Ok(());
    };
    match wait_for_restart_source(Some(process_id))
        .map_err(UpdateInstallError::RestartWait)?
    {
        RestartWaitOutcome::TimedOut => {
            Err(UpdateInstallError::RestartTimedOut { process_id })
        }
        _ => Ok(()),
    }
}
fn helper_target(
    arguments: &[OsString],
    argument_name: &str,
) -> Result<Option<PathBuf>, UpdateInstallError> {
    let Some(index) = arguments
        .iter()
        .position(|argument| argument_eq(argument, argument_name))
    else {
        return Ok(None);
    };
    arguments
        .get(index + 1)
        .map(PathBuf::from)
        .map(Some)
        .ok_or(UpdateInstallError::MissingTarget)
}
fn local_app_data() -> Result<PathBuf, UpdateInstallError> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or(UpdateInstallError::MissingLocalAppData)
}
fn journal_path() -> Result<PathBuf, UpdateInstallError> {
    local_app_data().map(|directory| directory.join("Lotus").join(UPDATE_STATE_NAME))
}
fn post_install_health_marker() -> Result<PathBuf, UpdateInstallError> {
    let executable =
        std::env::current_exe().map_err(UpdateInstallError::CurrentExecutable)?;
    executable
        .parent()
        .map(|directory| directory.join(POST_INSTALL_HEALTH_MARKER_NAME))
        .ok_or(UpdateInstallError::InvalidInstallDirectory)
}
fn read_journal() -> Result<Option<UpdateJournal>, UpdateInstallError> {
    let path = journal_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read(path).map_err(UpdateInstallError::JournalRead)?;
    serde_json::from_slice(&content)
        .map(Some)
        .map_err(UpdateInstallError::JournalDecode)
}
fn write_journal(journal: &UpdateJournal) -> Result<(), UpdateInstallError> {
    let path = journal_path()?;
    let directory = path
        .parent()
        .ok_or(UpdateInstallError::InvalidJournalPath)?;
    fs::create_dir_all(directory).map_err(UpdateInstallError::JournalWrite)?;
    let content = serde_json::to_vec(journal).map_err(UpdateInstallError::JournalEncode)?;
    let mut file = AtomicWriteFile::open(path).map_err(UpdateInstallError::JournalWrite)?;
    file.write_all(&content)
        .map_err(UpdateInstallError::JournalWrite)?;
    file.commit().map_err(UpdateInstallError::JournalWrite)
}
fn clear_journal() -> Result<(), UpdateInstallError> {
    let path = journal_path()?;
    if path.exists() {
        fs::remove_file(path).map_err(UpdateInstallError::JournalRemove)?;
    }
    Ok(())
}
fn validate_journal(journal: &UpdateJournal) -> Result<(), UpdateInstallError> {
    validate_staging_reference(&journal.staging_directory)?;
    if journal
        .source_executable
        .file_name()
        .is_none_or(|name| !name.eq_ignore_ascii_case("lotus.exe"))
    {
        return Err(UpdateInstallError::InvalidSource);
    }
    Ok(())
}
fn staging_directory_from_path(path: &Path) -> Result<&Path, UpdateInstallError> {
    if has_staging_path_shape(path) {
        validate_staging_reference(path)?;
        return Ok(path);
    }
    let Some(parent) = path.parent() else {
        return Err(UpdateInstallError::InvalidCleanupPath(path.to_owned()));
    };
    if path
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("lotus-setup.exe"))
        && has_staging_path_shape(parent)
    {
        validate_staging_reference(parent)?;
        Ok(parent)
    } else {
        Err(UpdateInstallError::InvalidCleanupPath(path.to_owned()))
    }
}
fn validate_staging_directory(path: &Path) -> Result<(), UpdateInstallError> {
    if is_staging_directory(path) {
        Ok(())
    } else {
        Err(UpdateInstallError::InvalidCleanupPath(path.to_owned()))
    }
}
fn is_staging_directory(path: &Path) -> bool {
    has_staging_path_shape(path) && path.join(STAGING_MARKER_NAME).is_file()
}
fn validate_staging_reference(path: &Path) -> Result<(), UpdateInstallError> {
    if has_staging_path_shape(path)
        && (!path.exists() || path.join(STAGING_MARKER_NAME).is_file())
    {
        Ok(())
    } else {
        Err(UpdateInstallError::InvalidCleanupPath(path.to_owned()))
    }
}
fn has_staging_path_shape(path: &Path) -> bool {
    path.parent()
        .is_some_and(|parent| paths_equal(parent, &std::env::temp_dir()))
        && path
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with("lotus-update-"))
}
fn is_stale(path: &Path) -> bool {
    path.metadata()
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age >= STALE_STAGING_AGE)
}
fn paths_equal(left: &Path, right: &Path) -> bool {
    left.to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy())
}
fn argument_eq(argument: &OsStr, expected: &str) -> bool {
    argument
        .to_str()
        .is_some_and(|argument| argument.eq_ignore_ascii_case(expected))
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
    #[error("the update helper is missing its installer target")]
    MissingTarget,
    #[error("the update journal is unavailable")]
    MissingJournal,
    #[error("the update journal records a failed installation")]
    FailedJournal,
    #[error("the update installed Lotus {actual}, but expected Lotus {expected}")]
    TargetVersionMismatch { expected: String, actual: String },
    #[error("the update source executable is invalid")]
    InvalidSource,
    #[error("the update journal path is invalid")]
    InvalidJournalPath,
    #[error("the Lotus installation directory is invalid")]
    InvalidInstallDirectory,
    #[error("invalid update helper arguments: {0}")]
    StartupArguments(#[source] crate::startup::StartupArgsError),
    #[error("the update helper could not wait for Lotus: {0}")]
    RestartWait(#[source] crate::startup::RestartWaitError),
    #[error("Lotus did not exit before the update timeout (process {process_id})")]
    RestartTimedOut { process_id: u32 },
    #[error("Lotus could not stage its update helper: {0}")]
    CopyHelper(#[source] std::io::Error),
    #[error("Lotus could not launch its update helper: {0}")]
    LaunchHelper(#[source] std::io::Error),
    #[error("Lotus could not launch its installer: {0}")]
    LaunchInstaller(#[source] std::io::Error),
    #[error("the Lotus installer exited unsuccessfully ({0:?})")]
    InstallerExit(Option<i32>),
    #[error("Lotus could not relaunch its pre-update executable: {0}")]
    LaunchSource(#[source] std::io::Error),
    #[error("refusing to clean an invalid update path: {0}")]
    InvalidCleanupPath(PathBuf),
    #[error("Lotus could not remove its staged update: {0}")]
    Cleanup(#[source] std::io::Error),
    #[error("Lotus could not enumerate stale update staging: {0}")]
    StaleEnumeration(#[source] std::io::Error),
    #[error("Lotus could not read its update journal: {0}")]
    JournalRead(#[source] std::io::Error),
    #[error("Lotus could not decode its update journal: {0}")]
    JournalDecode(#[source] serde_json::Error),
    #[error("Lotus could not encode its update journal: {0}")]
    JournalEncode(#[source] serde_json::Error),
    #[error("Lotus could not write its update journal: {0}")]
    JournalWrite(#[source] std::io::Error),
    #[error("Lotus could not remove its update journal: {0}")]
    JournalRemove(#[source] std::io::Error),
    #[error("Lotus could not clear its post-install health marker: {0}")]
    HealthMarkerRemove(#[source] std::io::Error),
}
