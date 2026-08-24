use lotus_core::settings::WindowPickerStyle;
use lotus_core::window::{TrackedWindowKey, WindowInfo};
use lotus_ui::geometry::{
    DpiScale, NonZeroPhysicalSize, PhysicalRect, PhysicalUnsignedPoint, physical_rect,
};
use lotus_ui::icon::Icon;
use lotus_ui::presentation::{
    FontWeight, HorizontalAlignment, ImageSampling, Presentation, PresentationPrimitive,
    PresentationRect, TextStyle, VerticalAlignment,
};
use lotus_ui::theme::Theme;

use crate::action_menu::{Action as SystemAction, ActionMenu};

const PADDING_DIP: u32 = 4;
const GAP_DIP: u32 = 4;
const APP_WIDTH_DIP: u32 = 196;
const APP_ROW_DIP: u32 = 42;
const POWER_WIDTH_DIP: u32 = 220;
const POWER_ROW_DIP: u32 = 44;
const COMPACT_WIDTH_DIP: u32 = 320;
const COMPACT_ROW_DIP: u32 = 46;
const THUMBNAIL_CARD_WIDTH_DIP: u32 = 224;
const THUMBNAIL_CARD_HEIGHT_DIP: u32 = 164;
const THUMBNAIL_HEADER_DIP: u32 = 36;
const MAX_COMPACT_ROWS: usize = 6;
const MAX_THUMBNAIL_CARDS: usize = 3;

