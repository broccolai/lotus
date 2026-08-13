use std::num::NonZeroU32;

use lotus_core::settings::{DockSettings, NotificationBadgeStyle};
use lotus_ui::theme::Theme;

use crate::appearance::{AccentPreset, SurfacePreset, theme_for};

const DIPS_PER_INCH: u64 = 96;
const WIDTH_DIP: u32 = 900;
const HEIGHT_DIP: u32 = 730;
const NAV_LEFT_DIP: u32 = 14;
const NAV_WIDTH_DIP: u32 = 180;
const NAV_HEIGHT_DIP: u32 = 44;
const CONTENT_LEFT_DIP: u32 = 244;
const CONTENT_RIGHT_DIP: u32 = 32;
const CONTENT_TOP_DIP: u32 = 76;
const CONTROL_COLUMN_LEFT_DIP: u32 = 250;
const CONTROL_COLUMN_RIGHT_DIP: u32 = 16;
const CONTROL_VALUE_GAP_DIP: u32 = 14;
const CONTROL_VALUE_WIDTH_DIP: u32 = 44;
const ROW_HEIGHT_DIP: u32 = 46;
const ROW_GAP_DIP: u32 = 4;
const ACTION_HEIGHT_DIP: u32 = NAV_HEIGHT_DIP;
const ACTION_GAP_DIP: u32 = 8;
const APPLY_WIDTH_DIP: u32 = 92;
const REVERT_WIDTH_DIP: u32 = 92;
const FOOTER_HEIGHT_DIP: u32 = 72;
const CLOSE_SIZE_DIP: u32 = 40;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsPage {
    Appearance,
    General,
    Taskbar,
    Search,
    About,
}

impl SettingsPage {
    pub const ALL: [Self; 5] =
        [Self::General, Self::Appearance, Self::Taskbar, Self::Search, Self::About];

