use std::mem::size_of;
use std::sync::mpsc::{self, Receiver, Sender, TryIter};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use thiserror::Error;
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_LWIN, VK_RWIN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HHOOK, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, PostThreadMessageW,
    SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_APP, WM_KEYDOWN, WM_KEYUP,
    WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::NativeError;

const LOTUS_INPUT_MARKER: usize = 0x4C4F_5455;
const SUPPRESS_EVENT: LRESULT = LRESULT(1);

const WINDOWS_KEY_WAKE_MESSAGE: u32 = WM_APP + 0x4C5;

static ACTIVE_CONTROLLER: Mutex<Option<Weak<HookContext>>> = Mutex::new(None);

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

pub const fn is_windows_key_wake(message: u32) -> bool {
    message == WINDOWS_KEY_WAKE_MESSAGE
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

struct HookContext {
    sequence: Mutex<WindowsKeySequence>,
    events: Sender<WindowsKeyEvent>,
    owner_thread: u32,
}

struct OwnedKeyboardHook(HHOOK);

impl Drop for OwnedKeyboardHook {
    fn drop(&mut self) {
        // SAFETY: This guard owns the successful SetWindowsHookExW result and
        // releases it exactly once while the callback function remains loaded.
        unsafe {
            let _ = UnhookWindowsHookEx(self.0);
        }
    }
}

fn install_hook() -> Result<OwnedKeyboardHook, NativeError> {
    // SAFETY: A null module name requests this process module. Its handle stays
    // loaded for the process lifetime.
    let module = unsafe { GetModuleHandleW(None) }?;
    // SAFETY: The callback has the required ABI and static lifetime. Thread id
    // zero is required for the documented global low-level keyboard hook.
    let hook = unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard_hook),
            Some(HINSTANCE(module.0)),
            0,
        )
    }?;
    Ok(OwnedKeyboardHook(hook))
}

fn claim_active_controller(context: &Arc<HookContext>) -> Result<(), WindowsKeyError> {
    let mut active = lock(&ACTIVE_CONTROLLER);
    if active.as_ref().and_then(Weak::upgrade).is_some() {
        return Err(WindowsKeyError::AlreadyEnabled);
    }
    *active = Some(Arc::downgrade(context));
    Ok(())
}

fn release_active_controller(context: &Arc<HookContext>) {
    let mut active = lock(&ACTIVE_CONTROLLER);
    let owns_slot = active
        .as_ref()
        .and_then(Weak::upgrade)
        .is_some_and(|owner| Arc::ptr_eq(&owner, context));
    if owns_slot {
        *active = None;
    }
}

fn release_pending_modifier(context: &HookContext) {
    let release = lock(&context.sequence).cancel();
    if let Some(SequenceAction::ReleaseWindows { windows_key }) = release {
        send_replay(context, &[ReplayKey::up(windows_key, true)]);
    }
}

unsafe extern "system" fn keyboard_hook(
    code: i32,
    message: WPARAM,
    data: LPARAM,
) -> LRESULT {
    if code < 0 || data.0 == 0 {
        return call_next(code, message, data);
    }

    let Some(context) = lock(&ACTIVE_CONTROLLER).as_ref().and_then(Weak::upgrade) else {
        return call_next(code, message, data);
    };
    // SAFETY: For nonnegative WH_KEYBOARD_LL callbacks, Windows documents
    // lParam as a valid pointer to KBDLLHOOKSTRUCT for the callback duration.
    let keyboard = unsafe { &*(data.0 as *const KBDLLHOOKSTRUCT) };
    let Ok(message_id) = u32::try_from(message.0) else {
        return call_next(code, message, data);
    };
    let Some(transition) = transition_from_message(message_id) else {
        return call_next(code, message, data);
    };
    let Ok(virtual_key) = u16::try_from(keyboard.vkCode) else {
        return call_next(code, message, data);
    };
    let event = KeyEvent {
        virtual_key,
        transition,
        extended: keyboard.flags.contains(LLKHF_EXTENDED),
        self_injected: keyboard.dwExtraInfo == LOTUS_INPUT_MARKER,
    };
    let decision = lock(&context.sequence).transition(event);

    match decision {
        HookDecision::Pass => call_next(code, message, data),
        HookDecision::Suppress => SUPPRESS_EVENT,
        HookDecision::Act(action) => {
            perform_action(&context, action);
            SUPPRESS_EVENT
        }
    }
}

fn call_next(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    // SAFETY: Forwarding the untouched callback parameters is required for all
    // events Lotus does not consume. The hook handle parameter is ignored for
    // low-level hooks and may be None.
    unsafe { CallNextHookEx(None, code, message, data) }
}

