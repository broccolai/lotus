use std::backtrace::Backtrace;
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::panic::PanicHookInfo;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, Once, TryLockError};

use atomic_write_file::AtomicWriteFile;
use lotus_core::settings::DockSettings;
use thiserror::Error;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::GetCurrentProcessId;

const LOG_DIRECTORY: &str = "Lotus\\logs";
const LOG_FILE: &str = "lotus.log";
const PREVIOUS_LOG_FILE: &str = "lotus.previous.log";
const ROTATION_THRESHOLD_BYTES: u64 = 1024 * 1024;
const EXPORTED_LOG_BYTES_PER_FILE: usize = 32 * 1024;

static PANIC_HOOK: Once = Once::new();
static LOG_WRITES: Mutex<()> = Mutex::new(());
static DROPPED_WRITES: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum DiagnosticsExportError {
    #[error("could not create the diagnostics export")]
    Open(#[source] io::Error),
    #[error("could not write the diagnostics export")]
    Write(#[source] io::Error),
    #[error("could not finish the diagnostics export")]
    Commit(#[source] io::Error),
}

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

/// Records numeric lifecycle state that is safe to retain in support exports.
pub fn record_state(context: &'static str, fields: &[(&'static str, u64)]) {
    let details = fields
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join(" ");
    write_entry("state", context, &format!("@state {details}"));
}

pub fn log_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .map(|directory| directory.join(LOG_DIRECTORY).join(LOG_FILE))
}

pub fn export_support_report(
    destination: &std::path::Path,
    settings: &DockSettings,
    integration: &str,
) -> Result<(), DiagnosticsExportError> {
    let report = support_report(settings, integration);
    let mut file =
        AtomicWriteFile::open(destination).map_err(DiagnosticsExportError::Open)?;
    file.write_all(report.as_bytes())
        .map_err(DiagnosticsExportError::Write)?;
    file.commit().map_err(DiagnosticsExportError::Commit)
}

fn support_report(settings: &DockSettings, integration: &str) -> String {
    let mut output = format!(
        "Lotus diagnostics\nversion: {}\n\nsettings summary\n{}\n\nintegration state\n{}\n\nresponsiveness\n{}",
        env!("CARGO_PKG_VERSION"),
        settings_summary(settings),
        integration,
        crate::responsiveness::METRICS.snapshot().to_text(),
    );
    append_recent_logs(&mut output);
    output
}

fn settings_summary(settings: &DockSettings) -> String {
    format!(
        "notification_badge_style={:?}\nupdate_channel={:?}\ndock_zone={:?}\nsystem_status_zone={:?}\nmedia_zone={:?}\nwindow_picker_style={:?}\nuse_acrylic={}\nshow_app_dock={}\nshow_unpinned_running_apps={}\nshow_running_indicators={}\nshow_on_all_monitors={}\nshow_desktop_button={}\nshow_system_status={}\nshow_volume_status={}\nshow_hdr_status={}\nshow_network_status={}\nshow_background_apps_status={}\nshow_date_time_status={}\nshow_date_in_status={}\nuse_24_hour_time={}\nshow_media_controls={}\nshow_media_metadata={}\nstart_with_windows={}\nhide_when_fullscreen={}\nreplace_windows_taskbar={}\nexclusive_taskbar_replacement={}\nsearch_enabled={}\nsearch_open_with_windows_key={}\nalt_tab_enabled={}\nnotification_disabled_apps_count={}\napplication_name_overrides_count={}\napplication_icon_overrides_count={}\nhidden_executables_count={}\nitem_order_count={}\npinned_apps_count={}\nextra_fields_count={}",
        settings.notification_badge_style,
        settings.update_channel,
        settings.dock_zone,
        settings.system_status_zone,
        settings.media_zone,
        settings.window_picker_style,
        settings.use_acrylic,
        settings.show_app_dock,
        settings.show_unpinned_running_apps,
        settings.show_running_indicators,
        settings.show_on_all_monitors,
        settings.show_desktop_button,
        settings.show_system_status,
        settings.show_volume_status,
        settings.show_hdr_status,
        settings.show_network_status,
        settings.show_background_apps_status,
        settings.show_date_time_status,
        settings.show_date_in_status,
        settings.use_24_hour_time,
        settings.show_media_controls,
        settings.show_media_metadata,
        settings.start_with_windows,
        settings.hide_when_fullscreen,
        settings.replace_windows_taskbar,
        settings.exclusive_taskbar_replacement,
        settings.search_enabled,
        settings.search_open_with_windows_key,
        settings.alt_tab_enabled,
        settings.notification_disabled_apps.len(),
        settings.application_name_overrides.len(),
        settings.application_icon_overrides.len(),
        settings.hidden_executables.len(),
        settings.item_order.len(),
        settings.pinned_apps.len(),
        settings.extra_fields.len(),
    )
}

