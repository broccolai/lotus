use lotus_core::settings::ApplicationIconOverride;

use super::{
    AccentPreset, CURRENT_ONBOARDING_VERSION, DockSettings, DockZone, ForegroundPreset,
    NotificationBadgeStyle, OnboardingModule, SettingsAction, SettingsControl,
    SettingsRect, SettingsScene, SettingsSlider, SettingsToggle, SettingsUpdateActivity,
    SurfacePreset, UpdateChannel, cycle_index,
};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct SettingsDraft {
    baseline: DockSettings,
    draft: DockSettings,
}

impl SettingsDraft {
    pub(super) fn new(settings: DockSettings) -> Self {
        let settings = settings.normalized();
        Self {
            baseline: settings.clone(),
            draft: settings,
        }
    }

    pub(super) const fn value(&self) -> &DockSettings {
        &self.draft
    }
    pub(super) fn is_dirty(&self) -> bool {
        self.draft != self.baseline
    }
    pub(super) fn begin(&mut self, applied: DockSettings) {
        let applied = applied.normalized();
        self.baseline = applied.clone();
        self.draft = applied;
    }
    pub(super) fn mark_applied(&mut self, applied: DockSettings) {
        self.begin(applied);
    }
    pub(super) fn revert(&mut self) {
        self.draft = self.baseline.clone();
    }
    pub(super) fn normalized(&self) -> DockSettings {
        self.draft.clone().normalized()
    }
    pub(super) fn merged_application_icon_overrides(
        &self,
        current: &DockSettings,
    ) -> Vec<ApplicationIconOverride> {
        lotus_core::settings::merge_application_icon_overrides(
            &self.baseline.application_icon_overrides,
            &self.draft.application_icon_overrides,
            &current.application_icon_overrides,
        )
    }
    pub(super) fn reconcile_application_icon_overrides(&mut self, current: &DockSettings) {
        self.draft.application_icon_overrides =
            self.merged_application_icon_overrides(current);
        self.baseline
            .application_icon_overrides
            .clone_from(&current.application_icon_overrides);
    }
    pub(super) fn complete_onboarding(&mut self) -> DockSettings {
        self.draft.onboarding_version = CURRENT_ONBOARDING_VERSION;
        self.normalized()
    }
    pub(super) fn has_mascot_image(&self) -> bool {
        self.draft.mascot_image_path.is_some()
    }
    pub(super) fn reset_mascot_image(&mut self) {
        self.draft.mascot_image_path = None;
    }

