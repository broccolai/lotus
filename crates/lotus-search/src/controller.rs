use std::io;
use std::time::Instant;

use lotus_core::launcher_model::{LauncherModel, QueryEdit, SelectionMove};
use lotus_core::search::{ApplicationEntry, SearchCatalog, SearchUsage};

use crate::command::{CommandEntry, CommandId, command_query, matching_commands};
use crate::usage::SearchUsageStore;

const ANIMATION_MILLISECONDS: u128 = 140;
const COMPLETE_PROGRESS: u16 = 1_000;

pub struct SearchController {
    model: LauncherModel,
    usage_store: SearchUsageStore,
    catalog_generation: u64,
    command_results: Vec<CommandEntry>,
    command_selected: Option<usize>,
    command_visible_start: usize,
    result_limit: usize,
}

impl SearchController {
    pub fn new(
        result_limit: usize,
        usage: SearchUsage,
        usage_store: SearchUsageStore,
    ) -> Self {
        Self {
            model: LauncherModel::with_usage(result_limit, usage),
            usage_store,
            catalog_generation: 0,
            command_results: Vec::new(),
            command_selected: None,
            command_visible_start: 0,
            result_limit,
        }
    }

    pub fn begin(&mut self, generation: Option<u64>, catalog: SearchCatalog) {
        if let Some(generation) = generation {
            self.catalog_generation = generation;
        }
        self.model.reset(catalog);
        self.refresh_commands();
    }

    pub fn refresh_catalog(&mut self, generation: u64, catalog: SearchCatalog) -> bool {
        if generation <= self.catalog_generation {
            return false;
        }
        self.catalog_generation = generation;
        self.model.replace_catalog(catalog);
        true
    }

    pub fn push_character(&mut self, character: char) {
        self.model.push_character(character);
        self.refresh_commands();
    }

    pub fn edit_query(&mut self, edit: QueryEdit) -> bool {
        let changed = self.model.edit_query(edit);
        if changed {
            self.refresh_commands();
        }
        changed
    }

    pub fn insert_text(&mut self, text: &str) -> bool {
        let changed = self.model.insert_text(text);
        if changed {
            self.refresh_commands();
        }
        changed
    }

    pub fn move_selection(&mut self, direction: SelectionMove) {
        if self.is_command_mode() {
            self.move_command_selection(direction);
            return;
        }
        self.model.move_selection(direction);
    }

    pub fn select_index(&mut self, index: usize) -> bool {
        if self.is_command_mode() {
            return self.select_command(index);
        }
        self.model.select_index(index)
    }

    pub fn set_result_limit(&mut self, result_limit: usize) -> bool {
        if self.result_limit == result_limit {
            return false;
        }
        self.result_limit = result_limit;
        let _changed = self.model.set_result_limit(result_limit);
        self.ensure_command_visible();
        true
    }

    pub fn query(&self) -> &str {
        self.model.query()
    }

    pub fn query_cursor(&self) -> usize {
        self.model.query_cursor()
    }

    pub fn results(&self) -> &[ApplicationEntry] {
        self.model.results()
    }

    pub fn commands(&self) -> &[CommandEntry] {
        let end = self
            .command_visible_start
            .saturating_add(self.result_limit)
            .min(self.command_results.len());
        &self.command_results[self.command_visible_start.min(end)..end]
    }

    pub fn is_command_mode(&self) -> bool {
        command_query(self.model.query()).is_some()
    }

    pub fn selected_index(&self) -> Option<usize> {
        if self.is_command_mode() {
            return self
                .command_selected
                .and_then(|index| index.checked_sub(self.command_visible_start))
                .filter(|index| *index < self.commands().len());
        }
        self.model.selected_index()
    }

    pub fn visible_start(&self) -> usize {
        if self.is_command_mode() {
            self.command_visible_start
        } else {
            self.model.visible_start()
        }
    }

    pub fn total_results(&self) -> usize {
        if self.is_command_mode() {
            self.command_results.len()
        } else {
            self.model.total_results()
        }
    }

    pub fn selected_entry(&self) -> Option<&ApplicationEntry> {
        (!self.is_command_mode())
            .then(|| self.model.selected_entry())
            .flatten()
    }

    pub fn selected_command(&self) -> Option<CommandId> {
        self.is_command_mode()
            .then(|| {
                self.command_selected
                    .and_then(|index| self.command_results.get(index))
            })
            .flatten()
            .map(|entry| entry.id)
    }

    pub fn record_launch(&mut self, launch_target: &str) -> io::Result<()> {
        if self.model.record_launch(launch_target) {
            self.usage_store.save(self.model.usage())?;
        }
        Ok(())
    }

    fn refresh_commands(&mut self) {
        let Some(query) = command_query(self.model.query()) else {
            self.command_results.clear();
            self.command_selected = None;
            self.command_visible_start = 0;
            return;
        };

        self.command_results = matching_commands(query);
        self.command_selected = (!self.command_results.is_empty()).then_some(0);
        self.command_visible_start = 0;
    }

    fn move_command_selection(&mut self, direction: SelectionMove) {
        if self.command_results.is_empty() {
            self.command_selected = None;
            return;
        }

        let current = self.command_selected.unwrap_or(0);
        self.command_selected = Some(match direction {
            SelectionMove::Previous => current.saturating_sub(1),
            SelectionMove::Next => current
                .saturating_add(1)
                .min(self.command_results.len() - 1),
        });
        self.ensure_command_visible();
    }

    fn select_command(&mut self, visible_index: usize) -> bool {
        let Some(index) = self.command_visible_start.checked_add(visible_index) else {
            return false;
        };
        if index >= self.command_results.len() || self.command_selected == Some(index) {
            return false;
        }

        self.command_selected = Some(index);
        self.ensure_command_visible();
        true
    }

    fn ensure_command_visible(&mut self) {
        let Some(selected) = self.command_selected else {
            self.command_visible_start = 0;
            return;
        };
        if self.result_limit == 0 {
            self.command_visible_start = 0;
        } else if selected < self.command_visible_start {
            self.command_visible_start = selected;
        } else if selected >= self.command_visible_start.saturating_add(self.result_limit) {
            self.command_visible_start =
                selected.saturating_add(1).saturating_sub(self.result_limit);
        }
    }
}

pub struct SearchPresentation {
    started: Option<Instant>,
    progress: u16,
}

impl Default for SearchPresentation {
    fn default() -> Self {
        Self {
            started: None,
            progress: COMPLETE_PROGRESS,
        }
    }
}

impl SearchPresentation {
    pub fn begin(&mut self) {
        self.started = Some(Instant::now());
        self.progress = 0;
    }

    pub fn finish(&mut self) {
        self.started = None;
        self.progress = COMPLETE_PROGRESS;
    }

    pub const fn is_animating(&self) -> bool {
        self.started.is_some()
    }

    pub const fn progress(&self) -> u16 {
        self.progress
    }

    pub fn advance(&mut self) {
        let Some(started) = self.started else {
            return;
        };
        let elapsed = started.elapsed().as_millis().min(ANIMATION_MILLISECONDS);
        self.progress =
            u16::try_from(elapsed * u128::from(COMPLETE_PROGRESS) / ANIMATION_MILLISECONDS)
                .unwrap_or(COMPLETE_PROGRESS);
        if self.progress == COMPLETE_PROGRESS {
            self.started = None;
        }
    }
}
