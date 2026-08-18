use std::num::NonZeroU32;

use lotus_media::{MediaControls, MediaHitTarget, PlaybackState};
pub use lotus_ui::icon::{RasterIcon, RasterIconId};
use lotus_ui::theme::Theme;

pub type DockIcon<Asset> = lotus_ui::icon::Icon<Asset>;

mod layout;

const DEFAULT_ICON_SIZE_DIP: u32 = 38;
const DEFAULT_ITEM_SPACING_DIP: u32 = 8;
const DEFAULT_HORIZONTAL_PADDING_DIP: u32 = 12;
const DEFAULT_VERTICAL_PADDING_DIP: u32 = 8;
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
    running: bool,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaItem<Asset> {
    pub source_id: String,
    pub title: String,
    pub artist: String,
    pub show_metadata: bool,
    pub artwork: DockIcon<Asset>,
    pub controls: MediaControls,
    pub playback: PlaybackState,
    pub symbols: MediaSymbols<Asset>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaSymbols<Asset> {
    pub previous: Asset,
    pub play: Asset,
    pub pause: Asset,
    pub next: Asset,
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

    pub fn symbol(kind: SystemStatusKind, symbol: char) -> Self {
        Self {
            kind,
            icon: None,
            primary_text: symbol.into(),
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
            running: false,
            exiting: false,
        }
    }

    pub const fn with_source_index(source_index: usize, icon: DockIcon<Asset>) -> Self {
        Self {
            source_index,
            icon,
            badge: None,
            running: false,
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

    pub fn set_running(&mut self, running: bool) {
        self.running = running;
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
    media: Option<MediaItem<Asset>>,
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
            media: None,
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

    pub fn replace_media(&mut self, media: Option<MediaItem<Asset>>) {
        self.media = media;
    }

    pub const fn media(&self) -> Option<&MediaItem<Asset>> {
        self.media.as_ref()
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
    pub running: bool,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaidOutMediaControl<Asset> {
    pub target: MediaHitTarget,
    pub icon: DockIcon<Asset>,
    pub bounds: PixelRect,
    pub enabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaidOutMedia<Asset> {
    pub source_id: String,
    pub artwork: LaidOutStatusIcon<Asset>,
    pub metadata: PixelRect,
    pub title: String,
    pub artist: String,
    pub controls: Vec<LaidOutMediaControl<Asset>>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct DockLayout<Asset> {
    pub items: Vec<LaidOutItem<Asset>>,
    pub launcher_button_visible: bool,
    pub divider: PixelRect,
    pub media_divider: Option<PixelRect>,
    pub media: Option<LaidOutMedia<Asset>>,
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
                            self.media.as_ref().and_then(|media| {
                                if media.artwork.bounds.contains(x, y)
                                    || media.metadata.contains(x, y)
                                {
                                    Some(DockHitTarget::Media(MediaHitTarget::Metadata))
                                } else {
                                    media
                                        .controls
                                        .iter()
                                        .find(|control| control.bounds.contains(x, y))
                                        .map(|control| DockHitTarget::Media(control.target))
                                }
                            })
                        })
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
    Media(MediaHitTarget),
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
