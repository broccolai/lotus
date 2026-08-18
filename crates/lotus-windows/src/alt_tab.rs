mod hook;
mod sequence;

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryIter};

use lotus_core::window::WindowId;
use lotus_switcher::model::Direction;
use thiserror::Error;
use windows::Win32::UI::WindowsAndMessaging::WM_APP;

use self::hook::{HookContext, OwnedHook, claim, install, release};
use crate::NativeError;

const ALT_TAB_WAKE_MESSAGE: u32 = WM_APP + 0x4C7;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AltTabEvent {
    Begin {
        direction: Direction,
        foreground: Option<WindowId>,
    },
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

        Self {
            context: Arc::new(HookContext::new(events)),
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
        if self.hook.take().is_none() {
            return;
        }

        if self.context.cancel() {
            self.context.emit(AltTabEvent::Cancel);
        }
        release(&self.context);
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
