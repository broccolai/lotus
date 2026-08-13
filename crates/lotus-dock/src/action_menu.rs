use lotus_ui::geometry::{
    DpiScale, NonZeroPhysicalSize, PhysicalRect, PhysicalUnsignedPoint, physical_rect,
};
use lotus_ui::theme::Theme;

const BUTTON_DIPS: u32 = 36;
const GAP_DIPS: u32 = 4;
const OUTER_PADDING_DIPS: u32 = 4;
const ITEM_COUNT: u32 = 5;
const WIDTH_DIPS: u32 =
    OUTER_PADDING_DIPS * 2 + BUTTON_DIPS * ITEM_COUNT + GAP_DIPS * (ITEM_COUNT - 1);
const HEIGHT_DIPS: u32 = OUTER_PADDING_DIPS * 2 + BUTTON_DIPS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    OpenSettings,
    OpenVolumeMixer,
    OpenTrayOverflow,
    RequestShutdown,
    QuitLotus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Previous,
    Next,
}

pub struct ActionMenu {
    scale: DpiScale,
    hovered: Option<Action>,
    keyboard_selected: Option<Action>,
    theme: Theme,
}

impl ActionMenu {
    pub fn new(dpi: u32) -> Option<Self> {
        Some(Self {
            scale: DpiScale::new(dpi)?,
            hovered: None,
            keyboard_selected: None,
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
        true
    }

    pub fn set_dpi(&mut self, dpi: u32) -> bool {
        let Some(scale) = DpiScale::new(dpi) else {
            return false;
        };
        if scale == self.scale {
            return false;
        }
        self.scale = scale;
        true
    }

    pub fn desired_size(&self) -> NonZeroPhysicalSize {
        NonZeroPhysicalSize::new(self.scale(WIDTH_DIPS), self.scale(HEIGHT_DIPS))
            .expect("context menu dimensions are nonzero")
    }

    pub fn items(&self) -> [(Action, PhysicalRect); 5] {
        let padding = self.scale(OUTER_PADDING_DIPS);
        let button = self.scale(BUTTON_DIPS);
        let gap = self.scale(GAP_DIPS);
        [
            (
                Action::RequestShutdown,
                physical_rect(padding, padding, button, button),
            ),
            (
                Action::OpenVolumeMixer,
                physical_rect(
                    padding.saturating_add(button).saturating_add(gap),
                    padding,
                    button,
                    button,
                ),
            ),
            (
                Action::OpenSettings,
                physical_rect(
                    padding
                        .saturating_add(button.saturating_mul(2))
                        .saturating_add(gap.saturating_mul(2)),
                    padding,
                    button,
                    button,
                ),
            ),
            (
                Action::OpenTrayOverflow,
                physical_rect(
                    padding
                        .saturating_add(button.saturating_mul(3))
                        .saturating_add(gap.saturating_mul(3)),
                    padding,
                    button,
                    button,
                ),
            ),
            (
                Action::QuitLotus,
                physical_rect(
                    padding
                        .saturating_add(button.saturating_mul(4))
                        .saturating_add(gap.saturating_mul(4)),
                    padding,
                    button,
                    button,
                ),
            ),
        ]
    }

    pub fn highlighted(&self, action: Action) -> bool {
        self.hovered == Some(action) || self.keyboard_selected == Some(action)
    }

    pub fn pointer_move(&mut self, x: i32, y: i32) -> bool {
        let hovered = self.action_at(x, y);
        let changed = self.hovered != hovered || self.keyboard_selected.is_some();
        self.hovered = hovered;
        self.keyboard_selected = None;
        changed
    }

    pub fn pointer_left(&mut self) -> bool {
        self.hovered.take().is_some()
    }

    pub fn pointer_action(&self, x: i32, y: i32) -> Option<Action> {
        self.action_at(x, y)
    }

    pub const fn selected_action(&self) -> Action {
        match self.keyboard_selected {
            Some(action) => action,
            None => Action::RequestShutdown,
        }
    }

    pub fn move_selection(&mut self, _direction: Direction) -> bool {
        let selected = match self.keyboard_selected {
            None | Some(Action::QuitLotus) => Action::RequestShutdown,
            Some(Action::RequestShutdown) => Action::OpenVolumeMixer,
            Some(Action::OpenVolumeMixer) => Action::OpenSettings,
            Some(Action::OpenSettings) => Action::OpenTrayOverflow,
            Some(Action::OpenTrayOverflow) => Action::QuitLotus,
        };
        let changed = self.keyboard_selected != Some(selected) || self.hovered.is_some();
        self.keyboard_selected = Some(selected);
        self.hovered = None;
        changed
    }

    fn action_at(&self, x: i32, y: i32) -> Option<Action> {
        let point =
            PhysicalUnsignedPoint::new(u32::try_from(x).ok()?, u32::try_from(y).ok()?);
        self.items()
            .into_iter()
            .find_map(|(action, bounds)| bounds.contains(point).then_some(action))
    }

    fn scale(&self, dips: u32) -> u32 {
        self.scale.physical(dips)
    }
}
