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
    pub(super) alt_down: bool,
    pub(super) self_injected: bool,
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
}

#[derive(Clone, Copy)]
pub(super) enum HookDecision {
    Pass,
    Suppress,
    Effect(SequenceEffect),
    EffectAndPass(SequenceEffect),
    EffectAndPassCancellingStart(SequenceEffect),
}

#[derive(Clone, Copy, Default)]
pub(super) struct PressedKeys([u64; 4]);

#[derive(Clone, Copy)]
struct KeyStateChange {
    was_down: bool,
    is_down: bool,
}

impl PressedKeys {
    pub(super) fn set(&mut self, key: u16, pressed: bool) {
        let word = usize::from(key / 64);
        let mask = 1_u64 << (key % 64);
        if pressed {
            self.0[word] |= mask;
        } else {
            self.0[word] &= !mask;
        }
    }

    fn is_down(&self, key: u16) -> bool {
        self.0[usize::from(key / 64)] & (1_u64 << (key % 64)) != 0
    }

    fn apply(&mut self, event: KeyEvent) -> KeyStateChange {
        let was_down = self.is_down(event.key);
        let is_down = event.transition == Transition::Down;
        self.set(event.key, is_down);
        KeyStateChange { was_down, is_down }
    }

    fn any_non_windows_down(&self) -> bool {
        self.0
            .iter()
            .enumerate()
            .any(|(index, word)| *word & !windows_mask(index) != 0)
    }

    fn any_windows_down(&self) -> bool {
        self.is_down(VK_LWIN.0) || self.is_down(VK_RWIN.0)
    }

    fn another_windows_key_down(&self, key: u16) -> bool {
        [VK_LWIN.0, VK_RWIN.0]
            .into_iter()
            .any(|windows_key| windows_key != key && self.is_down(windows_key))
    }
}

pub(super) struct InputSequence {
    config: InputConfig,
    pressed: PressedKeys,
    win_candidate: Option<u16>,
    win_disqualified: bool,
    alt_active: bool,
    shift_mask: u8,
    alt_mask: u8,
    last_alt: u16,
    captured: Option<u16>,
    sequence: u64,
    next_sequence: u64,
    pending_alt_tab_replay: Option<(u64, i32)>,
}

impl InputSequence {
    pub(super) fn new(config: InputConfig, pressed: PressedKeys) -> Self {
        Self {
            config,
            pressed,
            win_candidate: None,
            win_disqualified: false,
            alt_active: false,
            shift_mask: 0,
            alt_mask: 0,
            last_alt: VK_LMENU.0,
            captured: None,
            sequence: 0,
            next_sequence: 1,
            pending_alt_tab_replay: None,
        }
    }

    pub(super) const fn active_sequence(&self) -> u64 {
        self.sequence
    }

    pub(super) fn resync_pressed_keys(&mut self, pressed: PressedKeys) {
        self.pressed = pressed;
        self.clear_win_candidate();
    }

    pub(super) fn has_active_cleanup_state(&self) -> bool {
        self.captured.is_some() || self.alt_active || self.pending_alt_tab_replay.is_some()
    }

    pub(super) fn transition(&mut self, event: KeyEvent) -> (HookDecision, bool) {
        if event.self_injected {
            return (HookDecision::Pass, false);
        }
        let change = self.pressed.apply(event);
        let repeated_alt_tab = self.config.custom_alt_tab
            && event.key == VK_TAB.0
            && change.was_down
            && change.is_down;
        if repeated_alt_tab {
            if self.alt_active {
                let direction = if self.shift_mask != 0 {
                    Direction::Reverse
                } else {
                    Direction::Forward
                };
                self.record_alt_tab_cycle(direction);
                return (
                    HookDecision::Effect(SequenceEffect::Cycle(direction)),
                    false,
                );
            }
            if event.alt_down || self.captured == Some(VK_TAB.0) {
                return (HookDecision::Suppress, false);
            }
        }
        if change.was_down == change.is_down {
            return (HookDecision::Pass, false);
        }

        self.track_shift(event);
        self.track_alt(event);
        if self.config.custom_alt_tab
            && let Some(decision) = self.alt_tab(event)
        {
            return (decision, false);
        }
        if self.config.windows_key_search {
            return self.windows_key(event);
        }
        (HookDecision::Pass, false)
    }

    pub(super) fn fail_open_cleanup(
        &mut self,
        acknowledged: u64,
    ) -> (u64, Option<AltFallback>) {
        let sequence = self.sequence;
        let alt_fallback = self.pending_alt_tab_replay.and_then(|(pending, steps)| {
            (pending > acknowledged).then_some(AltFallback {
                steps,
                alt_is_held: self.alt_mask != 0,
                alt_key: self.last_alt,
                shift_mask: self.shift_mask,
            })
        });

        self.clear_win_candidate();
        self.alt_active = false;
        self.captured = None;
        self.pending_alt_tab_replay = None;
        (sequence, alt_fallback)
    }

    pub(super) fn capture_fail_open_event(&mut self, event: KeyEvent) {
        if event.self_injected {
            return;
        }
        let change = self.pressed.apply(event);
        if change.was_down == change.is_down {
            return;
        }
        self.track_shift(event);
        self.track_alt(event);
        self.clear_win_candidate();
    }

    pub(super) fn invalidate(&mut self) -> u64 {
        self.clear_win_candidate();
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

    fn windows_key(&mut self, event: KeyEvent) -> (HookDecision, bool) {
        let was_disqualified = self.win_disqualified;
        if is_windows_key(event.key) {
            match event.transition {
                Transition::Down if self.win_candidate.is_none() => {
                    self.win_candidate = Some(event.key);
                    self.win_disqualified = self.pressed.any_non_windows_down()
                        || self.pressed.another_windows_key_down(event.key);
                }
                Transition::Down => self.win_disqualified = true,
                Transition::Up
                    if self.win_candidate == Some(event.key)
                        && !self.win_disqualified
                        && !self.pressed.any_windows_down() =>
                {
                    self.clear_win_candidate();
                    self.sequence = self.next_sequence;
                    self.next_sequence += 1;
                    return (
                        HookDecision::EffectAndPassCancellingStart(SequenceEffect::Action(
                            InputAction::ToggleSearch {
                                sequence: self.sequence,
                                captured_at: 0,
                            },
                        )),
                        false,
                    );
                }
                Transition::Up if !self.pressed.any_windows_down() => {
                    self.clear_win_candidate();
                }
                Transition::Up => {}
            }
        } else if event.transition == Transition::Down && self.win_candidate.is_some() {
            self.win_disqualified = true;
        }

        (
            HookDecision::Pass,
            !was_disqualified && self.win_disqualified,
        )
    }

    fn clear_win_candidate(&mut self) {
        self.win_candidate = None;
        self.win_disqualified = false;
    }
}

const fn direction_delta(direction: Direction) -> i32 {
    match direction {
        Direction::Forward => 1,
        Direction::Reverse => -1,
    }
}

const fn is_windows_key(key: u16) -> bool {
    key == VK_LWIN.0 || key == VK_RWIN.0
}

fn windows_mask(word: usize) -> u64 {
    let mut mask = 0_u64;
    for key in [VK_LWIN.0, VK_RWIN.0] {
        if usize::from(key / 64) == word {
            mask |= 1_u64 << (key % 64);
        }
    }
    mask
}
