mod state;

use std::cell::RefCell;
use std::mem::size_of;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use lotus_switcher::model::Direction;
use thiserror::Error;
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, HHOOK, KBDLLHOOKSTRUCT, KillTimer, LLKHF_ALTDOWN,
    LLKHF_EXTENDED, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, SetTimer,
    SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_QUIT, WM_TIMER,
};

use self::state::{
    AltFallback, HookDecision, InputSequence, KeyEvent, ReplayKey, SequenceEffect,
    Transition,
};
use crate::NativeError;
use crate::messages::{INPUT_REPLAY, INPUT_WAKE};
use crate::responsiveness::{InputFailOpenReason, METRICS};

const START_TIMEOUT: Duration = Duration::from_secs(2);
const UI_HEARTBEAT_STALE_MS: u64 = 1_000;
const LOTUS_INPUT_MARKER: usize = 0x4C4F_5455;
const SUPPRESS: LRESULT = LRESULT(1);
const MAILBOX_CAPACITY: usize = 64;
const HEARTBEAT_INTERVAL_MS: u32 = 75;
const REPLAY_CAPACITY: usize = 16;
const SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(500);

thread_local! {
    static HOOK_STATE: RefCell<Option<HookState>> = const { RefCell::new(None) };
}

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
    ReplayIncomplete {
        inserted: u32,
        expected: u32,
    },
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
                    input_thread(
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
                request_stop(thread_id);
                drop(thread);
                Err(InputError::StartTimeout)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = thread.join();
                Err(InputError::StartStopped)
            }
        }
    }

    pub fn drain_actions(&self) -> Vec<InputAction> {
        let mut actions = self.receiver.try_iter().collect::<Vec<_>>();
        self.shared.wake_pending.store(false, Ordering::Release);
        actions.extend(self.receiver.try_iter());
        subtract_mailbox_depth(&self.shared.mailbox_depth, actions.len());
        actions
    }

    pub fn take_alt_tab_cycles(&self, sequence: u64) -> i32 {
        take_cycles(&self.shared, sequence)
    }

    pub fn claim(&self, sequence: u64) -> bool {
        let mut decision = lock_decision(&self.shared);
        if self.shared.fail_open.load(Ordering::Acquire)
            || !cleanup_is_complete(&self.shared)
            || sequence < decision.invalid_before
        {
            return false;
        }
        decision.claimed_sequence = decision.claimed_sequence.max(sequence);
        true
    }

    pub fn reject(&self, sequence: u64) {
        let mut decision = lock_decision(&self.shared);
        decision.invalid_before = decision.invalid_before.max(sequence.saturating_add(1));
        decision.rejected_sequence = decision.rejected_sequence.max(sequence);
        drop(decision);
        let _ = enter_fail_open(&self.shared, InputFailOpenReason::RejectedSequence);
        let _ = request_cleanup_for_sequence(&self.shared, sequence);
    }

    pub fn heartbeat(&self) {
        self.shared.heartbeat.store(tick_count(), Ordering::Release);
        try_recover_from_fail_open(&self.shared);
    }

    pub fn is_healthy(&self) -> bool {
        !self.shared.fail_open.load(Ordering::Acquire)
    }

    pub fn take_cancelled_sequence(&self) -> Option<u64> {
        let sequence = self.shared.cancelled_sequence.swap(0, Ordering::AcqRel);
        (sequence != 0).then_some(sequence)
    }
}

pub fn capture_age(captured_at: u64) -> Duration {
    Duration::from_millis(tick_count().saturating_sub(captured_at))
}

impl Drop for InputController {
    fn drop(&mut self) {
        self.shared.stopping.store(true, Ordering::Release);
        let _ = enter_fail_open(&self.shared, InputFailOpenReason::Shutdown);
        let _ = request_cleanup(&self.shared);
        request_stop(self.thread_id);
        if let Some(thread) = self.thread.take()
            && self.completion.recv_timeout(SHUTDOWN_TIMEOUT).is_ok()
        {
            let _ = thread.join();
        }
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
        let timer = unsafe { SetTimer(None, 0, HEARTBEAT_INTERVAL_MS, None) };
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

        let timer = unsafe { SetTimer(None, 0, HEARTBEAT_INTERVAL_MS, None) };
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

struct Shared {
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
}

impl Shared {
    fn new() -> Self {
        Self {
            heartbeat: AtomicU64::new(tick_count()),
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
        }
    }
}

#[derive(Default)]
struct DecisionState {
    invalid_before: u64,
    claimed_sequence: u64,
    rejected_sequence: u64,
}

fn lock_decision(shared: &Shared) -> std::sync::MutexGuard<'_, DecisionState> {
    match shared.decision.lock() {
        Ok(decision) => decision,
        Err(poisoned) => {
            let _ = enter_fail_open(shared, InputFailOpenReason::Panic);
            poisoned.into_inner()
        }
    }
}

struct HookState {
    sequence: InputSequence,
    sender: mpsc::SyncSender<InputAction>,
    shared: Arc<Shared>,
    ui_thread: u32,
    replay: [Option<ReplayKey>; REPLAY_CAPACITY],
    replay_len: usize,
    alt_fallback: Option<AltFallback>,
}

struct OwnedHook(HHOOK);

impl Drop for OwnedHook {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWindowsHookEx(self.0);
        }
    }
}

