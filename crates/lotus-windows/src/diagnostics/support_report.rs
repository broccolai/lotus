use std::fmt::Write as _;
use std::fs::File;
use std::hash::{BuildHasher, Hash, Hasher, RandomState};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::LazyLock;
use std::time::Instant;

use atomic_write_file::AtomicWriteFile;
use lotus_core::application::{
    ApplicationKey, ApplicationResolution, LaunchSpec, WindowApplicationAssignments,
    WindowApplicationFacts, normalized_path, normalized_value,
};
use lotus_core::dock::DockItem;
use lotus_core::settings::DockSettings;
use thiserror::Error;

use super::{PREVIOUS_LOG_FILE, current_process_id, local_timestamp, log_path};

const EXPORTED_LOG_BYTES_PER_FILE: u64 = 32 * 1024;
const SUPPORT_REPORT_ROW_CAP: usize = 512;

static PSEUDONYMIZER: LazyLock<Pseudonymizer> = LazyLock::new(Pseudonymizer::new);

#[derive(Debug, Error)]
pub enum DiagnosticsExportError {
    #[error("could not create the diagnostics export")]
    Open(#[source] io::Error),
    #[error("could not write the diagnostics export")]
    Write(#[source] io::Error),
    #[error("could not finish the diagnostics export")]
    Commit(#[source] io::Error),
}

pub struct SupportReport {
    file_name: String,
    text: String,
}

impl SupportReport {
    pub fn suggested_file_name(&self) -> &str {
        &self.file_name
    }

    pub fn write_to(&self, destination: &Path) -> Result<(), DiagnosticsExportError> {
        let mut file =
            AtomicWriteFile::open(destination).map_err(DiagnosticsExportError::Open)?;
        file.write_all(self.text.as_bytes())
            .map_err(DiagnosticsExportError::Write)?;
        file.commit().map_err(DiagnosticsExportError::Commit)
    }
}

pub fn capture_support_report(
    settings: &DockSettings,
    integration: &str,
    tracker: &crate::window_tracker::WindowTracker,
    dock_items: &[DockItem],
    assignments: &WindowApplicationAssignments,
    runtime: &str,
) -> SupportReport {
    let started = Instant::now();
    let timestamp = local_timestamp();
    let pseudonymizer = &PSEUDONYMIZER;
    let mut output = format!(
        "Lotus diagnostics\nformat_version: 1\nversion: {}\ncaptured_at: {timestamp}\nprocess_id: {}\ncapture_caveat: observations are sequential and not an atomic system snapshot; no refresh, reconciliation, activation, or repair was requested.\n\nsettings summary\n{}\n\nintegration state\n{}\n\nruntime state\n{}\n\nresponsiveness\n{}",
        env!("CARGO_PKG_VERSION"),
        current_process_id(),
        settings_summary(settings),
        integration,
        runtime,
        crate::responsiveness::METRICS.snapshot().to_text(),
    );
    append_dock_associations(&mut output, dock_items, assignments, pseudonymizer);
    tracker.append_diagnostics(&mut output, pseudonymizer);
    append_recent_logs(&mut output);
    let _ = writeln!(
        output,
        "\n\ncapture_duration_ms={}",
        started.elapsed().as_millis()
    );

    SupportReport {
        file_name: format!(
            "lotus-diagnostics-{}.txt",
            timestamp.replace([' ', ':', '.'], "-")
        ),
        text: output,
    }
}

pub(crate) struct Pseudonymizer {
    state: RandomState,
}

impl Pseudonymizer {
    fn new() -> Self {
        Self {
            state: RandomState::new(),
        }
    }

    pub(crate) fn token(&self, namespace: &str, value: &str) -> String {
        let mut hasher = self.state.build_hasher();
        namespace.hash(&mut hasher);
        value.hash(&mut hasher);
        format!("{namespace}:{:016x}", hasher.finish())
    }

    fn normalized_token(&self, namespace: &str, value: &str) -> String {
        normalized_value(value).map_or_else(
            || self.token(namespace, "<empty>"),
            |value| self.token(namespace, &value),
        )
    }

    fn normalized_path_token(&self, namespace: &str, value: &str) -> String {
        normalized_path(value).map_or_else(
            || self.token(namespace, "<empty>"),
            |value| self.token(namespace, &value),
        )
    }

    pub(crate) fn window_facts(&self, facts: &WindowApplicationFacts) -> String {
        let mut fields = vec![
            named_token_field(
                self,
                "window_aumid",
                "registered_id",
                facts.window_app_user_model_id.as_deref(),
            ),
            named_token_field(
                self,
                "process_aumid",
                "registered_id",
                facts.process_app_user_model_id.as_deref(),
            ),
            format!("prevent_pinning={}", facts.prevent_pinning),
        ];
        if let Some(relaunch) = &facts.relaunch {
            fields.push(format!(
                "relaunch_target={}",
                self.normalized_path_token("launch_target", &relaunch.target)
            ));
            fields.push(format!(
                "relaunch_signature={}",
                self.token("launch_signature", &relaunch.signature())
            ));
            fields.extend(chromium_context(self, relaunch.arguments.as_deref()));
        } else {
            fields.push("relaunch=none".to_owned());
        }
        fields.join(" ")
    }
}

fn token_field(
    pseudonymizer: &Pseudonymizer,
    namespace: &str,
    value: Option<&str>,
) -> String {
    value.map_or_else(
        || format!("{namespace}=none"),
        |value| format!("{namespace}={}", pseudonymizer.token(namespace, value)),
    )
}

fn named_token_field(
    pseudonymizer: &Pseudonymizer,
    label: &str,
    namespace: &str,
    value: Option<&str>,
) -> String {
    value.map_or_else(
        || format!("{label}=none"),
        |value| {
            format!(
                "{label}={}",
                pseudonymizer.normalized_token(namespace, value)
            )
        },
    )
}

fn named_path_token_field(
    pseudonymizer: &Pseudonymizer,
    label: &str,
    namespace: &str,
    value: Option<&str>,
) -> String {
    value.map_or_else(
        || format!("{label}=none"),
        |value| {
            format!(
                "{label}={}",
                pseudonymizer.normalized_path_token(namespace, value)
            )
        },
    )
}

fn chromium_context(pseudonymizer: &Pseudonymizer, arguments: Option<&str>) -> Vec<String> {
    let Some(arguments) = arguments else {
        return vec!["chromium_pwa=false".to_owned()];
    };
    let arguments = crate::launch::command_line_arguments(arguments);
    let mut app_id = None;
    let mut app = None;
    let mut profile = None;
    let mut user_data = None;
    let mut iterator = arguments.iter().map(String::as_str);
    while let Some(argument) = iterator.next() {
        let (name, inline_value) = argument
            .split_once('=')
            .map_or((argument, None), |(name, value)| (name, Some(value)));
        if name.eq_ignore_ascii_case("--app-id") {
            let value = inline_value.or_else(|| iterator.next());
            app_id = value;
        } else if name.eq_ignore_ascii_case("--app") {
            let value = inline_value.or_else(|| iterator.next());
            app = value;
        } else if name.eq_ignore_ascii_case("--profile-directory") {
            let value = inline_value.or_else(|| iterator.next());
            profile = value;
        } else if name.eq_ignore_ascii_case("--user-data-dir") {
            let value = inline_value.or_else(|| iterator.next());
            user_data = value;
        }
    }
    vec![
        format!("chromium_pwa={}", app_id.is_some() || app.is_some()),
        named_token_field(pseudonymizer, "chromium_app_id", "chromium_app_id", app_id),
        token_field(pseudonymizer, "chromium_app", app),
        named_token_field(
            pseudonymizer,
            "chromium_profile",
            "chromium_profile",
            profile,
        ),
        named_path_token_field(
            pseudonymizer,
            "chromium_user_data",
            "chromium_user_data",
            user_data,
        ),
    ]
}

fn append_dock_associations(
    output: &mut String,
    dock_items: &[DockItem],
    assignments: &WindowApplicationAssignments,
    pseudonymizer: &Pseudonymizer,
) {
    let _ = write!(
        output,
        "\n\ndock associations\nassignments catalog_generation={} window_revision={} total={} exported={} truncated={}\n",
        assignments.catalog_generation,
        assignments.window_revision,
        assignments.by_window.len(),
        assignments.by_window.len().min(SUPPORT_REPORT_ROW_CAP),
        assignments.by_window.len() > SUPPORT_REPORT_ROW_CAP,
    );
    let mut assigned = assignments
        .by_window
        .iter()
        .take(SUPPORT_REPORT_ROW_CAP)
        .collect::<Vec<_>>();
    assigned.sort_unstable_by_key(|(key, _)| **key);
    for (key, resolution) in assigned {
        let _ = writeln!(
            output,
            "assignment key={} pin_eligible={} presentation={} resolution={}\n",
            tracker_key(*key),
            assignments.can_pin(*key),
            assignments.presentation_by_window.contains_key(key),
            resolution_text(pseudonymizer, resolution),
        );
    }
    let _ = writeln!(
        output,
        "dock_items total={} exported={} truncated={}",
        dock_items.len(),
        dock_items.len().min(SUPPORT_REPORT_ROW_CAP),
        dock_items.len() > SUPPORT_REPORT_ROW_CAP
    );
    for (index, item) in dock_items.iter().take(SUPPORT_REPORT_ROW_CAP).enumerate() {
        let windows = item
            .windows
            .iter()
            .take(SUPPORT_REPORT_ROW_CAP)
            .map(|window| tracker_key(window.key()))
            .collect::<Vec<_>>()
            .join(",");
        let _ = writeln!(
            output,
            "dock_item index={index} key_type={} key={} pinned={} pin_eligible={} pin_source={} expected_window_total={} expected_window_exported={} expected_window_truncated={} expected_window_keys=[{windows}] launch_target={} launch_signature={} {}\n",
            application_key_kind(&item.application_key),
            application_key_token(pseudonymizer, &item.application_key),
            item.is_pinned,
            item.pin_eligible,
            item.pin_source
                .map_or_else(|| "none".to_owned(), tracker_key),
            item.windows.len(),
            item.windows.len().min(SUPPORT_REPORT_ROW_CAP),
            item.windows.len() > SUPPORT_REPORT_ROW_CAP,
            pseudonymizer.normalized_path_token("launch_target", &item.launch_target),
            pseudonymizer.token(
                "launch_signature",
                &launch_signature(&item.launch_target, item.arguments.as_deref())
            ),
            chromium_context(pseudonymizer, item.arguments.as_deref()).join(" "),
        );
    }
}

fn resolution_text(
    pseudonymizer: &Pseudonymizer,
    resolution: &ApplicationResolution,
) -> String {
    match resolution {
        ApplicationResolution::Resolved {
            key,
            registered_index,
            evidence,
        } => format!(
            "resolved key_type={} key={} registered_index={} evidence={evidence:?}",
            application_key_kind(key),
            application_key_token(pseudonymizer, key),
            registered_index,
        ),
        ApplicationResolution::Associated { key } => format!(
            "associated key_type={} key={}",
            application_key_kind(key),
            application_key_token(pseudonymizer, key),
        ),
        ApplicationResolution::Ambiguous {
            evidence,
            candidate_count,
        } => {
            format!("ambiguous evidence={evidence:?} candidate_count={candidate_count}")
        }
        ApplicationResolution::Unregistered { key, launch } => format!(
            "unregistered key_type={} key={} launch={}",
            application_key_kind(key),
            application_key_token(pseudonymizer, key),
            launch.as_ref().map_or_else(
                || "none".to_owned(),
                |launch| pseudonymizer.token("launch_signature", &launch.signature())
            ),
        ),
    }
}

fn application_key_kind(key: &ApplicationKey) -> &'static str {
    match key {
        ApplicationKey::Registered(_) => "registered",
        ApplicationKey::LaunchSignature(_) => "launch_signature",
        ApplicationKey::ExecutablePath(_) => "executable",
        ApplicationKey::Ephemeral(_) => "ephemeral",
    }
}

fn application_key_token(pseudonymizer: &Pseudonymizer, key: &ApplicationKey) -> String {
    match key {
        ApplicationKey::Registered(value) => {
            pseudonymizer.normalized_token("registered_id", value)
        }
        ApplicationKey::LaunchSignature(value) => {
            pseudonymizer.token("launch_signature", value)
        }
        ApplicationKey::ExecutablePath(value) => {
            pseudonymizer.normalized_path_token("executable", value)
        }
        ApplicationKey::Ephemeral(key) => tracker_key(*key),
    }
}

fn launch_signature(target: &str, arguments: Option<&str>) -> String {
    LaunchSpec::new(target, arguments)
        .map(|launch| launch.signature())
        .unwrap_or_default()
}

fn tracker_key(key: lotus_core::window::TrackedWindowKey) -> String {
    format!(
        "hwnd={} pid={} incarnation={}",
        key.id.get(),
        key.process_id,
        key.incarnation
    )
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
        output.push_str("\n\nrecent diagnostics status=unavailable\n");
        return;
    };

    for (label, path) in [
        ("previous", current.with_file_name(PREVIOUS_LOG_FILE)),
        ("current", current),
    ] {
        match read_log_tail(&path) {
            Ok((contents, truncated)) => {
                let _ = writeln!(
                    output,
                    "\n\nrecent diagnostics log={label} status=available truncated={truncated}"
                );
                output.push_str(&redact_log_headers(&contents));
            }
            Err(error) => {
                let _ = writeln!(
                    output,
                    "\n\nrecent diagnostics log={label} status=unavailable error_kind={:?}",
                    error.kind()
                );
            }
        }
    }
}

fn read_log_tail(path: &Path) -> io::Result<(String, bool)> {
    let mut file = File::open(path)?;
    let offset = file
        .metadata()?
        .len()
        .saturating_sub(EXPORTED_LOG_BYTES_PER_FILE);
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.take(EXPORTED_LOG_BYTES_PER_FILE)
        .read_to_end(&mut bytes)?;

    let start = if offset == 0 {
        0
    } else {
        bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |end| end + 1)
    };
    Ok((
        String::from_utf8_lossy(&bytes[start..]).into_owned(),
        offset != 0,
    ))
}

