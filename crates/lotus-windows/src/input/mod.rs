mod capture;
mod health;
mod mailbox;
mod replay;
mod shutdown;
mod state;

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use lotus_switcher::model::Direction;
use thiserror::Error;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer, WM_TIMER};

use crate::NativeError;
use crate::messages::INPUT_WAKE;
use crate::responsiveness::InputFailOpenReason;

const START_TIMEOUT: Duration = Duration::from_secs(2);
const MAILBOX_CAPACITY: usize = 64;
const SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InputConfig {
    pub windows_key_search: bool,
    pub custom_alt_tab: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputAction {
    ToggleSearch {
        sequence: u64,
        captured_at: u64,
    },
    AltTabBegin {
        sequence: u64,
        direction: Direction,
        captured_at: u64,
    },
    AltTabCyclesPending {
        sequence: u64,
    },
    AltTabCommit {
        sequence: u64,
        captured_at: u64,
    },
    AltTabCancel {
        sequence: u64,
    },
}

pub struct InputActionBatch {
    actions: Vec<InputAction>,
    cancelled_sequence: Option<u64>,
    shared: Arc<Shared>,
}

impl InputActionBatch {
    pub fn actions(&self) -> impl Iterator<Item = InputAction> + '_ {
        self.actions.iter().copied()
    }

    pub const fn cancelled_sequence(&self) -> Option<u64> {
        self.cancelled_sequence
    }

    pub fn take_alt_tab_cycles(&self, sequence: u64) -> i32 {
        mailbox::take_cycles(&self.shared, sequence)
    }

    pub fn claim(&self, sequence: u64) -> bool {
        claim_sequence(&self.shared, sequence)
    }

    pub fn reject(&self, sequence: u64) {
        reject_sequence(&self.shared, sequence);
    }
}