fn append_recent_logs(output: &mut String) {
    let Some(current) = log_path() else {
        return;
    };
    for path in [current.with_file_name(PREVIOUS_LOG_FILE), current] {
        let Ok(contents) = fs::read_to_string(&path) else {
            continue;
        };
        let mut start = contents.len().saturating_sub(EXPORTED_LOG_BYTES_PER_FILE);
        while start < contents.len() && !contents.is_char_boundary(start) {
            start += 1;
        }
        let recent = &contents[start..];
        output.push_str("\n\nrecent diagnostics\n");
        output.push_str(&redact_log_headers(recent));
    }
}

fn redact_log_headers(text: &str) -> String {
    text.lines()
        .filter(|line| line.starts_with('[') || is_numeric_state_line(line))
        .map(redact_support_text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_numeric_state_line(line: &str) -> bool {
    line.strip_prefix("@state ").is_some_and(|fields| {
        !fields.is_empty()
            && fields.split_whitespace().all(|field| {
                field.split_once('=').is_some_and(|(name, value)| {
                    !name.is_empty()
                        && name
                            .bytes()
                            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
                        && !value.is_empty()
                        && value.bytes().all(|byte| byte.is_ascii_digit())
                })
            })
    })
}

fn redact_support_text(text: &str) -> String {
    let mut redacted = text.to_owned();
    for variable in [
        "USERPROFILE",
        "LOCALAPPDATA",
        "APPDATA",
        "TEMP",
        "TMP",
        "USERNAME",
    ] {
        if let Some(value) = std::env::var_os(variable).filter(|value| !value.is_empty()) {
            let value = value.to_string_lossy();
            redacted = redacted.replace(value.as_ref(), "<redacted>");
        }
    }
    redact_user_profile_paths(&redacted)
}

fn redact_user_profile_paths(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut remainder = text;
    while let Some(index) = remainder.to_ascii_lowercase().find("\\users\\") {
        let (prefix, path) = remainder.split_at(index);
        output.push_str(prefix);
        output.push_str("\\Users\\<redacted>");
        let after_user = &path[7..];
        let boundary = after_user
            .find(['\\', '/', ' ', '\n', '\r', '\t'])
            .unwrap_or(after_user.len());
        remainder = &after_user[boundary..];
    }
    output.push_str(remainder);
    output
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
        DROPPED_WRITES.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let _guard = match LOG_WRITES.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(error)) => error.into_inner(),
        Err(TryLockError::WouldBlock) => {
            DROPPED_WRITES.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };
    let Some(directory) = path.parent() else {
        return;
    };
    if fs::create_dir_all(directory).is_err() {
        DROPPED_WRITES.fetch_add(1, Ordering::Relaxed);
        return;
    }
    rotate_if_needed(&path);
    let Ok(mut log) = OpenOptions::new().create(true).append(true).open(path) else {
        DROPPED_WRITES.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if writeln!(
        log,
        "[{}] version={} pid={} severity={} context={} dropped_writes={}\n{}\n",
        local_timestamp(),
        env!("CARGO_PKG_VERSION"),
        current_process_id(),
        severity,
        context,
        DROPPED_WRITES.load(Ordering::Relaxed),
        details
    )
    .is_err()
    {
        DROPPED_WRITES.fetch_add(1, Ordering::Relaxed);
    }
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
