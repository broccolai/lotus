#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Forward,
    Reverse,
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
}

pub struct SwitcherSession<T> {
    items: Vec<T>,
    selected: usize,
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
        Some(Self { items, selected })
    }

    pub fn cycle(&mut self, direction: Direction) {
        self.selected = match direction {
            Direction::Forward => (self.selected + 1) % self.items.len(),
            Direction::Reverse => {
                self.selected.checked_sub(1).unwrap_or(self.items.len() - 1)
            }
        };
    }

    pub fn cycle_by(&mut self, delta: i32) {
        let length = i64::try_from(self.items.len()).unwrap_or(i64::MAX);
        let selected = i64::try_from(self.selected).unwrap_or_default();
        self.selected = usize::try_from((selected + i64::from(delta)).rem_euclid(length))
            .unwrap_or_default();
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

#[cfg(test)]
mod tests {
    use super::RecentOrder;

    #[test]
    fn quick_switch_returns_to_the_previously_used_window() {
        let mut recent = RecentOrder::default();
        recent.record(3);
        assert_eq!(recent.arrange(vec![1, 2, 3], |item| *item), vec![3, 1, 2]);

        recent.record(1);
        assert_eq!(recent.arrange(vec![1, 2, 3], |item| *item), vec![1, 3, 2]);
    }
}