pub fn order_picker_windows(
    windows: &[WindowInfo],
    foreground: Option<TrackedWindowKey>,
    recent: &[TrackedWindowKey],
) -> Vec<WindowInfo> {
    let mut ordered = Vec::with_capacity(windows.len());
    for window in foreground
        .and_then(|key| windows.iter().find(|window| window.key() == key))
        .into_iter()
        .chain(
            recent
                .iter()
                .filter_map(|key| windows.iter().find(|window| window.key() == *key)),
        )
        .chain(windows.iter())
    {
        if !ordered
            .iter()
            .any(|existing: &WindowInfo| existing.key() == window.key())
        {
            ordered.push(window.clone());
        }
    }
    ordered
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppMenuAction {
    Open,
    CustomizeIcon,
    TogglePin,
    Close,
    ForceClose,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PowerAction {
    Lock,
    Restart,
    ShutDown,
    Cancel,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PopupAction {
    System(SystemAction),
    Power(PowerAction),
    App {
        action: AppMenuAction,
        identity: String,
    },
    Activate(TrackedWindowKey),
    CloseWindow(TrackedWindowKey),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PopupSymbol {
    Power,
    Lock,
    Restart,
    Settings,
    Quit,
    Open,
    Image,
    Pin,
    Unpin,
    Close,
    Previous,
    Next,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PickerWindow<Asset> {
    pub key: TrackedWindowKey,
    pub title: String,
    pub icon: Icon<Asset>,
    pub active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PopupIcon<Asset> {
    Symbol(PopupSymbol),
    Artwork(Icon<Asset>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PopupEntry<Asset> {
    pub action: PopupAction,
    pub label: String,
    pub icon: PopupIcon<Asset>,
    pub bounds: PhysicalRect,
    pub preview: Option<PhysicalRect>,
    pub close: Option<PhysicalRect>,
    pub active: bool,
    pub highlighted: bool,
    pub close_highlighted: bool,
}

pub struct DockPopup<Asset> {
    scale: DpiScale,
    kind: PopupKind<Asset>,
    hovered: Option<(usize, bool)>,
    selected: Option<usize>,
    offset: usize,
    theme: Theme,
}

enum PopupKind<Asset> {
    System(Box<ActionMenu>),
    Power,
    App {
        source_index: usize,
        identity: String,
        entries: Vec<AppEntry>,
    },
    Picker {
        source_index: usize,
        style: WindowPickerStyle,
        entries: Vec<PickerWindow<Asset>>,
    },
}

struct AppEntry {
    action: AppMenuAction,
    label: &'static str,
    symbol: PopupSymbol,
}

impl<Asset: Clone> DockPopup<Asset> {
    pub fn system(dpi: u32) -> Option<Self> {
        Some(Self {
            scale: DpiScale::new(dpi)?,
            kind: PopupKind::System(Box::new(ActionMenu::new(dpi)?)),
            hovered: None,
            selected: None,
            offset: 0,
            theme: Theme::default(),
        })
    }

    pub fn app(
        dpi: u32,
        source_index: usize,
        identity: String,
        running_windows: usize,
        pinned: bool,
        shift_held: bool,
    ) -> Option<Self> {
        let open = AppEntry {
            action: AppMenuAction::Open,
            label: "Open",
            symbol: PopupSymbol::Open,
        };
        let pin = AppEntry {
            action: AppMenuAction::TogglePin,
            label: if pinned {
                "Unpin from Lotus"
            } else {
                "Pin to Lotus"
            },
            symbol: if pinned {
                PopupSymbol::Unpin
            } else {
                PopupSymbol::Pin
            },
        };
        let customize = AppEntry {
            action: AppMenuAction::CustomizeIcon,
            label: "Customize icon",
            symbol: PopupSymbol::Image,
        };
        let close = (running_windows != 0).then_some(AppEntry {
            action: if shift_held {
                AppMenuAction::ForceClose
            } else {
                AppMenuAction::Close
            },
            label: if shift_held {
                if running_windows == 1 {
                    "Force close window"
                } else {
                    "Force close all windows"
                }
            } else if running_windows == 1 {
                "Close window"
            } else {
                "Close all windows"
            },
            symbol: PopupSymbol::Close,
        });
        Some(Self {
            scale: DpiScale::new(dpi)?,
            kind: PopupKind::App {
                source_index,
                identity,
                entries: [Some(open), Some(customize), Some(pin), close]
                    .into_iter()
                    .flatten()
                    .collect(),
            },
            hovered: None,
            selected: None,
            offset: 0,
            theme: Theme::default(),
        })
    }

    pub fn power(dpi: u32) -> Option<Self> {
        Some(Self {
            scale: DpiScale::new(dpi)?,
            kind: PopupKind::Power,
            hovered: None,
            selected: None,
            offset: 0,
            theme: Theme::default(),
        })
    }

    pub fn picker(
        dpi: u32,
        source_index: usize,
        style: WindowPickerStyle,
        entries: Vec<PickerWindow<Asset>>,
    ) -> Option<Self> {
        (!entries.is_empty()).then_some(Self {
            scale: DpiScale::new(dpi)?,
            kind: PopupKind::Picker {
                source_index,
                style,
                entries,
            },
            hovered: None,
            selected: None,
            offset: 0,
            theme: Theme::default(),
        })
    }

    pub const fn dpi(&self) -> u32 {
        self.scale.dpi()
    }

    pub const fn theme(&self) -> Theme {
        self.theme
    }

    pub fn set_theme(&mut self, theme: Theme) -> bool {
        if self.theme == theme {
            return false;
        }
        self.theme = theme;
        if let PopupKind::System(menu) = &mut self.kind {
            let _ = menu.set_theme(theme);
        }
        true
    }

    pub fn set_dpi(&mut self, dpi: u32) -> bool {
        let Some(scale) = DpiScale::new(dpi) else {
            return false;
        };
        if self.scale == scale {
            return false;
        }
        self.scale = scale;
        if let PopupKind::System(menu) = &mut self.kind {
            let _ = menu.set_dpi(dpi);
        }
        true
    }

    pub const fn source_index(&self) -> Option<usize> {
        match &self.kind {
            PopupKind::System(_) | PopupKind::Power => None,
            PopupKind::App { source_index, .. }
            | PopupKind::Picker { source_index, .. } => Some(*source_index),
        }
    }

    pub const fn picker_style(&self) -> Option<WindowPickerStyle> {
        match self.kind {
            PopupKind::Picker { style, .. } => Some(style),
            PopupKind::System(_) | PopupKind::Power | PopupKind::App { .. } => None,
        }
    }

    pub fn desired_size(&self) -> NonZeroPhysicalSize {
        match &self.kind {
            PopupKind::System(menu) => menu.desired_size(),
            PopupKind::Power => self.vertical_size(POWER_WIDTH_DIP, POWER_ROW_DIP, 4),
            PopupKind::App { entries, .. } => NonZeroPhysicalSize::new(
                self.scale.physical(APP_WIDTH_DIP),
                self.scale.physical(
                    PADDING_DIP * 2
                        + APP_ROW_DIP * u32::try_from(entries.len()).unwrap_or(u32::MAX)
                        + GAP_DIP
                            * u32::try_from(entries.len().saturating_sub(1))
                                .unwrap_or(u32::MAX),
                ),
            )
            .expect("app popup dimensions are nonzero"),
            PopupKind::Picker { style, entries, .. } => {
                self.picker_size(*style, entries.len())
            }
        }
    }

    pub fn entries(&self) -> Vec<PopupEntry<Asset>> {
        match &self.kind {
            PopupKind::System(menu) => menu
                .items()
                .into_iter()
                .enumerate()
                .map(|(index, (action, bounds))| PopupEntry {
                    action: PopupAction::System(action),
                    label: system_label(action).to_owned(),
                    icon: PopupIcon::Symbol(system_symbol(action)),
                    bounds,
                    preview: None,
                    close: None,
                    active: false,
                    highlighted: self.highlighted(index),
                    close_highlighted: false,
                })
                .collect(),
            PopupKind::Power => power_entries()
                .into_iter()
                .enumerate()
                .map(|(index, (action, label, symbol))| PopupEntry {
                    action: PopupAction::Power(action),
                    label: label.to_owned(),
                    icon: PopupIcon::Symbol(symbol),
                    bounds: self.vertical_row(index, POWER_WIDTH_DIP, POWER_ROW_DIP),
                    preview: None,
                    close: None,
                    active: false,
                    highlighted: self.highlighted(index),
                    close_highlighted: false,
                })
                .collect(),
            PopupKind::App {
                identity, entries, ..
            } => entries
                .iter()
                .enumerate()
                .map(|(index, entry)| PopupEntry {
                    action: PopupAction::App {
                        action: entry.action,
                        identity: identity.clone(),
                    },
                    label: entry.label.to_owned(),
                    icon: PopupIcon::Symbol(entry.symbol),
                    bounds: self.vertical_row(index, APP_WIDTH_DIP, APP_ROW_DIP),
                    preview: None,
                    close: None,
                    active: false,
                    highlighted: self.highlighted(index),
                    close_highlighted: false,
                })
                .collect(),
            PopupKind::Picker { style, entries, .. } => {
                self.picker_entries(*style, entries)
            }
        }
    }

    pub fn presentation(
        &self,
        asset_for: impl Fn(PopupSymbol) -> Asset,
    ) -> Presentation<Asset> {
        let theme = self.theme();
        let size = self.desired_size();
        let mut presentation = Presentation::new(Theme::default().canvas.with_alpha(0.0));
        presentation.push(PresentationPrimitive::FillRoundedRect {
            bounds: PresentationRect::new(
                0.5,
                0.5,
                as_f32(size.width()) - 0.5,
                as_f32(size.height()) - 0.5,
            ),
            radius: self.scale_f32(theme.radii.window),
            color: theme.chrome_overlay,
        });

        let icon_size = (20 * self.dpi()).div_ceil(96);
        let fallback_size = (42 * self.dpi()).div_ceil(96);
        for entry in &self.entries() {
            self.present_entry(
                &mut presentation,
                entry,
                icon_size,
                fallback_size,
                &asset_for,
            );
        }
        self.present_picker_navigation(&mut presentation, icon_size, &asset_for);
        presentation
    }

    pub fn pointer_move(&mut self, x: i32, y: i32) -> bool {
        let hovered = self.entry_at(x, y);
        let changed = self.hovered != hovered;
        self.hovered = hovered;
        self.selected = None;
        changed
    }

    pub fn pointer_left(&mut self) -> bool {
        self.hovered.take().is_some()
    }

    pub fn pointer_action(&self, x: i32, y: i32) -> Option<PopupAction> {
        let (index, close) = self.entry_at(x, y)?;
        let entry = self.entries().into_iter().nth(index)?;
        if close {
            entry.close.map(|_| match entry.action {
                PopupAction::Activate(window) => PopupAction::CloseWindow(window),
                ref action => action.clone(),
            })
        } else {
            Some(entry.action)
        }
    }

    pub fn selected_action(&self) -> Option<PopupAction> {
        let selected = match self.kind {
            PopupKind::Power => self.selected?,
            _ => self.selected.unwrap_or(0),
        };
        self.entries()
            .get(selected)
            .map(|entry| entry.action.clone())
    }

    pub fn move_selection(&mut self, next: bool) -> bool {
        let count = self.entries().len();
        if count == 0 {
            return false;
        }
        let selected = self.selected.unwrap_or_else(|| {
            if next {
                count - 1
            } else {
                0
            }
        });
        self.selected = Some(if next {
            (selected + 1) % count
        } else {
            selected.checked_sub(1).unwrap_or(count - 1)
        });
        self.hovered = None;
        self.keep_selection_visible();
        true
    }

    pub fn scroll(&mut self, next: bool) -> bool {
        let Some((visible, total)) = self.picker_extent() else {
            return false;
        };
        let maximum = total.saturating_sub(visible);
        let next_offset = if next {
            self.offset.saturating_add(1).min(maximum)
        } else {
            self.offset.saturating_sub(1)
        };
        if next_offset == self.offset {
            return false;
        }
        self.offset = next_offset;
        true
    }

    pub fn picker_previews(&self) -> Vec<(TrackedWindowKey, PhysicalRect)> {
        self.entries()
            .into_iter()
            .filter_map(|entry| match (entry.action, entry.preview) {
                (PopupAction::Activate(window), Some(preview)) => Some((window, preview)),
                _ => None,
            })
            .collect()
    }

    pub fn picker_navigation(&self) -> Option<(bool, bool)> {
        let (visible, total) = self.picker_extent()?;
        Some((
            self.offset != 0,
            self.offset.saturating_add(visible) < total,
        ))
    }

    fn present_entry(
        &self,
        presentation: &mut Presentation<Asset>,
        entry: &PopupEntry<Asset>,
        icon_size: u32,
        fallback_size: u32,
        asset_for: &impl Fn(PopupSymbol) -> Asset,
    ) {
        let theme = self.theme();
        let bounds = presentation_rect(entry.bounds);
        let radius = self.scale_f32(theme.radii.control);
        if entry.highlighted {
            presentation.push(PresentationPrimitive::FillRoundedRect {
                bounds,
                radius,
                color: theme.control_hover,
            });
        }
        if entry.active {
            presentation.push(PresentationPrimitive::StrokeRoundedRect {
                bounds,
                radius,
                width: 1.0,
                color: theme.control_selected,
            });
        }

        let artwork_size = if entry.preview.is_some() {
            fallback_size
        } else {
            icon_size
        };
        let icon = match &entry.icon {
            PopupIcon::Symbol(symbol) => Icon::Embedded(asset_for(*symbol)),
            PopupIcon::Artwork(icon) => icon.clone(),
        };
        presentation.push(PresentationPrimitive::Icon {
            icon,
            bounds: popup_icon_bounds(entry, artwork_size),
            tint: theme.text,
            opacity: 1.0,
            sampling: ImageSampling::Smooth,
            radius: 0.0,
        });
        self.present_label(presentation, entry);
        self.present_close(presentation, entry, icon_size, asset_for);
    }

    fn present_label(
        &self,
        presentation: &mut Presentation<Asset>,
        entry: &PopupEntry<Asset>,
    ) {
        if entry.label.is_empty() {
            return;
        }

        let mut bounds = presentation_rect(entry.bounds);
        if let Some(preview) = entry.preview {
            bounds.left += 12.0;
            bounds.bottom = as_f32(preview.min_y());
        } else {
            bounds.left += bounds.height();
        }
        if let Some(close) = entry.close {
            bounds.right = as_f32(close.min_x().saturating_sub(4));
        }
        presentation.push(PresentationPrimitive::Text {
            value: entry.label.clone(),
            bounds,
            style: TextStyle {
                size: 13.5 * as_f32(self.dpi()) / 96.0,
                family: lotus_ui::presentation::FontFamily::Interface,
                weight: FontWeight::Normal,
                horizontal: HorizontalAlignment::Leading,
                vertical: VerticalAlignment::Center,
            },
            color: self.theme().text,
        });
    }

    fn present_close(
        &self,
        presentation: &mut Presentation<Asset>,
        entry: &PopupEntry<Asset>,
        icon_size: u32,
        asset_for: &impl Fn(PopupSymbol) -> Asset,
    ) {
        let Some(close) = entry.close.filter(|_| entry.highlighted) else {
            return;
        };
        if entry.close_highlighted {
            presentation.push(PresentationPrimitive::FillRoundedRect {
                bounds: presentation_rect(close),
                radius: as_f32(close.height()) * 0.25,
                color: self.theme().control_hover,
            });
        }
        presentation.push(PresentationPrimitive::Icon {
            icon: Icon::Embedded(asset_for(PopupSymbol::Close)),
            bounds: centered_rect(close, icon_size),
            tint: self.theme().text,
            opacity: 1.0,
            sampling: ImageSampling::Smooth,
            radius: 0.0,
        });
    }

    fn present_picker_navigation(
        &self,
        presentation: &mut Presentation<Asset>,
        icon_size: u32,
        asset_for: &impl Fn(PopupSymbol) -> Asset,
    ) {
        let Some((previous, next)) = self.picker_navigation() else {
            return;
        };
        let size = self.desired_size();
        let diameter = (28 * self.dpi()).div_ceil(96);
        let top = size.height().saturating_sub(diameter) / 2;
        if previous {
            self.present_navigation(
                presentation,
                physical_rect(2, top, diameter, diameter),
                icon_size,
                PopupSymbol::Previous,
                asset_for,
            );
        }
        if next {
            self.present_navigation(
                presentation,
                physical_rect(
                    size.width().saturating_sub(diameter + 2),
                    top,
                    diameter,
                    diameter,
                ),
                icon_size,
                PopupSymbol::Next,
                asset_for,
            );
        }
    }

    fn present_navigation(
        &self,
        presentation: &mut Presentation<Asset>,
        bounds: PhysicalRect,
        icon_size: u32,
        symbol: PopupSymbol,
        asset_for: &impl Fn(PopupSymbol) -> Asset,
    ) {
        presentation.push(PresentationPrimitive::FillRoundedRect {
            bounds: presentation_rect(bounds),
            radius: as_f32(bounds.height()) / 2.0,
            color: self.theme().control_hover,
        });
        presentation.push(PresentationPrimitive::Icon {
            icon: Icon::Embedded(asset_for(symbol)),
            bounds: centered_rect(bounds, icon_size),
            tint: self.theme().text,
            opacity: 1.0,
            sampling: ImageSampling::Smooth,
            radius: 0.0,
        });
    }

    fn scale_f32(&self, dips: f32) -> f32 {
        as_f32(self.dpi()) * dips / 96.0
    }

    fn picker_entries(
        &self,
        style: WindowPickerStyle,
        entries: &[PickerWindow<Asset>],
    ) -> Vec<PopupEntry<Asset>> {
        let visible = match style {
            WindowPickerStyle::Compact => MAX_COMPACT_ROWS,
            WindowPickerStyle::Thumbnails => MAX_THUMBNAIL_CARDS,
        };
        entries
            .iter()
            .skip(self.offset)
            .take(visible)
            .enumerate()
            .map(|(visual, entry)| {
                let index = self.offset + visual;
                let (bounds, preview, close) = match style {
                    WindowPickerStyle::Compact => {
                        let bounds =
                            self.vertical_row(visual, COMPACT_WIDTH_DIP, COMPACT_ROW_DIP);
                        let close_size = self.scale.physical(24);
                        let close = physical_rect(
                            bounds
                                .max_x()
                                .saturating_sub(close_size + self.scale.physical(8)),
                            bounds.min_y().saturating_add(
                                bounds.height().saturating_sub(close_size) / 2,
                            ),
                            close_size,
                            close_size,
                        );
                        (bounds, None, Some(close))
                    }
                    WindowPickerStyle::Thumbnails => {
                        let padding = self.scale.physical(PADDING_DIP);
                        let gap = self.scale.physical(GAP_DIP);
                        let width = self.scale.physical(THUMBNAIL_CARD_WIDTH_DIP);
                        let height = self.scale.physical(THUMBNAIL_CARD_HEIGHT_DIP);
                        let left = padding.saturating_add(
                            u32::try_from(visual)
                                .unwrap_or(u32::MAX)
                                .saturating_mul(width.saturating_add(gap)),
                        );
                        let bounds = physical_rect(left, padding, width, height);
                        let header = self.scale.physical(THUMBNAIL_HEADER_DIP);
                        let inset = self.scale.physical(4);
                        let preview = physical_rect(
                            left.saturating_add(inset),
                            padding.saturating_add(header),
                            width.saturating_sub(inset * 2),
                            height.saturating_sub(header + inset),
                        );
                        let close_size = self.scale.physical(24);
                        let close = physical_rect(
                            bounds.max_x().saturating_sub(close_size + inset),
                            bounds.min_y().saturating_add(inset),
                            close_size,
                            close_size,
                        );
                        (bounds, Some(preview), Some(close))
                    }
                };
                let close_highlighted = self.hovered == Some((index, true));
                PopupEntry {
                    action: PopupAction::Activate(entry.key),
                    label: entry.title.clone(),
                    icon: PopupIcon::Artwork(entry.icon.clone()),
                    bounds,
                    preview,
                    close,
                    active: entry.active,
                    highlighted: self.highlighted(index),
                    close_highlighted,
                }
            })
            .collect()
    }

    fn picker_size(&self, style: WindowPickerStyle, count: usize) -> NonZeroPhysicalSize {
        let visible = match style {
            WindowPickerStyle::Compact => count.clamp(1, MAX_COMPACT_ROWS),
            WindowPickerStyle::Thumbnails => count.clamp(1, MAX_THUMBNAIL_CARDS),
        };
        let visible = u32::try_from(visible).unwrap_or(u32::MAX);
        let (width, height) = match style {
            WindowPickerStyle::Compact => (
                COMPACT_WIDTH_DIP,
                PADDING_DIP * 2 + visible * COMPACT_ROW_DIP + (visible - 1) * GAP_DIP,
            ),
            WindowPickerStyle::Thumbnails => (
                PADDING_DIP * 2
                    + visible * THUMBNAIL_CARD_WIDTH_DIP
                    + (visible - 1) * GAP_DIP,
                PADDING_DIP * 2 + THUMBNAIL_CARD_HEIGHT_DIP,
            ),
        };
        NonZeroPhysicalSize::new(self.scale.physical(width), self.scale.physical(height))
            .expect("picker popup dimensions are nonzero")
    }

    fn vertical_row(
        &self,
        index: usize,
        width_dips: u32,
        height_dips: u32,
    ) -> PhysicalRect {
        let padding = self.scale.physical(PADDING_DIP);
        let gap = self.scale.physical(GAP_DIP);
        let height = self.scale.physical(height_dips);
        let top = padding.saturating_add(
            u32::try_from(index)
                .unwrap_or(u32::MAX)
                .saturating_mul(height.saturating_add(gap)),
        );
        physical_rect(
            padding,
            top,
            self.scale.physical(width_dips).saturating_sub(padding * 2),
            height,
        )
    }

    fn vertical_size(
        &self,
        width_dips: u32,
        row_dips: u32,
        count: u32,
    ) -> NonZeroPhysicalSize {
        NonZeroPhysicalSize::new(
            self.scale.physical(width_dips),
            self.scale.physical(
                PADDING_DIP * 2 + row_dips * count + GAP_DIP * count.saturating_sub(1),
            ),
        )
        .expect("popup dimensions are nonzero")
    }

    fn highlighted(&self, index: usize) -> bool {
        self.hovered.is_some_and(|hovered| hovered.0 == index)
            || self.selected == Some(index)
    }

    fn entry_at(&self, x: i32, y: i32) -> Option<(usize, bool)> {
        let point =
            PhysicalUnsignedPoint::new(u32::try_from(x).ok()?, u32::try_from(y).ok()?);
        self.entries()
            .iter()
            .enumerate()
            .find_map(|(visual, entry)| {
                let index = self.offset + visual;
                entry
                    .close
                    .filter(|close| close.contains(point))
                    .map(|_| (index, true))
                    .or_else(|| entry.bounds.contains(point).then_some((index, false)))
            })
    }

    fn picker_extent(&self) -> Option<(usize, usize)> {
        let PopupKind::Picker { style, entries, .. } = &self.kind else {
            return None;
        };
        Some((
            match style {
                WindowPickerStyle::Compact => MAX_COMPACT_ROWS,
                WindowPickerStyle::Thumbnails => MAX_THUMBNAIL_CARDS,
            },
            entries.len(),
        ))
    }

    fn keep_selection_visible(&mut self) {
        let Some((visible, total)) = self.picker_extent() else {
            return;
        };
        let Some(selected) = self.selected else {
            return;
        };
        if selected < self.offset {
            self.offset = selected;
        } else if selected >= self.offset + visible {
            self.offset = selected + 1 - visible;
        }
        self.offset = self.offset.min(total.saturating_sub(visible));
    }
}

fn popup_icon_bounds<Asset>(entry: &PopupEntry<Asset>, size: u32) -> PresentationRect {
    if let Some(preview) = entry.preview {
        centered_rect(preview, size)
    } else if entry.label.is_empty() {
        centered_rect(entry.bounds, size)
    } else {
        let inset = entry.bounds.height().saturating_sub(size) / 2;
        PresentationRect::new(
            as_f32(entry.bounds.min_x().saturating_add(inset)),
            as_f32(entry.bounds.min_y().saturating_add(inset)),
            as_f32(entry.bounds.min_x().saturating_add(inset + size)),
            as_f32(entry.bounds.min_y().saturating_add(inset + size)),
        )
    }
}

fn centered_rect(bounds: PhysicalRect, size: u32) -> PresentationRect {
    let left = bounds
        .min_x()
        .saturating_add(bounds.width().saturating_sub(size) / 2);
    let top = bounds
        .min_y()
        .saturating_add(bounds.height().saturating_sub(size) / 2);
    PresentationRect::new(
        as_f32(left),
        as_f32(top),
        as_f32(left.saturating_add(size)),
        as_f32(top.saturating_add(size)),
    )
}

fn presentation_rect(bounds: PhysicalRect) -> PresentationRect {
    PresentationRect::new(
        as_f32(bounds.min_x()),
        as_f32(bounds.min_y()),
        as_f32(bounds.max_x()),
        as_f32(bounds.max_y()),
    )
}

#[allow(
    clippy::cast_precision_loss,
    reason = "popup dimensions remain below f32 exact range"
)]
const fn as_f32(value: u32) -> f32 {
    value as f32
}

const fn power_entries() -> [(PowerAction, &'static str, PopupSymbol); 4] {
    [
        (PowerAction::Lock, "Lock", PopupSymbol::Lock),
        (PowerAction::Restart, "Restart", PopupSymbol::Restart),
        (PowerAction::ShutDown, "Shut down", PopupSymbol::Power),
        (PowerAction::Cancel, "Cancel", PopupSymbol::Close),
    ]
}

const fn system_symbol(action: SystemAction) -> PopupSymbol {
    match action {
        SystemAction::RequestShutdown => PopupSymbol::Power,
        SystemAction::OpenSettings => PopupSymbol::Settings,
        SystemAction::QuitLotus => PopupSymbol::Quit,
    }
}

const fn system_label(action: SystemAction) -> &'static str {
    match action {
        SystemAction::RequestShutdown => "Power",
        SystemAction::OpenSettings => "Settings",
        SystemAction::QuitLotus => "Quit Lotus",
    }
}