fn input_thread(
    config: InputConfig,
    sender: mpsc::SyncSender<InputAction>,
    ready: mpsc::SyncSender<Result<u32, InputError>>,
    completion: &mpsc::SyncSender<()>,
    shared: Arc<Shared>,
    ui_thread: u32,
) {
    let mut message = MSG::default();
    let _ = unsafe { PeekMessageW(&raw mut message, None, 0, 0, PM_NOREMOVE) };
    let thread_id = unsafe { GetCurrentThreadId() };
    shared.worker_thread.store(thread_id, Ordering::Release);

    let hook = match install_hook() {
        Ok(hook) => hook,
        Err(error) => {
            let _ = ready.send(Err(error.into()));
            let _ = completion.send(());
            drop(ready);
            return;
        }
    };

    if shared.stopping.load(Ordering::Acquire) {
        shared.worker_thread.store(0, Ordering::Release);
        let _ = completion.send(());
        return;
    }

    HOOK_STATE.with(|slot| {
        *slot.borrow_mut() = Some(HookState {
            sequence: InputSequence::new(config),
            sender,
            shared: Arc::clone(&shared),
            ui_thread,
            replay: [None; REPLAY_CAPACITY],
            replay_len: 0,
            alt_fallback: None,
        });
    });
    let watchdog = unsafe { SetTimer(None, 0, HEARTBEAT_INTERVAL_MS, None) };
    if watchdog == 0 {
        HOOK_STATE.with(|slot| *slot.borrow_mut() = None);
        drop(hook);
        shared.worker_thread.store(0, Ordering::Release);
        let _ = ready.send(Err(InputError::Native(
            windows::core::Error::from_thread().into(),
        )));
        let _ = completion.send(());
        return;
    }
    let _ = ready.send(Ok(thread_id));
    drop(ready);

    loop {
        let result = unsafe { GetMessageW(&raw mut message, None, 0, 0) }.0;
        if result <= 0 || shared.stopping.load(Ordering::Acquire) {
            break;
        }
        if message.message == INPUT_REPLAY {
            let _ = flush_replay();
        }
        if message.message == WM_TIMER && message.wParam.0 == watchdog {
            watchdog_tick();
        }
    }

    let _ = enter_fail_open(&shared, InputFailOpenReason::Shutdown);
    let _ = request_cleanup(&shared);
    cleanup_input_state();
    let _ = flush_replay();
    let _ = unsafe { KillTimer(None, watchdog) };

    HOOK_STATE.with(|slot| *slot.borrow_mut() = None);
    drop(hook);
    shared.worker_thread.store(0, Ordering::Release);
    let _ = completion.send(());
    drop(shared);
}

fn install_hook() -> Result<OwnedHook, NativeError> {
    let module = unsafe { GetModuleHandleW(None) }?;
    let hook = unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard_hook),
            Some(HINSTANCE(module.0)),
            0,
        )
    }?;
    Ok(OwnedHook(hook))
}

unsafe extern "system" fn keyboard_hook(
    code: i32,
    message: WPARAM,
    data: LPARAM,
) -> LRESULT {
    let total_started = Instant::now();
    let lotus_started = Instant::now();
    let decision = if code < 0 || data.0 == 0 {
        HookOutcome::Pass
    } else if let Ok(result) = catch_unwind(AssertUnwindSafe(|| unsafe {
        keyboard_hook_inner(message, data)
    })) {
        result
    } else {
        HOOK_STATE.with(|slot| {
            if let Ok(state) = slot.try_borrow()
                && let Some(state) = state.as_ref()
            {
                let _ = enter_fail_open(&state.shared, InputFailOpenReason::Panic);
            }
        });
        HookOutcome::Pass
    };
    METRICS.record_input_callback();
    METRICS.record_input_hook_lotus(lotus_started.elapsed());
    let result = match decision {
        HookOutcome::Pass => call_next(code, message, data),
        HookOutcome::Suppress => SUPPRESS,
    };
    METRICS.record_input_hook_total(total_started.elapsed());
    result
}

enum HookOutcome {
    Pass,
    Suppress,
}

