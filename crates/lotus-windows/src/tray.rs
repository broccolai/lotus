mod discovery;
mod input;
mod placement;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use thiserror::Error;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_A, VK_LWIN,
    VK_N, VK_RETURN,
};

use crate::WindowHandle;
use crate::responsiveness::{FlyoutPhaseMetrics, METRICS};
use crate::shell_bridge::ShellBridgeLease;

const VK_B: VIRTUAL_KEY = VIRTUAL_KEY(b'B' as u16);
const FOCUS_SETTLE_TIME: Duration = Duration::from_millis(60);
const WORKER_ERROR_INTERVAL_MILLISECONDS: u64 = 1_000;

static LAST_WORKER_ERROR_MILLISECONDS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum TrayError {
    #[error("Windows accepted only {inserted} of {expected} shell-flyout key events")]
    InputIncomplete { inserted: u32, expected: u32 },

    #[error("the shell-flyout placement worker is unavailable")]
    WorkerUnavailable,
}

pub fn open_overflow(owner: WindowHandle) -> Result<(), TrayError> {
    open_overflow_with_anchor(owner, None)
}

pub fn open_overflow_at(owner: WindowHandle, screen_x: i32) -> Result<(), TrayError> {
    open_overflow_with_anchor(owner, Some(screen_x))
}

fn open_overflow_with_anchor(
    owner: WindowHandle,
    screen_x: Option<i32>,
) -> Result<(), TrayError> {
    input::send(&[
        input::key(VK_LWIN, KEYEVENTF_EXTENDEDKEY),
        input::key(VK_B, KEYBD_EVENT_FLAGS::default()),
        input::key(VK_B, KEYEVENTF_KEYUP),
        input::key(VK_LWIN, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP),
    ])?;

    submit(TrayRequest::Overflow {
        owner: owner.raw().0.addr(),
        screen_x,
        submitted: Instant::now(),
    })
}

pub fn open_quick_settings(owner: WindowHandle) -> Result<bool, TrayError> {
    open_windows_11_panel(owner, None, VK_A)
}

pub fn open_quick_settings_at(
    owner: WindowHandle,
    screen_x: i32,
) -> Result<bool, TrayError> {
    open_windows_11_panel(owner, Some(screen_x), VK_A)
}

pub fn open_calendar(owner: WindowHandle) -> Result<bool, TrayError> {
    open_windows_11_panel(owner, None, VK_N)
}

pub fn open_calendar_at(owner: WindowHandle, screen_x: i32) -> Result<bool, TrayError> {
    open_windows_11_panel(owner, Some(screen_x), VK_N)
}

fn open_windows_11_panel(
    owner: WindowHandle,
    screen_x: Option<i32>,
    key_code: VIRTUAL_KEY,
) -> Result<bool, TrayError> {
    if !discovery::supports_windows_11_panels() {
        return Ok(false);
    }

    let owner_window = owner.raw();
    let Some(_anchor) = discovery::window_anchor(owner_window) else {
        return Ok(true);
    };
    input::send(&[
        input::key(VK_LWIN, KEYEVENTF_EXTENDEDKEY),
        input::key(key_code, KEYBD_EVENT_FLAGS::default()),
        input::key(key_code, KEYEVENTF_KEYUP),
        input::key(VK_LWIN, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP),
    ])?;

    submit(TrayRequest::Panel {
        owner: owner_window.0.addr(),
        screen_x,
        submitted: Instant::now(),
    })
    .map(|()| true)
}

fn submit(request: TrayRequest) -> Result<(), TrayError> {
    coordinator().submit(request)
}

#[derive(Clone, Copy)]
enum TrayRequest {
    Overflow {
        owner: usize,
        screen_x: Option<i32>,
        submitted: Instant,
    },
    Panel {
        owner: usize,
        screen_x: Option<i32>,
        submitted: Instant,
    },
}

struct TrayCoordinator {
    state: Arc<TrayCoordinatorState>,
    running: bool,
}

struct TrayCoordinatorState {
    pending: Mutex<Option<TrayRequest>>,
    wake: Condvar,
}

static COORDINATOR: OnceLock<TrayCoordinator> = OnceLock::new();

fn coordinator() -> &'static TrayCoordinator {
    COORDINATOR.get_or_init(|| {
        let state = Arc::new(TrayCoordinatorState {
            pending: Mutex::new(None),
            wake: Condvar::new(),
        });
        let worker_state = Arc::clone(&state);
        let running = std::thread::Builder::new()
            .name("lotus-tray-placement".to_owned())
            .spawn(move || run_worker(&worker_state))
            .is_ok();

        TrayCoordinator { state, running }
    })
}

