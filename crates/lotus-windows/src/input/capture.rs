use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};
use std::time::Instant;

use lotus_switcher::model::Direction;
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, HHOOK, KBDLLHOOKSTRUCT, KillTimer, LLKHF_ALTDOWN, MSG,
    PM_NOREMOVE, PeekMessageW, SetTimer, SetWindowsHookExW, UnhookWindowsHookEx,
    WH_KEYBOARD_LL, WM_TIMER,
};

use super::health::{self, HEARTBEAT_INTERVAL_MS};
use super::mailbox::{self, CyclePush};
use super::state::{
    HookDecision, InputSequence, KeyEvent, PressedKeys, SequenceEffect, Transition,
};
use super::{InputAction, InputConfig, InputError, NativeError, Shared, replay};
use crate::messages::{ALT_TAB_FALLBACK_REPLAY, INPUT_RESYNC};
use crate::responsiveness::{InputFailOpenReason, METRICS};

const SUPPRESS: LRESULT = LRESULT(1);

thread_local! {
    pub(super) static HOOK_STATE: RefCell<Option<HookState>> = const { RefCell::new(None) };
}

pub(super) struct HookState {
    pub(super) sequence: InputSequence,
    pub(super) sender: mpsc::SyncSender<InputAction>,
    pub(super) shared: Arc<Shared>,
    pub(super) ui_thread: u32,
    pub(super) alt_fallback: Option<super::state::AltFallback>,
}

struct OwnedHook(HHOOK);

impl Drop for OwnedHook {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWindowsHookEx(self.0);
        }
    }
}

pub(super) fn input_thread(
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

    let initial_pressed = pressed_keys();
    HOOK_STATE.with(|slot| {
        *slot.borrow_mut() = Some(HookState {
            sequence: InputSequence::new(config, initial_pressed),
            sender,
            shared: Arc::clone(&shared),
            ui_thread,
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
        if message.message == ALT_TAB_FALLBACK_REPLAY {
            let _ = replay::flush_alt_tab_fallback();
        }
        if message.message == INPUT_RESYNC {
            health::resync_pressed_keys_if_requested();
        }
        if message.message == WM_TIMER && message.wParam.0 == watchdog {
            health::watchdog_tick();
        }
    }

    let _ = health::enter_fail_open(&shared, InputFailOpenReason::Shutdown);
    let _ = health::request_cleanup(&shared);
    health::cleanup_input_state();
    let _ = replay::flush_alt_tab_fallback();
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
                let _ = health::enter_fail_open(&state.shared, InputFailOpenReason::Panic);
            }
        });
        HookOutcome::Pass
    };
    let result = match decision {
        HookOutcome::Pass => {
            METRICS.record_input_hook_lotus(lotus_started.elapsed());
            call_next(code, message, data)
        }
        HookOutcome::Suppress => {
            METRICS.record_input_hook_lotus(lotus_started.elapsed());
            SUPPRESS
        }
        HookOutcome::PassAfterCancellingStart(shared) => {
            METRICS.record_input_start_cancel_attempt();
            if replay::cancel_native_start() {
                METRICS.record_input_start_cancel_success();
            } else {
                METRICS.record_input_start_cancel_failure();
                let _ = health::enter_fail_open(&shared, InputFailOpenReason::WakeFailure);
            }
            METRICS.record_input_hook_lotus(lotus_started.elapsed());
            call_next(code, message, data)
        }
    };
    METRICS.record_input_callback();
    METRICS.record_input_hook_total(total_started.elapsed());
    result
}

enum HookOutcome {
    Pass,
    Suppress,
    PassAfterCancellingStart(Arc<Shared>),
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
        alt_down: keyboard.flags.contains(LLKHF_ALTDOWN),
        self_injected: keyboard.dwExtraInfo == replay::LOTUS_INPUT_MARKER,
    };

    HOOK_STATE.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        let Some(state) = borrowed.as_mut() else {
            return HookOutcome::Pass;
        };
        if state.shared.fail_open.load(Ordering::Acquire)
            || health::heartbeat_stale(&state.shared)
        {
            state.sequence.capture_fail_open_event(event);
            let _ =
                health::enter_fail_open(&state.shared, InputFailOpenReason::HeartbeatStale);
            return HookOutcome::Pass;
        }
        if state
            .shared
            .pressed_resync_requested
            .load(Ordering::Acquire)
        {
            state.sequence.capture_fail_open_event(event);
            return HookOutcome::Pass;
        }
        if event.self_injected {
            return HookOutcome::Pass;
        }

        let (decision, win_disqualified) = state.sequence.transition(event);
        if win_disqualified {
            METRICS.record_input_win_sequence_disqualified();
        }
        match decision {
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
            HookDecision::EffectAndPassCancellingStart(effect) => {
                METRICS.record_input_win_bare_sequence();
                if accept_effect(state, effect) {
                    HookOutcome::PassAfterCancellingStart(Arc::clone(&state.shared))
                } else {
                    HookOutcome::Pass
                }
            }
        }
    })
}

fn accept_effect(state: &mut HookState, effect: SequenceEffect) -> bool {
    match effect {
        SequenceEffect::Action(action) => {
            let begin_sequence = match action {
                InputAction::AltTabBegin { sequence, .. } => Some(sequence),
                _ => None,
            };
            let accepted = mailbox::enqueue(state, mailbox::stamp_action(action));
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
            match mailbox::push_cycle(&state.shared, sequence, delta) {
                CyclePush::Coalesced => {
                    METRICS.record_input_action_coalesced();
                    true
                }
                CyclePush::Wake
                    if mailbox::enqueue(
                        state,
                        InputAction::AltTabCyclesPending { sequence },
                    ) =>
                {
                    true
                }
                CyclePush::Wake => {
                    mailbox::clear_cycles(&state.shared, sequence);
                    false
                }
                CyclePush::Exhausted => {
                    let _ = health::enter_fail_open(
                        &state.shared,
                        InputFailOpenReason::MailboxFull,
                    );
                    false
                }
            }
        }
    }
}

fn pressed_keys() -> PressedKeys {
    let mut pressed = PressedKeys::default();
    for key in 0_u16..=u16::from(u8::MAX) {
        pressed.set(key, unsafe { GetAsyncKeyState(i32::from(key)) } < 0);
    }
    pressed
}

pub(super) fn resync_pressed_keys() {
    let pressed = pressed_keys();
    HOOK_STATE.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            state.sequence.resync_pressed_keys(pressed);
        }
    });
}

fn call_next(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    unsafe { CallNextHookEx(None, code, message, data) }
}
