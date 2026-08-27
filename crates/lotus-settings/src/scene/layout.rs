use super::{
    ACTION_GAP_DIP, ACTION_HEIGHT_DIP, APPLY_WIDTH_DIP, CLOSE_SIZE_DIP,
    CONTENT_BOTTOM_INSET_DIP, CONTENT_LEFT_DIP, CONTENT_RIGHT_DIP, CONTENT_TOP_DIP,
    FOOTER_HEIGHT_DIP, HEIGHT_DIP, NAV_HEIGHT_DIP, NAV_LEFT_DIP, NAV_WIDTH_DIP,
    OnboardingModule, OnboardingStep, PageContentLayout, REVERT_WIDTH_DIP, ROW_GAP_DIP,
    ROW_HEIGHT_DIP, SECTION_GAP_DIP, SECTION_LABEL_HEIGHT_DIP, SettingsControl,
    SettingsControlLayout, SettingsLayout, SettingsPage, SettingsRect, SettingsScene,
    SettingsSection, SettingsSectionLayout, SettingsSlider, SettingsToggle, WIDTH_DIP,
    u32_index,
};

impl SettingsScene {
    pub fn layout(&self) -> SettingsLayout {
        if let Some(step) = self.onboarding.step() {
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
        let mut sections = Vec::new();
        for (index, page) in SettingsPage::ALL
            .into_iter()
            .filter(|page| *page != SettingsPage::About)
            .enumerate()
        {
            let module_offset = if index >= 3 {
                16
            } else {
                0
            };
            controls.push(SettingsControlLayout {
                control: SettingsControl::Navigate(page),
                bounds: self.rect(
                    NAV_LEFT_DIP,
                    76 + u32_index(index) * 50 + module_offset,
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
        let content = self.page_content_positions();
        let content_height = content.height;
        self.append_page_content(&mut controls, &mut sections, content, content_width);
        if self.page != SettingsPage::About {
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
        }
        controls.push(SettingsControlLayout {
            control: SettingsControl::Close,
            bounds: self.rect(
                NAV_LEFT_DIP + NAV_WIDTH_DIP - CLOSE_SIZE_DIP,
                20,
                CLOSE_SIZE_DIP,
                CLOSE_SIZE_DIP,
            ),
        });
        SettingsLayout {
            size,
            controls,
            sections,
            content_viewport,
            content_scroll_offset: self.scale(self.scroll_offset_dip),
            scrollbar_thumb: self.scrollbar_thumb_for(content_height),
        }
    }

    fn append_page_content(
        &self,
        controls: &mut Vec<SettingsControlLayout>,
        sections: &mut Vec<SettingsSectionLayout>,
        content: PageContentLayout,
        content_width: u32,
    ) {
        for (section, top) in content.sections {
            sections.push(SettingsSectionLayout {
                section,
                bounds: self.rect(
                    CONTENT_LEFT_DIP,
                    CONTENT_TOP_DIP.saturating_add(top),
                    content_width,
                    SECTION_LABEL_HEIGHT_DIP,
                ),
            });
        }
        for (control, relative_top) in content.controls {
            let top = match control {
                SettingsControl::RestartIntegration => {
                    HEIGHT_DIP
                        - FOOTER_HEIGHT_DIP
                        - 5 * ROW_HEIGHT_DIP
                        - 4 * ROW_GAP_DIP
                        - 22
                }
                SettingsControl::ReplaySetup => {
                    HEIGHT_DIP
                        - FOOTER_HEIGHT_DIP
                        - 4 * ROW_HEIGHT_DIP
                        - 3 * ROW_GAP_DIP
                        - 22
                }
                SettingsControl::ExportSettings => {
                    HEIGHT_DIP
                        - FOOTER_HEIGHT_DIP
                        - 3 * ROW_HEIGHT_DIP
                        - 2 * ROW_GAP_DIP
                        - 22
                }
                SettingsControl::ExportDiagnostics => {
                    HEIGHT_DIP - FOOTER_HEIGHT_DIP - 2 * ROW_HEIGHT_DIP - ROW_GAP_DIP - 22
                }
                SettingsControl::ResetLotus => {
                    HEIGHT_DIP - FOOTER_HEIGHT_DIP - ROW_HEIGHT_DIP - 22
                }
                _ => CONTENT_TOP_DIP.saturating_add(relative_top),
            };
            let bounds = self.page_control_bounds(control, top, content_width);
            controls.push(SettingsControlLayout { control, bounds });
        }
    }

    fn page_control_bounds(
        &self,
        control: SettingsControl,
        top: u32,
        content_width: u32,
    ) -> SettingsRect {
        match control {
            SettingsControl::ChooseApplicationIcon(_) => self.rect(
                CONTENT_LEFT_DIP + content_width - 116,
                top,
                108,
                ROW_HEIGHT_DIP,
            ),
            SettingsControl::ResetApplicationIcon(_) => self.rect(
                CONTENT_LEFT_DIP + content_width - 184,
                top,
                64,
                ROW_HEIGHT_DIP,
            ),
            _ => self.rect(CONTENT_LEFT_DIP, top, content_width, ROW_HEIGHT_DIP),
        }
    }

    pub(super) fn page_groups(&self) -> Vec<(SettingsSection, Vec<SettingsControl>)> {
        match self.page {
            SettingsPage::Appearance | SettingsPage::General | SettingsPage::About => {
                Vec::new()
            }
            SettingsPage::Apps => vec![],
            SettingsPage::Taskbar => vec![
                (
                    SettingsSection::Main,
                    vec![
                        SettingsControl::Toggle(SettingsToggle::ShowAppDock),
                        SettingsControl::Toggle(SettingsToggle::HideWhenFullscreen),
                        SettingsControl::Toggle(SettingsToggle::ShowOnAllMonitors),
                    ],
                ),
                (
                    SettingsSection::Appearance,
                    vec![
                        SettingsControl::Toggle(SettingsToggle::ShowDesktopButton),
                        SettingsControl::Toggle(SettingsToggle::ShowRunningIndicators),
                        SettingsControl::DockZone,
                        SettingsControl::Slider(SettingsSlider::IconSize),
                        SettingsControl::Slider(SettingsSlider::ItemSpacing),
                        SettingsControl::Slider(SettingsSlider::HorizontalPadding),
                        SettingsControl::Slider(SettingsSlider::VerticalPadding),
                        SettingsControl::Slider(SettingsSlider::BottomOffset),
                        SettingsControl::Slider(SettingsSlider::ScreenEdgeInset),
                    ],
                ),
                (
                    SettingsSection::Advanced,
                    vec![SettingsControl::Toggle(
                        SettingsToggle::ReplaceWindowsTaskbar,
                    )],
                ),
            ],
            SettingsPage::Status => vec![
                (
                    SettingsSection::Main,
                    vec![
                        SettingsControl::Toggle(SettingsToggle::ShowSystemStatus),
                        SettingsControl::Toggle(SettingsToggle::ShowMediaControls),
                    ],
                ),
                (
                    SettingsSection::Appearance,
                    vec![
                        SettingsControl::SystemStatusZone,
                        SettingsControl::MediaZone,
                        SettingsControl::Toggle(SettingsToggle::ShowMediaMetadata),
                        SettingsControl::Toggle(SettingsToggle::ShowVolumeStatus),
                        SettingsControl::Toggle(SettingsToggle::ShowHdrStatus),
                        SettingsControl::Toggle(SettingsToggle::ShowNetworkStatus),
                        SettingsControl::Toggle(SettingsToggle::ShowBackgroundAppsStatus),
                        SettingsControl::Toggle(SettingsToggle::ShowDateTimeStatus),
                        SettingsControl::Toggle(SettingsToggle::ShowDateInStatus),
                        SettingsControl::Toggle(SettingsToggle::Use24HourTime),
                    ],
                ),
            ],
            SettingsPage::Search => vec![
                (
                    SettingsSection::Main,
                    vec![
                        SettingsControl::Toggle(SettingsToggle::SearchEnabled),
                        SettingsControl::Toggle(SettingsToggle::SearchOpenWithWindowsKey),
                    ],
                ),
                (
                    SettingsSection::Advanced,
                    vec![SettingsControl::Slider(SettingsSlider::SearchResultLimit)],
                ),
            ],
        }
    }

    pub(super) fn page_controls(&self) -> Vec<SettingsControl> {
        match self.page {
            SettingsPage::Appearance => vec![
                SettingsControl::SurfacePreset,
                SettingsControl::AccentPreset,
                SettingsControl::ForegroundPreset,
                SettingsControl::Toggle(SettingsToggle::UseAcrylic),
                SettingsControl::Slider(SettingsSlider::BackgroundOpacity),
                SettingsControl::Slider(SettingsSlider::CornerRadius),
            ],
            SettingsPage::General => {
                let mut controls = vec![
                    SettingsControl::Toggle(SettingsToggle::StartWithWindows),
                    SettingsControl::Toggle(SettingsToggle::ShowUnpinnedRunningApps),
                    SettingsControl::Toggle(SettingsToggle::AltTabEnabled),
                    SettingsControl::NotificationBadgeStyle,
                    SettingsControl::UpdateChannel,
                    SettingsControl::ChooseMascotImage,
                ];
                if self.draft.has_mascot_image() {
                    controls.push(SettingsControl::ResetMascotImage);
                }
                controls
            }
            SettingsPage::Apps => {
                let mut controls = vec![SettingsControl::ApplicationSearch];
                for &index in self.filtered_application_indices() {
                    controls.push(SettingsControl::ApplicationRow(index));
                    if self.application_actions_visible(index) {
                        controls.push(SettingsControl::ChooseApplicationIcon(index));
                        if self
                            .applications()
                            .get(index)
                            .is_some_and(|application| application.customized)
                        {
                            controls.push(SettingsControl::ResetApplicationIcon(index));
                        }
                    }
                }
                controls
            }
            SettingsPage::About => vec![
                SettingsControl::RestartIntegration,
                SettingsControl::ReplaySetup,
                SettingsControl::ExportSettings,
                SettingsControl::ExportDiagnostics,
                SettingsControl::ResetLotus,
            ],
            SettingsPage::Taskbar | SettingsPage::Status | SettingsPage::Search => self
                .page_groups()
                .into_iter()
                .flat_map(|(_, controls)| controls)
                .collect(),
        }
    }

    pub(super) fn page_content_positions(&self) -> PageContentLayout {
        if self.page == SettingsPage::Apps {
            return self.application_content_positions();
        }

        let groups = self.page_groups();
        if groups.is_empty() {
            let controls = self
                .page_controls()
                .into_iter()
                .enumerate()
                .map(|(index, control)| {
                    (control, u32_index(index) * (ROW_HEIGHT_DIP + ROW_GAP_DIP))
                })
                .collect::<Vec<_>>();
            let height = controls.last().map_or(0, |(_, top)| top + ROW_HEIGHT_DIP);
            return PageContentLayout {
                sections: Vec::new(),
                controls,
                height,
            };
        }
        let mut sections = Vec::with_capacity(groups.len());
        let mut controls = Vec::new();
        let mut top = 0_u32;

        for (group_index, (section, group_controls)) in groups.into_iter().enumerate() {
            if group_index > 0 {
                top = top.saturating_add(SECTION_GAP_DIP);
            }
            sections.push((section, top));
            top = top.saturating_add(SECTION_LABEL_HEIGHT_DIP);

            let control_count = group_controls.len();
            for (control_index, control) in group_controls.into_iter().enumerate() {
                controls.push((control, top));
                top = top.saturating_add(ROW_HEIGHT_DIP);
                if control_index + 1 < control_count {
                    top = top.saturating_add(ROW_GAP_DIP);
                }
            }
        }

        PageContentLayout {
            sections,
            controls,
            height: top,
        }
    }

    fn application_content_positions(&self) -> PageContentLayout {
        let mut controls = vec![(SettingsControl::ApplicationSearch, 0)];
        let mut top = ROW_HEIGHT_DIP + ROW_GAP_DIP;

        for &index in self.filtered_application_indices() {
            controls.push((SettingsControl::ApplicationRow(index), top));
            if self.application_actions_visible(index) {
                controls.push((SettingsControl::ChooseApplicationIcon(index), top));
                if self
                    .applications()
                    .get(index)
                    .is_some_and(|application| application.customized)
                {
                    controls.push((SettingsControl::ResetApplicationIcon(index), top));
                }
            }
            top = top.saturating_add(ROW_HEIGHT_DIP + ROW_GAP_DIP);
        }

        PageContentLayout {
            sections: Vec::new(),
            controls,
            height: top.saturating_sub(ROW_GAP_DIP),
        }
    }

    pub(super) fn filtered_application_indices(&self) -> &[usize] {
        &self.filtered_application_indices
    }

    pub(super) const fn content_viewport_height_dip() -> u32 {
        HEIGHT_DIP - FOOTER_HEIGHT_DIP - CONTENT_BOTTOM_INSET_DIP - CONTENT_TOP_DIP
    }

    pub(super) fn content_height_dip(&self) -> u32 {
        if self.page == SettingsPage::About {
            return 0;
        }
        self.page_content_positions().height
    }

    pub(super) fn maximum_scroll_offset_dip(&self) -> u32 {
        self.content_height_dip()
            .saturating_sub(Self::content_viewport_height_dip())
    }

    fn scrollbar_thumb_for(&self, content_height: u32) -> Option<SettingsRect> {
        let viewport_height = Self::content_viewport_height_dip();
        let maximum = content_height.saturating_sub(viewport_height);
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

    pub(super) fn onboarding_layout(&self, step: OnboardingStep) -> SettingsLayout {
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
        if !self.onboarding_required() {
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
            sections: Vec::new(),
            content_viewport: SettingsRect {
                left: 0,
                top: 0,
                width: size.width(),
                height: size.height(),
            },
            content_scroll_offset: 0,
            scrollbar_thumb: None,
        }
    }
}
