mod discovery;
mod input;
mod placement;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use thiserror::Error;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_A, VK_LWIN,
    VK_N, VK_RETURN,
};

use self::placement::PlacementOutcome;
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrayIntegrationHealth {
    Healthy,
    Degraded,
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
    coordinator().send_input(&[
        input::key(VK_LWIN, KEYEVENTF_EXTENDEDKEY),
        input::key(VK_B, KEYBD_EVENT_FLAGS::default()),
        input::key(VK_B, KEYEVENTF_KEYUP),
        input::key(VK_LWIN, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP),
    ])?;

    submit(TrayRequest::Overflow {
        owner: owner.raw().0.addr(),
        screen_x,
        submitted: Instant::now(),
        epoch: 0,
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
    coordinator().send_input(&[
        input::key(VK_LWIN, KEYEVENTF_EXTENDEDKEY),
        input::key(key_code, KEYBD_EVENT_FLAGS::default()),
        input::key(key_code, KEYEVENTF_KEYUP),
        input::key(VK_LWIN, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP),
    ])?;

    submit(TrayRequest::Panel {
        owner: owner_window.0.addr(),
        screen_x,
        submitted: Instant::now(),
        epoch: 0,
    })
    .map(|()| true)
}

fn submit(request: TrayRequest) -> Result<(), TrayError> {
    coordinator().submit(request)
}

pub fn recover() -> TrayIntegrationHealth {
    coordinator().recover()
}

pub fn current_health() -> Option<TrayIntegrationHealth> {
    COORDINATOR.get().map(|coordinator| {
        if coordinator.state.worker_running.load(Ordering::Acquire) {
            TrayIntegrationHealth::Healthy
        } else {
            TrayIntegrationHealth::Degraded
        }
    })
}

#[derive(Clone, Copy)]
enum TrayRequest {
    Overflow {
        owner: usize,
        screen_x: Option<i32>,
        submitted: Instant,
        epoch: u64,
    },
    Panel {
        owner: usize,
        screen_x: Option<i32>,
        submitted: Instant,
        epoch: u64,
    },
}

struct TrayCoordinator {
    state: Arc<TrayCoordinatorState>,
}

struct TrayCoordinatorState {
    pending: Mutex<Option<TrayRequest>>,
    wake: Condvar,
    worker_running: AtomicBool,
    recovery_epoch: AtomicU64,
    side_effect: Mutex<()>,
}

static COORDINATOR: OnceLock<TrayCoordinator> = OnceLock::new();

fn coordinator() -> &'static TrayCoordinator {
    COORDINATOR.get_or_init(|| {
        let state = Arc::new(TrayCoordinatorState {
            pending: Mutex::new(None),
            wake: Condvar::new(),
            worker_running: AtomicBool::new(false),
            recovery_epoch: AtomicU64::new(0),
            side_effect: Mutex::new(()),
        });
        let coordinator = TrayCoordinator { state };
        let _ = coordinator.ensure_worker();
        coordinator
    })
}

impl TrayCoordinator {
    fn submit(&self, request: TrayRequest) -> Result<(), TrayError> {
        if !self.ensure_worker() {
            return Err(TrayError::WorkerUnavailable);
        }

        let mut pending = self
            .state
            .pending
            .lock()
            .map_err(|_| TrayError::WorkerUnavailable)?;
        let request = request.with_epoch(self.state.recovery_epoch.load(Ordering::Acquire));
        if pending.replace(request).is_some() {
            METRICS.record_flyout_superseded();
        }
        self.state.wake.notify_one();
        Ok(())
    }

    fn recover(&self) -> TrayIntegrationHealth {
        self.state.recovery_epoch.fetch_add(1, Ordering::AcqRel);
        let side_effect_drained = self.state.side_effect.lock().is_ok();
        let pending_cleared = self
            .state
            .pending
            .lock()
            .is_ok_and(|mut pending| pending.take().is_some());
        if pending_cleared {
            crate::diagnostics::record_diagnostic(
                "tray.recovery_pending_cleared",
                "a stale flyout placement request was discarded",
            );
        }
        self.state.wake.notify_all();

        if side_effect_drained && self.ensure_worker() {
            crate::diagnostics::record_diagnostic("tray.recovery", "worker=healthy");
            TrayIntegrationHealth::Healthy
        } else {
            crate::diagnostics::record_diagnostic("tray.recovery", "worker=degraded");
            TrayIntegrationHealth::Degraded
        }
    }

    fn send_input(
        &self,
        inputs: &[windows::Win32::UI::Input::KeyboardAndMouse::INPUT],
    ) -> Result<(), TrayError> {
        let epoch = self.state.recovery_epoch.load(Ordering::Acquire);
        self.with_current_side_effect(epoch, || input::send(inputs))
            .ok_or(TrayError::WorkerUnavailable)??;
        Ok(())
    }

    fn with_current_side_effect<T>(
        &self,
        epoch: u64,
        effect: impl FnOnce() -> T,
    ) -> Option<T> {
        with_current_side_effect(&self.state, epoch, effect)
    }

    fn ensure_worker(&self) -> bool {
        if self.state.worker_running.load(Ordering::Acquire) {
            return true;
        }
        if self
            .state
            .worker_running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return true;
        }

        let worker_state = Arc::clone(&self.state);
        let spawned = std::thread::Builder::new()
            .name("lotus-tray-placement".to_owned())
            .spawn(move || run_worker(&worker_state))
            .is_ok();
        if !spawned {
            self.state.worker_running.store(false, Ordering::Release);
            crate::diagnostics::record_diagnostic(
                "tray.worker_start_failed",
                "worker=unavailable",
            );
        }
        spawned
    }
}

