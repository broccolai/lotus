use std::num::NonZeroU32;

use lotus_core::settings::{
    ApplicationIconOverride, CURRENT_ONBOARDING_VERSION, DockSettings, DockZone,
    NotificationBadgeStyle, UpdateChannel,
};
use lotus_ui::icon::RasterIcon;
use lotus_ui::theme::Theme;

use crate::appearance::{AccentPreset, ForegroundPreset, SurfacePreset, theme_for};

mod draft;
mod layout;
mod onboarding;
mod presentation;

use draft::SettingsDraft;
use onboarding::OnboardingState;
pub use presentation::SettingsAssets;

const DIPS_PER_INCH: u64 = 96;
const WIDTH_DIP: u32 = 900;
const HEIGHT_DIP: u32 = 730;
const NAV_LEFT_DIP: u32 = 14;
const NAV_WIDTH_DIP: u32 = 180;
const NAV_HEIGHT_DIP: u32 = 44;
const CONTENT_LEFT_DIP: u32 = 244;
const CONTENT_RIGHT_DIP: u32 = 32;
const CONTENT_TOP_DIP: u32 = 18;
const CONTENT_BOTTOM_INSET_DIP: u32 = 12;
const CONTROL_COLUMN_LEFT_DIP: u32 = 250;
const CONTROL_COLUMN_RIGHT_DIP: u32 = 16;
const CONTROL_VALUE_GAP_DIP: u32 = 14;
const CONTROL_VALUE_WIDTH_DIP: u32 = 44;
const ROW_HEIGHT_DIP: u32 = 46;
const ROW_GAP_DIP: u32 = 4;
const SECTION_LABEL_HEIGHT_DIP: u32 = 26;
const SECTION_GAP_DIP: u32 = 12;
const ACTION_HEIGHT_DIP: u32 = NAV_HEIGHT_DIP;
const ACTION_GAP_DIP: u32 = 8;
const APPLY_WIDTH_DIP: u32 = 92;
const REVERT_WIDTH_DIP: u32 = 92;
const FOOTER_HEIGHT_DIP: u32 = 72;
const CLOSE_SIZE_DIP: u32 = 40;
const UPDATE_PROMPT_WIDTH_DIP: u32 = 372;
const UPDATE_PROMPT_HEIGHT_DIP: u32 = 236;
const UPDATE_PROMPT_BUTTON_HEIGHT_DIP: u32 = 44;
const UPDATE_PROMPT_BUTTON_WIDTH_DIP: u32 = 152;
const UPDATE_PROMPT_BUTTON_GAP_DIP: u32 = 12;
const UPDATE_PROMPT_INSET_DIP: u32 = 28;

