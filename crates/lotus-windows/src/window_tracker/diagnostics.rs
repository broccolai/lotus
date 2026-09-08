use std::collections::HashMap;
use std::ffi::c_void;
use std::fmt::Write as _;
use std::mem::size_of;
use std::sync::TryLockError;

use lotus_core::application::normalized_path;
use lotus_core::window::{TrackedWindowKey, WindowInfo};
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GA_ROOT, GW_OWNER, GetAncestor, GetWindow, GetWindowRect,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible,
};
use windows::core::BOOL;

use super::{STALE_TARGETS, TRACKED_WINDOWS, WindowTracker};
use crate::diagnostics::Pseudonymizer;

const REPORT_ROW_CAP: usize = 512;

pub(super) fn append_tracker_report(
    output: &mut String,
    pseudonymizer: &Pseudonymizer,
    tracker: &WindowTracker,
) {
    output.push_str("\n\nwindow tracker capture\n");
    let _ = writeln!(
        output,
        "ui_snapshot count={} window_revision={} fullscreen_revision={} presentation_revision={} fullscreen_key={} pending={}\n",
        tracker.windows.len(),
        tracker.window_revision,
        tracker.fullscreen_revision,
        tracker.presentation_revision,
        tracker
            .fullscreen_window
            .map_or_else(|| "none".to_owned(), |id| id.get().to_string()),
        tracker
            .shared
            .snapshot_pending
            .load(std::sync::atomic::Ordering::Acquire),
    );
    append_windows(output, "ui_window", tracker.windows.as_ref(), pseudonymizer);

    match tracker.shared.latest.try_lock() {
        Ok(guard) => {
            let snapshot = guard.clone();
            drop(guard);
            let _ = writeln!(
                output,
                "published_snapshot status=available count={} window_revision={} fullscreen_revision={} fullscreen_key={}\n",
                snapshot.windows.len(),
                snapshot.window_revision,
                snapshot.fullscreen_revision,
                snapshot
                    .fullscreen_window
                    .map_or_else(|| "none".to_owned(), |id| id.get().to_string()),
            );
            append_windows(
                output,
                "published_window",
                snapshot.windows.as_ref(),
                pseudonymizer,
            );
        }
        Err(error) => append_lock_status(output, "published_snapshot", &error),
    }

    match TRACKED_WINDOWS.try_lock() {
        Ok(registry) => {
            let total = registry.windows.len();
            let keys = registry
                .windows
                .iter()
                .take(REPORT_ROW_CAP)
                .map(|lifetime| lifetime.key)
                .collect::<Vec<_>>();
            drop(registry);
            let _ = writeln!(
                output,
                "registry status=available total={total} exported={} truncated={}",
                keys.len(),
                total > keys.len()
            );
            append_keys(output, "registry_key", &keys);
        }
        Err(error) => append_lock_status(output, "registry", &error),
    }

    match STALE_TARGETS.try_lock() {
        Ok(stale) => {
            let total = stale.len();
            let keys = stale
                .iter()
                .take(REPORT_ROW_CAP)
                .map(|(key, _)| *key)
                .collect::<Vec<_>>();
            drop(stale);
            let _ = writeln!(
                output,
                "retired_tombstones status=available total={total} exported={} truncated={}",
                keys.len(),
                total > keys.len()
            );
            append_keys(output, "retired_key", &keys);
        }
        Err(error) => append_lock_status(output, "retired_tombstones", &error),
    }

    append_live_windows(output, tracker.own_process_id);
}

fn append_lock_status<T>(output: &mut String, name: &str, error: &TryLockError<T>) {
    let status = match error {
        TryLockError::WouldBlock => "busy",
        TryLockError::Poisoned(_) => "poisoned",
    };
    let _ = writeln!(output, "{name} status={status}");
}

fn append_windows(
    output: &mut String,
    label: &str,
    windows: &[WindowInfo],
    pseudonymizer: &Pseudonymizer,
) {
    let _ = writeln!(
        output,
        "{label}_rows total={} exported={} truncated={}",
        windows.len(),
        windows.len().min(REPORT_ROW_CAP),
        windows.len() > REPORT_ROW_CAP
    );
    for window in windows.iter().take(REPORT_ROW_CAP) {
        let _ = writeln!(
            output,
            "{label} key={} executable={} facts={}\n",
            key_text(window.key()),
            executable_token(pseudonymizer, window),
            pseudonymizer.window_facts(&window.application_facts),
        );
    }
}

fn executable_token(pseudonymizer: &Pseudonymizer, window: &WindowInfo) -> String {
    let path = window.executable_path.to_string_lossy();
    normalized_path(&path).map_or_else(
        || "none".to_owned(),
        |path| pseudonymizer.token("executable", &path),
    )
}

