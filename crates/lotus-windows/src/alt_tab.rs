use std::sync::mpsc::{self, Receiver, Sender, TryIter};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use lotus_switcher::model::Direction;
use thiserror::Error;
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VK_ESCAPE, VK_LMENU, VK_RMENU, VK_SHIFT, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HHOOK, KBDLLHOOKSTRUCT, LLKHF_ALTDOWN, PostThreadMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_APP, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use lotus_core::window::WindowId;

use crate::NativeError;

const ALT_TAB_WAKE_MESSAGE: u32 = WM_APP + 0x4C7;
const SUPPRESS: LRESULT = LRESULT(1);

static ACTIVE_CONTROLLER: Mutex<Option<Weak<HookContext>>> = Mutex::new(None);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AltTabEvent {
    Begin { direction: Direction, foreground: Option<WindowId> },
    Cycle(Direction),
    Commit,
    Cancel,
}

#[derive(Debug, Error)]
pub enum AltTabError {
    #[error("another Lotus Alt+Tab controller is already enabled")]
    AlreadyEnabled,
    #[error(transparent)]
    Native(#[from] NativeError),
}

pub struct AltTabController {
    context: Arc<HookContext>,
    events: Receiver<AltTabEvent>,
    hook: Option<OwnedHook>,
}

impl AltTabController {
    pub fn new() -> Self {
        let (events, receiver) = mpsc::channel();
        // SAFETY: GetCurrentThreadId has no preconditions and captures the message-loop owner.
        let owner_thread = unsafe { GetCurrentThreadId() };
        Self {
            context: Arc::new(HookContext {
                state: Mutex::new(Sequence::default()),
                events,
                owner_thread,
            }),
            events: receiver,
            hook: None,
        }
    }

    pub fn enable(&mut self) -> Result<bool, AltTabError> {
        if self.hook.is_some() {
            return Ok(false);
        }
        claim(&self.context)?;
        self.hook = Some(install().inspect_err(|_| release(&self.context))?);
        Ok(true)
    }

    pub fn disable(&mut self) {
        if self.hook.take().is_some() {
            if lock(&self.context.state).cancel() {
                emit(&self.context, AltTabEvent::Cancel);
            }
            release(&self.context);
        }
    }

    pub fn drain_events(&self) -> TryIter<'_, AltTabEvent> {
        self.events.try_iter()
    }
}

impl Default for AltTabController {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for AltTabController {
    fn drop(&mut self) {
        self.disable();
    }
}

pub const fn is_alt_tab_wake(message: u32) -> bool {
    message == ALT_TAB_WAKE_MESSAGE
}

struct HookContext {
    state: Mutex<Sequence>,
    events: Sender<AltTabEvent>,
    owner_thread: u32,
}

struct OwnedHook(HHOOK);

impl Drop for OwnedHook {
    fn drop(&mut self) {
        // SAFETY: This guard owns the installed hook and releases it once.
        unsafe {
            let _ = UnhookWindowsHookEx(self.0);
        }
    }
}

fn install() -> Result<OwnedHook, NativeError> {
    // SAFETY: The process module remains loaded for the hook lifetime.
    let module = unsafe { GetModuleHandleW(None) }?;
    // SAFETY: The callback has static lifetime and the required low-level hook ABI.
    let hook = unsafe {
        SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), Some(HINSTANCE(module.0)), 0)
    }?;
    Ok(OwnedHook(hook))
}

fn claim(context: &Arc<HookContext>) -> Result<(), AltTabError> {
    let mut active = lock(&ACTIVE_CONTROLLER);
    if active.as_ref().and_then(Weak::upgrade).is_some() {
        return Err(AltTabError::AlreadyEnabled);
    }
    *active = Some(Arc::downgrade(context));
    Ok(())
}

