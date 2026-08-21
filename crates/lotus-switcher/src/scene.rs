use lotus_core::window::WindowId;
use lotus_ui::geometry::{
    DpiScale, NonZeroPhysicalSize, PhysicalRect, PhysicalUnsignedPoint, physical_rect,
};
use lotus_ui::theme::Theme;

const MAX_VISIBLE_ITEMS: usize = 7;
const PADDING_DIP: u32 = 12;
const ITEM_WIDTH_DIP: u32 = 112;
const ITEM_HEIGHT_DIP: u32 = 100;
const ITEM_GAP_DIP: u32 = 8;
const CLOSE_SIZE_DIP: u32 = 24;
const CLOSE_INSET_DIP: u32 = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SwitcherHitTarget {
    Item(WindowId),
    Close(WindowId),
}

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
    hovered: Option<SwitcherHitTarget>,
    theme: Theme,
}

impl<Asset> SwitcherScene<Asset> {
    pub fn new(dpi: u32, items: Vec<SwitcherItem<Asset>>, selected: usize) -> Option<Self> {
        let dpi = DpiScale::new(dpi)?;
        (selected < items.len()).then_some(Self {
            dpi,
            items,
            selected,
            hovered: None,
            theme: Theme::default(),
        })
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

    pub const fn hovered(&self) -> Option<SwitcherHitTarget> {
        self.hovered
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

    pub fn set_icon(&mut self, window: WindowId, icon: Option<Asset>) -> bool {
        let Some(item) = self.items.iter_mut().find(|item| item.window == window) else {
            return false;
        };
        if item.icon.is_none() && icon.is_none() {
            return false;
        }

        item.icon = icon;
        true
    }

    pub fn set_dpi(&mut self, dpi: u32) -> bool {
        let Some(dpi) = DpiScale::new(dpi) else {
            return false;
        };
        if self.dpi == dpi {
            return false;
        }
        self.dpi = dpi;
        true
    }

    pub fn pointer_move(&mut self, x: i32, y: i32) -> bool {
        let hovered = self.hit_test(x, y);
        if self.hovered == hovered {
            return false;
        }
        self.hovered = hovered;
        true
    }

    pub fn pointer_left(&mut self) -> bool {
        if self.hovered.take().is_none() {
            return false;
        }
        true
    }

    pub fn hit_test(&self, x: i32, y: i32) -> Option<SwitcherHitTarget> {
        let point =
            PhysicalUnsignedPoint::new(u32::try_from(x).ok()?, u32::try_from(y).ok()?);
        self.layout().items.into_iter().find_map(|item| {
            if item.close.contains(point) {
                Some(SwitcherHitTarget::Close(item.item.window))
            } else {
                item.bounds
                    .contains(point)
                    .then_some(SwitcherHitTarget::Item(item.item.window))
            }
        })
    }

    pub fn desired_size(&self) -> NonZeroPhysicalSize {
        let count = u32::try_from(self.visible_range().len()).unwrap_or(u32::MAX);
        let width_dips = PADDING_DIP
            .saturating_mul(2)
            .saturating_add(ITEM_WIDTH_DIP.saturating_mul(count))
            .saturating_add(ITEM_GAP_DIP.saturating_mul(count.saturating_sub(1)));
        NonZeroPhysicalSize::new(
            self.dpi.physical(width_dips),
            self.dpi.physical(
                PADDING_DIP
                    .saturating_mul(2)
                    .saturating_add(ITEM_HEIGHT_DIP),
            ),
        )
        .expect("switcher dimensions are nonzero")
    }

    pub fn layout(&self) -> SwitcherLayout<'_, Asset> {
        let range = self.visible_range();
        let padding = self.dpi.physical(PADDING_DIP);
        let width = self.dpi.physical(ITEM_WIDTH_DIP);
        let height = self.dpi.physical(ITEM_HEIGHT_DIP);
        let gap = self.dpi.physical(ITEM_GAP_DIP);
        let close_size = self.dpi.physical(CLOSE_SIZE_DIP);
        let close_inset = self.dpi.physical(CLOSE_INSET_DIP);
        let items = self.items[range.clone()]
            .iter()
            .enumerate()
            .map(|(offset, item)| {
                let offset = u32::try_from(offset).unwrap_or(u32::MAX);
                let bounds = physical_rect(
                    padding
                        .saturating_add(offset.saturating_mul(width.saturating_add(gap))),
                    padding,
                    width,
                    height,
                );
                LaidOutItem {
                    source_index: range.start
                        + usize::try_from(offset).unwrap_or(usize::MAX),
                    item,
                    bounds,
                    close: physical_rect(
                        bounds
                            .max_x()
                            .saturating_sub(close_inset)
                            .saturating_sub(close_size),
                        bounds.min_y().saturating_add(close_inset),
                        close_size,
                        close_size,
                    ),
                }
            })
            .collect();
        SwitcherLayout { items }
    }

    pub fn visible_range_with_margin(&self, margin: usize) -> std::ops::Range<usize> {
        let visible = self.visible_range();
        let start = visible.start.saturating_sub(margin);
        let end = visible.end.saturating_add(margin).min(self.items.len());
        start..end
    }

    fn visible_range(&self) -> std::ops::Range<usize> {
        let visible = self.items.len().min(MAX_VISIBLE_ITEMS);
        let start = self
            .selected
            .saturating_sub(visible / 2)
            .min(self.items.len() - visible);
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
    pub close: PhysicalRect,
}
