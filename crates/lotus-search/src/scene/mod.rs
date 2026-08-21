use std::num::NonZeroU32;

use lotus_ui::icon::Icon;
use lotus_ui::theme::Theme;

use crate::controller::SearchMode;

mod layout;

pub use layout::{LauncherLayout, LauncherSize, PixelRect};

const COMPLETE_PROGRESS: u16 = 1_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LauncherResultKind {
    Application,
    Command,
    Calculator,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LauncherResult<Asset> {
    pub title: String,
    pub icon: Option<Icon<Asset>>,
    pub kind: LauncherResultKind,
}

impl<Asset> LauncherResult<Asset> {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            icon: None,
            kind: LauncherResultKind::Application,
        }
    }

    pub fn with_icon(title: impl Into<String>, icon: Icon<Asset>) -> Self {
        Self {
            title: title.into(),
            icon: Some(icon),
            kind: LauncherResultKind::Application,
        }
    }

    pub fn command(title: impl Into<String>, icon: Icon<Asset>) -> Self {
        Self {
            title: title.into(),
            icon: Some(icon),
            kind: LauncherResultKind::Command,
        }
    }

    pub fn calculator(title: impl Into<String>, icon: Icon<Asset>) -> Self {
        Self {
            title: title.into(),
            icon: Some(icon),
            kind: LauncherResultKind::Calculator,
        }
    }

    pub fn initial(&self) -> String {
        self.title.chars().next().map_or_else(
            || "?".to_owned(),
            |character| character.to_uppercase().collect(),
        )
    }
}

#[derive(Debug, PartialEq)]
pub struct LauncherScene<Asset> {
    dpi: NonZeroU32,
    query: String,
    query_cursor: usize,
    mode: SearchMode,
    results: Vec<LauncherResult<Asset>>,
    selected: Option<usize>,
    hovered: Option<usize>,
    first_visible_result: usize,
    total_results: usize,
    footer_time: String,
    presentation_progress: u16,
    theme: Theme,
}

impl<Asset> LauncherScene<Asset> {
    pub fn new(
        dpi: u32,
        query: impl Into<String>,
        mode: SearchMode,
        results: Vec<LauncherResult<Asset>>,
        selected: Option<usize>,
    ) -> Option<Self> {
        let dpi = NonZeroU32::new(dpi)?;
        let selected = selected.filter(|index| *index < results.len());
        let query = query.into();
        let query_cursor = query.chars().count();
        let total_results = results.len();
        Some(Self {
            dpi,
            query,
            query_cursor,
            mode,
            results,
            selected,
            hovered: None,
            first_visible_result: 0,
            total_results,
            footer_time: String::new(),
            presentation_progress: COMPLETE_PROGRESS,
            theme: Theme::default(),
        })
    }

    pub const fn dpi(&self) -> u32 {
        self.dpi.get()
    }

    pub const fn theme(&self) -> Theme {
        self.theme
    }

    pub fn set_theme(&mut self, theme: Theme) -> bool {
        if self.theme == theme {
            return false;
        }
        self.theme = theme;
        true
    }

    pub fn set_dpi(&mut self, dpi: u32) -> bool {
        let Some(dpi) = NonZeroU32::new(dpi) else {
            return false;
        };
        self.dpi = dpi;
        true
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn set_query_cursor(&mut self, cursor: usize) {
        self.query_cursor = cursor.min(self.query.chars().count());
    }

    pub fn query_before_cursor(&self) -> &str {
        let byte = self
            .query
            .char_indices()
            .nth(self.query_cursor)
            .map_or(self.query.len(), |(index, _)| index);
        &self.query[..byte]
    }

    pub fn display_query(&self) -> &str {
        let Some(prefix) = self.command_prefix_end() else {
            return &self.query;
        };
        &self.query[prefix..]
    }

    pub fn display_query_before_cursor(&self) -> &str {
        let cursor_byte = self
            .query
            .char_indices()
            .nth(self.query_cursor)
            .map_or(self.query.len(), |(index, _)| index);
        let prefix = self.command_prefix_end().unwrap_or(0).min(cursor_byte);
        &self.query[prefix..cursor_byte]
    }

    pub const fn mode(&self) -> SearchMode {
        self.mode
    }

    pub fn results(&self) -> &[LauncherResult<Asset>] {
        &self.results
    }

    pub fn is_command_mode(&self) -> bool {
        self.mode == SearchMode::Commands
    }

    pub fn is_calculator_mode(&self) -> bool {
        self.mode == SearchMode::Calculator
    }

    fn command_prefix_end(&self) -> Option<usize> {
        if self.mode != SearchMode::Commands {
            return None;
        }
        let start = self.query.len() - self.query.trim_start().len();
        let after_trigger = start
            + self.query[start..]
                .char_indices()
                .nth(1)
                .map_or(self.query.len() - start, |(index, _)| index);
        Some(
            after_trigger + self.query[after_trigger..].len()
                - self.query[after_trigger..].trim_start().len(),
        )
    }

    pub const fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub const fn hovered(&self) -> Option<usize> {
        self.hovered
    }

    pub fn footer_time(&self) -> &str {
        &self.footer_time
    }

    pub fn set_footer_time(&mut self, footer_time: impl Into<String>) -> bool {
        let footer_time = footer_time.into();
        if self.footer_time == footer_time {
            return false;
        }
        self.footer_time = footer_time;
        true
    }

    pub fn set_hovered(&mut self, hovered: Option<usize>) -> bool {
        let hovered = hovered.filter(|index| *index < self.results.len());
        if self.hovered == hovered {
            return false;
        }
        self.hovered = hovered;
        true
    }

    pub fn set_result_viewport(&mut self, first_visible: usize, total_results: usize) {
        self.total_results = total_results.max(self.results.len());
        self.first_visible_result =
            first_visible.min(self.total_results.saturating_sub(self.results.len()));
    }

    pub fn set_presentation_progress(&mut self, progress: u16) -> bool {
        let progress = progress.min(COMPLETE_PROGRESS);
        if self.presentation_progress == progress {
            return false;
        }
        self.presentation_progress = progress;
        true
    }

    pub const fn needs_animation(&self) -> bool {
        self.presentation_progress < COMPLETE_PROGRESS
    }

    pub fn presentation(&self) -> LauncherPresentation {
        let scale_linear =
            f32::from(self.presentation_progress) / f32::from(COMPLETE_PROGRESS);
        let opacity_linear = (scale_linear * (140.0 / 120.0)).min(1.0);
        let scale_eased = ease_out_cubic(scale_linear);
        LauncherPresentation {
            opacity: ease_out_cubic(opacity_linear),
            scale: 0.97 + scale_eased * 0.03,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LauncherPresentation {
    pub opacity: f32,
    pub scale: f32,
}

fn ease_out_cubic(value: f32) -> f32 {
    1.0 - (1.0 - value).powi(3)
}