fn append_keys(output: &mut String, label: &str, keys: &[TrackedWindowKey]) {
    for key in keys {
        let _ = writeln!(output, "{label} key={}", key_text(*key));
    }
}

fn key_text(key: TrackedWindowKey) -> String {
    format!(
        "hwnd={} pid={} incarnation={}",
        key.id.get(),
        key.process_id,
        key.incarnation
    )
}

fn append_live_windows(output: &mut String, own_process_id: u32) {
    let mut state = LiveEnumeration::default();
    let result = unsafe {
        EnumWindows(
            Some(visit_live_window),
            LPARAM((&raw mut state).cast::<c_void>() as isize),
        )
    };
    if state.truncated {
        let _ = writeln!(
            output,
            "live_windows status=truncated total=unknown exported={}",
            state.windows.len()
        );
    } else {
        match result {
            Ok(()) => {
                let _ = writeln!(
                    output,
                    "live_windows status=available count={} truncated={}",
                    state.windows.len(),
                    state.truncated
                );
            }
            Err(error) => {
                let _ = writeln!(
                    output,
                    "live_windows status=error error_code={} observed_count={} truncated={}",
                    error.code().0,
                    state.windows.len(),
                    state.truncated
                );
            }
        }
    }
    let visible_count = state.windows.iter().filter(|window| window.visible).count();
    let minimized_count = state
        .windows
        .iter()
        .filter(|window| window.minimized)
        .count();
    let cloaked_unavailable_count = state
        .windows
        .iter()
        .filter(|window| window.cloaked == "unavailable")
        .count();
    let _ = writeln!(
        output,
        "live_window_counts visible={visible_count} minimized={minimized_count} cloaked_unavailable={cloaked_unavailable_count}",
    );
    for window in state.windows {
        let _ = writeln!(
            output,
            "live_window hwnd={} pid={} lotus_process={} browser={} visible={} minimized={} cloaked={} root={} owner={} rect={}\n",
            window.hwnd,
            window.process_id,
            window.process_id == own_process_id,
            window.browser,
            window.visible,
            window.minimized,
            window.cloaked,
            window.root,
            window.owner,
            window.rect,
        );
    }
}

#[derive(Default)]
struct LiveEnumeration {
    windows: Vec<LiveWindow>,
    browser_by_process: HashMap<u32, &'static str>,
    truncated: bool,
}

unsafe extern "system" fn visit_live_window(hwnd: HWND, parameter: LPARAM) -> BOOL {
    // EnumWindows invokes this callback synchronously while the enumeration state remains live.
    let state = unsafe { &mut *(parameter.0 as *mut LiveEnumeration) };
    if state.windows.len() >= REPORT_ROW_CAP {
        state.truncated = true;
        return BOOL(0);
    }
    let window = live_window(hwnd, state);
    state.windows.push(window);
    BOOL(1)
}

fn live_window(hwnd: HWND, state: &mut LiveEnumeration) -> LiveWindow {
    let mut process_id = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut process_id)) };
    let browser = *state
        .browser_by_process
        .entry(process_id)
        .or_insert_with(|| {
            crate::window_tracker::process_image_path(process_id)
                .as_deref()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str())
                .map_or("unavailable", browser_family)
        });
    let mut cloaked = 0_u32;
    let cloaked = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast::<c_void>(),
            u32::try_from(size_of::<u32>()).unwrap_or_default(),
        )
    }
    .map_or_else(
        |_| "unavailable",
        |()| {
            if cloaked == 0 {
                "false"
            } else {
                "true"
            }
        },
    );
    let mut rect = RECT::default();
    let rect = unsafe { GetWindowRect(hwnd, &raw mut rect) }.map_or_else(
        |_| "unavailable".to_owned(),
        |()| format!("{},{},{},{}", rect.left, rect.top, rect.right, rect.bottom),
    );
    LiveWindow {
        hwnd: u64::try_from(hwnd.0.addr()).unwrap_or_default(),
        process_id,
        browser,
        visible: unsafe { IsWindowVisible(hwnd) }.as_bool(),
        minimized: unsafe { IsIconic(hwnd) }.as_bool(),
        cloaked,
        root: unsafe { GetAncestor(hwnd, GA_ROOT) == hwnd },
        owner: unsafe { GetWindow(hwnd, GW_OWNER).is_ok() },
        rect,
    }
}

fn browser_family(name: &str) -> &'static str {
    if name.eq_ignore_ascii_case("chrome.exe") {
        "chrome"
    } else if name.eq_ignore_ascii_case("msedge.exe") {
        "edge"
    } else if name.eq_ignore_ascii_case("brave.exe") {
        "brave"
    } else {
        "other"
    }
}

struct LiveWindow {
    hwnd: u64,
    process_id: u32,
    browser: &'static str,
    visible: bool,
    minimized: bool,
    cloaked: &'static str,
    root: bool,
    owner: bool,
    rect: String,
}
