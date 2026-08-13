use lotus_core::window::WindowId;
use lotus_ui::geometry::{DpiScale, NonZeroPhysicalSize, PhysicalRect, physical_rect};
use lotus_ui::theme::Theme;

const MAX_VISIBLE_ITEMS: usize = 7;
const PADDING_DIP: u32 = 12;
const ITEM_WIDTH_DIP: u32 = 112;
const ITEM_HEIGHT_DIP: u32 = 100;
const ITEM_GAP_DIP: u32 = 8;

#[derive(Clone)]
pub struct SwitcherItem<Asset> {
    pub window: WindowId,
    pub title: String,
    pub icon: Option<Asset>,
}

pub struct SwitcherScene<Asset> {
    dpi: DpiScale,
    items: Vec<SwitcherItem<Asset>>,
    selected: usize,
    theme: Theme,
}

impl<Asset> SwitcherScene<Asset> {
    pub fn new(dpi: u32, items: Vec<SwitcherItem<Asset>>, selected: usize) -> Option<Self> {
        let dpi = DpiScale::new(dpi)?;
        (selected < items.len()).then_some(Self { dpi, items, selected, theme: Theme::default() })
    }

    pub const fn dpi(&self) -> u32 {
        self.dpi.dpi()
    }

    pub const fn selected(&self) -> usize {
        self.selected
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

    pub fn set_selected(&mut self, selected: usize) -> bool {
        if selected >= self.items.len() || selected == self.selected {
            return false;
        }
        self.selected = selected;
        true
    }

    pub fn set_dpi(&mut self, dpi: u32) -> bool {
        let Some(dpi) = DpiScale::new(dpi) else { return false };
        if self.dpi == dpi {
            return false;
        }
        self.dpi = dpi;
        true
    }

    pub fn desired_size(&self) -> NonZeroPhysicalSize {
        let count = u32::try_from(self.visible_range().len()).unwrap_or(u32::MAX);
        let width_dips = PADDING_DIP
            .saturating_mul(2)
            .saturating_add(ITEM_WIDTH_DIP.saturating_mul(count))
            .saturating_add(ITEM_GAP_DIP.saturating_mul(count.saturating_sub(1)));
        NonZeroPhysicalSize::new(
            self.dpi.physical(width_dips),
            self.dpi.physical(PADDING_DIP.saturating_mul(2).saturating_add(ITEM_HEIGHT_DIP)),
        )
        .expect("switcher dimensions are nonzero")
    }

    pub fn layout(&self) -> SwitcherLayout<'_, Asset> {
        let range = self.visible_range();
        let padding = self.dpi.physical(PADDING_DIP);
        let width = self.dpi.physical(ITEM_WIDTH_DIP);
        let height = self.dpi.physical(ITEM_HEIGHT_DIP);
        let gap = self.dpi.physical(ITEM_GAP_DIP);
        let items = self.items[range.clone()]
            .iter()
            .enumerate()
            .map(|(offset, item)| {
                let offset = u32::try_from(offset).unwrap_or(u32::MAX);
                LaidOutItem {
                    source_index: range.start + usize::try_from(offset).unwrap_or(usize::MAX),
                    item,
                    bounds: physical_rect(
                        padding.saturating_add(offset.saturating_mul(width.saturating_add(gap))),
                        padding,
                        width,
                        height,
                    ),
                }
            })
            .collect();
        SwitcherLayout { items }
    }

    fn visible_range(&self) -> std::ops::Range<usize> {
        let visible = self.items.len().min(MAX_VISIBLE_ITEMS);
        let start = self.selected.saturating_sub(visible / 2).min(self.items.len() - visible);
        start..start + visible
    }
}

pub struct SwitcherLayout<'a, Asset> {
    pub items: Vec<LaidOutItem<'a, Asset>>,
}

pub struct LaidOutItem<'a, Asset> {
    pub source_index: usize,
    pub item: &'a SwitcherItem<Asset>,
    pub bounds: PhysicalRect,
}
