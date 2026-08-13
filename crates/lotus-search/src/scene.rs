use std::num::NonZeroU32;

use lotus_ui::icon::Icon;
use lotus_ui::theme::Theme;

const DIPS_PER_INCH: u64 = 96;
const PANEL_WIDTH_DIP: u32 = 560;
const QUERY_LEFT_DIP: u32 = 12;
const QUERY_TOP_DIP: u32 = 12;
const QUERY_WIDTH_DIP: u32 = 536;
const QUERY_HEIGHT_DIP: u32 = 50;
const RESULTS_TOP_DIP: u32 = 68;
const ROW_SLOT_HEIGHT_DIP: u32 = 58;
const ROW_MARGIN_X_DIP: u32 = 12;
const ROW_MARGIN_Y_DIP: u32 = 2;
const ROW_BORDER_DIP: u32 = 1;
const ROW_PADDING_X_DIP: u32 = 12;
const ROW_PADDING_Y_DIP: u32 = 5;
const PANEL_BOTTOM_PADDING_DIP: u32 = 12;
const FOOTER_HEIGHT_DIP: u32 = 38;
const FOOTER_HORIZONTAL_INSET_DIP: u32 = 20;
const FOOTER_SEPARATOR_INSET_DIP: u32 = 12;
const ICON_COLUMN_DIP: u32 = 38;
const ICON_CELL_DIP: u32 = 28;
const MAX_RESULTS: usize = 5;
const RESULT_ICON_DIP: u32 = 26;
const COMPLETE_PROGRESS: u16 = 1_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LauncherResult<Asset> {
    pub title: String,
    pub icon: Option<Icon<Asset>>,
}

