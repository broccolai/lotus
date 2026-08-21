use lotus_core::window::WindowId;
use lotus_ui::geometry::{
    DpiScale, NonZeroPhysicalSize, PhysicalRect, PhysicalUnsignedPoint, physical_rect,
};
use lotus_ui::icon::Icon;
use lotus_ui::presentation::{
    FontWeight, HorizontalAlignment, ImageSampling, Presentation, PresentationPrimitive,
    PresentationRect, TextStyle, VerticalAlignment,
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

impl<Asset: Clone> SwitcherScene<Icon<Asset>> {
    pub fn presentation(&self, dismiss: Asset) -> Presentation<Asset> {
        let theme = self.theme();
        let size = self.desired_size();
        let mut presentation = Presentation::new(theme.canvas.with_alpha(0.0));
        presentation.push(PresentationPrimitive::FillRoundedRect {
            bounds: PresentationRect::new(
                0.5,
                0.5,
                as_f32(size.width()) - 0.5,
                as_f32(size.height()) - 0.5,
            ),
            radius: self.scaled(theme.radii.panel),
            color: theme.chrome_overlay,
        });
        for item in self.layout().items {
            self.present_item(&mut presentation, &item, dismiss.clone());
        }
        presentation
    }

    fn present_item(
        &self,
        presentation: &mut Presentation<Asset>,
        item: &LaidOutItem<'_, Icon<Asset>>,
        dismiss: Asset,
    ) {
        let theme = self.theme();
        let bounds = presentation_rect(item.bounds);
        if item.source_index == self.selected() {
            presentation.push(PresentationPrimitive::FillRoundedRect {
                bounds,
                radius: self.scaled(theme.radii.control),
                color: theme.control_selected,
            });
            let inset = self.scaled(0.5);
            presentation.push(PresentationPrimitive::StrokeRoundedRect {
                bounds: PresentationRect::new(
                    bounds.left + inset,
                    bounds.top + inset,
                    bounds.right - inset,
                    bounds.bottom - inset,
                ),
                radius: self.scaled((theme.radii.control - 0.5).max(1.0)),
                width: self.scaled(1.0),
                color: theme.border_strong,
            });
        } else if self.item_is_hovered(item.item.window) {
            presentation.push(PresentationPrimitive::FillRoundedRect {
                bounds,
                radius: self.scaled(theme.radii.control),
                color: theme.control_hover,
            });
        }

        let icon_bounds = PresentationRect::new(
            bounds.left,
            bounds.top,
            bounds.right,
            bounds.top + bounds.height() * 0.62,
        );
        self.present_artwork(presentation, item, icon_bounds);
        presentation.push(PresentationPrimitive::Text {
            value: item.item.title.clone(),
            bounds: PresentationRect::new(
                bounds.left,
                icon_bounds.bottom - self.scaled(4.0),
                bounds.right,
                bounds.bottom,
            ),
            style: centered_text(13.0, FontWeight::Normal),
            color: theme.text,
        });
        self.present_close(presentation, item, dismiss);
    }

    fn present_artwork(
        &self,
        presentation: &mut Presentation<Asset>,
        item: &LaidOutItem<'_, Icon<Asset>>,
        bounds: PresentationRect,
    ) {
        let Some(icon) = &item.item.icon else {
            presentation.push(PresentationPrimitive::Text {
                value: item
                    .item
                    .title
                    .chars()
                    .next()
                    .unwrap_or('?')
                    .to_uppercase()
                    .to_string(),
                bounds,
                style: centered_text(26.0, FontWeight::Semibold),
                color: self.theme().accent,
            });
            return;
        };

        let size = self.dpi.physical(38);
        let width = as_f32(size);
        let center = bounds.left.midpoint(bounds.right);
        let left = match icon {
            Icon::Raster(_) => (center - width / 2.0).round(),
            Icon::Embedded(_) => center - width / 2.0,
        };
        let top = bounds.top + self.scaled(12.0);
        let sampling = match icon {
            Icon::Raster(raster) if raster.width() == size && raster.height() == size => {
                ImageSampling::PixelAligned
            }
            Icon::Embedded(_) | Icon::Raster(_) => ImageSampling::Smooth,
        };
        presentation.push(PresentationPrimitive::Icon {
            icon: icon.clone(),
            bounds: PresentationRect::new(left, top, left + width, top + width),
            tint: self.theme().text,
            opacity: 1.0,
            sampling,
            radius: 0.0,
        });
    }

    fn present_close(
        &self,
        presentation: &mut Presentation<Asset>,
        item: &LaidOutItem<'_, Icon<Asset>>,
        dismiss: Asset,
    ) {
        if item.source_index != self.selected() && !self.item_is_hovered(item.item.window) {
            return;
        }

        let bounds = presentation_rect(item.close);
        if self.hovered() == Some(SwitcherHitTarget::Close(item.item.window)) {
            presentation.push(PresentationPrimitive::FillRoundedRect {
                bounds,
                radius: self.scaled(self.theme().radii.compact),
                color: self.theme().control_hover,
            });
        }
        let icon_size = self.dpi.physical(14);
        presentation.push(PresentationPrimitive::Icon {
            icon: Icon::Embedded(dismiss),
            bounds: centered_rect(bounds, as_f32(icon_size)),
            tint: self.theme().text,
            opacity: 1.0,
            sampling: ImageSampling::Smooth,
            radius: 0.0,
        });
    }

    fn item_is_hovered(&self, window: WindowId) -> bool {
        matches!(
            self.hovered(),
            Some(SwitcherHitTarget::Item(hovered) | SwitcherHitTarget::Close(hovered))
                if hovered == window
        )
    }

    fn scaled(&self, dips: f32) -> f32 {
        as_f32(self.dpi()) * dips / 96.0
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

fn centered_text(size: f32, weight: FontWeight) -> TextStyle {
    TextStyle {
        size,
        family: lotus_ui::presentation::FontFamily::Interface,
        weight,
        horizontal: HorizontalAlignment::Center,
        vertical: VerticalAlignment::Center,
    }
}

fn presentation_rect(bounds: PhysicalRect) -> PresentationRect {
    PresentationRect::new(
        as_f32(bounds.min_x()),
        as_f32(bounds.min_y()),
        as_f32(bounds.max_x()),
        as_f32(bounds.max_y()),
    )
}

fn centered_rect(bounds: PresentationRect, size: f32) -> PresentationRect {
    let center_x = bounds.left.midpoint(bounds.right);
    let center_y = bounds.top.midpoint(bounds.bottom);
    let half = size / 2.0;
    PresentationRect::new(
        center_x - half,
        center_y - half,
        center_x + half,
        center_y + half,
    )
}

#[allow(
    clippy::cast_precision_loss,
    reason = "switcher dimensions remain below f32 exact range"
)]
const fn as_f32(value: u32) -> f32 {
    value as f32
}