unsafe fn keyboard_hook_inner(message: WPARAM, data: LPARAM) -> HookOutcome {
    let keyboard = unsafe { &*(data.0 as *const KBDLLHOOKSTRUCT) };
    let Ok(message_id) = u32::try_from(message.0) else {
        return HookOutcome::Pass;
    };
    let Some(transition) = Transition::from_message(message_id) else {
        return HookOutcome::Pass;
    };
    let Ok(key) = u16::try_from(keyboard.vkCode) else {
        return HookOutcome::Pass;
    };
    let event = KeyEvent {
        key,
        transition,
        extended: keyboard.flags.contains(LLKHF_EXTENDED),
        alt_down: keyboard.flags.contains(LLKHF_ALTDOWN),
        self_injected: keyboard.dwExtraInfo == LOTUS_INPUT_MARKER,
    };

    HOOK_STATE.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        let Some(state) = borrowed.as_mut() else {
            return HookOutcome::Pass;
        };
        if state.shared.fail_open.load(Ordering::Acquire) || heartbeat_stale(&state.shared)
        {
            let captures_release = state.sequence.capture_fail_open_release(event);
            let _ = enter_fail_open(&state.shared, InputFailOpenReason::HeartbeatStale);
            return if captures_release {
                HookOutcome::Suppress
            } else {
                HookOutcome::Pass
            };
        }
        if event.self_injected {
            return HookOutcome::Pass;
        }
        if state
            .sequence
            .defer_replay_pending_windows(event, state.replay_len != 0)
        {
            return HookOutcome::Pass;
        }

        match state.sequence.transition(event) {
            HookDecision::Pass => HookOutcome::Pass,
            HookDecision::Suppress => HookOutcome::Suppress,
            HookDecision::Effect(effect) => {
                if accept_effect(state, effect) {
                    HookOutcome::Suppress
                } else {
                    HookOutcome::Pass
                }
            }
            HookDecision::EffectAndPass(effect) => {
                let _ = accept_effect(state, effect);
                HookOutcome::Pass
            }
        }
    })
}

fn accept_effect(state: &mut HookState, effect: SequenceEffect) -> bool {
    match effect {
        SequenceEffect::Replay(keys) => {
            let key_count = keys.iter().flatten().count();
            if state.replay_len.saturating_add(key_count) > state.replay.len() {
                let _ = enter_fail_open(&state.shared, InputFailOpenReason::WakeFailure);
                return false;
            }
            let replay_start = state.replay_len;
            for key in keys.into_iter().flatten() {
                state.replay[state.replay_len] = Some(key);
                state.replay_len += 1;
            }
            if queue_replay(state.shared.worker_thread.load(Ordering::Acquire)) {
                true
            } else {
                for key in &mut state.replay[replay_start..state.replay_len] {
                    *key = None;
                }
                state.replay_len = replay_start;
                METRICS.record_input_replay_failure();
                let _ = enter_fail_open(&state.shared, InputFailOpenReason::WakeFailure);
                false
            }
        }
        SequenceEffect::Action(action) => {
            let begin_sequence = match action {
                InputAction::AltTabBegin { sequence, .. } => Some(sequence),
                _ => None,
            };
            let accepted = enqueue(state, stamp_action(action));
            if !accepted && let Some(sequence) = begin_sequence {
                state.sequence.discard_pending_alt_tab_replay(sequence);
            }
            accepted
        }
        SequenceEffect::Cycle(direction) => {
            let sequence = state.sequence.active_sequence();
            let delta = if direction == Direction::Forward {
                1
            } else {
                -1
            };
            match push_cycle(&state.shared, sequence, delta) {
                CyclePush::Coalesced => {
                    METRICS.record_input_action_coalesced();
                    true
                }
                CyclePush::Wake
                    if enqueue(state, InputAction::AltTabCyclesPending { sequence }) =>
                {
                    true
                }
                CyclePush::Wake => {
                    clear_cycles(&state.shared, sequence);
                    false
                }
                CyclePush::Exhausted => {
                    let _ =
                        enter_fail_open(&state.shared, InputFailOpenReason::MailboxFull);
                    false
                }
            }
        }
    }
}

enum CyclePush {
    Wake,
    Coalesced,
    Exhausted,
}

fn push_cycle(shared: &Shared, sequence: u64, delta: i32) -> CyclePush {
    let generation = u32::try_from(sequence).unwrap_or(u32::MAX);
    let slot = &shared.cycle_slots[(generation & 1) as usize];
    loop {
        let current = slot.load(Ordering::Acquire);
        let (current_generation, current_delta) = unpack_cycle(current);
        let (next_delta, outcome) = match current_generation {
            0 => (delta, CyclePush::Wake),
            current if current == generation => {
                (current_delta.saturating_add(delta), CyclePush::Coalesced)
            }
            _ => return CyclePush::Exhausted,
        };
        let next = pack_cycle(generation, next_delta);
        if slot
            .compare_exchange(current, next, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return outcome;
        }
    }
}

