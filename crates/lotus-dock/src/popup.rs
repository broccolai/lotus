use lotus_core::settings::WindowPickerStyle;
use lotus_core::window::{TrackedWindowKey, WindowInfo};
use lotus_ui::geometry::{
    DpiScale, NonZeroPhysicalSize, PhysicalRect, PhysicalUnsignedPoint, physical_rect,
};
use lotus_ui::icon::Icon;
use lotus_ui::theme::Theme;

use crate::action_menu::{Action as SystemAction, ActionMenu};

mod entries;
mod presentation;

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
    OpenFileLocation(String),
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
    FileLocation(String),
    App {
        source_index: usize,
        identity: String,
        running_windows: usize,
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
                    "Force close"
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
                running_windows,
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

    pub fn file_location(dpi: u32, path: String) -> Option<Self> {
        Some(Self {
            scale: DpiScale::new(dpi)?,
            kind: PopupKind::FileLocation(path),
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

    pub fn set_shift_held(&mut self, shift_held: bool) -> bool {
        let PopupKind::App {
            running_windows,
            entries,
            ..
        } = &mut self.kind
        else {
            return false;
        };
        let Some(close) = entries.iter_mut().find(|entry| {
            matches!(
                entry.action,
                AppMenuAction::Close | AppMenuAction::ForceClose
            )
        }) else {
            return false;
        };
        let action = if shift_held {
            AppMenuAction::ForceClose
        } else {
            AppMenuAction::Close
        };
        let label = if shift_held {
            if *running_windows == 1 {
                "Force close"
            } else {
                "Force close all windows"
            }
        } else if *running_windows == 1 {
            "Close window"
        } else {
            "Close all windows"
        };
        if close.action == action && close.label == label {
            return false;
        }
        close.action = action;
        close.label = label;
        true
    }

    pub const fn source_index(&self) -> Option<usize> {
        match &self.kind {
            PopupKind::System(_) | PopupKind::Power | PopupKind::FileLocation(_) => None,
            PopupKind::App { source_index, .. }
            | PopupKind::Picker { source_index, .. } => Some(*source_index),
        }
    }

    pub const fn picker_style(&self) -> Option<WindowPickerStyle> {
        match self.kind {
            PopupKind::Picker { style, .. } => Some(style),
            PopupKind::System(_)
            | PopupKind::Power
            | PopupKind::FileLocation(_)
            | PopupKind::App { .. } => None,
        }
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