const fn update_prompt_left_dip() -> u32 {
    let content_width = WIDTH_DIP - CONTENT_LEFT_DIP - CONTENT_RIGHT_DIP;
    CONTENT_LEFT_DIP + (content_width - UPDATE_PROMPT_WIDTH_DIP) / 2
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsPage {
    Appearance,
    Apps,
    General,
    Taskbar,
    Status,
    Search,
    About,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsSection {
    Main,
    Appearance,
    Advanced,
}

impl SettingsSection {
    pub const fn title(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Appearance => "appearance",
            Self::Advanced => "advanced",
        }
    }
}

impl SettingsPage {
    pub const ALL: [Self; 7] = [
        Self::General,
        Self::Appearance,
        Self::Apps,
        Self::Taskbar,
        Self::Status,
        Self::Search,
        Self::About,
    ];

    pub const fn title(self) -> &'static str {
        match self {
            Self::Appearance => "appearance",
            Self::Apps => "apps",
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
    UseAcrylic,
    ShowAppDock,
    ShowUnpinnedRunningApps,
    ShowRunningIndicators,
    ShowOnAllMonitors,
    ShowDesktopButton,
    ShowSystemStatus,
    ShowVolumeStatus,
    ShowHdrStatus,
    ShowNetworkStatus,
    ShowBackgroundAppsStatus,
    ShowDateTimeStatus,
    ShowDateInStatus,
    Use24HourTime,
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
            Self::SystemStatus => "Volume, HDR, network, background apps and time",
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
    ScreenEdgeInset,
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
            Self::BottomOffset | Self::ScreenEdgeInset => (0, 96),
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
    ForegroundPreset,
    NotificationBadgeStyle,
    UpdateChannel,
    DockZone,
    SystemStatusZone,
    MediaZone,
    Toggle(SettingsToggle),
    Slider(SettingsSlider),
    ChooseMascotImage,
    ResetMascotImage,
    ApplicationSearch,
    ApplicationRow(usize),
    ChooseApplicationIcon(usize),
    ResetApplicationIcon(usize),
    CheckForUpdates,
    CancelUpdate,
    AcceptUpdate,
    RestartIntegration,
    ReplaySetup,
    ExportSettings,
    ExportDiagnostics,
    ResetLotus,
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
    RefreshPresentation,
    Reverted,
    OpenApplications,
    ChooseBackgroundColor,
    ChooseAccentColor,
    ChooseForegroundColor,
    ChooseMascotImage,
    ChooseApplicationIcon(String),
    ResetApplicationIcon(String),
    CheckForUpdates,
    CancelUpdate,
    AcceptUpdate,
    RestartIntegration,
    ReplaySetup,
    ExportSettings,
    ExportDiagnostics,
    ResetLotus,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingsUpdatePrompt {
    version: String,
    installed: bool,
}

impl SettingsUpdatePrompt {
    pub fn version(&self) -> &str {
        &self.version
    }

    pub const fn is_installed(&self) -> bool {
        self.installed
    }
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
pub struct SettingsApplicationRecord {
    pub id: String,
    pub name: String,
    pub icon: Option<RasterIcon>,
    pub app_user_model_id: Option<String>,
    pub match_executables: Vec<String>,
    pub customized: bool,
    pub missing_icon: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SettingsSectionLayout {
    pub section: SettingsSection,
    pub bounds: SettingsRect,
}

struct PageContentLayout {
    sections: Vec<(SettingsSection, u32)>,
    controls: Vec<(SettingsControl, u32)>,
    height: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingsLayout {
    pub size: SettingsSize,
    pub controls: Vec<SettingsControlLayout>,
    pub sections: Vec<SettingsSectionLayout>,
    pub content_viewport: SettingsRect,
    pub content_scroll_offset: u32,
    pub scrollbar_thumb: Option<SettingsRect>,
}

impl SettingsLayout {
    pub fn content_intersects_viewport(&self, bounds: SettingsRect) -> bool {
        let translated_top = i64::from(bounds.top) - i64::from(self.content_scroll_offset);
        let translated_bottom = translated_top + i64::from(bounds.height);
        let viewport_top = i64::from(self.content_viewport.top);
        let viewport_bottom = viewport_top + i64::from(self.content_viewport.height);

        translated_bottom > viewport_top && translated_top < viewport_bottom
    }

    pub fn hit_test(&self, x: u32, y: u32) -> Option<SettingsControl> {
        self.controls
            .iter()
            .rev()
            .find(|entry| {
                if !is_page_content(entry.control) {
                    return entry.bounds.contains(x, y);
                }
                self.content_viewport.contains(x, y)
                    && entry
                        .bounds
                        .contains(x, y.saturating_add(self.content_scroll_offset))
            })
            .map(|entry| entry.control)
    }

    pub fn bounds(&self, control: SettingsControl) -> Option<SettingsRect> {
        self.controls
            .iter()
            .find(|entry| entry.control == control)
            .map(|entry| {
                if is_page_content(control) {
                    SettingsRect {
                        top: entry.bounds.top.saturating_sub(self.content_scroll_offset),
                        ..entry.bounds
                    }
                } else {
                    entry.bounds
                }
            })
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
    available_size: Option<SettingsSize>,
    page: SettingsPage,
    draft: SettingsDraft,
    hovered: Option<SettingsControl>,
    focused: Option<SettingsControl>,
    focus_visible: bool,
    installed: bool,
    update_activity: SettingsUpdateActivity,
    update_prompt: Option<SettingsUpdatePrompt>,
    focus_before_update_prompt: Option<SettingsControl>,
    onboarding: OnboardingState,
    scroll_offset_dip: u32,
    applications: Vec<SettingsApplicationRecord>,
    application_query: String,
    filtered_application_indices: Vec<usize>,
    selected_application: Option<usize>,
}

impl SettingsScene {
    pub fn new(dpi: u32, settings: DockSettings, installed: bool) -> Option<Self> {
        let dpi = NonZeroU32::new(dpi)?;
        let settings = settings.normalized();
        Some(Self {
            dpi,
            available_size: None,
            page: SettingsPage::General,
            draft: SettingsDraft::new(settings),
            hovered: None,
            focused: Some(SettingsControl::Navigate(SettingsPage::General)),
            focus_visible: false,
            installed,
            update_activity: SettingsUpdateActivity::Idle,
            update_prompt: None,
            focus_before_update_prompt: None,
            onboarding: OnboardingState::default(),
            scroll_offset_dip: 0,
            applications: Vec::new(),
            application_query: String::new(),
            filtered_application_indices: Vec::new(),
            selected_application: None,
        })
    }

    pub const fn dpi(&self) -> u32 {
        self.dpi.get()
    }

    pub fn effective_dpi(&self) -> u32 {
        let raw_dpi = u64::from(self.dpi.get());
        let Some(available) = self.available_size else {
            return self.dpi.get();
        };
        let full_width = scaled_at_dpi(WIDTH_DIP, raw_dpi);
        let full_height = scaled_at_dpi(HEIGHT_DIP, raw_dpi);
        let numerator = u64::from(available.width())
            .saturating_mul(full_height)
            .min(u64::from(available.height()).saturating_mul(full_width));
        let denominator = full_width.saturating_mul(full_height).max(1);
        let fitted = raw_dpi.saturating_mul(numerator.min(denominator)) / denominator;
        u32::try_from(fitted).unwrap_or(u32::MAX).max(1)
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

    pub fn set_available_size(&mut self, width: u32, height: u32) -> bool {
        let size = SettingsSize::new(width, height);
        if self.available_size == size {
            return false;
        }
        self.available_size = size;
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
        self.draft.value()
    }
    pub fn theme(&self) -> Theme {
        theme_for(self.draft.value())
    }
    pub fn is_dirty(&self) -> bool {
        self.draft.is_dirty()
    }
    pub const fn update_activity(&self) -> SettingsUpdateActivity {
        self.update_activity
    }
    pub const fn is_installed(&self) -> bool {
        self.installed
    }

    pub fn update_prompt(&self) -> Option<&SettingsUpdatePrompt> {
        self.update_prompt.as_ref()
    }

    pub fn show_update_prompt(&mut self, version: String, installed: bool) -> bool {
        let prompt = SettingsUpdatePrompt { version, installed };
        if self.update_prompt.as_ref() == Some(&prompt) {
            return false;
        }
        if self.update_prompt.is_none() {
            self.focus_before_update_prompt = self.focused;
        }
        self.update_prompt = Some(prompt);
        self.hovered = None;
        self.focused = Some(SettingsControl::AcceptUpdate);
        self.focus_visible = false;
        true
    }

    pub fn dismiss_update_prompt(&mut self) -> bool {
        if self.update_prompt.take().is_none() {
            return false;
        }
        self.hovered = None;
        let previous = self.focus_before_update_prompt.take();
        self.focused = previous
            .filter(|control| self.layout().bounds(*control).is_some())
            .or(Some(SettingsControl::CheckForUpdates));
        self.focus_visible = false;
        true
    }

    pub fn applications(&self) -> &[SettingsApplicationRecord] {
        &self.applications
    }

    pub fn selected_application(&self) -> Option<&SettingsApplicationRecord> {
        self.selected_application
            .and_then(|index| self.applications.get(index))
    }

    pub fn application_actions_visible(&self, index: usize) -> bool {
        matches!(
            self.hovered,
            Some(
                SettingsControl::ApplicationRow(candidate)
                    | SettingsControl::ChooseApplicationIcon(candidate)
                    | SettingsControl::ResetApplicationIcon(candidate)
            ) if candidate == index
        ) || matches!(
            self.focused,
            Some(
                SettingsControl::ApplicationRow(candidate)
                    | SettingsControl::ChooseApplicationIcon(candidate)
                    | SettingsControl::ResetApplicationIcon(candidate)
            ) if candidate == index
        )
    }

    pub fn set_applications(
        &mut self,
        applications: Vec<SettingsApplicationRecord>,
    ) -> bool {
        if self.applications == applications {
            return false;
        }
        self.applications = applications;
        self.refresh_application_filter();
        self.selected_application = None;
        self.scroll_offset_dip = 0;
        true
    }

    pub fn set_application_query(&mut self, query: &str) -> bool {
        let query = query.to_owned();
        if self.application_query == query {
            return false;
        }
        self.application_query = query;
        self.refresh_application_filter();
        self.selected_application = None;
        self.scroll_offset_dip = 0;
        true
    }

    fn refresh_application_filter(&mut self) {
        let query = self.application_query.trim().to_ascii_lowercase();
        self.filtered_application_indices = self
            .applications
            .iter()
            .enumerate()
            .filter(|(_, application)| {
                query.is_empty() || application.name.to_ascii_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect();
    }

    pub fn application_query(&self) -> &str {
        &self.application_query
    }

    pub fn merge_application_icon_overrides(
        &self,
        current: &DockSettings,
    ) -> Vec<ApplicationIconOverride> {
        self.draft.merged_application_icon_overrides(current)
    }

    pub fn reconcile_application_icon_overrides(&mut self, current: &DockSettings) {
        self.draft.reconcile_application_icon_overrides(current);
    }

    pub fn select_application(&mut self, index: usize) -> bool {
        if index >= self.applications.len() || self.selected_application == Some(index) {
            return false;
        }
        self.selected_application = Some(index);
        true
    }

    pub fn set_application_icon(&mut self, id: &str, icon: RasterIcon) -> bool {
        let Some(application) = self
            .applications
            .iter_mut()
            .find(|application| application.id.eq_ignore_ascii_case(id))
        else {
            return false;
        };
        if application.icon.as_ref() == Some(&icon) {
            return false;
        }
        application.icon = Some(icon);
        true
    }

    pub fn open_application_manager(&mut self, id: &str) -> bool {
        let Some(index) = self.applications.iter().position(|app| app.id == id) else {
            return false;
        };
        self.page = SettingsPage::Apps;
        self.application_query.clear();
        self.refresh_application_filter();
        self.selected_application = Some(index);
        self.scroll_offset_dip = 0;
        self.focused = Some(SettingsControl::ApplicationRow(index));
        self.reveal_focused_control();
        true
    }

    pub const fn onboarding_step(&self) -> Option<OnboardingStep> {
        self.onboarding.step()
    }

    pub const fn onboarding_required(&self) -> bool {
        self.onboarding.required()
    }

    pub fn begin_onboarding(&mut self, applied: DockSettings, required: bool) {
        let applied = applied.normalized();
        self.draft.begin(applied);
        self.page = SettingsPage::General;
        self.onboarding.begin(required);
        self.scroll_offset_dip = 0;
        self.hovered = None;
        self.focused = Some(SettingsControl::OnboardingNext);
        self.focus_visible = false;
    }

    pub fn end_onboarding(&mut self) {
        self.onboarding.end();
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
        self.available_size.unwrap_or_else(|| SettingsSize {
            width: nonzero(self.scale(WIDTH_DIP)),
            height: nonzero(self.scale(HEIGHT_DIP)),
        })
    }

    pub fn scroll(&mut self, direction: i32) -> bool {
        if self.update_prompt.is_some()
            || self.onboarding.step().is_some()
            || direction == 0
        {
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
        let hovered = self.hit_test(x, y);
        self.set_hovered(hovered)
    }

    pub fn scrollbar_press(&mut self, x: u32, y: u32) -> Option<u32> {
        if self.update_prompt.is_some() || self.onboarding.step().is_some() {
            return None;
        }
        let layout = self.layout();
        let thumb = layout.scrollbar_thumb?;
        let tolerance = self.scale(8);
        let in_hit_width = x >= thumb.left.saturating_sub(tolerance)
            && x < thumb
                .left
                .saturating_add(thumb.width)
                .saturating_add(tolerance);
        if !in_hit_width
            || y < layout.content_viewport.top
            || y >= layout
                .content_viewport
                .top
                .saturating_add(layout.content_viewport.height)
        {
            return None;
        }
        let grab_offset = if y >= thumb.top && y < thumb.top.saturating_add(thumb.height) {
            y.saturating_sub(thumb.top)
        } else {
            thumb.height / 2
        };
        self.set_scrollbar_from_pointer(i32::try_from(y).unwrap_or(i32::MAX), grab_offset);
        Some(grab_offset)
    }

    pub fn set_scrollbar_from_pointer(&mut self, y: i32, grab_offset: u32) -> bool {
        let layout = self.layout();
        let Some(thumb) = layout.scrollbar_thumb else {
            return false;
        };
        let maximum = self.maximum_scroll_offset_dip();
        let travel = layout.content_viewport.height.saturating_sub(thumb.height);
        if maximum == 0 || travel == 0 {
            return false;
        }
        let track_top = i64::from(layout.content_viewport.top);
        let target = (i64::from(y) - i64::from(grab_offset))
            .clamp(track_top, track_top.saturating_add(i64::from(travel)));
        let position = u32::try_from(target.saturating_sub(track_top)).unwrap_or(travel);
        let next = u32::try_from(
            (u64::from(position) * u64::from(maximum) + u64::from(travel / 2))
                / u64::from(travel),
        )
        .unwrap_or(maximum);
        if next == self.scroll_offset_dip {
            return false;
        }
        self.scroll_offset_dip = next.min(maximum);
        self.hovered = None;
        true
    }

    pub fn hit_test(&self, x: u32, y: u32) -> Option<SettingsControl> {
        let control = self.layout().hit_test(x, y)?;
        if self.update_prompt.is_some()
            && !matches!(
                control,
                SettingsControl::CancelUpdate | SettingsControl::AcceptUpdate
            )
        {
            return None;
        }
        Some(control)
    }

    pub fn pointer_activate(&mut self, x: u32, y: u32) -> SettingsAction {
        let Some(control) = self.hit_test(x, y) else {
            return SettingsAction::None;
        };
        self.focused = Some(control);
        self.focus_visible = false;
        if matches!(
            control,
            SettingsControl::SurfacePreset
                | SettingsControl::AccentPreset
                | SettingsControl::ForegroundPreset
                | SettingsControl::NotificationBadgeStyle
                | SettingsControl::UpdateChannel
                | SettingsControl::DockZone
                | SettingsControl::SystemStatusZone
                | SettingsControl::MediaZone
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
        let Some(control) = self.hit_test(x, y) else {
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
            | SettingsControl::ForegroundPreset
            | SettingsControl::NotificationBadgeStyle
            | SettingsControl::UpdateChannel
            | SettingsControl::DockZone
            | SettingsControl::SystemStatusZone
            | SettingsControl::MediaZone
            | SettingsControl::Toggle(_)
            | SettingsControl::ChooseMascotImage
            | SettingsControl::ResetMascotImage
            | SettingsControl::ApplicationSearch
            | SettingsControl::ApplicationRow(_)
            | SettingsControl::ChooseApplicationIcon(_)
            | SettingsControl::ResetApplicationIcon(_)
            | SettingsControl::CheckForUpdates
            | SettingsControl::CancelUpdate
            | SettingsControl::AcceptUpdate
            | SettingsControl::RestartIntegration
            | SettingsControl::ReplaySetup
            | SettingsControl::ExportSettings
            | SettingsControl::ExportDiagnostics
            | SettingsControl::ResetLotus
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
        let SettingsControl::Slider(slider) = self.hit_test(x, y)? else {
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
        if self.update_prompt.is_some() {
            return self.update_prompt_key(key);
        }
        if key == SettingsKey::Escape {
            if self.onboarding.required() {
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
            return SettingsAction::Changed;
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
            (SettingsKey::Left, SettingsControl::ForegroundPreset) => {
                self.cycle_foreground_preset(true)
            }
            (SettingsKey::Right, SettingsControl::ForegroundPreset) => {
                self.cycle_foreground_preset(false)
            }
            (SettingsKey::Left, SettingsControl::NotificationBadgeStyle) => {
                self.cycle_notification_badge_style(true)
            }
            (SettingsKey::Right, SettingsControl::NotificationBadgeStyle) => {
                self.cycle_notification_badge_style(false)
            }
            (SettingsKey::Left, SettingsControl::UpdateChannel) => {
                self.cycle_update_channel(true)
            }
            (SettingsKey::Right, SettingsControl::UpdateChannel) => {
                self.cycle_update_channel(false)
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
            (SettingsKey::Left, SettingsControl::OnboardingZone(module)) => {
                self.cycle_onboarding_zone(module, true)
            }
            (SettingsKey::Right, SettingsControl::OnboardingZone(module)) => {
                self.cycle_onboarding_zone(module, false)
            }
            _ => SettingsAction::None,
        }
    }

    fn update_prompt_key(&mut self, key: SettingsKey) -> SettingsAction {
        match key {
            SettingsKey::Escape => SettingsAction::CancelUpdate,
            SettingsKey::Activate => self
                .focused
                .map_or(SettingsAction::None, |control| self.activate(control)),
            SettingsKey::Tab
            | SettingsKey::ReverseTab
            | SettingsKey::Left
            | SettingsKey::Right
            | SettingsKey::Up
            | SettingsKey::Down => {
                self.focused =
                    Some(if self.focused == Some(SettingsControl::AcceptUpdate) {
                        SettingsControl::CancelUpdate
                    } else {
                        SettingsControl::AcceptUpdate
                    });
                SettingsAction::RefreshPresentation
            }
        }
    }

    pub fn mark_applied(&mut self, applied: DockSettings) {
        let applied = applied.normalized();
        self.draft.mark_applied(applied);
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
        u32::try_from(scaled_at_dpi(dips, u64::from(self.effective_dpi())))
            .unwrap_or(u32::MAX)
    }
}

fn scaled_at_dpi(dips: u32, dpi: u64) -> u64 {
    (u64::from(dips).saturating_mul(dpi) + DIPS_PER_INCH / 2) / DIPS_PER_INCH
}

fn nonzero(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap_or(NonZeroU32::MIN)
}

fn is_page_content(control: SettingsControl) -> bool {
    matches!(
        control,
        SettingsControl::SurfacePreset
            | SettingsControl::AccentPreset
            | SettingsControl::ForegroundPreset
            | SettingsControl::NotificationBadgeStyle
            | SettingsControl::UpdateChannel
            | SettingsControl::DockZone
            | SettingsControl::SystemStatusZone
            | SettingsControl::MediaZone
            | SettingsControl::Toggle(_)
            | SettingsControl::Slider(_)
            | SettingsControl::ChooseMascotImage
            | SettingsControl::ResetMascotImage
            | SettingsControl::ApplicationSearch
            | SettingsControl::ApplicationRow(_)
            | SettingsControl::ChooseApplicationIcon(_)
            | SettingsControl::ResetApplicationIcon(_)
            | SettingsControl::RestartIntegration
            | SettingsControl::ReplaySetup
            | SettingsControl::ExportSettings
            | SettingsControl::ExportDiagnostics
            | SettingsControl::ResetLotus
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