fn take_cycles(shared: &Shared, sequence: u64) -> i32 {
    let generation = u32::try_from(sequence).unwrap_or(u32::MAX);
    let slot = &shared.cycle_slots[(generation & 1) as usize];
    loop {
        let current = slot.load(Ordering::Acquire);
        let (current_generation, delta) = unpack_cycle(current);
        if current_generation != generation {
            return 0;
        }
        if slot
            .compare_exchange(current, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return delta;
        }
    }
}

fn clear_cycles(shared: &Shared, sequence: u64) {
    let _ = take_cycles(shared, sequence);
}

fn pack_cycle(generation: u32, delta: i32) -> u64 {
    (u64::from(generation) << 32) | u64::from(delta.cast_unsigned())
}

fn unpack_cycle(value: u64) -> (u32, i32) {
    let generation = u32::try_from(value >> 32).unwrap_or_default();
    let delta = u32::try_from(value & u64::from(u32::MAX))
        .unwrap_or_default()
        .cast_signed();
    (generation, delta)
}

fn enqueue(state: &HookState, action: InputAction) -> bool {
    let depth = state.shared.mailbox_depth.fetch_add(1, Ordering::AcqRel) + 1;
    if state.sender.try_send(action).is_err() {
        state.shared.mailbox_depth.fetch_sub(1, Ordering::AcqRel);
        METRICS.record_input_action_dropped();
        let _ = enter_fail_open(&state.shared, InputFailOpenReason::MailboxFull);
        return false;
    }
    METRICS.record_input_action_enqueued();
    METRICS.record_input_mailbox_depth(depth);
    if state.shared.wake_pending.swap(true, Ordering::AcqRel) {
        METRICS.record_input_wake_coalesced();
        return true;
    }
    if unsafe { PostThreadMessageW(state.ui_thread, INPUT_WAKE, WPARAM(0), LPARAM(0)) }
        .is_ok()
    {
        METRICS.record_input_wake_posted();
    } else {
        state.shared.wake_pending.store(false, Ordering::Release);
        METRICS.record_input_wake_failure();
        let _ = enter_fail_open(&state.shared, InputFailOpenReason::WakeFailure);
        return false;
    }
    true
}

fn subtract_mailbox_depth(depth: &AtomicU32, count: usize) {
    let count = u32::try_from(count).unwrap_or(u32::MAX);
    let _ = depth.try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
        Some(current.saturating_sub(count))
    });
}

fn stamp_action(action: InputAction) -> InputAction {
    let captured_at = tick_count();
    match action {
        InputAction::ToggleSearch { sequence, .. } => InputAction::ToggleSearch {
            sequence,
            captured_at,
        },
        InputAction::AltTabBegin {
            sequence,
            direction,
            ..
        } => InputAction::AltTabBegin {
            sequence,
            direction,
            captured_at,
        },
        InputAction::AltTabCommit { sequence, .. } => InputAction::AltTabCommit {
            sequence,
            captured_at,
        },
        other => other,
    }
}

fn queue_replay(thread_id: u32) -> bool {
    if thread_id == 0 {
        return false;
    }

    unsafe { PostThreadMessageW(thread_id, INPUT_REPLAY, WPARAM(0), LPARAM(0)) }.is_ok()
}

fn flush_replay() -> bool {
    HOOK_STATE.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        let Some(state) = borrowed.as_mut() else {
            return true;
        };
        if state.replay_len == 0 && state.alt_fallback.is_none() {
            let requested = state.shared.cleanup_requested_epoch.load(Ordering::Acquire);
            mark_cleanup_complete(&state.shared, requested);
            return true;
        }
        let shared = Arc::clone(&state.shared);
        let requested = shared.cleanup_requested_epoch.load(Ordering::Acquire);
        let mut keys = [None; REPLAY_CAPACITY];
        let mut inputs = [empty_input(); REPLAY_CAPACITY];
        let replay_len = state.replay_len;
        let alt_fallback = state.alt_fallback;
        for (index, key) in state.replay[..replay_len]
            .iter()
            .flatten()
            .copied()
            .enumerate()
        {
            keys[index] = Some(key);
            inputs[index] = input_from_replay(key);
        }
        let expected = u32::try_from(replay_len).unwrap_or_default();
        state.replay = [None; REPLAY_CAPACITY];
        state.replay_len = 0;
        drop(borrowed);
        let inserted = unsafe {
            SendInput(
                &inputs[..replay_len],
                i32::try_from(size_of::<INPUT>()).unwrap_or_default(),
            )
        };
        let inserted_count = usize::try_from(inserted)
            .unwrap_or_default()
            .min(replay_len);
        let retried = if inserted_count < replay_len {
            unsafe {
                SendInput(
                    &inputs[inserted_count..replay_len],
                    i32::try_from(size_of::<INPUT>()).unwrap_or_default(),
                )
            }
        } else {
            0
        };
        let delivered = inserted.saturating_add(retried);
        if delivered != expected {
            METRICS.record_input_replay_failure();
            let delivered_count = usize::try_from(delivered)
                .unwrap_or_default()
                .min(replay_len);
            compensate_replay(&keys[..delivered_count]);
            let mut borrowed = slot.borrow_mut();
            if let Some(state) = borrowed.as_mut() {
                retain_undelivered_replay(state, &keys[..replay_len], delivered_count);
                state
                    .shared
                    .cancelled_sequence
                    .store(state.sequence.active_sequence(), Ordering::Release);
                let _ = enter_fail_open(&state.shared, InputFailOpenReason::WakeFailure);
                let _ = enqueue(
                    state,
                    InputAction::ReplayIncomplete {
                        inserted: delivered,
                        expected,
                    },
                );
            }
            return false;
        }
        if let Some(fallback) = alt_fallback
            && !replay_alt_fallback(fallback)
        {
            METRICS.record_input_replay_failure();
            let mut borrowed = slot.borrow_mut();
            if let Some(state) = borrowed.as_mut() {
                let _ = enter_fail_open(&state.shared, InputFailOpenReason::WakeFailure);
            }
            return false;
        }
        let mut borrowed = slot.borrow_mut();
        if let Some(state) = borrowed.as_mut() {
            state.alt_fallback = None;
        }
        mark_cleanup_complete(&shared, requested);
        true
    })
}