fn run_worker(state: &Arc<TrayCoordinatorState>) {
    let _running = WorkerRunningGuard { state };
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

        if let Some(request) = request
            && std::panic::catch_unwind(|| process_request(state, request)).is_err()
        {
            crate::diagnostics::record_diagnostic(
                "tray.worker_request_failed",
                "panic=contained",
            );
        }
    }
}

struct WorkerRunningGuard<'a> {
    state: &'a TrayCoordinatorState,
}

impl Drop for WorkerRunningGuard<'_> {
    fn drop(&mut self) {
        self.state.worker_running.store(false, Ordering::Release);
    }
}

impl TrayRequest {
    const fn with_epoch(self, epoch: u64) -> Self {
        match self {
            Self::Overflow {
                owner,
                screen_x,
                submitted,
                ..
            } => Self::Overflow {
                owner,
                screen_x,
                submitted,
                epoch,
            },
            Self::Panel {
                owner,
                screen_x,
                submitted,
                ..
            } => Self::Panel {
                owner,
                screen_x,
                submitted,
                epoch,
            },
        }
    }

    const fn epoch(self) -> u64 {
        match self {
            Self::Overflow { epoch, .. } | Self::Panel { epoch, .. } => epoch,
        }
    }
}

fn process_request(state: &TrayCoordinatorState, request: TrayRequest) {
    let worker_started = Instant::now();
    let submitted = match request {
        TrayRequest::Overflow { submitted, .. } | TrayRequest::Panel { submitted, .. } => {
            submitted
        }
    };
    let outcome = match request {
        TrayRequest::Overflow {
            owner,
            screen_x,
            submitted: _,
            epoch: _,
        } => process_overflow(state, request.epoch(), owner, screen_x),
        TrayRequest::Panel {
            owner,
            screen_x,
            submitted: _,
            epoch: _,
        } => process_panel(state, request.epoch(), owner, screen_x),
    };
    record_flyout_phases(worker_started, submitted, outcome);
}

