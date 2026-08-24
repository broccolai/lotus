#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct RuntimeWork(u16);

impl RuntimeWork {
    pub(super) const WINDOW_EVENTS: Self = Self(1 << 0);
    pub(super) const SETTINGS_EVENTS: Self = Self(1 << 1);
    pub(super) const SWITCHER_EVENTS: Self = Self(1 << 2);
    pub(super) const MONITOR_EVENTS: Self = Self(1 << 3);
    pub(super) const WAKES: Self = Self(1 << 4);
    pub(super) const MONITOR_SYNC: Self = Self(1 << 5);
    pub(super) const FRAME: Self = Self(1 << 6);
    pub(super) const ANIMATION_TICK: Self = Self(1 << 7);

    pub(super) const fn contains(self, work: Self) -> bool {
        self.0 & work.0 != 0
    }

    pub(super) fn insert(&mut self, work: Self) {
        self.0 |= work.0;
    }

    pub(super) const fn needs_event_drain(self) -> bool {
        self.contains(Self::WINDOW_EVENTS)
            || self.contains(Self::SETTINGS_EVENTS)
            || self.contains(Self::SWITCHER_EVENTS)
            || self.contains(Self::MONITOR_EVENTS)
    }
}