fn retain_undelivered_replay(
    state: &mut HookState,
    replay: &[Option<ReplayKey>],
    delivered: usize,
) {
    let pending = &replay[delivered.min(replay.len())..];
    state.replay = [None; REPLAY_CAPACITY];
    state.replay[..pending.len()].copy_from_slice(pending);
    state.replay_len = pending.len();
}

fn replay_alt_fallback(fallback: AltFallback) -> bool {
    let direction = fallback.steps.signum();
    for _ in 0..fallback.steps.unsigned_abs() {
        let keys = alt_fallback_keys(fallback, direction);
        if !send_replay_keys(&keys) {
            return false;
        }
    }
    true
}

fn alt_fallback_keys(
    fallback: AltFallback,
    direction: i32,
) -> [Option<ReplayKey>; REPLAY_CAPACITY] {
    let reverse = direction < 0;
    let mut keys = [None; REPLAY_CAPACITY];
    let mut index = 0;
    if !reverse {
        index = append_shift_keys(&mut keys, index, fallback.shift_mask, Transition::Up);
    }
    if !fallback.alt_is_held {
        keys[index] = Some(alt_key(fallback.alt_key, Transition::Down));
        index += 1;
    }
    if reverse && fallback.shift_mask == 0 {
        keys[index] = Some(ReplayKey {
            key: 0x10,
            transition: Transition::Down,
            extended: false,
        });
        index += 1;
    }
    keys[index] = Some(ReplayKey {
        key: 0x09,
        transition: Transition::Down,
        extended: false,
    });
    index += 1;
    keys[index] = Some(ReplayKey {
        key: 0x09,
        transition: Transition::Up,
        extended: false,
    });
    index += 1;
    if reverse && fallback.shift_mask == 0 {
        keys[index] = Some(ReplayKey {
            key: 0x10,
            transition: Transition::Up,
            extended: false,
        });
        index += 1;
    }
    if !fallback.alt_is_held {
        keys[index] = Some(alt_key(fallback.alt_key, Transition::Up));
        index += 1;
    }
    if !reverse {
        let _end =
            append_shift_keys(&mut keys, index, fallback.shift_mask, Transition::Down);
    }
    keys
}

fn append_shift_keys(
    keys: &mut [Option<ReplayKey>; REPLAY_CAPACITY],
    mut index: usize,
    mask: u8,
    transition: Transition,
) -> usize {
    for (bit, key) in [(0b001, 0xA0), (0b010, 0xA1), (0b100, 0x10)] {
        if mask & bit != 0 {
            keys[index] = Some(ReplayKey {
                key,
                transition,
                extended: false,
            });
            index += 1;
        }
    }
    index
}

fn alt_key(key: u16, transition: Transition) -> ReplayKey {
    ReplayKey {
        key,
        transition,
        extended: key == 0xA5,
    }
}

fn send_replay_keys(keys: &[Option<ReplayKey>]) -> bool {
    let mut inputs = [empty_input(); REPLAY_CAPACITY];
    let count = keys
        .iter()
        .flatten()
        .enumerate()
        .map(|(index, key)| {
            inputs[index] = input_from_replay(*key);
            index + 1
        })
        .last()
        .unwrap_or_default();
    if count == 0 {
        return true;
    }
    let expected = u32::try_from(count).unwrap_or_default();
    let inserted = unsafe {
        SendInput(
            &inputs[..count],
            i32::try_from(size_of::<INPUT>()).unwrap_or_default(),
        )
    };
    let inserted_count = usize::try_from(inserted).unwrap_or_default().min(count);
    let retry = if inserted_count < count {
        unsafe {
            SendInput(
                &inputs[inserted_count..count],
                i32::try_from(size_of::<INPUT>()).unwrap_or_default(),
            )
        }
    } else {
        0
    };
    inserted.saturating_add(retry) == expected
}