    pub const fn title(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::General => "General",
            Self::Taskbar => "Taskbar",
            Self::Search => "Search",
            Self::About => "About",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsToggle {
    ShowUnpinnedRunningApps,
    ShowDesktopButton,
    StartWithWindows,
    ReplaceWindowsTaskbar,
    ExclusiveTaskbarReplacement,
    HideWhenFullscreen,
    SearchOpenWithWindowsKey,
    AltTabEnabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsSlider {
    IconSize,
    ItemSpacing,
    HorizontalPadding,
    VerticalPadding,
    BottomOffset,
    CornerRadius,
    BackgroundOpacity,
    SearchResultLimit,
}

impl SettingsSlider {
    pub const fn range(self) -> (u32, u32) {
        match self {
            Self::IconSize => (24, 72),
            Self::ItemSpacing => (2, 24),
            Self::HorizontalPadding => (4, 48),
            Self::VerticalPadding => (4, 32),
            Self::BottomOffset => (0, 96),
            Self::CornerRadius => (0, 48),
            Self::BackgroundOpacity => (8, 95),
            Self::SearchResultLimit => (1, 8),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsControl {
    Navigate(SettingsPage),
    SurfacePreset,
    AccentPreset,
    NotificationBadgeStyle,
    Toggle(SettingsToggle),
    Slider(SettingsSlider),
    ChooseMascotImage,
    ResetMascotImage,
    CheckForUpdates,
    Revert,
    Apply,
    Close,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SettingsAction {
    None,
    Changed,
    ChooseBackgroundColor,
    ChooseAccentColor,
    ChooseMascotImage,
    CheckForUpdates,
    Apply(Box<DockSettings>),
    Close,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SettingsUpdateActivity {
    #[default]
    Idle,
    Checking,
    Installing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsKey {
    Tab,
    ReverseTab,
    Left,
    Right,
    Up,
    Down,
    Activate,
    Escape,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SettingsPointerStyle {
    #[default]
    Default,
    Action,
    HorizontalAdjustment,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SettingsRect {
    pub left: u32,
    pub top: u32,
    pub width: u32,
    pub height: u32,
}

impl SettingsRect {
    pub const fn contains(self, x: u32, y: u32) -> bool {
        x >= self.left
            && x < self.left.saturating_add(self.width)
            && y >= self.top
            && y < self.top.saturating_add(self.height)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SettingsControlLayout {
    pub control: SettingsControl,
    pub bounds: SettingsRect,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingsLayout {
    pub size: SettingsSize,
    pub controls: Vec<SettingsControlLayout>,
}

impl SettingsLayout {
    pub fn hit_test(&self, x: u32, y: u32) -> Option<SettingsControl> {
        self.controls
            .iter()
            .rev()
            .find(|entry| entry.bounds.contains(x, y))
            .map(|entry| entry.control)
    }

    pub fn bounds(&self, control: SettingsControl) -> Option<SettingsRect> {
        self.controls.iter().find(|entry| entry.control == control).map(|entry| entry.bounds)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SettingsSize {
    width: NonZeroU32,
    height: NonZeroU32,
}

impl SettingsSize {
    pub const fn new(width: u32, height: u32) -> Option<Self> {
        let Some(width) = NonZeroU32::new(width) else {
            return None;
        };
        let Some(height) = NonZeroU32::new(height) else {
            return None;
        };
        Some(Self { width, height })
    }

    pub const fn width(self) -> u32 {
        self.width.get()
    }

    pub const fn height(self) -> u32 {
        self.height.get()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SettingsScene {
    dpi: NonZeroU32,
    page: SettingsPage,
    baseline: DockSettings,
    draft: DockSettings,
    hovered: Option<SettingsControl>,
    focused: Option<SettingsControl>,
    focus_visible: bool,
    installed: bool,
    update_activity: SettingsUpdateActivity,
}

impl SettingsScene {
    pub fn new(dpi: u32, settings: DockSettings, installed: bool) -> Option<Self> {
        let dpi = NonZeroU32::new(dpi)?;
        let settings = settings.normalized();
        Some(Self {
            dpi,
            page: SettingsPage::General,
            baseline: settings.clone(),
            draft: settings,
            hovered: None,
            focused: Some(SettingsControl::Navigate(SettingsPage::General)),
            focus_visible: false,
            installed,
            update_activity: SettingsUpdateActivity::Idle,
        })
    }

    pub const fn dpi(&self) -> u32 {
        self.dpi.get()
    }

    pub fn set_dpi(&mut self, dpi: u32) -> bool {
        let Some(dpi) = NonZeroU32::new(dpi) else { return false };
        if self.dpi == dpi {
            return false;
        }
        self.dpi = dpi;
        true
    }

    pub const fn page(&self) -> SettingsPage {
        self.page
    }
    pub const fn hovered(&self) -> Option<SettingsControl> {
        self.hovered
    }
    pub const fn focused(&self) -> Option<SettingsControl> {
        self.focused
    }
    pub const fn focus_visible(&self) -> bool {
        self.focus_visible
    }
    pub fn draft(&self) -> &DockSettings {
        &self.draft
    }
    pub fn theme(&self) -> Theme {
        theme_for(&self.draft)
    }
    pub fn is_dirty(&self) -> bool {
        self.draft != self.baseline
    }
    pub const fn update_activity(&self) -> SettingsUpdateActivity {
        self.update_activity
    }
    pub const fn is_installed(&self) -> bool {
        self.installed
    }
    pub fn set_update_activity(&mut self, activity: SettingsUpdateActivity) -> bool {
        if self.update_activity == activity {
            return false;
        }
        self.update_activity = activity;
        true
    }

    pub fn desired_size(&self) -> SettingsSize {
        SettingsSize {
            width: nonzero(self.scale(WIDTH_DIP)),
            height: nonzero(self.scale(HEIGHT_DIP)),
        }
    }

    pub fn layout(&self) -> SettingsLayout {
        let size = self.desired_size();
        let mut controls = Vec::new();
        for (index, page) in SettingsPage::ALL.into_iter().take(4).enumerate() {
            controls.push(SettingsControlLayout {
                control: SettingsControl::Navigate(page),
                bounds: self.rect(
                    NAV_LEFT_DIP,
                    76 + u32_index(index) * 50,
                    NAV_WIDTH_DIP,
                    NAV_HEIGHT_DIP,
                ),
            });
        }
        controls.push(SettingsControlLayout {
            control: SettingsControl::Navigate(SettingsPage::About),
            bounds: self.rect(NAV_LEFT_DIP, HEIGHT_DIP - 115, NAV_WIDTH_DIP, NAV_HEIGHT_DIP),
        });
        controls.push(SettingsControlLayout {
            control: SettingsControl::CheckForUpdates,
            bounds: self.rect(
                NAV_LEFT_DIP,
                HEIGHT_DIP - FOOTER_HEIGHT_DIP + (FOOTER_HEIGHT_DIP - NAV_HEIGHT_DIP) / 2,
                NAV_WIDTH_DIP,
                NAV_HEIGHT_DIP,
            ),
        });
        let content_width = WIDTH_DIP - CONTENT_LEFT_DIP - CONTENT_RIGHT_DIP;
        for (index, control) in self.page_controls().into_iter().enumerate() {
            let bounds = self.rect(
                CONTENT_LEFT_DIP,
                CONTENT_TOP_DIP + u32_index(index) * (ROW_HEIGHT_DIP + ROW_GAP_DIP),
                content_width,
                ROW_HEIGHT_DIP,
            );
            controls.push(SettingsControlLayout { control, bounds });
        }
        controls.push(SettingsControlLayout {
            control: SettingsControl::Revert,
            bounds: self.rect(
                WIDTH_DIP - CONTENT_RIGHT_DIP - APPLY_WIDTH_DIP - ACTION_GAP_DIP - REVERT_WIDTH_DIP,
                HEIGHT_DIP - FOOTER_HEIGHT_DIP + (FOOTER_HEIGHT_DIP - ACTION_HEIGHT_DIP) / 2,
                REVERT_WIDTH_DIP,
                ACTION_HEIGHT_DIP,
            ),
        });
        controls.push(SettingsControlLayout {
            control: SettingsControl::Apply,
            bounds: self.rect(
                WIDTH_DIP - CONTENT_RIGHT_DIP - APPLY_WIDTH_DIP,
                HEIGHT_DIP - FOOTER_HEIGHT_DIP + (FOOTER_HEIGHT_DIP - ACTION_HEIGHT_DIP) / 2,
                APPLY_WIDTH_DIP,
                ACTION_HEIGHT_DIP,
            ),
        });
        controls.push(SettingsControlLayout {
            control: SettingsControl::Close,
            bounds: self.rect(
                WIDTH_DIP - CONTENT_RIGHT_DIP - CLOSE_SIZE_DIP,
                12,
                CLOSE_SIZE_DIP,
                CLOSE_SIZE_DIP,
            ),
        });
        SettingsLayout { size, controls }
    }

    pub fn set_hovered(&mut self, control: Option<SettingsControl>) -> bool {
        if self.hovered == control {
            return false;
        }
        self.hovered = control;
        true
    }

    pub fn pointer_move(&mut self, x: u32, y: u32) -> bool {
        let hovered = self.layout().hit_test(x, y);
        self.set_hovered(hovered)
    }

    pub fn pointer_activate(&mut self, x: u32, y: u32) -> SettingsAction {
        let Some(control) = self.layout().hit_test(x, y) else { return SettingsAction::None };
        self.focused = Some(control);
        self.focus_visible = false;
        if matches!(
            control,
            SettingsControl::SurfacePreset
                | SettingsControl::AccentPreset
                | SettingsControl::NotificationBadgeStyle
        ) {
            let bounds = self.layout().bounds(control).expect("active control has layout bounds");
            return self.set_picker_from_pointer(control, bounds, x);
        }
        if let SettingsControl::Slider(slider) = control {
            return if self.slider_at(x, y) == Some(slider) {
                self.set_slider_from_pointer(slider, x)
            } else {
                SettingsAction::None
            };
        }
        self.activate(control)
    }

    pub fn pointer_style(&self, x: u32, y: u32) -> SettingsPointerStyle {
        let Some(control) = self.layout().hit_test(x, y) else {
            return SettingsPointerStyle::Default;
        };
        match control {
            SettingsControl::Slider(slider) if self.slider_at(x, y) == Some(slider) => {
                SettingsPointerStyle::HorizontalAdjustment
            }
            SettingsControl::Slider(_) => SettingsPointerStyle::Default,
            SettingsControl::Apply | SettingsControl::Revert if !self.is_dirty() => {
                SettingsPointerStyle::Default
            }
            SettingsControl::CheckForUpdates
                if self.update_activity != SettingsUpdateActivity::Idle =>
            {
                SettingsPointerStyle::Default
            }
            SettingsControl::Navigate(_)
            | SettingsControl::SurfacePreset
            | SettingsControl::AccentPreset
            | SettingsControl::NotificationBadgeStyle
            | SettingsControl::Toggle(_)
            | SettingsControl::ChooseMascotImage
            | SettingsControl::ResetMascotImage
            | SettingsControl::CheckForUpdates
            | SettingsControl::Revert
            | SettingsControl::Apply
            | SettingsControl::Close => SettingsPointerStyle::Action,
        }
    }

    pub fn slider_at(&self, x: u32, y: u32) -> Option<SettingsSlider> {
        let SettingsControl::Slider(slider) = self.layout().hit_test(x, y)? else {
            return None;
        };
        let bounds = self.layout().bounds(SettingsControl::Slider(slider))?;
        let (left, width) = self.slider_track(bounds);
        let tolerance = self.scale(8);
        (x >= left.saturating_sub(tolerance)
            && x < left.saturating_add(width).saturating_add(tolerance))
        .then_some(slider)
    }

    pub fn set_slider_from_pointer(&mut self, slider: SettingsSlider, x: u32) -> SettingsAction {
        let Some(bounds) = self.layout().bounds(SettingsControl::Slider(slider)) else {
            return SettingsAction::None;
        };
        let (track_left, track_width) = self.slider_track(bounds);
        let offset = x.saturating_sub(track_left).min(track_width);
        let (minimum, maximum) = slider.range();
        let value = minimum.saturating_add(offset.saturating_mul(maximum - minimum) / track_width);
        self.set_slider(slider, value);
        SettingsAction::Changed
    }

    pub fn key(&mut self, key: SettingsKey) -> SettingsAction {
        self.focus_visible = true;
        if key == SettingsKey::Escape {
            return SettingsAction::Close;
        }
        if matches!(
            key,
            SettingsKey::Tab | SettingsKey::ReverseTab | SettingsKey::Up | SettingsKey::Down
        ) {
            self.move_focus(matches!(key, SettingsKey::ReverseTab | SettingsKey::Up));
            return SettingsAction::None;
        }
        let Some(focused) = self.focused else { return SettingsAction::None };
        match (key, focused) {
            (SettingsKey::Activate, control) => self.activate(control),
            (SettingsKey::Left, SettingsControl::Slider(slider)) => self.adjust_slider(slider, -1),
            (SettingsKey::Right, SettingsControl::Slider(slider)) => self.adjust_slider(slider, 1),
            (SettingsKey::Left, SettingsControl::SurfacePreset) => self.cycle_surface_preset(true),
            (SettingsKey::Right, SettingsControl::SurfacePreset) => {
                self.cycle_surface_preset(false)
            }
            (SettingsKey::Left, SettingsControl::AccentPreset) => self.cycle_accent_preset(true),
            (SettingsKey::Right, SettingsControl::AccentPreset) => self.cycle_accent_preset(false),
            (SettingsKey::Left, SettingsControl::NotificationBadgeStyle) => {
                self.cycle_notification_badge_style(true)
            }
            (SettingsKey::Right, SettingsControl::NotificationBadgeStyle) => {
                self.cycle_notification_badge_style(false)
            }
            _ => SettingsAction::None,
        }
    }

    pub fn mark_applied(&mut self, applied: DockSettings) {
        let applied = applied.normalized();
        self.baseline = applied.clone();
        self.draft = applied;
    }

    fn page_controls(&self) -> Vec<SettingsControl> {
        match self.page {
            SettingsPage::Appearance => vec![
                SettingsControl::SurfacePreset,
                SettingsControl::AccentPreset,
                SettingsControl::Slider(SettingsSlider::BackgroundOpacity),
                SettingsControl::Slider(SettingsSlider::CornerRadius),
            ],
            SettingsPage::General => {
                let mut controls = vec![
                    SettingsControl::Toggle(SettingsToggle::StartWithWindows),
                    SettingsControl::Toggle(SettingsToggle::ShowUnpinnedRunningApps),
                    SettingsControl::Toggle(SettingsToggle::AltTabEnabled),
                    SettingsControl::NotificationBadgeStyle,
                    SettingsControl::ChooseMascotImage,
                ];
                if self.draft.mascot_image_path.is_some() {
                    controls.push(SettingsControl::ResetMascotImage);
                }
                controls
            }
            SettingsPage::Taskbar => vec![
                SettingsControl::Toggle(SettingsToggle::ReplaceWindowsTaskbar),
                SettingsControl::Toggle(SettingsToggle::ExclusiveTaskbarReplacement),
                SettingsControl::Toggle(SettingsToggle::HideWhenFullscreen),
                SettingsControl::Toggle(SettingsToggle::ShowDesktopButton),
                SettingsControl::Slider(SettingsSlider::IconSize),
                SettingsControl::Slider(SettingsSlider::ItemSpacing),
                SettingsControl::Slider(SettingsSlider::HorizontalPadding),
                SettingsControl::Slider(SettingsSlider::VerticalPadding),
                SettingsControl::Slider(SettingsSlider::BottomOffset),
            ],
            SettingsPage::Search => vec![
                SettingsControl::Toggle(SettingsToggle::SearchOpenWithWindowsKey),
                SettingsControl::Slider(SettingsSlider::SearchResultLimit),
            ],
            SettingsPage::About => Vec::new(),
        }
    }

    fn activate(&mut self, control: SettingsControl) -> SettingsAction {
        match control {
            SettingsControl::Navigate(page) => {
                self.page = page;
                self.focused = Some(SettingsControl::Navigate(page));
                SettingsAction::Changed
            }
            SettingsControl::Toggle(toggle) => {
                let value = self.toggle(toggle);
                self.set_toggle(toggle, !value);
                SettingsAction::Changed
            }
            SettingsControl::Apply if self.is_dirty() => {
                SettingsAction::Apply(Box::new(self.draft.clone().normalized()))
            }
            SettingsControl::ChooseMascotImage => SettingsAction::ChooseMascotImage,
            SettingsControl::CheckForUpdates
                if self.update_activity == SettingsUpdateActivity::Idle =>
            {
                SettingsAction::CheckForUpdates
            }
            SettingsControl::ResetMascotImage => {
                self.draft.mascot_image_path = None;
                SettingsAction::Changed
            }
            SettingsControl::Revert if self.is_dirty() => {
                self.draft = self.baseline.clone();
                SettingsAction::Changed
            }
            SettingsControl::SurfacePreset => SettingsAction::ChooseBackgroundColor,
            SettingsControl::AccentPreset => SettingsAction::ChooseAccentColor,
            SettingsControl::NotificationBadgeStyle
            | SettingsControl::Slider(_)
            | SettingsControl::CheckForUpdates
            | SettingsControl::Revert
            | SettingsControl::Apply => SettingsAction::None,
            SettingsControl::Close => SettingsAction::Close,
        }
    }

    fn move_focus(&mut self, reverse: bool) {
        let layout = self.layout();
        let focusable: Vec<_> = layout
            .controls
            .into_iter()
            .filter(|entry| {
                !matches!(entry.control, SettingsControl::Apply | SettingsControl::Revert)
                    || self.is_dirty()
            })
            .map(|entry| entry.control)
            .collect();
        if focusable.is_empty() {
            self.focused = None;
            return;
        }
        let current =
            self.focused.and_then(|value| focusable.iter().position(|item| *item == value));
        let next = match (current, reverse) {
            (Some(0) | None, true) => focusable.len() - 1,
            (Some(index), true) => index - 1,
            (Some(index), false) => (index + 1) % focusable.len(),
            (None, false) => 0,
        };
        self.focused = Some(focusable[next]);
    }

    fn set_picker_from_pointer(
        &mut self,
        control: SettingsControl,
        bounds: SettingsRect,
        x: u32,
    ) -> SettingsAction {
        let picker = self.control_column(bounds);
        let left = picker.left;
        let width = picker.width.max(1);
        if x < left || x >= left.saturating_add(width) {
            return SettingsAction::None;
        }
        let offset = x.saturating_sub(left).min(width.saturating_sub(1));
        match control {
            SettingsControl::SurfacePreset => {
                let index = usize::try_from(offset.saturating_mul(4) / width).unwrap_or_default();
                let Some(preset) = SurfacePreset::ALL.get(index) else {
                    return SettingsAction::ChooseBackgroundColor;
                };
                preset.color().clone_into(&mut self.draft.background_color);
            }
            SettingsControl::AccentPreset => {
                let index = usize::try_from(offset.saturating_mul(6) / width).unwrap_or_default();
                let Some(preset) = AccentPreset::ALL.get(index) else {
                    return SettingsAction::ChooseAccentColor;
                };
                preset.color().clone_into(&mut self.draft.accent_color);
            }
            SettingsControl::NotificationBadgeStyle => {
                let styles = [
                    NotificationBadgeStyle::Off,
                    NotificationBadgeStyle::Dot,
                    NotificationBadgeStyle::Count,
                ];
                let index = usize::try_from(offset.saturating_mul(3) / width).unwrap_or_default();
                let Some(style) = styles.get(index) else { return SettingsAction::None };
                self.draft.notification_badge_style = *style;
            }
            _ => return SettingsAction::None,
        }
        SettingsAction::Changed
    }

    fn cycle_surface_preset(&mut self, reverse: bool) -> SettingsAction {
        let current = SurfacePreset::selected(&self.draft)
            .and_then(|selected| SurfacePreset::ALL.iter().position(|item| *item == selected))
            .unwrap_or_default();
        let next = cycle_index(current, SurfacePreset::ALL.len(), reverse);
        SurfacePreset::ALL[next].color().clone_into(&mut self.draft.background_color);
        SettingsAction::Changed
    }

    fn cycle_accent_preset(&mut self, reverse: bool) -> SettingsAction {
        let current = AccentPreset::selected(&self.draft)
            .and_then(|selected| AccentPreset::ALL.iter().position(|item| *item == selected))
            .unwrap_or_default();
        let next = cycle_index(current, AccentPreset::ALL.len(), reverse);
        AccentPreset::ALL[next].color().clone_into(&mut self.draft.accent_color);
        SettingsAction::Changed
    }

    fn cycle_notification_badge_style(&mut self, reverse: bool) -> SettingsAction {
        let styles = [
            NotificationBadgeStyle::Off,
            NotificationBadgeStyle::Dot,
            NotificationBadgeStyle::Count,
        ];
        let current = styles
            .iter()
            .position(|style| *style == self.draft.notification_badge_style)
            .unwrap_or_default();
        self.draft.notification_badge_style = styles[cycle_index(current, styles.len(), reverse)];
        SettingsAction::Changed
    }

    fn adjust_slider(&mut self, slider: SettingsSlider, delta: i32) -> SettingsAction {
        let (minimum, maximum) = slider.range();
        let value = self.slider_value(slider);
        let value = if delta < 0 { value.saturating_sub(1) } else { value.saturating_add(1) }
            .clamp(minimum, maximum);
        self.set_slider(slider, value);
        SettingsAction::Changed
    }

    pub fn toggle(&self, toggle: SettingsToggle) -> bool {
        match toggle {
            SettingsToggle::ShowUnpinnedRunningApps => self.draft.show_unpinned_running_apps,
            SettingsToggle::ShowDesktopButton => self.draft.show_desktop_button,
            SettingsToggle::StartWithWindows => self.draft.start_with_windows,
            SettingsToggle::ReplaceWindowsTaskbar => self.draft.replace_windows_taskbar,
            SettingsToggle::ExclusiveTaskbarReplacement => self.draft.exclusive_taskbar_replacement,
            SettingsToggle::HideWhenFullscreen => self.draft.hide_when_fullscreen,
            SettingsToggle::SearchOpenWithWindowsKey => self.draft.search_open_with_windows_key,
            SettingsToggle::AltTabEnabled => self.draft.alt_tab_enabled,
        }
    }

    fn set_toggle(&mut self, toggle: SettingsToggle, value: bool) {
        match toggle {
            SettingsToggle::ShowUnpinnedRunningApps => {
                self.draft.show_unpinned_running_apps = value;
            }
            SettingsToggle::ShowDesktopButton => self.draft.show_desktop_button = value,
            SettingsToggle::StartWithWindows => self.draft.start_with_windows = value,
            SettingsToggle::ReplaceWindowsTaskbar => self.draft.replace_windows_taskbar = value,
            SettingsToggle::ExclusiveTaskbarReplacement => {
                self.draft.exclusive_taskbar_replacement = value;
            }
            SettingsToggle::HideWhenFullscreen => self.draft.hide_when_fullscreen = value,
            SettingsToggle::SearchOpenWithWindowsKey => {
                self.draft.search_open_with_windows_key = value;
            }
            SettingsToggle::AltTabEnabled => self.draft.alt_tab_enabled = value,
        }
    }

    pub fn slider_value(&self, slider: SettingsSlider) -> u32 {
        match slider {
            SettingsSlider::IconSize => self.draft.icon_size,
            SettingsSlider::ItemSpacing => self.draft.item_spacing,
            SettingsSlider::HorizontalPadding => self.draft.horizontal_padding,
            SettingsSlider::VerticalPadding => self.draft.vertical_padding,
            SettingsSlider::BottomOffset => self.draft.bottom_offset,
            SettingsSlider::CornerRadius => self.draft.corner_radius,
            SettingsSlider::BackgroundOpacity => {
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "normalized opacity is finite and clamped to 0.08..=0.95"
                )]
                let percentage = (self.draft.background_opacity * 100.0).round() as u32;
                percentage
            }
            SettingsSlider::SearchResultLimit => self.draft.search_result_limit,
        }
    }

    pub fn set_mascot_image_path(&mut self, path: Option<String>) {
        self.draft.mascot_image_path = path;
    }

    pub fn set_background_color(&mut self, color: String) {
        self.draft.background_color = color;
    }

    pub fn set_accent_color(&mut self, color: String) {
        self.draft.accent_color = color;
    }

    fn set_slider(&mut self, slider: SettingsSlider, value: u32) {
        let (minimum, maximum) = slider.range();
        let value = value.clamp(minimum, maximum);
        match slider {
            SettingsSlider::IconSize => self.draft.icon_size = value,
            SettingsSlider::ItemSpacing => self.draft.item_spacing = value,
            SettingsSlider::HorizontalPadding => self.draft.horizontal_padding = value,
            SettingsSlider::VerticalPadding => self.draft.vertical_padding = value,
            SettingsSlider::BottomOffset => self.draft.bottom_offset = value,
            SettingsSlider::CornerRadius => self.draft.corner_radius = value,
            SettingsSlider::BackgroundOpacity => {
                self.draft.background_opacity = f64::from(value) / 100.0;
            }
            SettingsSlider::SearchResultLimit => self.draft.search_result_limit = value,
        }
    }

    fn rect(&self, left: u32, top: u32, width: u32, height: u32) -> SettingsRect {
        SettingsRect {
            left: self.scale(left),
            top: self.scale(top),
            width: self.scale(width),
            height: self.scale(height),
        }
    }

    pub fn control_column(&self, bounds: SettingsRect) -> SettingsRect {
        SettingsRect {
            left: bounds.left.saturating_add(self.scale(CONTROL_COLUMN_LEFT_DIP)),
            top: bounds.top.saturating_add(self.scale(6)),
            width: bounds
                .width
                .saturating_sub(self.scale(CONTROL_COLUMN_LEFT_DIP + CONTROL_COLUMN_RIGHT_DIP)),
            height: bounds.height.saturating_sub(self.scale(12)),
        }
    }

    pub fn slider_track(&self, bounds: SettingsRect) -> (u32, u32) {
        let column = self.control_column(bounds);
        let reserved = self.scale(CONTROL_VALUE_GAP_DIP + CONTROL_VALUE_WIDTH_DIP);
        (column.left, column.width.saturating_sub(reserved).max(1))
    }

    pub fn slider_value_bounds(&self, bounds: SettingsRect) -> SettingsRect {
        let column = self.control_column(bounds);
        let width = self.scale(CONTROL_VALUE_WIDTH_DIP);
        SettingsRect {
            left: column.left.saturating_add(column.width.saturating_sub(width)),
            top: column.top,
            width,
            height: column.height,
        }
    }

    fn scale(&self, dips: u32) -> u32 {
        let scaled = u64::from(dips) * u64::from(self.dpi.get());
        u32::try_from((scaled + DIPS_PER_INCH / 2) / DIPS_PER_INCH).unwrap_or(u32::MAX)
    }
}

fn nonzero(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap_or(NonZeroU32::MIN)
}
fn u32_index(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn cycle_index(current: usize, length: usize, reverse: bool) -> usize {
    if reverse { current.checked_sub(1).unwrap_or(length - 1) } else { (current + 1) % length }
}
