use lotus_switcher::model::Direction;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VK_ESCAPE, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RMENU, VK_RSHIFT, VK_RWIN,
    VK_SHIFT, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{
    WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use super::{InputAction, InputConfig};

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
    pub(super) extended: bool,
    pub(super) alt_down: bool,
    pub(super) self_injected: bool,
}
#[derive(Clone, Copy)]
pub(super) struct ReplayKey {
    pub(super) key: u16,
    pub(super) transition: Transition,
    pub(super) extended: bool,
}
#[derive(Clone, Copy)]
pub(super) struct AltFallback {
    pub(super) steps: i32,
    pub(super) alt_is_held: bool,
    pub(super) alt_key: u16,
    pub(super) shift_mask: u8,
}
#[derive(Clone, Copy)]
pub(super) enum SequenceEffect {
    Action(InputAction),
    Cycle(Direction),
    Replay([Option<ReplayKey>; 8]),
}
#[derive(Clone, Copy)]
pub(super) enum HookDecision {
    Pass,
    Suppress,
    Effect(SequenceEffect),
    EffectAndPass(SequenceEffect),
}

pub(super) struct InputSequence {
    config: InputConfig,
    windows: Option<u16>,
    replayed: bool,
    alt_active: bool,
    shift_mask: u8,
    alt_mask: u8,
    last_alt: u16,
    captured: Option<u16>,
    sequence: u64,
    next_sequence: u64,
    pending_windows_replay: Option<(u64, u16)>,
    pending_alt_tab_replay: Option<(u64, i32)>,
    passthrough_windows: u8,
    windows_released_during_fail_open: bool,
}
impl InputSequence {
    pub(super) fn new(config: InputConfig) -> Self {
        Self {
            config,
            windows: None,
            replayed: false,
            alt_active: false,
            shift_mask: 0,
            alt_mask: 0,
            last_alt: VK_LMENU.0,
            captured: None,
            sequence: 0,
            next_sequence: 1,
            pending_windows_replay: None,
            pending_alt_tab_replay: None,
            passthrough_windows: 0,
            windows_released_during_fail_open: false,
        }
    }
    pub(super) const fn active_sequence(&self) -> u64 {
        self.sequence
    }
    pub(super) fn defer_replay_pending_windows(
        &mut self,
        event: KeyEvent,
        replay_pending: bool,
    ) -> bool {
        let bit = windows_bit(event.key);
        if event.transition == Transition::Up && self.passthrough_windows & bit != 0 {
            self.passthrough_windows &= !bit;
            return true;
        }
        if bit != 0
            && event.transition == Transition::Down
            && replay_pending
            && self.windows.is_none()
            && self.pending_alt_tab_replay.is_none()
        {
            self.passthrough_windows |= bit;
            return true;
        }
        false
    }
    pub(super) fn transition(&mut self, event: KeyEvent) -> HookDecision {
        if event.self_injected {
            return HookDecision::Pass;
        }
        self.track_shift(event);
        self.track_alt(event);
        if self.config.custom_alt_tab
            && let Some(decision) = self.alt_tab(event)
        {
            return decision;
        }
        if self.config.windows_key_search {
            return self.windows_key(event);
        }
        HookDecision::Pass
    }
    pub(super) fn fail_open_replay(
        &mut self,
        acknowledged: u64,
    ) -> ([Option<ReplayKey>; 8], u64, Option<AltFallback>) {
        let sequence = self.sequence;
        let mut replay = [None; 8];
        match (self.windows, self.replayed, self.pending_windows_replay) {
            (Some(key), false, _) if self.windows_released_during_fail_open => {
                replay[0] = Some(ReplayKey {
                    key,
                    transition: Transition::Down,
                    extended: true,
                });
                replay[1] = Some(ReplayKey {
                    key,
                    transition: Transition::Up,
                    extended: true,
                });
            }
            (Some(key), false, _) => {
                replay[0] = Some(ReplayKey {
                    key,
                    transition: Transition::Down,
                    extended: true,
                });
            }
            (None, _, Some((pending, key))) if pending > acknowledged => {
                replay[0] = Some(ReplayKey {
                    key,
                    transition: Transition::Down,
                    extended: true,
                });
                replay[1] = Some(ReplayKey {
                    key,
                    transition: Transition::Up,
                    extended: true,
                });
            }
            _ => {}
        }
        let alt_fallback = self.pending_alt_tab_replay.and_then(|(pending, steps)| {
            (pending > acknowledged).then_some(AltFallback {
                steps,
                alt_is_held: self.alt_mask != 0,
                alt_key: self.last_alt,
                shift_mask: self.shift_mask,
            })
        });

        self.windows = None;
        self.replayed = false;
        self.alt_active = false;
        self.captured = None;
        self.pending_windows_replay = None;
        self.pending_alt_tab_replay = None;
        self.windows_released_during_fail_open = false;
        (replay, sequence, alt_fallback)
    }

    pub(super) fn capture_fail_open_release(&mut self, event: KeyEvent) -> bool {
        self.track_shift(event);
        self.track_alt(event);
        let bit = windows_bit(event.key);
        if event.transition == Transition::Up && self.passthrough_windows & bit != 0 {
            self.passthrough_windows &= !bit;
            return false;
        }
        let captures_release = event.transition == Transition::Up
            && !self.replayed
            && self.windows == Some(event.key);
        self.windows_released_during_fail_open |= captures_release;
        captures_release
    }

    pub(super) fn invalidate(&mut self) -> u64 {
        self.alt_active = false;
        self.captured = None;
        self.sequence
    }