fn compensate_replay(keys: &[Option<ReplayKey>]) {
    let mut inputs = [empty_input(); REPLAY_CAPACITY];
    let mut count = 0;
    for key in keys.iter().flatten() {
        inputs[count] = input_from_replay(ReplayKey {
            key: key.key,
            transition: Transition::Up,
            extended: key.extended,
        });
        count += 1;
    }
    if count != 0 {
        let _ = unsafe {
            SendInput(
                &inputs[..count],
                i32::try_from(size_of::<INPUT>()).unwrap_or_default(),
            )
        };
    }
}

fn empty_input() -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT::default(),
        },
    }
}
fn input_from_replay(key: ReplayKey) -> INPUT {
    let mut flags = if key.extended {
        KEYEVENTF_EXTENDEDKEY
    } else {
        KEYBD_EVENT_FLAGS::default()
    };
    if key.transition == Transition::Up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(key.key),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: LOTUS_INPUT_MARKER,
            },
        },
    }
}
fn heartbeat_stale(shared: &Shared) -> bool {
    tick_count().saturating_sub(shared.heartbeat.load(Ordering::Acquire))
        > UI_HEARTBEAT_STALE_MS
}

fn enter_fail_open(shared: &Shared, reason: InputFailOpenReason) -> bool {
    let was_open = shared.fail_open.swap(true, Ordering::AcqRel);
    if !was_open {
        METRICS.record_input_fail_open(reason);
        let _ = request_cleanup(shared);
    }
    shared.healthy_heartbeats.store(0, Ordering::Release);
    for slot in &shared.cycle_slots {
        slot.store(0, Ordering::Release);
    }
    !was_open
}

fn request_cleanup(shared: &Shared) -> bool {
    let mut requested = shared.cleanup_requested_epoch.load(Ordering::Acquire);
    loop {
        if shared.cleanup_completed_epoch.load(Ordering::Acquire) != requested {
            METRICS.record_input_cleanup_redundant_suppressed();
            return false;
        }
        match shared.cleanup_requested_epoch.compare_exchange_weak(
            requested,
            requested.saturating_add(1),
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                METRICS.record_input_cleanup_requested();
                return true;
            }
            Err(current) => requested = current,
        }
    }
}

fn request_cleanup_for_sequence(shared: &Shared, sequence: u64) -> bool {
    if sequence == 0 {
        return false;
    }
    let mut covered = shared.cleanup_sequence.load(Ordering::Acquire);
    while sequence > covered {
        match shared.cleanup_sequence.compare_exchange_weak(
            covered,
            sequence,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return request_cleanup(shared),
            Err(current) => covered = current,
        }
    }
    METRICS.record_input_cleanup_redundant_suppressed();
    false
}

fn cleanup_is_complete(shared: &Shared) -> bool {
    shared.cleanup_completed_epoch.load(Ordering::Acquire)
        == shared.cleanup_requested_epoch.load(Ordering::Acquire)
}

fn try_recover_from_fail_open(shared: &Shared) {
    let _decision = lock_decision(shared);
    let requested = shared.cleanup_requested_epoch.load(Ordering::Acquire);
    let completed = shared.cleanup_completed_epoch.load(Ordering::Acquire);
    if shared.fail_open.load(Ordering::Acquire) && requested == completed {
        if shared.healthy_heartbeats.fetch_add(1, Ordering::AcqRel) >= 2 {
            shared.fail_open.store(false, Ordering::Release);
            if shared.cleanup_requested_epoch.load(Ordering::Acquire) != completed {
                shared.fail_open.store(true, Ordering::Release);
            }
            shared.healthy_heartbeats.store(0, Ordering::Release);
        }
    } else {
        shared.healthy_heartbeats.store(0, Ordering::Release);
    }
}

fn watchdog_tick() {
    if heartbeat_stale_shared() {
        HOOK_STATE.with(|slot| {
            if let Some(state) = slot.borrow().as_ref() {
                let _ = enter_fail_open(&state.shared, InputFailOpenReason::HeartbeatStale);
            }
        });
    }
    cleanup_input_state();
}

fn heartbeat_stale_shared() -> bool {
    HOOK_STATE.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|state| heartbeat_stale(&state.shared))
    })
}