impl TrayCoordinator {
    fn submit(&self, request: TrayRequest) -> Result<(), TrayError> {
        if !self.running {
            return Err(TrayError::WorkerUnavailable);
        }

        let mut pending = self
            .state
            .pending
            .lock()
            .map_err(|_| TrayError::WorkerUnavailable)?;
        if pending.replace(request).is_some() {
            METRICS.record_flyout_superseded();
        }
        self.state.wake.notify_one();
        Ok(())
    }
}

fn run_worker(state: &Arc<TrayCoordinatorState>) {
    loop {
        let request = {
            let Ok(mut pending) = state.pending.lock() else {
                return;
            };
            while pending.is_none() {
                pending = match state.wake.wait(pending) {
                    Ok(pending) => pending,
                    Err(_) => return,
                };
            }
            pending.take()
        };

        if let Some(request) = request {
            process_request(request);
        }
    }
}

fn process_request(request: TrayRequest) {
    let worker_started = Instant::now();
    let mut discovery_wait = Duration::ZERO;
    let mut bridge_configuration = Duration::ZERO;
    let mut positioning = Duration::ZERO;
    let mut timed_out = false;
    let mut success = false;
    let submitted = match request {
        TrayRequest::Overflow { submitted, .. } | TrayRequest::Panel { submitted, .. } => {
            submitted
        }
    };

    match request {
        TrayRequest::Overflow {
            owner,
            screen_x,
            submitted: _,
        } => {
            let settle_started = Instant::now();
            std::thread::sleep(FOCUS_SETTLE_TIME);
            discovery_wait = discovery_wait.saturating_add(settle_started.elapsed());
            if let Err(error) = input::send(&[
                input::key(VK_RETURN, KEYBD_EVENT_FLAGS::default()),
                input::key(VK_RETURN, KEYEVENTF_KEYUP),
            ]) {
                log_worker_error(&error);
            }
            let owner = HWND(std::ptr::with_exposed_provenance_mut(owner));
            if let Some(anchor) = discovery::window_anchor(owner) {
                let outcome = placement::place_flyout(
                    screen_x,
                    anchor.0,
                    anchor.1,
                    None,
                    Duration::ZERO,
                    discovery::find_overflow,
                );
                discovery_wait = discovery_wait.saturating_add(outcome.discovery_wait);
                bridge_configuration = outcome.bridge_configuration;
                positioning = outcome.positioning;
                timed_out = outcome.timed_out;
                success = outcome.success;
            }
        }
        TrayRequest::Panel {
            owner,
            screen_x,
            submitted: _,
        } => {
            let owner = HWND(std::ptr::with_exposed_provenance_mut(owner));
            if let Some(anchor) = discovery::window_anchor(owner) {
                let bridge_started = Instant::now();
                let bridge_window = discovery::find_shell_bridge_window();
                let bridge = bridge_window
                    .and_then(|window| ShellBridgeLease::attach(window, owner));
                let bridge_setup = bridge_started.elapsed();
                let outcome = placement::place_flyout(
                    screen_x,
                    anchor.0,
                    anchor.1,
                    bridge.as_ref(),
                    bridge_setup,
                    discovery::find_shell_panel,
                );
                discovery_wait = outcome.discovery_wait;
                bridge_configuration = outcome.bridge_configuration;
                positioning = outcome.positioning;
                timed_out = outcome.timed_out;
                success = outcome.success;
            }
        }
    }

    METRICS.record_flyout_phases(FlyoutPhaseMetrics {
        worker_start: worker_started.duration_since(submitted),
        discovery_wait,
        bridge_configuration,
        positioning,
        total: submitted.elapsed(),
        timeout: timed_out,
        success,
    });
}

fn log_worker_error(error: &dyn std::fmt::Display) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis().min(u128::from(u64::MAX)))
                .unwrap_or(u64::MAX)
        });
    let previous = LAST_WORKER_ERROR_MILLISECONDS.load(Ordering::Relaxed);
    if now.saturating_sub(previous) < WORKER_ERROR_INTERVAL_MILLISECONDS
        || LAST_WORKER_ERROR_MILLISECONDS
            .compare_exchange(previous, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
    {
        return;
    }
    crate::diagnostics::record_message("tray worker", &error.to_string());
}