impl<Asset> LauncherResult<Asset> {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            icon: None,
        }
    }

    pub fn with_icon(title: impl Into<String>, icon: Icon<Asset>) -> Self {
        Self {
            title: title.into(),
            icon: Some(icon),
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
        mut results: Vec<LauncherResult<Asset>>,
        selected: Option<usize>,
    ) -> Option<Self> {
        let dpi = NonZeroU32::new(dpi)?;
        results.truncate(MAX_RESULTS);
        let selected = selected.filter(|index| *index < results.len());
        let query = query.into();
        let query_cursor = query.chars().count();
        let total_results = results.len();
        Some(Self {
            dpi,
            query,
            query_cursor,
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

    pub fn results(&self) -> &[LauncherResult<Asset>] {
        &self.results
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

    pub fn result_icon_size(&self) -> NonZeroU32 {
        nonzero(self.scale(RESULT_ICON_DIP))
    }

    pub fn desired_size(&self) -> LauncherSize {
        let displayed_slots = self.results.len().max(1);
        let displayed_slots = u32::try_from(displayed_slots).unwrap_or(u32::MAX);
        let content_height = self
            .scale(RESULTS_TOP_DIP)
            .saturating_add(displayed_slots.saturating_mul(self.scale(ROW_SLOT_HEIGHT_DIP)))
            .saturating_sub(self.scale(ROW_MARGIN_Y_DIP))
            .saturating_add(self.scale(PANEL_BOTTOM_PADDING_DIP))
            .saturating_add(self.scale(FOOTER_HEIGHT_DIP));
        LauncherSize {
            width: nonzero(self.scale(PANEL_WIDTH_DIP)),
            height: nonzero(content_height),
        }
    }

    pub fn layout(&self) -> LauncherLayout {
        let size = self.desired_size();
        let query = PixelRect {
            left: self.scale(QUERY_LEFT_DIP),
            top: self.scale(QUERY_TOP_DIP),
            width: self.scale(QUERY_WIDTH_DIP),
            height: self.scale(QUERY_HEIGHT_DIP),
        };
        let results_top = self.scale(RESULTS_TOP_DIP);
        let rows = self.result_rows(size, results_top);
        let footer = self.footer_layout(size);
        let empty_state = self.empty_state(size, results_top, footer.separator.top);
        let scrollbar_thumb = self.scrollbar_thumb(size, results_top, footer.separator.top);

        LauncherLayout {
            size,
            query,
            rows: rows.bounds,
            row_surfaces: rows.surfaces,
            row_contents: rows.contents,
            row_icon_cells: rows.icon_cells,
            row_icons: rows.icons,
            row_texts: rows.texts,
            empty_state,
            footer_separator: footer.separator,
            footer_label: footer.label,
            footer_time: footer.time,
            scrollbar_thumb,
            selected: self.selected,
            hovered: self.hovered,
        }
    }

    fn result_rows(&self, size: LauncherSize, results_top: u32) -> LauncherRows {
        let row_slot_height = self.scale(ROW_SLOT_HEIGHT_DIP);
        let bounds = self
            .results
            .iter()
            .enumerate()
            .map(|(index, _)| PixelRect {
                left: 0,
                top: results_top.saturating_add(
                    u32::try_from(index)
                        .unwrap_or(u32::MAX)
                        .saturating_mul(row_slot_height),
                ),
                width: size.width(),
                height: row_slot_height,
            })
            .collect::<Vec<_>>();
        let margin_x = self.scale(ROW_MARGIN_X_DIP);
        let margin_y = self.scale(ROW_MARGIN_Y_DIP);
        let surfaces = bounds
            .iter()
            .map(|row| PixelRect {
                left: margin_x,
                top: row.top.saturating_add(margin_y),
                width: row.width.saturating_sub(margin_x.saturating_mul(2)),
                height: row.height.saturating_sub(margin_y.saturating_mul(2)),
            })
            .collect::<Vec<_>>();
        let content_inset_x = self.scale(ROW_BORDER_DIP.saturating_add(ROW_PADDING_X_DIP));
        let content_inset_y = self.scale(ROW_BORDER_DIP.saturating_add(ROW_PADDING_Y_DIP));
        let contents = surfaces
            .iter()
            .map(|surface| PixelRect {
                left: surface.left.saturating_add(content_inset_x),
                top: surface.top.saturating_add(content_inset_y),
                width: surface
                    .width
                    .saturating_sub(content_inset_x.saturating_mul(2)),
                height: surface
                    .height
                    .saturating_sub(content_inset_y.saturating_mul(2)),
            })
            .collect::<Vec<_>>();
        let icon_cell_size = self.scale(ICON_CELL_DIP);
        let icon_size = self.scale(RESULT_ICON_DIP);
        let icon_cells = contents
            .iter()
            .map(|content| PixelRect {
                left: content.left,
                top: content.top,
                width: icon_cell_size,
                height: content.height,
            })
            .collect::<Vec<_>>();
        let icons = self
            .results
            .iter()
            .zip(&icon_cells)
            .map(|(result, cell)| {
                result.icon.as_ref().map(|_| PixelRect {
                    left: cell
                        .left
                        .saturating_add((cell.width.saturating_sub(icon_size)) / 2),
                    top: cell
                        .top
                        .saturating_add((cell.height.saturating_sub(icon_size)) / 2),
                    width: icon_size,
                    height: icon_size,
                })
            })
            .collect::<Vec<_>>();
        let icon_column = self.scale(ICON_COLUMN_DIP);
        let texts = contents
            .iter()
            .map(|content| PixelRect {
                left: content.left.saturating_add(icon_column),
                top: content.top,
                width: content.width.saturating_sub(icon_column),
                height: content.height,
            })
            .collect::<Vec<_>>();

        LauncherRows {
            bounds,
            surfaces,
            contents,
            icon_cells,
            icons,
            texts,
        }
    }

    fn scrollbar_thumb(
        &self,
        size: LauncherSize,
        results_top: u32,
        footer_top: u32,
    ) -> Option<PixelRect> {
        let visible = self.results.len();
        if visible == 0 || self.total_results <= visible {
            return None;
        }
        let inset = self.scale(6);
        let width = self.scale(3).max(1);
        let track_top = results_top.saturating_add(inset);
        let track_height = footer_top.saturating_sub(inset).saturating_sub(track_top);
        let proportional = u32::try_from(
            u64::from(track_height) * u64::try_from(visible).unwrap_or(u64::MAX)
                / u64::try_from(self.total_results).unwrap_or(u64::MAX),
        )
        .unwrap_or(track_height);
        let thumb_height = proportional.max(self.scale(20)).min(track_height);
        let travel = track_height.saturating_sub(thumb_height);
        let maximum_start = self.total_results.saturating_sub(visible);
        let offset = u32::try_from(
            u64::from(travel)
                * u64::try_from(self.first_visible_result).unwrap_or(u64::MAX)
                / u64::try_from(maximum_start).unwrap_or(1),
        )
        .unwrap_or(travel);
        Some(PixelRect {
            left: size.width().saturating_sub(inset).saturating_sub(width),
            top: track_top.saturating_add(offset),
            width,
            height: thumb_height,
        })
    }

    fn scale(&self, dips: u32) -> u32 {
        let scaled = u64::from(dips) * u64::from(self.dpi.get());
        u32::try_from((scaled + DIPS_PER_INCH / 2) / DIPS_PER_INCH).unwrap_or(u32::MAX)
    }

    fn footer_layout(&self, size: LauncherSize) -> LauncherFooterLayout {
        let height = self.scale(FOOTER_HEIGHT_DIP);
        let top = size.height().saturating_sub(height);
        let separator_inset = self.scale(FOOTER_SEPARATOR_INSET_DIP);
        let horizontal_inset = self.scale(FOOTER_HORIZONTAL_INSET_DIP);
        let text_width = size
            .width()
            .saturating_sub(horizontal_inset.saturating_mul(2))
            / 2;
        LauncherFooterLayout {
            separator: PixelRect {
                left: separator_inset,
                top,
                width: size
                    .width()
                    .saturating_sub(separator_inset.saturating_mul(2)),
                height: self.scale(1),
            },
            label: PixelRect {
                left: horizontal_inset,
                top,
                width: text_width,
                height,
            },
            time: PixelRect {
                left: size
                    .width()
                    .saturating_sub(horizontal_inset.saturating_add(text_width)),
                top,
                width: text_width,
                height,
            },
        }
    }

    fn empty_state(
        &self,
        size: LauncherSize,
        results_top: u32,
        footer_top: u32,
    ) -> Option<PixelRect> {
        self.results.is_empty().then_some(PixelRect {
            left: 0,
            top: results_top,
            width: size.width(),
            height: footer_top.saturating_sub(results_top),
        })
    }
}

struct LauncherFooterLayout {
    separator: PixelRect,
    label: PixelRect,
    time: PixelRect,
}

struct LauncherRows {
    bounds: Vec<PixelRect>,
    surfaces: Vec<PixelRect>,
    contents: Vec<PixelRect>,
    icon_cells: Vec<PixelRect>,
    icons: Vec<Option<PixelRect>>,
    texts: Vec<PixelRect>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LauncherSize {
    width: NonZeroU32,
    height: NonZeroU32,
}

impl LauncherSize {
    pub const fn width(self) -> u32 {
        self.width.get()
    }

    pub const fn height(self) -> u32 {
        self.height.get()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PixelRect {
    pub left: u32,
    pub top: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Eq, PartialEq)]
pub struct LauncherLayout {
    pub size: LauncherSize,
    pub query: PixelRect,
    pub rows: Vec<PixelRect>,
    pub row_surfaces: Vec<PixelRect>,
    pub row_contents: Vec<PixelRect>,
    pub row_icon_cells: Vec<PixelRect>,
    pub row_icons: Vec<Option<PixelRect>>,
    pub row_texts: Vec<PixelRect>,
    pub empty_state: Option<PixelRect>,
    pub footer_separator: PixelRect,
    pub footer_label: PixelRect,
    pub footer_time: PixelRect,
    pub scrollbar_thumb: Option<PixelRect>,
    pub selected: Option<usize>,
    pub hovered: Option<usize>,
}

impl LauncherLayout {
    pub fn hit_test_result(&self, x: u32, y: u32) -> Option<usize> {
        self.rows.iter().position(|row| row.contains(x, y))
    }
}

impl PixelRect {
    fn contains(self, x: u32, y: u32) -> bool {
        x >= self.left
            && x < self.left.saturating_add(self.width)
            && y >= self.top
            && y < self.top.saturating_add(self.height)
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

fn nonzero(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap_or(NonZeroU32::MIN)
}