    pub(super) fn toggle_value(&self, toggle: SettingsToggle) -> bool {
        match toggle {
            SettingsToggle::UseAcrylic => self.draft.use_acrylic,
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
            SettingsToggle::Use24HourTime => self.draft.use_24_hour_time,
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

    pub(super) fn set_toggle_value(&mut self, toggle: SettingsToggle, value: bool) {
        match toggle {
            SettingsToggle::UseAcrylic => self.draft.use_acrylic = value,
            SettingsToggle::ShowAppDock => self.draft.show_app_dock = value,
            SettingsToggle::ShowUnpinnedRunningApps => {
                self.draft.show_unpinned_running_apps = value;
            }
            SettingsToggle::ShowRunningIndicators => {
                self.draft.show_running_indicators = value;
            }
            SettingsToggle::ShowOnAllMonitors => self.draft.show_on_all_monitors = value,
            SettingsToggle::ShowDesktopButton => self.draft.show_desktop_button = value,
            SettingsToggle::ShowSystemStatus => self.draft.show_system_status = value,
            SettingsToggle::ShowVolumeStatus => self.draft.show_volume_status = value,
            SettingsToggle::ShowNetworkStatus => self.draft.show_network_status = value,
            SettingsToggle::ShowBackgroundAppsStatus => {
                self.draft.show_background_apps_status = value;
            }
            SettingsToggle::ShowDateTimeStatus => self.draft.show_date_time_status = value,
            SettingsToggle::ShowDateInStatus => self.draft.show_date_in_status = value,
            SettingsToggle::Use24HourTime => self.draft.use_24_hour_time = value,
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

    pub(super) fn slider_value(&self, slider: SettingsSlider) -> u32 {
        match slider {
            SettingsSlider::IconSize => self.draft.icon_size,
            SettingsSlider::ItemSpacing => self.draft.item_spacing,
            SettingsSlider::HorizontalPadding => self.draft.horizontal_padding,
            SettingsSlider::VerticalPadding => self.draft.vertical_padding,
            SettingsSlider::BottomOffset => self.draft.bottom_offset,
            SettingsSlider::ScreenEdgeInset => self.draft.screen_edge_inset,
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

    pub(super) fn set_slider(&mut self, slider: SettingsSlider, value: u32) {
        let (minimum, maximum) = slider.range();
        let value = value.clamp(minimum, maximum);
        match slider {
            SettingsSlider::IconSize => self.draft.icon_size = value,
            SettingsSlider::ItemSpacing => self.draft.item_spacing = value,
            SettingsSlider::HorizontalPadding => self.draft.horizontal_padding = value,
            SettingsSlider::VerticalPadding => self.draft.vertical_padding = value,
            SettingsSlider::BottomOffset => self.draft.bottom_offset = value,
            SettingsSlider::ScreenEdgeInset => self.draft.screen_edge_inset = value,
            SettingsSlider::CornerRadius => self.draft.corner_radius = value,
            SettingsSlider::BackgroundOpacity => {
                self.draft.background_opacity = f64::from(value) / 100.0;
            }
            SettingsSlider::SearchResultLimit => self.draft.search_result_limit = value,
        }
    }

    pub(super) fn set_mascot_image_path(&mut self, path: Option<String>) {
        self.draft.mascot_image_path = path;
    }
    pub(super) fn set_background_color(&mut self, color: String) {
        self.draft.background_color = color;
    }
    pub(super) fn set_accent_color(&mut self, color: String) {
        self.draft.accent_color = color;
    }
    pub(super) fn set_foreground_color(&mut self, color: String) {
        self.draft.foreground_color = color;
    }

    pub(super) fn select_picker(
        &mut self,
        control: SettingsControl,
        index: usize,
    ) -> SettingsAction {
        match control {
            SettingsControl::SurfacePreset => match SurfacePreset::ALL.get(index) {
                Some(preset) => preset.color().clone_into(&mut self.draft.background_color),
                None => return SettingsAction::ChooseBackgroundColor,
            },
            SettingsControl::AccentPreset => match AccentPreset::ALL.get(index) {
                Some(preset) => preset.color().clone_into(&mut self.draft.accent_color),
                None => return SettingsAction::ChooseAccentColor,
            },
            SettingsControl::ForegroundPreset => match ForegroundPreset::ALL.get(index) {
                Some(preset) => preset.color().clone_into(&mut self.draft.foreground_color),
                None => return SettingsAction::ChooseForegroundColor,
            },
            SettingsControl::NotificationBadgeStyle => {
                let styles = [
                    NotificationBadgeStyle::Off,
                    NotificationBadgeStyle::Dot,
                    NotificationBadgeStyle::Count,
                ];
                let Some(style) = styles.get(index) else {
                    return SettingsAction::None;
                };
                self.draft.notification_badge_style = *style;
            }
            SettingsControl::UpdateChannel => {
                let channels = [UpdateChannel::Stable, UpdateChannel::Alpha];
                let Some(channel) = channels.get(index) else {
                    return SettingsAction::None;
                };
                self.draft.update_channel = *channel;
            }
            SettingsControl::DockZone
            | SettingsControl::SystemStatusZone
            | SettingsControl::MediaZone => {
                let Some(zone) = DockZone::ALL.get(index) else {
                    return SettingsAction::None;
                };
                self.set_zone(control, *zone);
            }
            _ => return SettingsAction::None,
        }
        SettingsAction::Changed
    }

    pub(super) fn cycle_surface_preset(&mut self, reverse: bool) {
        let current = SurfacePreset::selected(&self.draft)
            .and_then(|selected| {
                SurfacePreset::ALL.iter().position(|item| *item == selected)
            })
            .unwrap_or_default();
        SurfacePreset::ALL[cycle_index(current, SurfacePreset::ALL.len(), reverse)]
            .color()
            .clone_into(&mut self.draft.background_color);
    }
    pub(super) fn cycle_accent_preset(&mut self, reverse: bool) {
        let current = AccentPreset::selected(&self.draft)
            .and_then(|selected| {
                AccentPreset::ALL.iter().position(|item| *item == selected)
            })
            .unwrap_or_default();
        AccentPreset::ALL[cycle_index(current, AccentPreset::ALL.len(), reverse)]
            .color()
            .clone_into(&mut self.draft.accent_color);
    }
    pub(super) fn cycle_foreground_preset(&mut self, reverse: bool) {
        let current = ForegroundPreset::selected(&self.draft)
            .and_then(|selected| {
                ForegroundPreset::ALL
                    .iter()
                    .position(|item| *item == selected)
            })
            .unwrap_or_default();
        ForegroundPreset::ALL[cycle_index(current, ForegroundPreset::ALL.len(), reverse)]
            .color()
            .clone_into(&mut self.draft.foreground_color);
    }

    pub(super) fn cycle_notification_badge_style(&mut self, reverse: bool) {
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
    }
    pub(super) fn cycle_update_channel(&mut self, reverse: bool) {
        let channels = [UpdateChannel::Stable, UpdateChannel::Alpha];
        let current = channels
            .iter()
            .position(|channel| *channel == self.draft.update_channel)
            .unwrap_or_default();
        self.draft.update_channel = channels[cycle_index(current, channels.len(), reverse)];
    }
    pub(super) fn cycle_zone(&mut self, status: bool, reverse: bool) {
        self.cycle_zone_for(
            if status {
                SettingsControl::SystemStatusZone
            } else {
                SettingsControl::DockZone
            },
            reverse,
        );
    }
    pub(super) fn cycle_media_zone(&mut self, reverse: bool) {
        self.cycle_zone_for(SettingsControl::MediaZone, reverse);
    }

    pub(super) fn onboarding_module_enabled(&self, module: OnboardingModule) -> bool {
        match module {
            OnboardingModule::AppDock => self.draft.show_app_dock,
            OnboardingModule::Search => self.draft.search_enabled,
            OnboardingModule::SystemStatus => self.draft.show_system_status,
            OnboardingModule::Media => self.draft.show_media_controls,
            OnboardingModule::AltTab => self.draft.alt_tab_enabled,
        }
    }
    pub(super) fn set_onboarding_module(
        &mut self,
        module: OnboardingModule,
        enabled: bool,
    ) {
        match module {
            OnboardingModule::AppDock => self.draft.show_app_dock = enabled,
            OnboardingModule::Search => self.draft.search_enabled = enabled,
            OnboardingModule::SystemStatus => self.draft.show_system_status = enabled,
            OnboardingModule::Media => self.draft.show_media_controls = enabled,
            OnboardingModule::AltTab => self.draft.alt_tab_enabled = enabled,
        }
    }
    pub(super) const fn onboarding_zone(&self, module: OnboardingModule) -> DockZone {
        match module {
            OnboardingModule::AppDock => self.draft.dock_zone,
            OnboardingModule::SystemStatus => self.draft.system_status_zone,
            OnboardingModule::Media => self.draft.media_zone,
            OnboardingModule::Search | OnboardingModule::AltTab => DockZone::Center,
        }
    }
    pub(super) fn set_onboarding_zone(&mut self, module: OnboardingModule, zone: DockZone) {
        match module {
            OnboardingModule::AppDock => self.draft.dock_zone = zone,
            OnboardingModule::SystemStatus => self.draft.system_status_zone = zone,
            OnboardingModule::Media => self.draft.media_zone = zone,
            OnboardingModule::Search | OnboardingModule::AltTab => {}
        }
    }

    fn cycle_zone_for(&mut self, control: SettingsControl, reverse: bool) {
        let selected = match control {
            SettingsControl::DockZone => self.draft.dock_zone,
            SettingsControl::SystemStatusZone => self.draft.system_status_zone,
            SettingsControl::MediaZone => self.draft.media_zone,
            _ => return,
        };
        let current = DockZone::ALL
            .iter()
            .position(|zone| *zone == selected)
            .unwrap_or_default();
        self.set_zone(
            control,
            DockZone::ALL[cycle_index(current, DockZone::ALL.len(), reverse)],
        );
    }
    fn set_zone(&mut self, control: SettingsControl, zone: DockZone) {
        match control {
            SettingsControl::DockZone => self.draft.dock_zone = zone,
            SettingsControl::SystemStatusZone => self.draft.system_status_zone = zone,
            SettingsControl::MediaZone => self.draft.media_zone = zone,
            _ => {}
        }
    }
}

impl SettingsScene {
    pub(super) fn activate(&mut self, control: SettingsControl) -> SettingsAction {
        match control {
            SettingsControl::Navigate(page) => {
                self.page = page;
                self.scroll_offset_dip = 0;
                self.focused = Some(SettingsControl::Navigate(page));
                if page == crate::scene::SettingsPage::Apps {
                    SettingsAction::OpenApplications
                } else {
                    SettingsAction::Changed
                }
            }
            SettingsControl::Toggle(toggle) => {
                self.draft
                    .set_toggle_value(toggle, !self.draft.toggle_value(toggle));
                SettingsAction::Changed
            }
            SettingsControl::Apply if self.is_dirty() => {
                SettingsAction::Apply(Box::new(self.draft.normalized()))
            }
            SettingsControl::ChooseMascotImage => SettingsAction::ChooseMascotImage,
            SettingsControl::ApplicationSearch => {
                self.focused = Some(SettingsControl::ApplicationSearch);
                SettingsAction::Changed
            }
            SettingsControl::ApplicationRow(index) => {
                self.selected_application = Some(index);
                SettingsAction::Changed
            }
            SettingsControl::ChooseApplicationIcon(index) => {
                self.selected_application = Some(index);
                self.applications
                    .get(index)
                    .map_or(SettingsAction::None, |app| {
                        SettingsAction::ChooseApplicationIcon(app.id.clone())
                    })
            }
            SettingsControl::ResetApplicationIcon(index) => {
                self.selected_application = Some(index);
                self.applications
                    .get(index)
                    .map_or(SettingsAction::None, |app| {
                        SettingsAction::ResetApplicationIcon(app.id.clone())
                    })
            }
            SettingsControl::CheckForUpdates
                if self.update_activity == SettingsUpdateActivity::Idle =>
            {
                SettingsAction::CheckForUpdates
            }
            SettingsControl::RestartIntegration => SettingsAction::RestartIntegration,
            SettingsControl::ReplaySetup => SettingsAction::ReplaySetup,
            SettingsControl::ExportSettings => SettingsAction::ExportSettings,
            SettingsControl::ExportDiagnostics => SettingsAction::ExportDiagnostics,
            SettingsControl::ResetLotus => SettingsAction::ResetLotus,
            SettingsControl::OnboardingModule(module) => {
                self.draft.set_onboarding_module(
                    module,
                    !self.draft.onboarding_module_enabled(module),
                );
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
            SettingsControl::OnboardingFinish => SettingsAction::CompleteOnboarding(
                Box::new(self.draft.complete_onboarding()),
            ),
            SettingsControl::ResetMascotImage => {
                self.draft.reset_mascot_image();
                SettingsAction::Changed
            }
            SettingsControl::Revert if self.is_dirty() => {
                self.draft.revert();
                SettingsAction::Reverted
            }
            SettingsControl::SurfacePreset => SettingsAction::ChooseBackgroundColor,
            SettingsControl::AccentPreset => SettingsAction::ChooseAccentColor,
            SettingsControl::ForegroundPreset => SettingsAction::ChooseForegroundColor,
            SettingsControl::NotificationBadgeStyle
            | SettingsControl::UpdateChannel
            | SettingsControl::DockZone
            | SettingsControl::SystemStatusZone
            | SettingsControl::MediaZone
            | SettingsControl::Slider(_)
            | SettingsControl::CheckForUpdates
            | SettingsControl::Revert
            | SettingsControl::Apply => SettingsAction::None,
            SettingsControl::Close => SettingsAction::Close,
        }
    }

    pub(super) fn set_picker_from_pointer(
        &mut self,
        control: SettingsControl,
        bounds: SettingsRect,
        x: u32,
    ) -> SettingsAction {
        let picker = self.control_column(bounds);
        if x < picker.left || x >= picker.left.saturating_add(picker.width.max(1)) {
            return SettingsAction::None;
        }
        let width = picker.width.max(1);
        let offset = x.saturating_sub(picker.left).min(width.saturating_sub(1));
        let count = match control {
            SettingsControl::SurfacePreset => 4,
            SettingsControl::AccentPreset => 6,
            SettingsControl::UpdateChannel => 2,
            SettingsControl::ForegroundPreset
            | SettingsControl::NotificationBadgeStyle
            | SettingsControl::DockZone
            | SettingsControl::SystemStatusZone
            | SettingsControl::MediaZone => 3,
            _ => return SettingsAction::None,
        };
        let index =
            usize::try_from(offset.saturating_mul(count) / width).unwrap_or_default();
        self.draft.select_picker(control, index)
    }

    pub(super) fn cycle_surface_preset(&mut self, reverse: bool) -> SettingsAction {
        self.draft.cycle_surface_preset(reverse);
        SettingsAction::Changed
    }
    pub(super) fn cycle_accent_preset(&mut self, reverse: bool) -> SettingsAction {
        self.draft.cycle_accent_preset(reverse);
        SettingsAction::Changed
    }
    pub(super) fn cycle_foreground_preset(&mut self, reverse: bool) -> SettingsAction {
        self.draft.cycle_foreground_preset(reverse);
        SettingsAction::Changed
    }

    pub(super) fn cycle_notification_badge_style(
        &mut self,
        reverse: bool,
    ) -> SettingsAction {
        self.draft.cycle_notification_badge_style(reverse);
        SettingsAction::Changed
    }
    pub(super) fn cycle_update_channel(&mut self, reverse: bool) -> SettingsAction {
        self.draft.cycle_update_channel(reverse);
        SettingsAction::Changed
    }
    pub(super) fn cycle_zone(&mut self, status: bool, reverse: bool) -> SettingsAction {
        self.draft.cycle_zone(status, reverse);
        SettingsAction::Changed
    }
    pub(super) fn cycle_media_zone(&mut self, reverse: bool) -> SettingsAction {
        self.draft.cycle_media_zone(reverse);
        SettingsAction::Changed
    }

    pub(super) fn adjust_slider(
        &mut self,
        slider: SettingsSlider,
        delta: i32,
    ) -> SettingsAction {
        let (minimum, maximum) = slider.range();
        let value = if delta < 0 {
            self.draft.slider_value(slider).saturating_sub(1)
        } else {
            self.draft.slider_value(slider).saturating_add(1)
        }
        .clamp(minimum, maximum);
        self.draft.set_slider(slider, value);
        SettingsAction::Changed
    }

    pub fn toggle(&self, toggle: SettingsToggle) -> bool {
        self.draft.toggle_value(toggle)
    }
    pub fn slider_value(&self, slider: SettingsSlider) -> u32 {
        self.draft.slider_value(slider)
    }
    pub fn set_mascot_image_path(&mut self, path: Option<String>) {
        self.draft.set_mascot_image_path(path);
    }
    pub fn set_application_icon_override(
        &mut self,
        mut override_: ApplicationIconOverride,
    ) {
        if let Some(existing) = self
            .draft
            .draft
            .application_icon_overrides
            .iter_mut()
            .find(|existing| existing.id.eq_ignore_ascii_case(&override_.id))
        {
            for (name, value) in std::mem::take(&mut existing.extra_fields) {
                override_.extra_fields.entry(name).or_insert(value);
            }
            *existing = override_;
        } else {
            self.draft.draft.application_icon_overrides.push(override_);
        }
    }
    pub fn reset_application_icon_override(&mut self, id: &str) {
        self.draft
            .draft
            .application_icon_overrides
            .retain(|override_| !override_.id.eq_ignore_ascii_case(id));
    }
    pub fn set_background_color(&mut self, color: String) {
        self.draft.set_background_color(color);
    }
    pub fn set_accent_color(&mut self, color: String) {
        self.draft.set_accent_color(color);
    }
    pub fn set_foreground_color(&mut self, color: String) {
        self.draft.set_foreground_color(color);
    }
    pub(super) fn set_slider(&mut self, slider: SettingsSlider, value: u32) {
        self.draft.set_slider(slider, value);
    }
}
