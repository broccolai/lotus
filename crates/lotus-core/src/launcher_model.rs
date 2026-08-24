use std::ops::Range;

use crate::search::{ApplicationEntry, SearchCatalog, SearchUsage};

pub const DEFAULT_VISIBLE_RESULT_LIMIT: usize = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionMove {
    Previous,
    Next,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorMove {
    Home,
    End,
    Previous,
    Next,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryEdit {
    DeleteBackward,
    DeletePreviousWord,
    DeleteForward,
    MoveCursor(CursorMove),
    SelectAll,
}

#[derive(Clone, Debug)]
pub struct LauncherModel {
    catalog: SearchCatalog,
    query: String,
    results: Vec<ApplicationEntry>,
    selected: Option<usize>,
    result_limit: usize,
    visible_start: usize,
    cursor: usize,
    selection_anchor: Option<usize>,
    usage: SearchUsage,
}

impl LauncherModel {
    pub fn new(result_limit: usize) -> Self {
        Self {
            catalog: SearchCatalog::default(),
            query: String::new(),
            results: Vec::new(),
            selected: None,
            result_limit,
            visible_start: 0,
            cursor: 0,
            selection_anchor: None,
            usage: SearchUsage::default(),
        }
    }

    pub fn with_usage(result_limit: usize, usage: SearchUsage) -> Self {
        Self {
            usage,
            ..Self::new(result_limit)
        }
    }

    pub fn reset(&mut self, catalog: SearchCatalog) {
        self.catalog = catalog;
        self.reset_query();
    }

    pub fn reset_query(&mut self) {
        self.query.clear();
        self.selected = None;
        self.cursor = 0;
        self.selection_anchor = None;
        self.refresh_results();
    }

    pub fn replace_catalog(&mut self, catalog: SearchCatalog) {
        self.catalog = catalog;
        self.refresh_results();
    }

    pub fn set_result_limit(&mut self, result_limit: usize) -> bool {
        if self.result_limit == result_limit {
            return false;
        }
        self.result_limit = result_limit;
        self.refresh_results();
        true
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub const fn query_cursor(&self) -> usize {
        self.cursor
    }

    pub fn query_selection(&self) -> Option<Range<usize>> {
        self.selection_anchor.and_then(|anchor| {
            (anchor != self.cursor)
                .then_some(anchor.min(self.cursor)..anchor.max(self.cursor))
        })
    }

    pub fn results(&self) -> &[ApplicationEntry] {
        let end = self
            .visible_start
            .saturating_add(self.result_limit)
            .min(self.results.len());
        &self.results[self.visible_start.min(end)..end]
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected
            .and_then(|index| index.checked_sub(self.visible_start))
            .filter(|index| *index < self.results().len())
    }

    pub const fn visible_start(&self) -> usize {
        self.visible_start
    }

    pub const fn total_results(&self) -> usize {
        self.results.len()
    }

    pub fn selected_entry(&self) -> Option<&ApplicationEntry> {
        self.selected.and_then(|index| self.results.get(index))
    }

    pub fn record_launch(&mut self, launch_target: &str) -> bool {
        let changed = self.usage.record_launch(launch_target);
        if changed && self.query.is_empty() {
            self.refresh_results();
        }
        changed
    }

    pub const fn usage(&self) -> &SearchUsage {
        &self.usage
    }

    pub fn push_character(&mut self, character: char) {
        let mut encoded = [0; 4];
        let _changed = self.insert_text(character.encode_utf8(&mut encoded));
    }

    pub fn insert_text(&mut self, text: &str) -> bool {
        let range = self.query_selection().unwrap_or(self.cursor..self.cursor);
        if range.is_empty() && text.is_empty() {
            return false;
        }
        self.replace_query_range(range, text);
        true
    }

    pub fn edit_query(&mut self, edit: QueryEdit) -> bool {
        match edit {
            QueryEdit::DeleteBackward => self.delete_backward(),
            QueryEdit::DeletePreviousWord => self.delete_previous_word(),
            QueryEdit::DeleteForward => self.delete_forward(),
            QueryEdit::MoveCursor(movement) => self.move_cursor(movement),
            QueryEdit::SelectAll => self.select_all(),
        }
    }

    pub fn backspace(&mut self) -> bool {
        self.edit_query(QueryEdit::DeleteBackward)
    }

    pub fn move_selection(&mut self, movement: SelectionMove) {
        if self.results.is_empty() {
            self.selected = None;
            return;
        }

        let current = self.selected.unwrap_or(0);
        self.selected = Some(match movement {
            SelectionMove::Previous => current.saturating_sub(1),
            SelectionMove::Next => current.saturating_add(1).min(self.results.len() - 1),
        });
        self.ensure_selected_visible();
    }

    pub fn select_index(&mut self, index: usize) -> bool {
        let Some(index) = self.visible_start.checked_add(index) else {
            return false;
        };
        if index >= self.results.len() || self.selected == Some(index) {
            return false;
        }
        self.selected = Some(index);
        self.ensure_selected_visible();
        true
    }

    fn refresh_results(&mut self) {
        self.results = self
            .catalog
            .search_with_usage(&self.query, usize::MAX, &self.usage)
            .into_iter()
            .cloned()
            .collect();
        self.visible_start = 0;
        self.selected = (self.result_limit != 0 && !self.results.is_empty()).then_some(0);
    }

    fn ensure_selected_visible(&mut self) {
        let Some(selected) = self.selected else {
            self.visible_start = 0;
            return;
        };
        if self.result_limit == 0 {
            self.visible_start = 0;
        } else if selected < self.visible_start {
            self.visible_start = selected;
        } else if selected >= self.visible_start.saturating_add(self.result_limit) {
            self.visible_start =
                selected.saturating_add(1).saturating_sub(self.result_limit);
        }
    }

    fn delete_backward(&mut self) -> bool {
        if let Some(selection) = self.query_selection() {
            self.replace_query_range(selection, "");
            return true;
        }
        if self.cursor == 0 {
            return false;
        }
        self.replace_query_range(self.cursor - 1..self.cursor, "");
        true
    }

    fn delete_forward(&mut self) -> bool {
        if let Some(selection) = self.query_selection() {
            self.replace_query_range(selection, "");
            return true;
        }
        if self.cursor == self.query.chars().count() {
            return false;
        }
        self.replace_query_range(self.cursor..self.cursor + 1, "");
        true
    }

    fn delete_previous_word(&mut self) -> bool {
        if let Some(selection) = self.query_selection() {
            self.replace_query_range(selection, "");
            return true;
        }

        let characters = self.query.chars().collect::<Vec<_>>();
        let mut start = self.cursor;
        while start != 0 && characters[start - 1].is_whitespace() {
            start -= 1;
        }
        while start != 0 && !characters[start - 1].is_whitespace() {
            start -= 1;
        }
        if start == self.cursor {
            return false;
        }

        self.replace_query_range(start..self.cursor, "");
        true
    }

    fn move_cursor(&mut self, movement: CursorMove) -> bool {
        let previous_cursor = self.cursor;
        let previous_anchor = self.selection_anchor;
        let character_count = self.query.chars().count();
        self.cursor = match (movement, self.query_selection()) {
            (CursorMove::Previous, Some(selection)) => selection.start,
            (CursorMove::Next, Some(selection)) => selection.end,
            (CursorMove::Home, _) => 0,
            (CursorMove::End, _) => character_count,
            (CursorMove::Previous, None) => self.cursor.saturating_sub(1),
            (CursorMove::Next, None) => self.cursor.saturating_add(1).min(character_count),
        };
        self.selection_anchor = None;
        self.cursor != previous_cursor || self.selection_anchor != previous_anchor
    }

    fn select_all(&mut self) -> bool {
        let character_count = self.query.chars().count();
        let changed = self.cursor != character_count
            || self.selection_anchor != (character_count != 0).then_some(0);
        self.cursor = character_count;
        self.selection_anchor = (character_count != 0).then_some(0);
        changed
    }

    fn replace_query_range(&mut self, range: Range<usize>, replacement: &str) {
        let byte_range = character_byte_index(&self.query, range.start)
            ..character_byte_index(&self.query, range.end);
        self.query.replace_range(byte_range, replacement);
        self.cursor = range.start + replacement.chars().count();
        self.selection_anchor = None;
        self.refresh_results();
    }
}

fn character_byte_index(text: &str, character_index: usize) -> usize {
    text.char_indices()
        .nth(character_index)
        .map_or(text.len(), |(index, _)| index)
}
