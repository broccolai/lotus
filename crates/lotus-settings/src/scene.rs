use std::num::NonZeroU32;

use lotus_core::settings::{
    CURRENT_ONBOARDING_VERSION, DockSettings, DockZone, NotificationBadgeStyle,
    WindowPickerStyle,
};
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
const CONTENT_BOTTOM_INSET_DIP: u32 = 12;
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
    Status,
    Search,
    About,
}

impl SettingsPage {
    pub const ALL: [Self; 6] = [
        Self::General,
        Self::Appearance,
        Self::Taskbar,
        Self::Status,
        Self::Search,
        Self::About,
    ];

    pub const fn title(self) -> &'static str {
        match self {
            Self::Appearance => "appearance",
            Self::General => "general",
            Self::Taskbar => "taskbar",
            Self::Status => "status",
            Self::Search => "search",
            Self::About => "about",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsToggle {
    ShowAppDock,
    ShowUnpinnedRunningApps,
    ShowRunningIndicators,
    ShowOnAllMonitors,
    ShowDesktopButton,
    ShowSystemStatus,
    ShowVolumeStatus,
    ShowNetworkStatus,
    ShowBackgroundAppsStatus,
    ShowDateTimeStatus,
    ShowDateInStatus,
    ShowMediaControls,
    ShowMediaMetadata,
    StartWithWindows,
    ReplaceWindowsTaskbar,
    HideWhenFullscreen,
    SearchEnabled,
    SearchOpenWithWindowsKey,
    AltTabEnabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OnboardingStep {
    Welcome,
    Modules,
    Layout,
    Integration,
    Ready,
}

impl OnboardingStep {
    pub const fn number(self) -> u32 {
        match self {
            Self::Welcome => 0,
            Self::Modules => 1,
            Self::Layout => 2,
            Self::Integration => 3,
            Self::Ready => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OnboardingModule {
    AppDock,
    Search,
    SystemStatus,
    Media,
    AltTab,
}

impl OnboardingModule {
    pub const fn title(self) -> &'static str {
        match self {
            Self::AppDock => "application dock",
            Self::Search => "search",
            Self::SystemStatus => "system status",
            Self::Media => "media controls",
            Self::AltTab => "alt-tab",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::AppDock => "Applications, windows and the lotus menu",
            Self::Search => "Applications, commands and calculations",
            Self::SystemStatus => "Volume, network, background apps and time",
            Self::Media => "Artwork, track information and playback",
            Self::AltTab => "Replace the Windows window switcher",
        }
    }
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
    DockZone,
    SystemStatusZone,
    MediaZone,
    WindowPickerStyle,
    Toggle(SettingsToggle),
    Slider(SettingsSlider),
    ChooseMascotImage,
    ResetMascotImage,
    CheckForUpdates,
    ReplaySetup,
    OnboardingModule(OnboardingModule),
    OnboardingZone(OnboardingModule),
    OnboardingBack,
    OnboardingNext,
    OnboardingFinish,
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
    ReplaySetup,
    CompleteOnboarding(Box<DockSettings>),
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
    pub content_viewport: SettingsRect,
    pub scrollbar_thumb: Option<SettingsRect>,
}

impl SettingsLayout {
    pub fn hit_test(&self, x: u32, y: u32) -> Option<SettingsControl> {
        self.controls
            .iter()
            .rev()
            .find(|entry| {
                entry.bounds.contains(x, y)
                    && (!is_page_content(entry.control)
                        || self.content_viewport.contains(x, y))
            })
            .map(|entry| entry.control)
    }

    pub fn bounds(&self, control: SettingsControl) -> Option<SettingsRect> {
        self.controls
            .iter()
            .find(|entry| entry.control == control)
            .map(|entry| entry.bounds)
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
    onboarding: Option<OnboardingStep>,
    onboarding_required: bool,
    scroll_offset_dip: u32,
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
            onboarding: None,
            onboarding_required: false,
            scroll_offset_dip: 0,
        })
    }

    pub const fn dpi(&self) -> u32 {
        self.dpi.get()
    }

    pub fn set_dpi(&mut self, dpi: u32) -> bool {
        let Some(dpi) = NonZeroU32::new(dpi) else {
            return false;
        };
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

    pub const fn onboarding_step(&self) -> Option<OnboardingStep> {
        self.onboarding
    }

    pub const fn onboarding_required(&self) -> bool {
        self.onboarding_required
    }

    pub fn begin_onboarding(&mut self, applied: DockSettings, required: bool) {
        let applied = applied.normalized();
        self.baseline = applied.clone();
        self.draft = applied;
        self.page = SettingsPage::General;
        self.onboarding = Some(OnboardingStep::Welcome);
        self.onboarding_required = required;
        self.scroll_offset_dip = 0;
        self.hovered = None;
        self.focused = Some(SettingsControl::OnboardingNext);
        self.focus_visible = false;
    }

    pub fn end_onboarding(&mut self) {
        self.onboarding = None;
        self.onboarding_required = false;
        self.scroll_offset_dip = 0;
        self.hovered = None;
        self.focused = Some(SettingsControl::Navigate(SettingsPage::General));
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
        if let Some(step) = self.onboarding {
            return self.onboarding_layout(step);
        }
        let size = self.desired_size();
        let content_viewport = self.rect(
            CONTENT_LEFT_DIP,
            CONTENT_TOP_DIP,
            WIDTH_DIP - CONTENT_LEFT_DIP - 14,
            Self::content_viewport_height_dip(),
        );
        let mut controls = Vec::new();
        for (index, page) in SettingsPage::ALL
            .into_iter()
            .filter(|page| *page != SettingsPage::About)
            .enumerate()
        {
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
            bounds: self.rect(
                NAV_LEFT_DIP,
                HEIGHT_DIP - 115,
                NAV_WIDTH_DIP,
                NAV_HEIGHT_DIP,
            ),
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
            let top = if control == SettingsControl::ReplaySetup {
                HEIGHT_DIP - FOOTER_HEIGHT_DIP - ROW_HEIGHT_DIP - 22
            } else {
                (CONTENT_TOP_DIP + u32_index(index) * (ROW_HEIGHT_DIP + ROW_GAP_DIP))
                    .saturating_sub(self.scroll_offset_dip)
            };
            let bounds = self.rect(CONTENT_LEFT_DIP, top, content_width, ROW_HEIGHT_DIP);
            controls.push(SettingsControlLayout { control, bounds });
        }
        controls.push(SettingsControlLayout {
            control: SettingsControl::Revert,
            bounds: self.rect(
                WIDTH_DIP
                    - CONTENT_RIGHT_DIP
                    - APPLY_WIDTH_DIP
                    - ACTION_GAP_DIP
                    - REVERT_WIDTH_DIP,
                HEIGHT_DIP - FOOTER_HEIGHT_DIP
                    + (FOOTER_HEIGHT_DIP - ACTION_HEIGHT_DIP) / 2,
                REVERT_WIDTH_DIP,
                ACTION_HEIGHT_DIP,
            ),
        });
        controls.push(SettingsControlLayout {
            control: SettingsControl::Apply,
            bounds: self.rect(
                WIDTH_DIP - CONTENT_RIGHT_DIP - APPLY_WIDTH_DIP,
                HEIGHT_DIP - FOOTER_HEIGHT_DIP
                    + (FOOTER_HEIGHT_DIP - ACTION_HEIGHT_DIP) / 2,
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
        SettingsLayout {
            size,
            controls,
            content_viewport,
            scrollbar_thumb: self.scrollbar_thumb(),
        }
    }

    pub fn scroll(&mut self, direction: i32) -> bool {
        if self.onboarding.is_some() || direction == 0 {
            return false;
        }

        let maximum = self.maximum_scroll_offset_dip();
        let next = if direction > 0 {
            self.scroll_offset_dip
                .saturating_add(ROW_HEIGHT_DIP + ROW_GAP_DIP)
                .min(maximum)
        } else {
            self.scroll_offset_dip
                .saturating_sub(ROW_HEIGHT_DIP + ROW_GAP_DIP)
        };
        if next == self.scroll_offset_dip {
            return false;
        }

        self.scroll_offset_dip = next;
        self.hovered = None;
        true
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
        let Some(control) = self.layout().hit_test(x, y) else {
            return SettingsAction::None;
        };
        self.focused = Some(control);
        self.focus_visible = false;
        if matches!(
            control,
            SettingsControl::SurfacePreset
                | SettingsControl::AccentPreset
                | SettingsControl::NotificationBadgeStyle
                | SettingsControl::DockZone
                | SettingsControl::SystemStatusZone
                | SettingsControl::MediaZone
                | SettingsControl::WindowPickerStyle
        ) {
            let bounds = self
                .layout()
                .bounds(control)
                .expect("active control has layout bounds");
            return self.set_picker_from_pointer(control, bounds, x);
        }
        if let SettingsControl::Slider(slider) = control {
            return if self.slider_at(x, y) == Some(slider) {
                self.set_slider_from_pointer(slider, x)
            } else {
                SettingsAction::None
            };
        }
        if let SettingsControl::OnboardingZone(module) = control {
            let bounds = self
                .layout()
                .bounds(control)
                .expect("active control has layout bounds");
            return self.set_onboarding_zone_from_pointer(module, bounds, x);
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
            | SettingsControl::DockZone
            | SettingsControl::SystemStatusZone
            | SettingsControl::MediaZone
            | SettingsControl::WindowPickerStyle
            | SettingsControl::Toggle(_)
            | SettingsControl::ChooseMascotImage
            | SettingsControl::ResetMascotImage
            | SettingsControl::CheckForUpdates
            | SettingsControl::ReplaySetup
            | SettingsControl::OnboardingModule(_)
            | SettingsControl::OnboardingZone(_)
            | SettingsControl::OnboardingBack
            | SettingsControl::OnboardingNext
            | SettingsControl::OnboardingFinish
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

    pub fn set_slider_from_pointer(
        &mut self,
        slider: SettingsSlider,
        x: u32,
    ) -> SettingsAction {
        let Some(bounds) = self.layout().bounds(SettingsControl::Slider(slider)) else {
            return SettingsAction::None;
        };
        let (track_left, track_width) = self.slider_track(bounds);
        let offset = x.saturating_sub(track_left).min(track_width);
        let (minimum, maximum) = slider.range();
        let value =
            minimum.saturating_add(offset.saturating_mul(maximum - minimum) / track_width);
        self.set_slider(slider, value);
        SettingsAction::Changed
    }

    pub fn key(&mut self, key: SettingsKey) -> SettingsAction {
        self.focus_visible = true;
        if key == SettingsKey::Escape {
            if self.onboarding_required {
                return SettingsAction::None;
            }
            return SettingsAction::Close;
        }
        if matches!(
            key,
            SettingsKey::Tab
                | SettingsKey::ReverseTab
                | SettingsKey::Up
                | SettingsKey::Down
        ) {
            self.move_focus(matches!(key, SettingsKey::ReverseTab | SettingsKey::Up));
            return SettingsAction::None;
        }
        let Some(focused) = self.focused else {
            return SettingsAction::None;
        };
        match (key, focused) {
            (SettingsKey::Activate, control) => self.activate(control),
            (SettingsKey::Left, SettingsControl::Slider(slider)) => {
                self.adjust_slider(slider, -1)
            }
            (SettingsKey::Right, SettingsControl::Slider(slider)) => {
                self.adjust_slider(slider, 1)
            }
            (SettingsKey::Left, SettingsControl::SurfacePreset) => {
                self.cycle_surface_preset(true)
            }
            (SettingsKey::Right, SettingsControl::SurfacePreset) => {
                self.cycle_surface_preset(false)
            }
            (SettingsKey::Left, SettingsControl::AccentPreset) => {
                self.cycle_accent_preset(true)
            }
            (SettingsKey::Right, SettingsControl::AccentPreset) => {
                self.cycle_accent_preset(false)
            }
            (SettingsKey::Left, SettingsControl::NotificationBadgeStyle) => {
                self.cycle_notification_badge_style(true)
            }
            (SettingsKey::Right, SettingsControl::NotificationBadgeStyle) => {
                self.cycle_notification_badge_style(false)
            }
            (SettingsKey::Left, SettingsControl::DockZone) => self.cycle_zone(false, true),
            (SettingsKey::Right, SettingsControl::DockZone) => {
                self.cycle_zone(false, false)
            }
            (SettingsKey::Left, SettingsControl::SystemStatusZone) => {
                self.cycle_zone(true, true)
            }
            (SettingsKey::Right, SettingsControl::SystemStatusZone) => {
                self.cycle_zone(true, false)
            }
            (SettingsKey::Left, SettingsControl::MediaZone) => self.cycle_media_zone(true),
            (SettingsKey::Right, SettingsControl::MediaZone) => {
                self.cycle_media_zone(false)
            }
            (
                SettingsKey::Left | SettingsKey::Right,
                SettingsControl::WindowPickerStyle,
            ) => self.cycle_window_picker_style(),
            (SettingsKey::Left, SettingsControl::OnboardingZone(module)) => {
                self.cycle_onboarding_zone(module, true)
            }
            (SettingsKey::Right, SettingsControl::OnboardingZone(module)) => {
                self.cycle_onboarding_zone(module, false)
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
                SettingsControl::Toggle(SettingsToggle::ShowAppDock),
                SettingsControl::Toggle(SettingsToggle::ReplaceWindowsTaskbar),
                SettingsControl::Toggle(SettingsToggle::HideWhenFullscreen),
                SettingsControl::Toggle(SettingsToggle::ShowOnAllMonitors),
                SettingsControl::Toggle(SettingsToggle::ShowDesktopButton),
                SettingsControl::Toggle(SettingsToggle::ShowRunningIndicators),
                SettingsControl::DockZone,
                SettingsControl::WindowPickerStyle,
                SettingsControl::Slider(SettingsSlider::IconSize),
                SettingsControl::Slider(SettingsSlider::ItemSpacing),
                SettingsControl::Slider(SettingsSlider::HorizontalPadding),
                SettingsControl::Slider(SettingsSlider::VerticalPadding),
                SettingsControl::Slider(SettingsSlider::BottomOffset),
            ],
            SettingsPage::Status => vec![
                SettingsControl::Toggle(SettingsToggle::ShowSystemStatus),
                SettingsControl::SystemStatusZone,
                SettingsControl::Toggle(SettingsToggle::ShowMediaControls),
                SettingsControl::Toggle(SettingsToggle::ShowMediaMetadata),
                SettingsControl::MediaZone,
                SettingsControl::Toggle(SettingsToggle::ShowVolumeStatus),
                SettingsControl::Toggle(SettingsToggle::ShowNetworkStatus),
                SettingsControl::Toggle(SettingsToggle::ShowBackgroundAppsStatus),
                SettingsControl::Toggle(SettingsToggle::ShowDateTimeStatus),
                SettingsControl::Toggle(SettingsToggle::ShowDateInStatus),
            ],
            SettingsPage::Search => vec![
                SettingsControl::Toggle(SettingsToggle::SearchEnabled),
                SettingsControl::Toggle(SettingsToggle::SearchOpenWithWindowsKey),
                SettingsControl::Slider(SettingsSlider::SearchResultLimit),
            ],
            SettingsPage::About => vec![SettingsControl::ReplaySetup],
        }
    }

    const fn content_viewport_height_dip() -> u32 {
        HEIGHT_DIP - FOOTER_HEIGHT_DIP - CONTENT_BOTTOM_INSET_DIP - CONTENT_TOP_DIP
    }

    fn content_height_dip(&self) -> u32 {
        let count = u32::try_from(self.page_controls().len()).unwrap_or(u32::MAX);
        if count == 0 || self.page == SettingsPage::About {
            return 0;
        }

        count
            .saturating_mul(ROW_HEIGHT_DIP)
            .saturating_add(count.saturating_sub(1).saturating_mul(ROW_GAP_DIP))
    }

    fn maximum_scroll_offset_dip(&self) -> u32 {
        self.content_height_dip()
            .saturating_sub(Self::content_viewport_height_dip())
    }

    fn scrollbar_thumb(&self) -> Option<SettingsRect> {
        let content_height = self.content_height_dip();
        let viewport_height = Self::content_viewport_height_dip();
        let maximum = self.maximum_scroll_offset_dip();
        if maximum == 0 {
            return None;
        }

        let thumb_height = viewport_height
            .saturating_mul(viewport_height)
            .checked_div(content_height)
            .unwrap_or(viewport_height)
            .max(32);
        let travel = viewport_height.saturating_sub(thumb_height);
        let thumb_top = CONTENT_TOP_DIP.saturating_add(
            travel
                .saturating_mul(self.scroll_offset_dip)
                .checked_div(maximum)
                .unwrap_or_default(),
        );
        Some(self.rect(WIDTH_DIP - 18, thumb_top, 3, thumb_height))
    }

    fn onboarding_layout(&self, step: OnboardingStep) -> SettingsLayout {
        let size = self.desired_size();
        let mut controls = Vec::new();
        match step {
            OnboardingStep::Welcome | OnboardingStep::Ready => {}
            OnboardingStep::Modules => {
                for (index, module) in [
                    OnboardingModule::AppDock,
                    OnboardingModule::Search,
                    OnboardingModule::SystemStatus,
                    OnboardingModule::Media,
                    OnboardingModule::AltTab,
                ]
                .into_iter()
                .enumerate()
                {
                    let index = u32_index(index);
                    let left = if index == 4 {
                        282
                    } else {
                        104 + index % 2 * 352
                    };
                    controls.push(SettingsControlLayout {
                        control: SettingsControl::OnboardingModule(module),
                        bounds: self.rect(left, 200 + index / 2 * 88, 336, 72),
                    });
                }
            }
            OnboardingStep::Layout => {
                for (index, module) in [
                    OnboardingModule::AppDock,
                    OnboardingModule::Media,
                    OnboardingModule::SystemStatus,
                ]
                .into_iter()
                .enumerate()
                {
                    controls.push(SettingsControlLayout {
                        control: SettingsControl::OnboardingZone(module),
                        bounds: self.rect(420, 240 + u32_index(index) * 78, 320, 46),
                    });
                }
            }
            OnboardingStep::Integration => {
                for (index, toggle) in [
                    SettingsToggle::StartWithWindows,
                    SettingsToggle::HideWhenFullscreen,
                    SettingsToggle::SearchOpenWithWindowsKey,
                    SettingsToggle::ReplaceWindowsTaskbar,
                    SettingsToggle::ShowOnAllMonitors,
                ]
                .into_iter()
                .enumerate()
                {
                    controls.push(SettingsControlLayout {
                        control: SettingsControl::Toggle(toggle),
                        bounds: self.rect(160, 184 + u32_index(index) * 62, 580, 52),
                    });
                }
            }
        }
        if !matches!(step, OnboardingStep::Welcome | OnboardingStep::Ready) {
            controls.push(SettingsControlLayout {
                control: SettingsControl::OnboardingBack,
                bounds: self.rect(316, HEIGHT_DIP - 104, 116, 46),
            });
        }
        controls.push(SettingsControlLayout {
            control: if step == OnboardingStep::Ready {
                SettingsControl::OnboardingFinish
            } else {
                SettingsControl::OnboardingNext
            },
            bounds: match step {
                OnboardingStep::Welcome => self.rect(376, 500, 148, 48),
                OnboardingStep::Ready => self.rect(366, 346, 168, 48),
                _ => self.rect(448, HEIGHT_DIP - 104, 136, 46),
            },
        });
        if !self.onboarding_required {
            controls.push(SettingsControlLayout {
                control: SettingsControl::Close,
                bounds: self.rect(
                    WIDTH_DIP - CONTENT_RIGHT_DIP - CLOSE_SIZE_DIP,
                    12,
                    CLOSE_SIZE_DIP,
                    CLOSE_SIZE_DIP,
                ),
            });
        }
        SettingsLayout {
            size,
            controls,
            content_viewport: SettingsRect {
                left: 0,
                top: 0,
                width: size.width(),
                height: size.height(),
            },
            scrollbar_thumb: None,
        }
    }

    fn activate(&mut self, control: SettingsControl) -> SettingsAction {
        match control {
            SettingsControl::Navigate(page) => {
                self.page = page;
                self.scroll_offset_dip = 0;
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
            SettingsControl::ReplaySetup => SettingsAction::ReplaySetup,
            SettingsControl::OnboardingModule(module) => {
                let enabled = !self.onboarding_module_enabled(module);
                self.set_onboarding_module(module, enabled);
                SettingsAction::Changed
            }
            SettingsControl::OnboardingZone(module) => {
                self.cycle_onboarding_zone(module, false)
            }
            SettingsControl::OnboardingBack => {
                self.move_onboarding(false);
                SettingsAction::Changed
            }
            SettingsControl::OnboardingNext => {
                self.move_onboarding(true);
                SettingsAction::Changed
            }
            SettingsControl::OnboardingFinish => {
                self.draft.onboarding_version = CURRENT_ONBOARDING_VERSION;
                SettingsAction::CompleteOnboarding(Box::new(
                    self.draft.clone().normalized(),
                ))
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
            | SettingsControl::DockZone
            | SettingsControl::SystemStatusZone
            | SettingsControl::MediaZone
            | SettingsControl::WindowPickerStyle
            | SettingsControl::Slider(_)
            | SettingsControl::CheckForUpdates
            | SettingsControl::Revert
            | SettingsControl::Apply => SettingsAction::None,
            SettingsControl::Close => SettingsAction::Close,
        }
    }

    pub fn onboarding_module_enabled(&self, module: OnboardingModule) -> bool {
        match module {
            OnboardingModule::AppDock => self.draft.show_app_dock,
            OnboardingModule::Search => self.draft.search_enabled,
            OnboardingModule::SystemStatus => self.draft.show_system_status,
            OnboardingModule::Media => self.draft.show_media_controls,
            OnboardingModule::AltTab => self.draft.alt_tab_enabled,
        }
    }

    pub const fn onboarding_zone(&self, module: OnboardingModule) -> DockZone {
        match module {
            OnboardingModule::AppDock => self.draft.dock_zone,
            OnboardingModule::SystemStatus => self.draft.system_status_zone,
            OnboardingModule::Media => self.draft.media_zone,
            OnboardingModule::Search | OnboardingModule::AltTab => DockZone::Center,
        }
    }

    fn set_onboarding_module(&mut self, module: OnboardingModule, enabled: bool) {
        match module {
            OnboardingModule::AppDock => self.draft.show_app_dock = enabled,
            OnboardingModule::Search => self.draft.search_enabled = enabled,
            OnboardingModule::SystemStatus => self.draft.show_system_status = enabled,
            OnboardingModule::Media => self.draft.show_media_controls = enabled,
            OnboardingModule::AltTab => self.draft.alt_tab_enabled = enabled,
        }
    }

    fn cycle_onboarding_zone(
        &mut self,
        module: OnboardingModule,
        reverse: bool,
    ) -> SettingsAction {
        let current = self.onboarding_zone(module);
        let index = DockZone::ALL
            .iter()
            .position(|zone| *zone == current)
            .unwrap_or_default();
        let next = if reverse {
            DockZone::ALL[(index + DockZone::ALL.len() - 1) % DockZone::ALL.len()]
        } else {
            DockZone::ALL[(index + 1) % DockZone::ALL.len()]
        };
        self.set_onboarding_zone(module, next);
        SettingsAction::Changed
    }

    fn set_onboarding_zone_from_pointer(
        &mut self,
        module: OnboardingModule,
        bounds: SettingsRect,
        x: u32,
    ) -> SettingsAction {
        let relative = x
            .saturating_sub(bounds.left)
            .min(bounds.width.saturating_sub(1));
        let index = relative.saturating_mul(3) / bounds.width.max(1);
        let zone = DockZone::ALL[usize::try_from(index).unwrap_or_default().min(2)];
        self.set_onboarding_zone(module, zone);
        SettingsAction::Changed
    }

    fn set_onboarding_zone(&mut self, module: OnboardingModule, zone: DockZone) {
        match module {
            OnboardingModule::AppDock => self.draft.dock_zone = zone,
            OnboardingModule::SystemStatus => self.draft.system_status_zone = zone,
            OnboardingModule::Media => self.draft.media_zone = zone,
            OnboardingModule::Search | OnboardingModule::AltTab => {}
        }
    }

    fn move_onboarding(&mut self, forward: bool) {
        let Some(step) = self.onboarding else {
            return;
        };
        let number = if forward {
            step.number().saturating_add(1).min(4)
        } else {
            step.number().saturating_sub(1)
        };
        self.onboarding = Some(match number {
            0 => OnboardingStep::Welcome,
            1 => OnboardingStep::Modules,
            2 => OnboardingStep::Layout,
            3 => OnboardingStep::Integration,
            _ => OnboardingStep::Ready,
        });
        self.focused = Some(if self.onboarding == Some(OnboardingStep::Ready) {
            SettingsControl::OnboardingFinish
        } else {
            SettingsControl::OnboardingNext
        });
    }

    fn move_focus(&mut self, reverse: bool) {
        let layout = self.layout();
        let focusable: Vec<_> = layout
            .controls
            .into_iter()
            .filter(|entry| {
                !matches!(
                    entry.control,
                    SettingsControl::Apply | SettingsControl::Revert
                ) || self.is_dirty()
            })
            .map(|entry| entry.control)
            .collect();
        if focusable.is_empty() {
            self.focused = None;
            return;
        }
        let current = self
            .focused
            .and_then(|value| focusable.iter().position(|item| *item == value));
        let next = match (current, reverse) {
            (Some(0) | None, true) => focusable.len() - 1,
            (Some(index), true) => index - 1,
            (Some(index), false) => (index + 1) % focusable.len(),
            (None, false) => 0,
        };
        self.focused = Some(focusable[next]);
        self.reveal_focused_control();
    }

    fn reveal_focused_control(&mut self) {
        let Some(control) = self.focused.filter(|control| is_page_content(*control)) else {
            return;
        };
        let Some(index) = self
            .page_controls()
            .iter()
            .position(|item| *item == control)
        else {
            return;
        };

        let top = u32_index(index).saturating_mul(ROW_HEIGHT_DIP + ROW_GAP_DIP);
        let bottom = top.saturating_add(ROW_HEIGHT_DIP);
        let viewport_height = Self::content_viewport_height_dip();
        if top < self.scroll_offset_dip {
            self.scroll_offset_dip = top;
        } else if bottom > self.scroll_offset_dip.saturating_add(viewport_height) {
            self.scroll_offset_dip = bottom.saturating_sub(viewport_height);
        }
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
                let index =
                    usize::try_from(offset.saturating_mul(4) / width).unwrap_or_default();
                let Some(preset) = SurfacePreset::ALL.get(index) else {
                    return SettingsAction::ChooseBackgroundColor;
                };
                preset.color().clone_into(&mut self.draft.background_color);
            }
            SettingsControl::AccentPreset => {
                let index =
                    usize::try_from(offset.saturating_mul(6) / width).unwrap_or_default();
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
                let index =
                    usize::try_from(offset.saturating_mul(3) / width).unwrap_or_default();
                let Some(style) = styles.get(index) else {
                    return SettingsAction::None;
                };
                self.draft.notification_badge_style = *style;
            }
            SettingsControl::DockZone
            | SettingsControl::SystemStatusZone
            | SettingsControl::MediaZone => {
                let index =
                    usize::try_from(offset.saturating_mul(3) / width).unwrap_or_default();
                let Some(zone) = DockZone::ALL.get(index) else {
                    return SettingsAction::None;
                };
                match control {
                    SettingsControl::DockZone => self.draft.dock_zone = *zone,
                    SettingsControl::SystemStatusZone => {
                        self.draft.system_status_zone = *zone;
                    }
                    SettingsControl::MediaZone => self.draft.media_zone = *zone,
                    _ => {}
                }
            }
            SettingsControl::WindowPickerStyle => {
                self.draft.window_picker_style = if offset.saturating_mul(2) / width == 0 {
                    WindowPickerStyle::Thumbnails
                } else {
                    WindowPickerStyle::Compact
                };
            }
            _ => return SettingsAction::None,
        }
        SettingsAction::Changed
    }

    fn cycle_surface_preset(&mut self, reverse: bool) -> SettingsAction {
        let current = SurfacePreset::selected(&self.draft)
            .and_then(|selected| {
                SurfacePreset::ALL.iter().position(|item| *item == selected)
            })
            .unwrap_or_default();
        let next = cycle_index(current, SurfacePreset::ALL.len(), reverse);
        SurfacePreset::ALL[next]
            .color()
            .clone_into(&mut self.draft.background_color);
        SettingsAction::Changed
    }

    fn cycle_accent_preset(&mut self, reverse: bool) -> SettingsAction {
        let current = AccentPreset::selected(&self.draft)
            .and_then(|selected| {
                AccentPreset::ALL.iter().position(|item| *item == selected)
            })
            .unwrap_or_default();
        let next = cycle_index(current, AccentPreset::ALL.len(), reverse);
        AccentPreset::ALL[next]
            .color()
            .clone_into(&mut self.draft.accent_color);
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
        self.draft.notification_badge_style =
            styles[cycle_index(current, styles.len(), reverse)];
        SettingsAction::Changed
    }

    fn cycle_zone(&mut self, status: bool, reverse: bool) -> SettingsAction {
        let selected = if status {
            self.draft.system_status_zone
        } else {
            self.draft.dock_zone
        };
        let current = DockZone::ALL
            .iter()
            .position(|zone| *zone == selected)
            .unwrap_or_default();
        let next = DockZone::ALL[cycle_index(current, DockZone::ALL.len(), reverse)];
        if status {
            self.draft.system_status_zone = next;
        } else {
            self.draft.dock_zone = next;
        }
        SettingsAction::Changed
    }

    fn cycle_media_zone(&mut self, reverse: bool) -> SettingsAction {
        let current = DockZone::ALL
            .iter()
            .position(|zone| *zone == self.draft.media_zone)
            .unwrap_or_default();
        self.draft.media_zone =
            DockZone::ALL[cycle_index(current, DockZone::ALL.len(), reverse)];
        SettingsAction::Changed
    }

    fn cycle_window_picker_style(&mut self) -> SettingsAction {
        self.draft.window_picker_style = match self.draft.window_picker_style {
            WindowPickerStyle::Compact => WindowPickerStyle::Thumbnails,
            WindowPickerStyle::Thumbnails => WindowPickerStyle::Compact,
        };
        SettingsAction::Changed
    }

    fn adjust_slider(&mut self, slider: SettingsSlider, delta: i32) -> SettingsAction {
        let (minimum, maximum) = slider.range();
        let value = self.slider_value(slider);
        let value = if delta < 0 {
            value.saturating_sub(1)
        } else {
            value.saturating_add(1)
        }
        .clamp(minimum, maximum);
        self.set_slider(slider, value);
        SettingsAction::Changed
    }

    pub fn toggle(&self, toggle: SettingsToggle) -> bool {
        match toggle {
            SettingsToggle::ShowAppDock => self.draft.show_app_dock,
            SettingsToggle::ShowUnpinnedRunningApps => {
                self.draft.show_unpinned_running_apps
            }
            SettingsToggle::ShowRunningIndicators => self.draft.show_running_indicators,
            SettingsToggle::ShowOnAllMonitors => self.draft.show_on_all_monitors,
            SettingsToggle::ShowDesktopButton => self.draft.show_desktop_button,
            SettingsToggle::ShowSystemStatus => self.draft.show_system_status,
            SettingsToggle::ShowVolumeStatus => self.draft.show_volume_status,
            SettingsToggle::ShowNetworkStatus => self.draft.show_network_status,
            SettingsToggle::ShowBackgroundAppsStatus => {
                self.draft.show_background_apps_status
            }
            SettingsToggle::ShowDateTimeStatus => self.draft.show_date_time_status,
            SettingsToggle::ShowDateInStatus => self.draft.show_date_in_status,
            SettingsToggle::ShowMediaControls => self.draft.show_media_controls,
            SettingsToggle::ShowMediaMetadata => self.draft.show_media_metadata,
            SettingsToggle::StartWithWindows => self.draft.start_with_windows,
            SettingsToggle::ReplaceWindowsTaskbar => self.draft.replace_windows_taskbar,
            SettingsToggle::HideWhenFullscreen => self.draft.hide_when_fullscreen,
            SettingsToggle::SearchEnabled => self.draft.search_enabled,
            SettingsToggle::SearchOpenWithWindowsKey => {
                self.draft.search_open_with_windows_key
            }
            SettingsToggle::AltTabEnabled => self.draft.alt_tab_enabled,
        }
    }

    fn set_toggle(&mut self, toggle: SettingsToggle, value: bool) {
        match toggle {
            SettingsToggle::ShowAppDock => self.draft.show_app_dock = value,
            SettingsToggle::ShowUnpinnedRunningApps => {
                self.draft.show_unpinned_running_apps = value;
            }
            SettingsToggle::ShowRunningIndicators => {
                self.draft.show_running_indicators = value;
            }
            SettingsToggle::ShowOnAllMonitors => {
                self.draft.show_on_all_monitors = value;
            }
            SettingsToggle::ShowDesktopButton => self.draft.show_desktop_button = value,
            SettingsToggle::ShowSystemStatus => self.draft.show_system_status = value,
            SettingsToggle::ShowVolumeStatus => self.draft.show_volume_status = value,
            SettingsToggle::ShowNetworkStatus => self.draft.show_network_status = value,
            SettingsToggle::ShowBackgroundAppsStatus => {
                self.draft.show_background_apps_status = value;
            }
            SettingsToggle::ShowDateTimeStatus => {
                self.draft.show_date_time_status = value;
            }
            SettingsToggle::ShowDateInStatus => self.draft.show_date_in_status = value,
            SettingsToggle::ShowMediaControls => self.draft.show_media_controls = value,
            SettingsToggle::ShowMediaMetadata => self.draft.show_media_metadata = value,
            SettingsToggle::StartWithWindows => self.draft.start_with_windows = value,
            SettingsToggle::ReplaceWindowsTaskbar => {
                self.draft.replace_windows_taskbar = value;
                self.draft.exclusive_taskbar_replacement = value;
            }
            SettingsToggle::HideWhenFullscreen => self.draft.hide_when_fullscreen = value,
            SettingsToggle::SearchEnabled => self.draft.search_enabled = value,
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
            left: bounds
                .left
                .saturating_add(self.scale(CONTROL_COLUMN_LEFT_DIP)),
            top: bounds.top.saturating_add(self.scale(6)),
            width: bounds.width.saturating_sub(
                self.scale(CONTROL_COLUMN_LEFT_DIP + CONTROL_COLUMN_RIGHT_DIP),
            ),
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
            left: column
                .left
                .saturating_add(column.width.saturating_sub(width)),
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

fn is_page_content(control: SettingsControl) -> bool {
    matches!(
        control,
        SettingsControl::SurfacePreset
            | SettingsControl::AccentPreset
            | SettingsControl::NotificationBadgeStyle
            | SettingsControl::DockZone
            | SettingsControl::SystemStatusZone
            | SettingsControl::MediaZone
            | SettingsControl::WindowPickerStyle
            | SettingsControl::Toggle(_)
            | SettingsControl::Slider(_)
            | SettingsControl::ChooseMascotImage
            | SettingsControl::ResetMascotImage
            | SettingsControl::ReplaySetup
    )
}

fn u32_index(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn cycle_index(current: usize, length: usize, reverse: bool) -> usize {
    if reverse {
        current.checked_sub(1).unwrap_or(length - 1)
    } else {
        (current + 1) % length
    }
}
