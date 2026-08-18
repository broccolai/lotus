use lotus_switcher::model::Direction;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VK_ESCAPE, VK_LMENU, VK_RMENU, VK_SHIFT, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{
    WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use super::AltTabEvent;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Transition {
    Down,
    Up,
}

impl Transition {
    pub(super) const fn from_message(message: u32) -> Option<Self> {
        match message {
            WM_KEYDOWN | WM_SYSKEYDOWN => Some(Self::Down),
            WM_KEYUP | WM_SYSKEYUP => Some(Self::Up),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct KeyEvent {
    pub(super) key: u16,
    pub(super) transition: Transition,
    pub(super) alt_flag: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Decision {
    Pass,
    Suppress,
    Emit(AltTabEvent),
    EmitAndPass(AltTabEvent),
}

#[derive(Default)]
pub(super) struct Sequence {
    modifiers: u8,
    status: SequenceStatus,
    captured: CapturedKey,
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
    pub(super) fn transition(&mut self, event: KeyEvent) -> Decision {
        if event.key == VK_SHIFT.0 {
            self.set_modifier(SHIFT_DOWN, event.transition);
            return Decision::Pass;
        }
        if matches!(event.key, key if key == VK_LMENU.0 || key == VK_RMENU.0) {
            return self.alt(event.transition);
        }
        if event.key == VK_ESCAPE.0
            && (self.status == SequenceStatus::Active
                || self.captured == CapturedKey::Escape)
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
            return if captured {
                Decision::Suppress
            } else {
                Decision::Pass
            };
        }

        self.captured = CapturedKey::Tab;
        let direction = if self.modifier(SHIFT_DOWN) {
            Direction::Reverse
        } else {
            Direction::Forward
        };
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
            return if captured {
                Decision::Suppress
            } else {
                Decision::Pass
            };
        }

        self.cancel();
        self.captured = CapturedKey::Escape;
        Decision::Emit(AltTabEvent::Cancel)
    }

    pub(super) fn cancel(&mut self) -> bool {
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