fn cleanup_input_state() {
    let mut direct_replay = false;
    HOOK_STATE.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        let Some(state) = borrowed.as_mut() else {
            return;
        };
        let requested = state.shared.cleanup_requested_epoch.load(Ordering::Acquire);
        if state.shared.cleanup_completed_epoch.load(Ordering::Acquire) == requested {
            return;
        }

        let active_cleanup_state = state.sequence.has_active_cleanup_state();
        let sequence = state.sequence.invalidate();
        state
            .shared
            .cleanup_sequence
            .fetch_max(sequence, Ordering::AcqRel);
        let (acknowledged, rejected) = {
            let mut decision = lock_decision(&state.shared);
            decision.invalid_before =
                decision.invalid_before.max(sequence.saturating_add(1));
            (
                decision.claimed_sequence,
                decision.rejected_sequence == sequence,
            )
        };
        let (replay, _, alt_fallback) = state.sequence.fail_open_replay(if rejected {
            0
        } else {
            acknowledged
        });
        if state.replay_len == 0 {
            state.replay[..replay.len()].copy_from_slice(&replay);
            state.replay_len = state.replay.iter().flatten().count();
        }
        state.alt_fallback = state.alt_fallback.or(alt_fallback);
        for slot in &state.shared.cycle_slots {
            slot.store(0, Ordering::Release);
        }
        if active_cleanup_state {
            state
                .shared
                .cancelled_sequence
                .store(sequence, Ordering::Release);
            METRICS.record_input_sequence_cancel();
            METRICS.record_input_cleanup_active_sequence_cancel();
            signal_input_wake(state);
        } else {
            METRICS.record_input_cleanup_idle();
        }
        let replay_required = state.replay_len != 0 || state.alt_fallback.is_some();
        if !replay_required {
            mark_cleanup_complete(&state.shared, requested);
        } else if !queue_replay(state.shared.worker_thread.load(Ordering::Acquire)) {
            direct_replay = true;
        }
    });
    if direct_replay {
        let _ = flush_replay();
    }
}

fn mark_cleanup_complete(shared: &Shared, epoch: u64) {
    let _decision = lock_decision(shared);
    if shared.cleanup_requested_epoch.load(Ordering::Acquire) == epoch
        && shared.cleanup_completed_epoch.load(Ordering::Acquire) != epoch
    {
        shared
            .cleanup_completed_epoch
            .store(epoch, Ordering::Release);
        METRICS.record_input_cleanup_completed();
    }
}