fn redact_log_headers(text: &str) -> String {
    let mut output = String::new();
    let mut state_expected = false;
    for line in text.lines() {
        if is_log_header(line) {
            let _ = writeln!(output, "{line}");
            state_expected = line.contains(" severity=state ");
        } else {
            if state_expected && is_numeric_state_line(line) {
                let _ = writeln!(output, "{line}");
            }
            state_expected = false;
        }
    }
    output
}

fn is_log_header(line: &str) -> bool {
    let Some((timestamp, fields)) = line
        .strip_prefix('[')
        .and_then(|line| line.split_once("] "))
    else {
        return false;
    };
    if timestamp.len() != 23
        || !timestamp
            .bytes()
            .enumerate()
            .all(|(index, byte)| match index {
                4 | 7 => byte == b'-',
                10 => byte == b' ',
                13 | 16 => byte == b':',
                19 => byte == b'.',
                _ => byte.is_ascii_digit(),
            })
    {
        return false;
    }
    let mut fields = fields.split_whitespace();
    ["version", "pid", "severity", "context", "dropped_writes"]
        .into_iter()
        .all(|name| {
            fields.next().is_some_and(|field| {
                field
                    .strip_prefix(&format!("{name}="))
                    .is_some_and(|value| valid_log_field(name, value))
            })
        })
        && fields.next().is_none()
}

fn valid_log_field(name: &str, value: &str) -> bool {
    !value.is_empty()
        && match name {
            "pid" | "dropped_writes" => value.bytes().all(|byte| byte.is_ascii_digit()),
            "version" => value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+')
            }),
            "severity" => matches!(value, "error" | "diagnostic" | "state" | "panic"),
            "context" => value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || matches!(byte, b'_' | b'.')),
            _ => false,
        }
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
