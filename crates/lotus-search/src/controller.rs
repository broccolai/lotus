use std::io;
use std::time::Instant;

use lotus_core::launcher_model::{LauncherModel, QueryEdit, SelectionMove};
use lotus_core::search::{ApplicationEntry, SearchCatalog, SearchUsage};

use crate::usage::SearchUsageStore;

const ANIMATION_MILLISECONDS: u128 = 140;
const COMPLETE_PROGRESS: u16 = 1_000;

pub struct SearchController {
    model: LauncherModel,
    usage_store: SearchUsageStore,
    catalog_generation: u64,
}

impl SearchController {
    pub fn new(result_limit: usize, usage: SearchUsage, usage_store: SearchUsageStore) -> Self {
        Self {
            model: LauncherModel::with_usage(result_limit, usage),
            usage_store,
            catalog_generation: 0,
        }
    }

    pub fn begin(&mut self, generation: Option<u64>, catalog: SearchCatalog) {
        if let Some(generation) = generation {
            self.catalog_generation = generation;
        }
        self.model.reset(catalog);
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
    }

    pub fn edit_query(&mut self, edit: QueryEdit) -> bool {
        self.model.edit_query(edit)
    }

    pub fn insert_text(&mut self, text: &str) -> bool {
        self.model.insert_text(text)
    }

    pub fn move_selection(&mut self, direction: SelectionMove) {
        self.model.move_selection(direction);
    }

    pub fn select_index(&mut self, index: usize) -> bool {
        self.model.select_index(index)
    }

    pub fn set_result_limit(&mut self, result_limit: usize) -> bool {
        self.model.set_result_limit(result_limit)
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

    pub fn selected_index(&self) -> Option<usize> {
        self.model.selected_index()
    }

    pub fn visible_start(&self) -> usize {
        self.model.visible_start()
    }

    pub fn total_results(&self) -> usize {
        self.model.total_results()
    }

    pub fn selected_entry(&self) -> Option<&ApplicationEntry> {
        self.model.selected_entry()
    }

    pub fn record_launch(&mut self, launch_target: &str) -> io::Result<()> {
        if self.model.record_launch(launch_target) {
            self.usage_store.save(self.model.usage())?;
        }
        Ok(())
    }
}

pub struct SearchPresentation {
    started: Option<Instant>,
    progress: u16,
}

impl Default for SearchPresentation {
    fn default() -> Self {
        Self { started: None, progress: COMPLETE_PROGRESS }
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