fn signal_input_wake(state: &HookState) {
    if state.shared.wake_pending.swap(true, Ordering::AcqRel) {
        return;
    }
    if unsafe { PostThreadMessageW(state.ui_thread, INPUT_WAKE, WPARAM(0), LPARAM(0)) }
        .is_err()
    {
        state.shared.wake_pending.store(false, Ordering::Release);
    }
}
fn tick_count() -> u64 {
    unsafe { GetTickCount64() }
}
fn request_stop(thread_id: u32) {
    if thread_id != 0 {
        let _ = unsafe { PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
    }
}
fn call_next(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    unsafe { CallNextHookEx(None, code, message, data) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_hook_state(config: InputConfig, test: impl FnOnce(&Arc<Shared>)) {
        let (sender, _receiver) = mpsc::sync_channel(MAILBOX_CAPACITY);
        let shared = Arc::new(Shared::new());
        HOOK_STATE.with(|slot| {
            *slot.borrow_mut() = Some(HookState {
                sequence: InputSequence::new(config),
                sender,
                shared: Arc::clone(&shared),
                ui_thread: 0,
                replay: [None; REPLAY_CAPACITY],
                replay_len: 0,
                alt_fallback: None,
            });
        });

        test(&shared);

        HOOK_STATE.with(|slot| *slot.borrow_mut() = None);
    }

    fn make_heartbeat_stale(shared: &Shared) {
        shared
            .heartbeat
            .store(tick_count().saturating_sub(10_000), Ordering::Release);
    }

    #[test]
    fn prolonged_stale_heartbeat_requests_one_cleanup_episode() {
        with_hook_state(InputConfig::default(), |shared| {
            make_heartbeat_stale(shared);
            for _ in 0..134 {
                watchdog_tick();
            }

            assert!(shared.fail_open.load(Ordering::Acquire));
            assert_eq!(shared.cleanup_requested_epoch.load(Ordering::Acquire), 1);
            assert_eq!(shared.cleanup_completed_epoch.load(Ordering::Acquire), 1);
            assert_eq!(shared.cancelled_sequence.load(Ordering::Acquire), 0);
        });
    }

    #[test]
    fn stale_heartbeat_while_idle_neither_cancels_nor_wakes() {
        with_hook_state(InputConfig::default(), |shared| {
            make_heartbeat_stale(shared);
            watchdog_tick();

            assert_eq!(shared.cancelled_sequence.load(Ordering::Acquire), 0);
            assert!(!shared.wake_pending.load(Ordering::Acquire));
        });
    }

    #[test]
    fn recovered_heartbeat_allows_a_second_stale_episode() {
        with_hook_state(InputConfig::default(), |shared| {
            make_heartbeat_stale(shared);
            watchdog_tick();
            for _ in 0..3 {
                try_recover_from_fail_open(shared);
            }
            assert!(!shared.fail_open.load(Ordering::Acquire));

            make_heartbeat_stale(shared);
            watchdog_tick();

            assert!(shared.fail_open.load(Ordering::Acquire));
            assert_eq!(shared.cleanup_requested_epoch.load(Ordering::Acquire), 2);
            assert_eq!(shared.cleanup_completed_epoch.load(Ordering::Acquire), 2);
        });
    }

    #[test]
    fn queued_cleanup_cannot_recover_before_replay_completion() {
        let shared = Shared::new();
        let _ = enter_fail_open(&shared, InputFailOpenReason::HeartbeatStale);

        for _ in 0..4 {
            try_recover_from_fail_open(&shared);
        }
        assert!(shared.fail_open.load(Ordering::Acquire));

        mark_cleanup_complete(&shared, 1);
        for _ in 0..3 {
            try_recover_from_fail_open(&shared);
        }
        assert!(!shared.fail_open.load(Ordering::Acquire));
    }

    #[test]
    fn new_rejected_sequence_can_request_a_later_cleanup_generation() {
        let shared = Shared::new();
        let _ = enter_fail_open(&shared, InputFailOpenReason::RejectedSequence);
        mark_cleanup_complete(&shared, 1);

        assert!(request_cleanup_for_sequence(&shared, 1));
        mark_cleanup_complete(&shared, 2);
        assert!(!request_cleanup_for_sequence(&shared, 1));
        assert!(request_cleanup_for_sequence(&shared, 2));
        assert_eq!(shared.cleanup_requested_epoch.load(Ordering::Acquire), 3);
    }

    #[test]
    fn stale_heartbeat_cancels_a_captured_windows_sequence_once() {
        with_hook_state(
            InputConfig {
                windows_key_search: true,
                custom_alt_tab: false,
            },
            |shared| {
                HOOK_STATE.with(|slot| {
                    let mut borrowed = slot.borrow_mut();
                    let state = borrowed.as_mut().expect("test hook state");
                    let _ = state.sequence.transition(KeyEvent {
                        key: 0x5B,
                        transition: Transition::Down,
                        extended: true,
                        alt_down: false,
                        self_injected: false,
                    });
                    let _ = state.sequence.transition(KeyEvent {
                        key: 0x5B,
                        transition: Transition::Up,
                        extended: true,
                        alt_down: false,
                        self_injected: false,
                    });
                });
                make_heartbeat_stale(shared);
                watchdog_tick();
                watchdog_tick();

                assert_eq!(shared.cancelled_sequence.load(Ordering::Acquire), 1);
                assert_eq!(shared.cleanup_requested_epoch.load(Ordering::Acquire), 1);
            },
        );
    }

    #[test]
    fn fail_open_extracts_one_alt_tab_fallback_and_clears_the_sequence() {
        let mut sequence = InputSequence::new(InputConfig {
            windows_key_search: false,
            custom_alt_tab: true,
        });
        let _ = sequence.transition(KeyEvent {
            key: 0xA4,
            transition: Transition::Down,
            extended: false,
            alt_down: true,
            self_injected: false,
        });
        let _ = sequence.transition(KeyEvent {
            key: 0x09,
            transition: Transition::Down,
            extended: false,
            alt_down: true,
            self_injected: false,
        });

        let (_, _, fallback) = sequence.fail_open_replay(0);
        let fallback = fallback.expect("captured Alt+Tab must retain a native fallback");
        assert_eq!(fallback.steps, 1);
        assert!(fallback.alt_is_held);
        assert!(!sequence.has_active_cleanup_state());

        let (_, _, repeated) = sequence.fail_open_replay(0);
        assert!(repeated.is_none());
    }

    #[test]
    fn partial_replay_retries_only_the_undelivered_suffix() {
        with_hook_state(InputConfig::default(), |_| {
            HOOK_STATE.with(|slot| {
                let mut borrowed = slot.borrow_mut();
                let state = borrowed.as_mut().expect("test hook state");
                let replay = [
                    Some(ReplayKey {
                        key: 0x5B,
                        transition: Transition::Down,
                        extended: true,
                    }),
                    Some(ReplayKey {
                        key: 0x5B,
                        transition: Transition::Up,
                        extended: true,
                    }),
                ];

                retain_undelivered_replay(state, &replay, 1);

                assert_eq!(state.replay_len, 1);
                let pending = state.replay[0].expect("key-up suffix must remain pending");
                assert_eq!(pending.key, 0x5B);
                assert_eq!(pending.transition, Transition::Up);
            });
        });
    }
}