#[derive(Debug, Error)]
pub enum InputError {
    #[error(transparent)]
    Native(#[from] NativeError),
    #[error("Lotus input thread did not start")]
    StartTimeout,
    #[error("Lotus input thread stopped during startup")]
    StartStopped,
    #[error("Lotus could not create its input thread: {0}")]
    Thread(#[from] std::io::Error),
}

pub struct InputController {
    receiver: mpsc::Receiver<InputAction>,
    completion: mpsc::Receiver<()>,
    shared: Arc<Shared>,
    thread_id: u32,
    thread: Option<thread::JoinHandle<()>>,
}

impl InputController {
    pub fn start(config: InputConfig) -> Result<Self, InputError> {
        if !config.windows_key_search && !config.custom_alt_tab {
            return Err(InputError::StartStopped);
        }

        let (sender, receiver) = mpsc::sync_channel(MAILBOX_CAPACITY);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let (completion_sender, completion) = mpsc::sync_channel(1);
        let shared = Arc::new(Shared::new());
        let worker_shared = Arc::clone(&shared);
        let ui_thread = unsafe { GetCurrentThreadId() };
        let thread =
            thread::Builder::new()
                .name("lotus-input".into())
                .spawn(move || {
                    capture::input_thread(
                        config,
                        sender,
                        ready_sender,
                        &completion_sender,
                        worker_shared,
                        ui_thread,
                    );
                })?;

        match ready_receiver.recv_timeout(START_TIMEOUT) {
            Ok(Ok(thread_id)) => Ok(Self {
                receiver,
                completion,
                shared,
                thread_id,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                shared.stopping.store(true, Ordering::Release);
                let thread_id = shared.worker_thread.load(Ordering::Acquire);
                shutdown::request_stop(thread_id);
                drop(thread);
                Err(InputError::StartTimeout)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = thread.join();
                Err(InputError::StartStopped)
            }
        }
    }

    pub fn drain_action_batch(&self) -> InputActionBatch {
        let mut actions = self.receiver.try_iter().collect::<Vec<_>>();
        self.shared.wake_pending.store(false, Ordering::Release);
        actions.extend(self.receiver.try_iter());
        mailbox::subtract_depth(&self.shared.mailbox_depth, actions.len());
        let cancelled_sequence = self.take_cancelled_sequence();

        InputActionBatch {
            actions,
            cancelled_sequence,
            shared: Arc::clone(&self.shared),
        }
    }

    pub fn heartbeat(&self) {
        self.shared
            .heartbeat
            .store(health::tick_count(), Ordering::Release);
        health::try_recover_from_fail_open(&self.shared);
    }

    pub fn is_healthy(&self) -> bool {
        !self.shared.fail_open.load(Ordering::Acquire)
    }

    fn take_cancelled_sequence(&self) -> Option<u64> {
        let sequence = self.shared.cancelled_sequence.swap(0, Ordering::AcqRel);
        (sequence != 0).then_some(sequence)
    }
}

pub fn capture_age(captured_at: u64) -> Duration {
    Duration::from_millis(health::tick_count().saturating_sub(captured_at))
}

fn claim_sequence(shared: &Shared, sequence: u64) -> bool {
    let mut decision = lock_decision(shared);
    if shared.fail_open.load(Ordering::Acquire)
        || !health::cleanup_is_complete(shared)
        || sequence < decision.invalid_before
    {
        return false;
    }
    decision.claimed_sequence = decision.claimed_sequence.max(sequence);
    true
}

fn reject_sequence(shared: &Shared, sequence: u64) {
    let mut decision = lock_decision(shared);
    decision.invalid_before = decision.invalid_before.max(sequence.saturating_add(1));
    decision.rejected_sequence = decision.rejected_sequence.max(sequence);
    drop(decision);
    let _ = health::enter_fail_open(shared, InputFailOpenReason::RejectedSequence);
    let _ = health::request_cleanup_for_sequence(shared, sequence);
}

impl Drop for InputController {
    fn drop(&mut self) {
        shutdown::stop(self, SHUTDOWN_TIMEOUT);
    }
}

pub const fn is_input_wake(message: u32) -> bool {
    message == INPUT_WAKE
}

pub struct UiHeartbeatTimer(Option<usize>);

impl UiHeartbeatTimer {
    pub fn start(enabled: bool) -> Result<Self, NativeError> {
        if !enabled {
            return Ok(Self(None));
        }
        let timer = unsafe { SetTimer(None, 0, health::HEARTBEAT_INTERVAL_MS, None) };
        if timer == 0 {
            return Err(windows::core::Error::from_thread().into());
        }
        Ok(Self(Some(timer)))
    }

    pub fn matches(&self, message: u32, parameter: usize) -> bool {
        message == WM_TIMER && self.0 == Some(parameter)
    }

    pub fn set_enabled(&mut self, enabled: bool) -> Result<(), NativeError> {
        if enabled == self.0.is_some() {
            return Ok(());
        }

        if let Some(timer) = self.0.take() {
            let _ = unsafe { KillTimer(None, timer) };
            return Ok(());
        }

        let timer = unsafe { SetTimer(None, 0, health::HEARTBEAT_INTERVAL_MS, None) };
        if timer == 0 {
            return Err(windows::core::Error::from_thread().into());
        }
        self.0 = Some(timer);
        Ok(())
    }
}

impl Drop for UiHeartbeatTimer {
    fn drop(&mut self) {
        if let Some(timer) = self.0 {
            let _ = unsafe { KillTimer(None, timer) };
        }
    }
}

pub(super) struct Shared {
    heartbeat: AtomicU64,
    fail_open: AtomicBool,
    stopping: AtomicBool,
    wake_pending: AtomicBool,
    mailbox_depth: AtomicU32,
    cycle_slots: [AtomicU64; 2],
    cleanup_requested_epoch: AtomicU64,
    cleanup_completed_epoch: AtomicU64,
    cancelled_sequence: AtomicU64,
    cleanup_sequence: AtomicU64,
    decision: Mutex<DecisionState>,
    healthy_heartbeats: AtomicU32,
    worker_thread: AtomicU32,
    pressed_resync_requested: AtomicBool,
}

impl Shared {
    fn new() -> Self {
        Self {
            heartbeat: AtomicU64::new(health::tick_count()),
            fail_open: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            wake_pending: AtomicBool::new(false),
            mailbox_depth: AtomicU32::new(0),
            cycle_slots: std::array::from_fn(|_| AtomicU64::new(0)),
            cleanup_requested_epoch: AtomicU64::new(0),
            cleanup_completed_epoch: AtomicU64::new(0),
            cancelled_sequence: AtomicU64::new(0),
            cleanup_sequence: AtomicU64::new(0),
            decision: Mutex::new(DecisionState::default()),
            healthy_heartbeats: AtomicU32::new(0),
            worker_thread: AtomicU32::new(0),
            pressed_resync_requested: AtomicBool::new(false),
        }
    }
}

#[derive(Default)]
pub(super) struct DecisionState {
    invalid_before: u64,
    claimed_sequence: u64,
    rejected_sequence: u64,
}

pub(super) fn lock_decision(shared: &Shared) -> std::sync::MutexGuard<'_, DecisionState> {
    match shared.decision.lock() {
        Ok(decision) => decision,
        Err(poisoned) => {
            let _ = health::enter_fail_open(shared, InputFailOpenReason::Panic);
            poisoned.into_inner()
        }
    }
}