fn release(context: &Arc<HookContext>) {
    let mut active = lock(&ACTIVE_CONTROLLER);
    if active.as_ref().and_then(Weak::upgrade).is_some_and(|owner| Arc::ptr_eq(&owner, context)) {
        *active = None;
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    if code < 0 || data.0 == 0 {
        return call_next(code, message, data);
    }
    let Some(context) = lock(&ACTIVE_CONTROLLER).as_ref().and_then(Weak::upgrade) else {
        return call_next(code, message, data);
    };
    // SAFETY: Windows supplies KBDLLHOOKSTRUCT for nonnegative low-level keyboard callbacks.
    let keyboard = unsafe { &*(data.0 as *const KBDLLHOOKSTRUCT) };
    let original_message = message;
    let Ok(message) = u32::try_from(original_message.0) else {
        return call_next(code, original_message, data);
    };
    let Some(transition) = Transition::from_message(message) else {
        return call_next(code, WPARAM(message as usize), data);
    };
    let Ok(key) = u16::try_from(keyboard.vkCode) else {
        return call_next(code, WPARAM(message as usize), data);
    };
    let action = lock(&context.state).transition(KeyEvent {
        key,
        transition,
        alt_flag: keyboard.flags.contains(LLKHF_ALTDOWN),
    });
    match action {
        Decision::Pass => call_next(code, WPARAM(message as usize), data),
        Decision::Suppress => SUPPRESS,
        Decision::Emit(event) => {
            emit(&context, event);
            SUPPRESS
        }
        Decision::EmitAndPass(event) => {
            emit(&context, event);
            call_next(code, WPARAM(message as usize), data)
        }
    }
}

fn call_next(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    // SAFETY: Untouched hook parameters must be forwarded for input Lotus does not consume.
    unsafe { CallNextHookEx(None, code, message, data) }
}

fn emit(context: &HookContext, event: AltTabEvent) {
    if context.events.send(event).is_err() {
        return;
    }
    // SAFETY: The message carries no pointer and targets the captured UI thread.
    let _ = unsafe {
        PostThreadMessageW(context.owner_thread, ALT_TAB_WAKE_MESSAGE, WPARAM(0), LPARAM(0))
    };
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Transition {
    Down,
    Up,
}

impl Transition {
    const fn from_message(message: u32) -> Option<Self> {
        match message {
            WM_KEYDOWN | WM_SYSKEYDOWN => Some(Self::Down),
            WM_KEYUP | WM_SYSKEYUP => Some(Self::Up),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
struct KeyEvent {
    key: u16,
    transition: Transition,
    alt_flag: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Decision {
    Pass,
    Suppress,
    Emit(AltTabEvent),
    EmitAndPass(AltTabEvent),
}

struct Sequence {
    modifiers: u8,
    status: SequenceStatus,
    captured: CapturedKey,
}

impl Default for Sequence {
    fn default() -> Self {
        Self { modifiers: 0, status: SequenceStatus::Idle, captured: CapturedKey::None }
    }
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum SequenceStatus {
    #[default]
    Idle,
    Active,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum CapturedKey {
    #[default]
    None,
    Tab,
    Escape,
}

const ALT_DOWN: u8 = 1;
const SHIFT_DOWN: u8 = 2;

impl Sequence {
    fn transition(&mut self, event: KeyEvent) -> Decision {
        if event.key == VK_SHIFT.0 {
            self.set_modifier(SHIFT_DOWN, event.transition);
            return Decision::Pass;
        }
        if matches!(event.key, key if key == VK_LMENU.0 || key == VK_RMENU.0) {
            return self.alt(event.transition);
        }
        if event.key == VK_ESCAPE.0
            && (self.status == SequenceStatus::Active || self.captured == CapturedKey::Escape)
        {
            return self.escape(event.transition);
        }
        if event.key == VK_TAB.0
            && (self.modifier(ALT_DOWN)
                || event.alt_flag
                || self.status == SequenceStatus::Active
                || self.captured == CapturedKey::Tab)
        {
            return self.tab(event.transition);
        }
        if self.status == SequenceStatus::Active && event.transition == Transition::Down {
            self.cancel();
            return Decision::EmitAndPass(AltTabEvent::Cancel);
        }
        Decision::Pass
    }

    fn alt(&mut self, transition: Transition) -> Decision {
        self.set_modifier(ALT_DOWN, transition);
        if transition == Transition::Up && self.status == SequenceStatus::Active {
            self.status = SequenceStatus::Idle;
            self.captured = CapturedKey::None;
            return Decision::EmitAndPass(AltTabEvent::Commit);
        }
        Decision::Pass
    }

    fn tab(&mut self, transition: Transition) -> Decision {
        if transition == Transition::Up {
            let captured = self.captured == CapturedKey::Tab;
            self.captured = CapturedKey::None;
            return if captured { Decision::Suppress } else { Decision::Pass };
        }
        self.captured = CapturedKey::Tab;
        let direction =
            if self.modifier(SHIFT_DOWN) { Direction::Reverse } else { Direction::Forward };
        if self.status == SequenceStatus::Active {
            Decision::Emit(AltTabEvent::Cycle(direction))
        } else {
            self.status = SequenceStatus::Active;
            Decision::Emit(AltTabEvent::Begin {
                direction,
                foreground: crate::activation::foreground_window(),
            })
        }
    }

    fn escape(&mut self, transition: Transition) -> Decision {
        if transition == Transition::Up {
            let captured = self.captured == CapturedKey::Escape;
            self.captured = CapturedKey::None;
            return if captured { Decision::Suppress } else { Decision::Pass };
        }
        self.cancel();
        self.captured = CapturedKey::Escape;
        Decision::Emit(AltTabEvent::Cancel)
    }

    fn cancel(&mut self) -> bool {
        let was_active = self.status == SequenceStatus::Active;
        self.status = SequenceStatus::Idle;
        self.captured = CapturedKey::None;
        was_active
    }

    const fn modifier(&self, modifier: u8) -> bool {
        self.modifiers & modifier != 0
    }

    fn set_modifier(&mut self, modifier: u8, transition: Transition) {
        match transition {
            Transition::Down => self.modifiers |= modifier,
            Transition::Up => self.modifiers &= !modifier,
        }
    }
}
