#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Forward,
    Reverse,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReconcileOutcome {
    Unchanged,
    Refreshed,
    Pruned { removed: usize },
    Empty { removed: usize },
}

pub struct RecentOrder<K> {
    items: Vec<K>,
}

impl<K> Default for RecentOrder<K> {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}

impl<K: Copy + Eq> RecentOrder<K> {
    pub fn record(&mut self, item: K) {
        self.items.retain(|candidate| *candidate != item);
        self.items.insert(0, item);
    }

    pub fn arrange<T>(&mut self, mut items: Vec<T>, identity: impl Fn(&T) -> K) -> Vec<T> {
        self.items
            .retain(|recent| items.iter().any(|item| identity(item) == *recent));
        items.sort_by_key(|item| {
            self.items
                .iter()
                .position(|recent| *recent == identity(item))
                .unwrap_or(usize::MAX)
        });
        items
    }

    pub fn retain(&mut self, current: impl IntoIterator<Item = K>) {
        let current = current.into_iter().collect::<Vec<_>>();
        self.items
            .retain(|recent| current.iter().any(|candidate| candidate == recent));
    }
}

pub struct SwitcherSession<T> {
    items: Vec<T>,
    selected: usize,
    direction: Direction,
}

impl<T> SwitcherSession<T> {
    pub fn begin(items: Vec<T>, direction: Direction) -> Option<Self> {
        if items.is_empty() {
            return None;
        }
        let selected = match direction {
            Direction::Forward if items.len() > 1 => 1,
            Direction::Forward => 0,
            Direction::Reverse => items.len() - 1,
        };
        Some(Self {
            items,
            selected,
            direction,
        })
    }

    pub fn cycle(&mut self, direction: Direction) {
        self.direction = direction;
        self.selected = match direction {
            Direction::Forward => (self.selected + 1) % self.items.len(),
            Direction::Reverse => {
                self.selected.checked_sub(1).unwrap_or(self.items.len() - 1)
            }
        };
    }

    pub fn cycle_by(&mut self, delta: i32) {
        if delta != 0 {
            self.direction = if delta > 0 {
                Direction::Forward
            } else {
                Direction::Reverse
            };
        }
        let length = i64::try_from(self.items.len()).unwrap_or(i64::MAX);
        let selected = i64::try_from(self.selected).unwrap_or_default();
        self.selected = usize::try_from((selected + i64::from(delta)).rem_euclid(length))
            .unwrap_or_default();
    }

    pub fn reconcile<K: Copy + Eq>(
        &mut self,
        latest: &[T],
        identity: impl Fn(&T) -> K,
    ) -> ReconcileOutcome
    where
        T: Clone + PartialEq,
    {
        let selected_identity = identity(&self.items[self.selected]);
        let mut survivors = Vec::with_capacity(self.items.len());
        let mut survivor_indices = Vec::with_capacity(self.items.len());
        for (index, item) in self.items.iter().enumerate() {
            if let Some(current) = latest
                .iter()
                .find(|current| identity(current) == identity(item))
            {
                survivor_indices.push(index);
                survivors.push(current.clone());
            }
        }

        let removed = self.items.len().saturating_sub(survivors.len());
        if survivors.is_empty() {
            return ReconcileOutcome::Empty { removed };
        }

        let next_selected = survivors
            .iter()
            .position(|item| identity(item) == selected_identity)
            .or_else(|| match self.direction {
                Direction::Forward => survivor_indices
                    .iter()
                    .position(|index| *index > self.selected)
                    .or(Some(0)),
                Direction::Reverse => survivor_indices
                    .iter()
                    .rposition(|index| *index < self.selected)
                    .or(Some(survivor_indices.len() - 1)),
            })
            .unwrap_or_default();
        let changed = self.items != survivors || self.selected != next_selected;
        self.items = survivors;
        self.selected = next_selected;
        if removed != 0 {
            ReconcileOutcome::Pruned { removed }
        } else if changed {
            ReconcileOutcome::Refreshed
        } else {
            ReconcileOutcome::Unchanged
        }
    }

    pub const fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selected(&self) -> &T {
        &self.items[self.selected]
    }

    pub fn items(&self) -> &[T] {
        &self.items
    }
}
