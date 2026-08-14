use std::num::NonZeroU32;

pub use lotus_ui::icon::{RasterIcon, RasterIconId};
use lotus_ui::theme::Theme;

pub type DockIcon<Asset> = lotus_ui::icon::Icon<Asset>;

const DEFAULT_ICON_SIZE_DIP: u32 = 38;
const DEFAULT_ITEM_SPACING_DIP: u32 = 8;
const DEFAULT_HORIZONTAL_PADDING_DIP: u32 = 12;
const DEFAULT_VERTICAL_PADDING_DIP: u32 = 8;
const DIVIDER_WIDTH_DIP: u32 = 2;
const DIVIDER_HEIGHT_DIP: u32 = 18;
const SHOW_DESKTOP_WIDTH_DIP: u32 = 10;
const STATUS_ICON_SIZE_DIP: u32 = 18;
const STATUS_ICON_SLOT_WIDTH_DIP: u32 = 32;
const STATUS_CLOCK_WIDTH_DIP: u32 = 72;
const DRAG_VERTICAL_TOLERANCE_DIP: u32 = 24;
const DIPS_PER_INCH: u64 = 96;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DockBadge {
    Dot,
    Count(u32),
    AtLeast(u32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DockItem<Asset> {
    source_index: usize,
    pub icon: DockIcon<Asset>,
    pub badge: Option<DockBadge>,
    exiting: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemStatusKind {
    Volume,
    Network,
    BackgroundApps,
    DateTime,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DockAnchor {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SystemStatusItem<Asset> {
    pub kind: SystemStatusKind,
    pub icon: Option<DockIcon<Asset>>,
    pub primary_text: String,
    pub secondary_text: String,
}

impl<Asset> SystemStatusItem<Asset> {
    pub fn icon(kind: SystemStatusKind, icon: Asset) -> Self {
        Self {
            kind,
            icon: Some(DockIcon::Embedded(icon)),
            primary_text: String::new(),
            secondary_text: String::new(),
        }
    }

    pub fn date_time(primary_text: String, secondary_text: String) -> Self {
        Self {
            kind: SystemStatusKind::DateTime,
            icon: None,
            primary_text,
            secondary_text,
        }
    }
}

impl<Asset> DockItem<Asset> {
    pub const fn new(icon: Asset) -> Self {
        Self {
            source_index: 0,
            icon: DockIcon::Embedded(icon),
            badge: None,
            exiting: false,
        }
    }

    pub const fn with_source_index(source_index: usize, icon: DockIcon<Asset>) -> Self {
        Self {
            source_index,
            icon,
            badge: None,
            exiting: false,
        }
    }

    pub const fn embedded(source_index: usize, icon: Asset) -> Self {
        Self::with_source_index(source_index, DockIcon::Embedded(icon))
    }

    pub fn raster(source_index: usize, icon: RasterIcon) -> Self {
        Self::with_source_index(source_index, DockIcon::Raster(icon))
    }

    pub const fn source_index(&self) -> usize {
        self.source_index
    }

    pub fn set_source_index(&mut self, source_index: usize) {
        self.source_index = source_index;
    }

    pub fn set_exiting(&mut self, exiting: bool) {
        self.exiting = exiting;
    }

    pub const fn is_exiting(&self) -> bool {
        self.exiting
    }

    pub fn set_badge(&mut self, badge: Option<DockBadge>) {
        self.badge = badge;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DockMetrics {
    icon_size: NonZeroU32,
    item_spacing: u32,
    horizontal_padding: u32,
    vertical_padding: u32,
}

impl DockMetrics {
    pub const fn new(
        icon_size: u32,
        item_spacing: u32,
        horizontal_padding: u32,
        vertical_padding: u32,
    ) -> Option<Self> {
        let Some(icon_size) = NonZeroU32::new(icon_size) else {
            return None;
        };
        Some(Self {
            icon_size,
            item_spacing,
            horizontal_padding,
            vertical_padding,
        })
    }

    pub const fn defaults() -> Self {
        Self {
            icon_size: NonZeroU32::new(DEFAULT_ICON_SIZE_DIP).unwrap(),
            item_spacing: DEFAULT_ITEM_SPACING_DIP,
            horizontal_padding: DEFAULT_HORIZONTAL_PADDING_DIP,
            vertical_padding: DEFAULT_VERTICAL_PADDING_DIP,
        }
    }

    pub const fn icon_size(self) -> u32 {
        self.icon_size.get()
    }

    pub const fn item_spacing(self) -> u32 {
        self.item_spacing
    }

    pub const fn horizontal_padding(self) -> u32 {
        self.horizontal_padding
    }

    pub const fn vertical_padding(self) -> u32 {
        self.vertical_padding
    }
}

#[derive(Debug, PartialEq)]
pub struct DockScene<Asset> {
    dpi: NonZeroU32,
    metrics: DockMetrics,
    mascot: DockIcon<Asset>,
    anchor: DockAnchor,
    launcher_button_visible: bool,
    show_desktop_button: bool,
    status_items: Vec<SystemStatusItem<Asset>>,
    items: Vec<DockItem<Asset>>,
    interaction: DockInteractionState,
    drag: Option<DockDragState>,
    theme: Theme,
}

impl<Asset: Clone> DockScene<Asset> {
    pub fn new(
        dpi: u32,
        metrics: DockMetrics,
        mascot: DockIcon<Asset>,
        items: Vec<DockItem<Asset>>,
    ) -> Option<Self> {
        NonZeroU32::new(dpi).map(|dpi| Self {
            dpi,
            metrics,
            mascot,
            anchor: DockAnchor::Center,
            launcher_button_visible: true,
            show_desktop_button: false,
            status_items: Vec::new(),
            items,
            interaction: DockInteractionState::default(),
            drag: None,
            theme: Theme::default(),
        })
    }

    pub fn initial(dpi: u32, icon: Asset) -> Option<Self> {
        Self::new(
            dpi,
            DockMetrics::defaults(),
            DockIcon::Embedded(icon.clone()),
            vec![DockItem::new(icon)],
        )
    }

    pub const fn dpi(&self) -> u32 {
        self.dpi.get()
    }

    pub const fn theme(&self) -> Theme {
        self.theme
    }

    pub fn set_theme(&mut self, theme: Theme) -> bool {
        replace_if_changed(&mut self.theme, theme)
    }

    pub fn icon_size_pixels(&self) -> u32 {
        nonzero_or_one(self.scaled_metrics().icon_size).get()
    }

    pub fn set_dpi(&mut self, dpi: u32) -> bool {
        let Some(dpi) = NonZeroU32::new(dpi) else {
            return false;
        };
        self.dpi = dpi;
        true
    }

    pub fn replace_items(&mut self, items: Vec<DockItem<Asset>>) {
        self.items = items;
        if self.drag.is_some_and(|drag| {
            !self
                .items
                .iter()
                .any(|item| item.source_index == drag.source_index)
        }) {
            self.drag = None;
        }
    }

    pub fn set_mascot(&mut self, mascot: DockIcon<Asset>) {
        self.mascot = mascot;
    }

    pub const fn mascot(&self) -> &DockIcon<Asset> {
        &self.mascot
    }

    pub fn set_anchor(&mut self, anchor: DockAnchor) {
        self.anchor = anchor;
    }

    pub const fn anchor(&self) -> DockAnchor {
        self.anchor
    }

    pub fn set_launcher_button_visible(&mut self, visible: bool) {
        self.launcher_button_visible = visible;
    }

    pub const fn launcher_button_visible(&self) -> bool {
        self.launcher_button_visible
    }

    pub fn set_show_desktop_button(&mut self, visible: bool) {
        self.show_desktop_button = visible;
    }

    pub fn replace_status_items(&mut self, items: Vec<SystemStatusItem<Asset>>) {
        self.status_items = items;
    }

    pub fn status_items(&self) -> &[SystemStatusItem<Asset>] {
        &self.status_items
    }

    pub fn items(&self) -> &[DockItem<Asset>] {
        &self.items
    }

    pub const fn interaction(&self) -> DockInteractionState {
        self.interaction
    }

    pub fn set_hovered(&mut self, target: Option<DockHitTarget>) -> bool {
        replace_if_changed(&mut self.interaction.hovered, target)
    }

    pub fn set_pressed(&mut self, target: Option<DockHitTarget>) -> bool {
        replace_if_changed(&mut self.interaction.pressed, target)
    }

    pub fn begin_drag(&mut self, source_index: usize, x: i32, y: i32) -> bool {
        if !self
            .items
            .iter()
            .any(|item| item.source_index == source_index)
        {
            return false;
        }
        replace_if_changed(
            &mut self.drag,
            Some(DockDragState {
                source_index,
                pointer_x: x,
                pointer_y: y,
            }),
        )
    }

    pub fn update_drag(&mut self, x: i32, y: i32) -> bool {
        let Some(drag) = &mut self.drag else {
            return false;
        };
        let changed = (drag.pointer_x, drag.pointer_y) != (x, y);
        drag.pointer_x = x;
        drag.pointer_y = y;
        changed
    }

    pub fn cancel_drag(&mut self) -> bool {
        self.drag.take().is_some()
    }

    pub const fn drag(&self) -> Option<DockDragState> {
        self.drag
    }

    pub fn drag_insertion_slot(
        &self,
        surface_width: u32,
        surface_height: u32,
    ) -> Option<usize> {
        let drag = self.drag?;
        if !self.drag_drop_eligible(surface_height) {
            return None;
        }
        Some(
            self.layout(surface_width, surface_height)
                .insertion_slot(drag.pointer_x),
        )
    }

    pub fn drag_drop_eligible(&self, surface_height: u32) -> bool {
        let Some(drag) = self.drag else {
            return false;
        };
        let tolerance = i64::from(scale_dips(DRAG_VERTICAL_TOLERANCE_DIP, self.dpi));
        let pointer_y = i64::from(drag.pointer_y);
        pointer_y >= -tolerance && pointer_y <= i64::from(surface_height) + tolerance
    }

    pub fn desired_size(&self) -> DockSize {
        let metrics = self.scaled_metrics();
        let item_count = u32::try_from(self.items.len()).unwrap_or(u32::MAX);
        let slot_width = metrics
            .icon_size
            .saturating_add(metrics.spacing.saturating_mul(2));
        let item_strip_width = item_count.saturating_mul(slot_width);
        let show_desktop_width = self
            .show_desktop_button
            .then_some(metrics.show_desktop_width);
        let status_width = self.status_items.iter().fold(0_u32, |width, item| {
            width.saturating_add(match item.kind {
                SystemStatusKind::DateTime => metrics.status_clock_width,
                SystemStatusKind::Volume
                | SystemStatusKind::Network
                | SystemStatusKind::BackgroundApps => metrics.status_icon_slot_width,
            })
        });
        let status_chrome_width = if self.status_separator_visible() {
            metrics
                .spacing
                .saturating_add(metrics.divider_width)
                .saturating_add(metrics.spacing)
        } else {
            0
        };
        let launcher_width = self.launcher_button_visible.then_some(
            slot_width
                .saturating_add(metrics.spacing)
                .saturating_add(metrics.divider_width)
                .saturating_add(metrics.spacing),
        );
        let width = metrics
            .horizontal_padding
            .saturating_mul(2)
            .saturating_add(item_strip_width)
            .saturating_add(launcher_width.unwrap_or_default())
            .saturating_add(status_chrome_width)
            .saturating_add(status_width)
            .saturating_add(show_desktop_width.unwrap_or_default());
        let height = metrics
            .icon_size
            .saturating_add(metrics.vertical_padding.saturating_mul(2));
        DockSize {
            width: nonzero_or_one(width),
            height: nonzero_or_one(height),
        }
    }

    pub fn layout(&self, surface_width: u32, surface_height: u32) -> DockLayout<Asset> {
        let metrics = self.scaled_metrics();
        let desired = self.desired_size();
        let content_left = surface_width.saturating_sub(desired.width()) / 2;
        let content_top = surface_height.saturating_sub(desired.height()) / 2;
        let icon_top = content_top.saturating_add(metrics.vertical_padding);
        let mut cursor = content_left.saturating_add(metrics.horizontal_padding);
        let slot_width = metrics
            .icon_size
            .saturating_add(metrics.spacing.saturating_mul(2));
        let jirachi_hit_bounds = PixelRect {
            left: cursor,
            top: icon_top,
            width: slot_width,
            height: metrics.icon_size,
        };
        let jirachi = PixelRect::square(
            cursor.saturating_add(metrics.spacing),
            icon_top,
            metrics.icon_size,
        );
        let divider = PixelRect {
            left: cursor
                .saturating_add(slot_width)
                .saturating_add(metrics.spacing),
            top: surface_height.saturating_sub(metrics.divider_height) / 2,
            width: metrics.divider_width,
            height: metrics.divider_height,
        };
        if self.launcher_button_visible {
            cursor = divider
                .left
                .saturating_add(metrics.divider_width)
                .saturating_add(metrics.spacing);
        }
        let items = self
            .items
            .iter()
            .map(|item| {
                let hit_bounds = PixelRect {
                    left: cursor,
                    top: icon_top,
                    width: slot_width,
                    height: metrics.icon_size,
                };
                let laid_out = LaidOutItem {
                    source_index: item.source_index,
                    icon: item.icon.clone(),
                    badge: item.badge,
                    exiting: item.exiting,
                    bounds: PixelRect::square(
                        cursor.saturating_add(metrics.spacing),
                        icon_top,
                        metrics.icon_size,
                    ),
                    hit_bounds,
                };
                cursor = cursor.saturating_add(slot_width);
                laid_out
            })
            .collect();

        let (status_divider, status_items) = self.layout_status_items(
            &mut cursor,
            content_top,
            surface_height,
            desired,
            &metrics,
        );

        let show_desktop = if self.show_desktop_button {
            Some(PixelRect {
                left: cursor,
                top: content_top,
                width: metrics.show_desktop_width,
                height: desired.height(),
            })
        } else {
            None
        };

        DockLayout {
            items,
            launcher_button_visible: self.launcher_button_visible,
            divider,
            status_divider,
            jirachi,
            jirachi_hit_bounds,
            status_items,
            show_desktop,
            icon_size: nonzero_or_one(metrics.icon_size),
        }
    }

    fn layout_status_items(
        &self,
        cursor: &mut u32,
        content_top: u32,
        surface_height: u32,
        desired: DockSize,
        metrics: &ScaledMetrics,
    ) -> (Option<PixelRect>, Vec<LaidOutStatusItem<Asset>>) {
        let divider = self.status_separator_visible().then(|| {
            let divider = PixelRect {
                left: cursor.saturating_add(metrics.spacing),
                top: surface_height.saturating_sub(metrics.divider_height) / 2,
                width: metrics.divider_width,
                height: metrics.divider_height,
            };
            *cursor = cursor
                .saturating_add(metrics.spacing)
                .saturating_add(metrics.divider_width)
                .saturating_add(metrics.spacing);
            divider
        });
        let items = self
            .status_items
            .iter()
            .map(|item| layout_status_item(item, cursor, content_top, desired, metrics))
            .collect();
        (divider, items)
    }

    fn status_separator_visible(&self) -> bool {
        !self.items.is_empty() && !self.status_items.is_empty()
    }

    fn scaled_metrics(&self) -> ScaledMetrics {
        ScaledMetrics {
            icon_size: scale_dips(self.metrics.icon_size.get(), self.dpi),
            spacing: scale_dips(self.metrics.item_spacing, self.dpi),
            horizontal_padding: scale_dips(self.metrics.horizontal_padding, self.dpi),
            vertical_padding: scale_dips(self.metrics.vertical_padding, self.dpi),
            divider_width: scale_dips(DIVIDER_WIDTH_DIP, self.dpi),
            divider_height: scale_dips(DIVIDER_HEIGHT_DIP, self.dpi),
            show_desktop_width: scale_dips(SHOW_DESKTOP_WIDTH_DIP, self.dpi),
            status_icon_size: scale_dips(STATUS_ICON_SIZE_DIP, self.dpi),
            status_icon_slot_width: scale_dips(STATUS_ICON_SLOT_WIDTH_DIP, self.dpi),
            status_clock_width: scale_dips(STATUS_CLOCK_WIDTH_DIP, self.dpi),
        }
    }
}

fn layout_status_item<Asset: Clone>(
    item: &SystemStatusItem<Asset>,
    cursor: &mut u32,
    content_top: u32,
    desired: DockSize,
    metrics: &ScaledMetrics,
) -> LaidOutStatusItem<Asset> {
    let width = match item.kind {
        SystemStatusKind::DateTime => metrics.status_clock_width,
        SystemStatusKind::Volume
        | SystemStatusKind::Network
        | SystemStatusKind::BackgroundApps => metrics.status_icon_slot_width,
    };
    let hit_bounds = PixelRect {
        left: *cursor,
        top: content_top,
        width,
        height: desired.height(),
    };
    let icon = item.icon.as_ref().map(|icon| LaidOutStatusIcon {
        icon: icon.clone(),
        bounds: PixelRect::square(
            cursor.saturating_add(width.saturating_sub(metrics.status_icon_size) / 2),
            content_top.saturating_add(
                desired.height().saturating_sub(metrics.status_icon_size) / 2,
            ),
            metrics.status_icon_size,
        ),
    });
    let laid_out = LaidOutStatusItem {
        kind: item.kind,
        hit_bounds,
        icon,
        primary_text: item.primary_text.clone(),
        secondary_text: item.secondary_text.clone(),
    };
    *cursor = cursor.saturating_add(width);
    laid_out
}

struct ScaledMetrics {
    icon_size: u32,
    spacing: u32,
    horizontal_padding: u32,
    vertical_padding: u32,
    divider_width: u32,
    divider_height: u32,
    show_desktop_width: u32,
    status_icon_size: u32,
    status_icon_slot_width: u32,
    status_clock_width: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DockSize {
    pub(super) width: NonZeroU32,
    pub(super) height: NonZeroU32,
}

impl DockSize {
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

impl PixelRect {
    const fn square(left: u32, top: u32, side: u32) -> Self {
        Self {
            left,
            top,
            width: side,
            height: side,
        }
    }

    const fn contains(self, x: u32, y: u32) -> bool {
        x >= self.left
            && x < self.left.saturating_add(self.width)
            && y >= self.top
            && y < self.top.saturating_add(self.height)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaidOutItem<Asset> {
    pub source_index: usize,
    pub icon: DockIcon<Asset>,
    pub badge: Option<DockBadge>,
    pub exiting: bool,
    pub bounds: PixelRect,
    pub hit_bounds: PixelRect,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaidOutStatusIcon<Asset> {
    pub icon: DockIcon<Asset>,
    pub bounds: PixelRect,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaidOutStatusItem<Asset> {
    pub kind: SystemStatusKind,
    pub hit_bounds: PixelRect,
    pub icon: Option<LaidOutStatusIcon<Asset>>,
    pub primary_text: String,
    pub secondary_text: String,
}

#[derive(Debug, Eq, PartialEq)]
pub struct DockLayout<Asset> {
    pub items: Vec<LaidOutItem<Asset>>,
    pub launcher_button_visible: bool,
    pub divider: PixelRect,
    pub status_divider: Option<PixelRect>,
    pub jirachi: PixelRect,
    pub jirachi_hit_bounds: PixelRect,
    pub status_items: Vec<LaidOutStatusItem<Asset>>,
    pub show_desktop: Option<PixelRect>,
    pub icon_size: NonZeroU32,
}

impl<Asset> DockLayout<Asset> {
    pub fn hit_test(&self, x: u32, y: u32) -> Option<DockHitTarget> {
        self.items
            .iter()
            .find(|item| item.hit_bounds.contains(x, y))
            .map_or_else(
                || {
                    (self.launcher_button_visible && self.jirachi_hit_bounds.contains(x, y))
                        .then_some(DockHitTarget::Jirachi)
                        .or_else(|| {
                            self.status_items
                                .iter()
                                .find(|item| item.hit_bounds.contains(x, y))
                                .map(|item| DockHitTarget::SystemStatus(item.kind))
                        })
                        .or_else(|| {
                            self.show_desktop
                                .filter(|bounds| bounds.contains(x, y))
                                .map(|_| DockHitTarget::ShowDesktop)
                        })
                },
                |item| Some(DockHitTarget::Item(item.source_index)),
            )
    }

    pub fn insertion_slot(&self, x: i32) -> usize {
        self.items
            .iter()
            .position(|item| {
                i64::from(x)
                    < i64::from(item.bounds.left.saturating_add(item.bounds.width / 2))
            })
            .unwrap_or(self.items.len())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DockHitTarget {
    Item(usize),
    Jirachi,
    SystemStatus(SystemStatusKind),
    ShowDesktop,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DockInteractionState {
    pub hovered: Option<DockHitTarget>,
    pub pressed: Option<DockHitTarget>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DockDragState {
    pub source_index: usize,
    pub pointer_x: i32,
    pub pointer_y: i32,
}

fn replace_if_changed<T: PartialEq>(slot: &mut T, value: T) -> bool {
    if *slot == value {
        return false;
    }
    *slot = value;
    true
}

fn scale_dips(dips: u32, dpi: NonZeroU32) -> u32 {
    let scaled = u64::from(dips) * u64::from(dpi.get());
    let rounded = (scaled + DIPS_PER_INCH / 2) / DIPS_PER_INCH;
    u32::try_from(rounded).unwrap_or(u32::MAX)
}

fn nonzero_or_one(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap_or(NonZeroU32::MIN)
}
