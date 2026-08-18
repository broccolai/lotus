use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const CURRENT_APPEARANCE_VERSION: u32 = 3;
pub const CURRENT_ONBOARDING_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NotificationBadgeStyle {
    #[default]
    Off,
    Dot,
    Count,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WindowPickerStyle {
    Compact,
    #[default]
    Thumbnails,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateChannel {
    #[default]
    Stable,
    Alpha,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DockZone {
    Left,
    #[default]
    Center,
    Right,
}

impl DockZone {
    pub const ALL: [Self; 3] = [Self::Left, Self::Center, Self::Right];
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent persisted preferences are not mutually exclusive state"
)]
pub struct DockSettings {
    pub onboarding_version: u32,
    pub icon_size: u32,
    pub item_spacing: u32,
    pub horizontal_padding: u32,
    pub vertical_padding: u32,
    pub bottom_offset: u32,
    pub screen_edge_inset: u32,
    pub corner_radius: u32,
    pub appearance_version: u32,
    pub background_opacity: f64,
    pub background_color: String,
    pub accent_color: String,
    pub foreground_color: String,
    pub mascot_image_path: Option<String>,
    pub show_app_dock: bool,
    pub show_unpinned_running_apps: bool,
    pub show_running_indicators: bool,
    pub show_on_all_monitors: bool,
    pub show_desktop_button: bool,
    pub show_system_status: bool,
    pub dock_zone: DockZone,
    pub system_status_zone: DockZone,
    pub show_volume_status: bool,
    pub show_network_status: bool,
    pub show_background_apps_status: bool,
    pub show_date_time_status: bool,
    pub show_date_in_status: bool,
    pub use_24_hour_time: bool,
    pub show_media_controls: bool,
    pub show_media_metadata: bool,
    pub media_zone: DockZone,
    pub start_with_windows: bool,
    pub update_channel: UpdateChannel,
    pub hide_when_fullscreen: bool,
    pub replace_windows_taskbar: bool,
    pub exclusive_taskbar_replacement: bool,
    pub search_enabled: bool,
    pub search_open_with_windows_key: bool,
    pub alt_tab_enabled: bool,
    pub window_picker_style: WindowPickerStyle,
    pub notification_badge_style: NotificationBadgeStyle,
    pub notification_disabled_apps: Vec<String>,
    pub search_result_limit: u32,
    pub application_name_overrides: BTreeMap<String, String>,
    pub hidden_executables: Vec<String>,
    pub item_order: Vec<String>,
    pub pinned_apps: Vec<PinnedApp>,
}

impl Default for DockSettings {
    fn default() -> Self {
        Self {
            onboarding_version: 0,
            icon_size: 38,
            item_spacing: 8,
            horizontal_padding: 12,
            vertical_padding: 8,
            bottom_offset: 10,
            screen_edge_inset: 12,
            corner_radius: 8,
            appearance_version: CURRENT_APPEARANCE_VERSION,
            background_opacity: 0.56,
            background_color: "#11141A".into(),
            accent_color: "#F5A5A5".into(),
            foreground_color: "#F7F8FB".into(),
            mascot_image_path: None,
            show_app_dock: true,
            show_unpinned_running_apps: true,
            show_running_indicators: true,
            show_on_all_monitors: false,
            show_desktop_button: false,
            show_system_status: true,
            dock_zone: DockZone::Center,
            system_status_zone: DockZone::Center,
            show_volume_status: true,
            show_network_status: true,
            show_background_apps_status: true,
            show_date_time_status: true,
            show_date_in_status: true,
            use_24_hour_time: true,
            show_media_controls: true,
            show_media_metadata: true,
            media_zone: DockZone::Center,
            start_with_windows: true,
            update_channel: UpdateChannel::Stable,
            hide_when_fullscreen: true,
            replace_windows_taskbar: true,
            exclusive_taskbar_replacement: true,
            search_enabled: true,
            search_open_with_windows_key: true,
            alt_tab_enabled: false,
            window_picker_style: WindowPickerStyle::Thumbnails,
            notification_badge_style: NotificationBadgeStyle::Off,
            notification_disabled_apps: Vec::new(),
            search_result_limit: 5,
            application_name_overrides: BTreeMap::new(),
            hidden_executables: Vec::new(),
            item_order: Vec::new(),
            pinned_apps: Vec::new(),
        }
    }
}

impl DockSettings {
    #[must_use]
    pub fn normalized(mut self) -> Self {
        self.normalize_dimensions();
        self.normalize_appearance();
        self.normalize_collections();

        self
    }

    pub fn dock_height(&self) -> u32 {
        self.icon_size + self.vertical_padding * 2
    }

    fn normalize_dimensions(&mut self) {
        self.icon_size = self.icon_size.clamp(24, 72);
        self.item_spacing = self.item_spacing.clamp(2, 24);
        self.horizontal_padding = self.horizontal_padding.clamp(4, 48);
        self.vertical_padding = self.vertical_padding.clamp(4, 32);
        self.bottom_offset = self.bottom_offset.min(96);
        self.screen_edge_inset = self.screen_edge_inset.min(96);
        self.corner_radius = self.corner_radius.min(48);
        self.search_result_limit = self.search_result_limit.clamp(1, 8);
    }

    fn normalize_appearance(&mut self) {
        self.background_opacity = self.background_opacity.clamp(0.08, 0.95);
        self.window_picker_style = WindowPickerStyle::Thumbnails;

        if !is_hex_color(&self.background_color) {
            self.background_color = "#11141A".into();
        }
        if !is_hex_color(&self.accent_color) {
            self.accent_color = "#F5A5A5".into();
        }
        if !is_hex_color(&self.foreground_color) {
            self.foreground_color = "#F7F8FB".into();
        }

        self.mascot_image_path = self
            .mascot_image_path
            .take()
            .and_then(|path| (!path.trim().is_empty()).then(|| path.trim().to_owned()));
    }

    fn normalize_collections(&mut self) {
        self.notification_disabled_apps =
            normalized_unique_strings(std::mem::take(&mut self.notification_disabled_apps));

        self.pinned_apps.retain(PinnedApp::is_launchable);
        self.pinned_apps.iter_mut().for_each(PinnedApp::normalize);
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PinnedApp {
    pub id: String,
    pub name: String,
    pub launch_target: String,
    pub arguments: Option<String>,
    pub icon_source: Option<String>,
    pub app_user_model_id: Option<String>,
    pub match_executables: Vec<String>,
}

impl PinnedApp {
    fn is_launchable(&self) -> bool {
        !self.id.trim().is_empty() && !self.launch_target.trim().is_empty()
    }

    fn normalize(&mut self) {
        self.id = self.id.trim().into();
        self.name = match self.name.trim() {
            "" => "Application".into(),
            name => name.into(),
        };
        self.launch_target = self.launch_target.trim().into();
        self.app_user_model_id = self.app_user_model_id.take().and_then(|identity| {
            (!identity.trim().is_empty()).then(|| identity.trim().into())
        });
    }
}

fn is_hex_color(value: &str) -> bool {
    let Some(hex) = value.strip_prefix('#') else {
        return false;
    };

    matches!(hex.len(), 6 | 8) && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn normalized_unique_strings(values: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::with_capacity(values.len());

    for value in values {
        let value = value.trim();
        if !value.is_empty()
            && !normalized
                .iter()
                .any(|saved: &String| saved.eq_ignore_ascii_case(value))
        {
            normalized.push(value.to_owned());
        }
    }

    normalized
}
