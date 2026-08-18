use std::sync::mpsc::{self, Receiver, Sender, TryIter};
use std::sync::{Arc, Mutex};

use thiserror::Error;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_APP};

use super::hook::{
    OwnedKeyboardHook, claim_active_controller, install_hook, release_active_controller,
    send_replay,
};
use super::sequence::{ReplayKey, SequenceAction, WindowsKeySequence};
use crate::NativeError;

const WINDOWS_KEY_WAKE_MESSAGE: u32 = WM_APP + 0x4C5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowsKeyEvent {
    ToggleRequested,
    ReplayIncomplete { inserted: u32, expected: u32 },
}

#[derive(Debug, Error)]
pub enum WindowsKeyError {
    #[error("another Lotus Windows-key controller is already enabled")]
    AlreadyEnabled,
    #[error(transparent)]
    Native(#[from] NativeError),
}

pub struct WindowsKeyController {
    context: Arc<HookContext>,
    events: Receiver<WindowsKeyEvent>,
    hook: Option<OwnedKeyboardHook>,
}

impl WindowsKeyController {
    pub fn new() -> Self {
        let (events, receiver) = mpsc::channel();
        // SAFETY: GetCurrentThreadId has no preconditions and captures the
        // thread that must own this controller and pump its hook messages.
        let owner_thread = unsafe { GetCurrentThreadId() };

        Self {
            context: Arc::new(HookContext {
                sequence: Mutex::new(WindowsKeySequence::default()),
                events,
                owner_thread,
            }),
            events: receiver,
            hook: None,
        }
    }

    pub fn enable(&mut self) -> Result<bool, WindowsKeyError> {
        if self.hook.is_some() {
            return Ok(false);
        }

        claim_active_controller(&self.context)?;
        let hook =
            install_hook().inspect_err(|_| release_active_controller(&self.context))?;
        self.hook = Some(hook);
        Ok(true)
    }

    pub fn disable(&mut self) {
        if self.hook.is_none() {
            return;
        }

        release_pending_modifier(&self.context);
        self.hook.take();
        release_active_controller(&self.context);
    }

    pub fn drain_events(&self) -> TryIter<'_, WindowsKeyEvent> {
        self.events.try_iter()
    }
}

impl Default for WindowsKeyController {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for WindowsKeyController {
    fn drop(&mut self) {
        self.disable();
    }
}

pub const fn is_windows_key_wake(message: u32) -> bool {
    message == WINDOWS_KEY_WAKE_MESSAGE
}

pub(super) struct HookContext {
    pub(super) sequence: Mutex<WindowsKeySequence>,
    events: Sender<WindowsKeyEvent>,
    owner_thread: u32,
}

pub(super) fn perform_action(context: &HookContext, action: SequenceAction) {
    match action {
        SequenceAction::Toggle => emit_event(context, WindowsKeyEvent::ToggleRequested),
        SequenceAction::ReplayChord {
            windows_key,
            chord_key,
            chord_extended,
        } => {
            let inputs = [
                ReplayKey::down(windows_key, true),
                ReplayKey::down(chord_key, chord_extended),
            ];
            if !send_replay(context, &inputs) {
                let _ = lock(&context.sequence).cancel();
                send_replay(context, &[ReplayKey::up(windows_key, true)]);
            }
        }
        SequenceAction::ReleaseWindows { windows_key } => {
            send_replay(context, &[ReplayKey::up(windows_key, true)]);
        }
    }
}

pub(super) fn emit_event(context: &HookContext, event: WindowsKeyEvent) {
    if context.events.send(event).is_err() {
        return;
    }

    // SAFETY: The controller captured its owning UI thread and posts only
    // value data; no pointer or borrowed state crosses the thread boundary.
    let _ = unsafe {
        PostThreadMessageW(
            context.owner_thread,
            WINDOWS_KEY_WAKE_MESSAGE,
            WPARAM(0),
            LPARAM(0),
        )
    };
}

fn release_pending_modifier(context: &HookContext) {
    let release = lock(&context.sequence).cancel();
    if let Some(SequenceAction::ReleaseWindows { windows_key }) = release {
        send_replay(context, &[ReplayKey::up(windows_key, true)]);
    }
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