    pub(super) fn discard_pending_alt_tab_replay(&mut self, sequence: u64) {
        if self
            .pending_alt_tab_replay
            .is_some_and(|(pending, _)| pending == sequence)
        {
            self.pending_alt_tab_replay = None;
        }
    }
    fn alt_tab(&mut self, event: KeyEvent) -> Option<HookDecision> {
        if event.key == VK_LMENU.0 || event.key == VK_RMENU.0 {
            if event.transition == Transition::Up && self.alt_active {
                self.alt_active = false;
                self.captured = None;
                return Some(HookDecision::EffectAndPass(SequenceEffect::Action(
                    InputAction::AltTabCommit {
                        sequence: self.sequence,
                        captured_at: 0,
                    },
                )));
            }
            return self.alt_active.then_some(HookDecision::Pass);
        }
        if event.key == VK_TAB.0
            && (event.alt_down || self.alt_active || self.captured == Some(VK_TAB.0))
        {
            if event.transition == Transition::Up {
                let captured = self.captured == Some(VK_TAB.0);
                self.captured = None;
                return Some(if captured {
                    HookDecision::Suppress
                } else {
                    HookDecision::Pass
                });
            }
            self.captured = Some(VK_TAB.0);
            let direction = if self.shift_mask != 0 {
                Direction::Reverse
            } else {
                Direction::Forward
            };
            if self.alt_active {
                self.record_alt_tab_cycle(direction);
                return Some(HookDecision::Effect(SequenceEffect::Cycle(direction)));
            }
            self.alt_active = true;
            self.sequence = self.next_sequence;
            self.next_sequence += 1;
            self.pending_alt_tab_replay = Some((self.sequence, direction_delta(direction)));
            return Some(HookDecision::Effect(SequenceEffect::Action(
                InputAction::AltTabBegin {
                    sequence: self.sequence,
                    direction,
                    captured_at: 0,
                },
            )));
        }
        if event.key == VK_ESCAPE.0
            && (self.alt_active || self.captured == Some(VK_ESCAPE.0))
        {
            if event.transition == Transition::Up {
                let captured = self.captured == Some(VK_ESCAPE.0);
                self.captured = None;
                return Some(if captured {
                    HookDecision::Suppress
                } else {
                    HookDecision::Pass
                });
            }
            self.alt_active = false;
            self.captured = Some(VK_ESCAPE.0);
            return Some(HookDecision::Effect(SequenceEffect::Action(
                InputAction::AltTabCancel {
                    sequence: self.sequence,
                },
            )));
        }
        if self.alt_active && event.transition == Transition::Down {
            self.alt_active = false;
            return Some(HookDecision::EffectAndPass(SequenceEffect::Action(
                InputAction::AltTabCancel {
                    sequence: self.sequence,
                },
            )));
        }
        None
    }

    fn track_shift(&mut self, event: KeyEvent) {
        let bit = match event.key {
            key if key == VK_LSHIFT.0 => 0b001,
            key if key == VK_RSHIFT.0 => 0b010,
            key if key == VK_SHIFT.0 => 0b100,
            _ => return,
        };
        match event.transition {
            Transition::Down => self.shift_mask |= bit,
            Transition::Up => self.shift_mask &= !bit,
        }
    }

    fn record_alt_tab_cycle(&mut self, direction: Direction) {
        if let Some((sequence, steps)) = self.pending_alt_tab_replay {
            self.pending_alt_tab_replay =
                Some((sequence, steps.saturating_add(direction_delta(direction))));
        }
    }

    fn track_alt(&mut self, event: KeyEvent) {
        let bit = match event.key {
            key if key == VK_LMENU.0 => 0b001,
            key if key == VK_RMENU.0 => 0b010,
            key if key == VK_MENU.0 => 0b100,
            _ => return,
        };
        match event.transition {
            Transition::Down => {
                self.alt_mask |= bit;
                self.last_alt = event.key;
            }
            Transition::Up => self.alt_mask &= !bit,
        }
    }
    fn windows_key(&mut self, event: KeyEvent) -> HookDecision {
        if event.key == VK_LWIN.0 || event.key == VK_RWIN.0 {
            if event.transition == Transition::Down {
                if self.windows.is_none() && !self.replayed {
                    self.windows = Some(event.key);
                }
                return HookDecision::Suppress;
            }
            let action = if self.replayed {
                self.windows.map(|key| {
                    SequenceEffect::Replay([
                        Some(ReplayKey {
                            key,
                            transition: Transition::Up,
                            extended: true,
                        }),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    ])
                })
            } else {
                self.windows.map(|_| {
                    self.sequence = self.next_sequence;
                    self.next_sequence += 1;
                    self.pending_windows_replay =
                        self.windows.map(|key| (self.sequence, key));
                    SequenceEffect::Action(InputAction::ToggleSearch {
                        sequence: self.sequence,
                        captured_at: 0,
                    })
                })
            };
            self.windows = None;
            self.replayed = false;
            return action.map_or(HookDecision::Suppress, HookDecision::Effect);
        }
        if !self.replayed
            && event.transition == Transition::Down
            && let Some(windows) = self.windows
        {
            self.replayed = true;
            self.pending_windows_replay = None;
            return HookDecision::Effect(SequenceEffect::Replay([
                Some(ReplayKey {
                    key: windows,
                    transition: Transition::Down,
                    extended: true,
                }),
                Some(ReplayKey {
                    key: event.key,
                    transition: Transition::Down,
                    extended: event.extended,
                }),
                None,
                None,
                None,
                None,
                None,
                None,
            ]));
        }
        HookDecision::Pass
    }
}

const fn direction_delta(direction: Direction) -> i32 {
    match direction {
        Direction::Forward => 1,
        Direction::Reverse => -1,
    }
}

const fn windows_bit(key: u16) -> u8 {
    if key == VK_LWIN.0 {
        0b001
    } else if key == VK_RWIN.0 {
        0b010
    } else {
        0
    }
}
