use windows::Win32::UI::Input::KeyboardAndMouse::{VK_LWIN, VK_RWIN};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum KeyTransition {
    Down,
    Up,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct KeyEvent {
    pub(super) virtual_key: u16,
    pub(super) transition: KeyTransition,
    pub(super) extended: bool,
    pub(super) self_injected: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ReplayKey {
    pub(super) virtual_key: u16,
    pub(super) transition: KeyTransition,
    pub(super) extended: bool,
}

impl ReplayKey {
    pub(super) const fn down(virtual_key: u16, extended: bool) -> Self {
        Self {
            virtual_key,
            transition: KeyTransition::Down,
            extended,
        }
    }

    pub(super) const fn up(virtual_key: u16, extended: bool) -> Self {
        Self {
            virtual_key,
            transition: KeyTransition::Up,
            extended,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SequenceAction {
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
pub(super) enum HookDecision {
    Pass,
    Suppress,
    Act(SequenceAction),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct WindowsKeySequence {
    pending_windows_key: Option<u16>,
    replayed: bool,
}

impl WindowsKeySequence {
    pub(super) fn transition(&mut self, event: KeyEvent) -> HookDecision {
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

    pub(super) fn cancel(&mut self) -> Option<SequenceAction> {
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