fn perform_action(context: &HookContext, action: SequenceAction) {
    match action {
        SequenceAction::Toggle => {
            emit_event(context, WindowsKeyEvent::ToggleRequested);
        }
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

fn send_replay(context: &HookContext, replay: &[ReplayKey]) -> bool {
    let inputs = replay
        .iter()
        .copied()
        .map(input_from_replay)
        .collect::<Vec<_>>();
    let expected = u32::try_from(inputs.len()).unwrap_or(u32::MAX);
    // SAFETY: INPUT values are fully initialized keyboard variants and the
    // byte size matches the Win32 INPUT structure used by this crate version.
    let inserted = unsafe {
        SendInput(
            &inputs,
            i32::try_from(size_of::<INPUT>()).unwrap_or(i32::MAX),
        )
    };
    if inserted == expected {
        return true;
    }
    emit_event(
        context,
        WindowsKeyEvent::ReplayIncomplete { inserted, expected },
    );
    false
}

fn emit_event(context: &HookContext, event: WindowsKeyEvent) {
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

fn input_from_replay(replay: ReplayKey) -> INPUT {
    let mut flags = if replay.extended {
        KEYEVENTF_EXTENDEDKEY
    } else {
        KEYBD_EVENT_FLAGS::default()
    };
    if replay.transition == KeyTransition::Up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(replay.virtual_key),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: LOTUS_INPUT_MARKER,
            },
        },
    }
}

fn transition_from_message(message: u32) -> Option<KeyTransition> {
    match message {
        WM_KEYDOWN | WM_SYSKEYDOWN => Some(KeyTransition::Down),
        WM_KEYUP | WM_SYSKEYUP => Some(KeyTransition::Up),
        _ => None,
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KeyTransition {
    Down,
    Up,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct KeyEvent {
    virtual_key: u16,
    transition: KeyTransition,
    extended: bool,
    self_injected: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ReplayKey {
    virtual_key: u16,
    transition: KeyTransition,
    extended: bool,
}

impl ReplayKey {
    const fn down(virtual_key: u16, extended: bool) -> Self {
        Self {
            virtual_key,
            transition: KeyTransition::Down,
            extended,
        }
    }

    const fn up(virtual_key: u16, extended: bool) -> Self {
        Self {
            virtual_key,
            transition: KeyTransition::Up,
            extended,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SequenceAction {
    Toggle,
    ReplayChord {
        windows_key: u16,
        chord_key: u16,
        chord_extended: bool,
    },
    ReleaseWindows {
        windows_key: u16,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HookDecision {
    Pass,
    Suppress,
    Act(SequenceAction),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct WindowsKeySequence {
    pending_windows_key: Option<u16>,
    replayed: bool,
}

impl WindowsKeySequence {
    fn transition(&mut self, event: KeyEvent) -> HookDecision {
        if event.self_injected {
            return HookDecision::Pass;
        }

        if is_windows_key(event.virtual_key) {
            return self.windows_key_transition(event);
        }

        if !self.replayed
            && event.transition == KeyTransition::Down
            && let Some(windows_key) = self.pending_windows_key
        {
            self.replayed = true;
            return HookDecision::Act(SequenceAction::ReplayChord {
                windows_key,
                chord_key: event.virtual_key,
                chord_extended: event.extended,
            });
        }

        HookDecision::Pass
    }

    fn windows_key_transition(&mut self, event: KeyEvent) -> HookDecision {
        if event.transition == KeyTransition::Down {
            if self.pending_windows_key.is_none() && !self.replayed {
                self.pending_windows_key = Some(event.virtual_key);
            }
            return HookDecision::Suppress;
        }

        let action = if self.replayed {
            self.pending_windows_key
                .map(|windows_key| SequenceAction::ReleaseWindows { windows_key })
        } else {
            self.pending_windows_key.map(|_| SequenceAction::Toggle)
        };
        self.reset();
        action.map_or(HookDecision::Suppress, HookDecision::Act)
    }

    fn cancel(&mut self) -> Option<SequenceAction> {
        let release = if self.replayed {
            self.pending_windows_key
                .map(|windows_key| SequenceAction::ReleaseWindows { windows_key })
        } else {
            None
        };
        self.reset();
        release
    }

    fn reset(&mut self) {
        self.pending_windows_key = None;
        self.replayed = false;
    }
}

fn is_windows_key(virtual_key: u16) -> bool {
    virtual_key == VK_LWIN.0 || virtual_key == VK_RWIN.0
}
