use std::backtrace::Backtrace;
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::panic::PanicHookInfo;
use std::path::PathBuf;
use std::sync::{Mutex, Once, TryLockError};

use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::GetCurrentProcessId;

const LOG_DIRECTORY: &str = "Lotus\\logs";
const LOG_FILE: &str = "lotus.log";
const PREVIOUS_LOG_FILE: &str = "lotus.previous.log";
const ROTATION_THRESHOLD_BYTES: u64 = 1024 * 1024;

static PANIC_HOOK: Once = Once::new();
static LOG_WRITES: Mutex<()> = Mutex::new(());

pub fn install_panic_hook() {
    PANIC_HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                record_panic(info);
            }));
            previous(info);
        }));
    });
}

pub fn record_error(context: &str, error: &(dyn Error + 'static)) {
    let mut details = format!("{error}");
    let mut source = error.source();
    while let Some(error) = source {
        details.push_str("\ncaused by: ");
        details.push_str(&error.to_string());
        source = error.source();
    }
    write_entry("error", context, &details);
}

pub fn record_message(context: &str, message: &str) {
    write_entry("error", context, message);
}

pub fn record_diagnostic(context: &str, message: &str) {
    write_entry("diagnostic", context, message);
}

pub fn log_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .map(|directory| directory.join(LOG_DIRECTORY).join(LOG_FILE))
}

fn record_panic(info: &PanicHookInfo<'_>) {
    let location = info
        .location()
        .map_or_else(|| "unknown location".to_owned(), ToString::to_string);
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("unnamed thread");
    let payload = if let Some(message) = info.payload().downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = info.payload().downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    };
    let details = format!(
        "thread: {thread_name}\nlocation: {location}\npanic: {payload}\nbacktrace:\n{}",
        Backtrace::force_capture()
    );
    write_entry("panic", "panic", &details);
}

fn write_entry(severity: &str, context: &str, details: &str) {
    let Some(path) = log_path() else {
        return;
    };
    let _guard = match LOG_WRITES.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(error)) => error.into_inner(),
        Err(TryLockError::WouldBlock) => return,
    };
    let Some(directory) = path.parent() else {
        return;
    };
    if fs::create_dir_all(directory).is_err() {
        return;
    }
    rotate_if_needed(&path);
    let Ok(mut log) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = writeln!(
        log,
        "[{}] version={} pid={} severity={} context={}\n{}\n",
        local_timestamp(),
        env!("CARGO_PKG_VERSION"),
        current_process_id(),
        severity,
        context,
        details
    );
}

fn rotate_if_needed(path: &std::path::Path) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if metadata.len() < ROTATION_THRESHOLD_BYTES {
        return;
    }
    let previous = path.with_file_name(PREVIOUS_LOG_FILE);
    let _ = fs::remove_file(&previous);
    let _ = fs::rename(path, previous);
}

fn local_timestamp() -> String {
    let time = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        time.wYear,
        time.wMonth,
        time.wDay,
        time.wHour,
        time.wMinute,
        time.wSecond,
        time.wMilliseconds
    )
}

fn current_process_id() -> u32 {
    unsafe { GetCurrentProcessId() }
}