fn process_overflow(
    state: &TrayCoordinatorState,
    epoch: u64,
    owner: usize,
    screen_x: Option<i32>,
) -> PlacementOutcome {
    let settle_started = Instant::now();
    std::thread::sleep(FOCUS_SETTLE_TIME);
    let settled = settle_started.elapsed();
    if !current_epoch_matches(state, epoch) {
        return PlacementOutcome::cancelled(settled);
    }
    if let Some(Err(error)) = with_current_side_effect(state, epoch, || {
        input::send(&[
            input::key(VK_RETURN, KEYBD_EVENT_FLAGS::default()),
            input::key(VK_RETURN, KEYEVENTF_KEYUP),
        ])
    }) {
        log_worker_error(&error);
    }
    let owner = HWND(std::ptr::with_exposed_provenance_mut(owner));
    let Some(anchor) =
        with_current_side_effect(state, epoch, || discovery::window_anchor(owner))
            .flatten()
    else {
        return PlacementOutcome::cancelled(settled);
    };
    let mut outcome = placement::place_flyout(
        placement::PlacementRequest {
            screen_x,
            anchor_x: anchor.0,
            anchor_y: anchor.1,
            bridge: None,
            bridge_setup: Duration::ZERO,
        },
        discovery::find_overflow,
        || current_epoch_matches(state, epoch),
        |window, x, y| {
            with_current_side_effect(state, epoch, || {
                placement::set_window_position(window, x, y)
            })
            .unwrap_or(false)
        },
        |_bridge, _x, _y| false,
    );
    outcome.discovery_wait = outcome.discovery_wait.saturating_add(settled);
    outcome
}

fn process_panel(
    state: &TrayCoordinatorState,
    epoch: u64,
    owner: usize,
    screen_x: Option<i32>,
) -> PlacementOutcome {
    let owner = HWND(std::ptr::with_exposed_provenance_mut(owner));
    let Some(anchor) =
        with_current_side_effect(state, epoch, || discovery::window_anchor(owner))
            .flatten()
    else {
        return PlacementOutcome::cancelled(Duration::ZERO);
    };
    let bridge_started = Instant::now();
    let bridge_window = discovery::find_shell_bridge_window();
    let bridge = bridge_window.and_then(|window| {
        with_current_side_effect(state, epoch, || ShellBridgeLease::attach(window, owner))
            .flatten()
    });
    placement::place_flyout(
        placement::PlacementRequest {
            screen_x,
            anchor_x: anchor.0,
            anchor_y: anchor.1,
            bridge: bridge.as_ref(),
            bridge_setup: bridge_started.elapsed(),
        },
        discovery::find_shell_panel,
        || current_epoch_matches(state, epoch),
        |window, x, y| {
            with_current_side_effect(state, epoch, || {
                placement::set_window_position(window, x, y)
            })
            .unwrap_or(false)
        },
        |bridge, x, y| {
            with_current_side_effect(state, epoch, || bridge.configure(x, y))
                .unwrap_or(false)
        },
    )
}

fn record_flyout_phases(
    worker_started: Instant,
    submitted: Instant,
    outcome: PlacementOutcome,
) {
    METRICS.record_flyout_phases(FlyoutPhaseMetrics {
        worker_start: worker_started.duration_since(submitted),
        discovery_wait: outcome.discovery_wait,
        bridge_configuration: outcome.bridge_configuration,
        positioning: outcome.positioning,
        total: submitted.elapsed(),
        timeout: outcome.timed_out,
        success: outcome.success,
    });
}

fn current_epoch_matches(state: &TrayCoordinatorState, epoch: u64) -> bool {
    state
        .side_effect
        .lock()
        .is_ok_and(|_| state.recovery_epoch.load(Ordering::Acquire) == epoch)
}

fn with_current_side_effect<T>(
    state: &TrayCoordinatorState,
    epoch: u64,
    effect: impl FnOnce() -> T,
) -> Option<T> {
    let _permit = state.side_effect.lock().ok()?;
    (state.recovery_epoch.load(Ordering::Acquire) == epoch).then(effect)
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
