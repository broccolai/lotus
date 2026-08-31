use lotus_core::settings::{DockZone, NotificationBadgeStyle, UpdateChannel};
use lotus_ui::icon::Icon;
use lotus_ui::presentation::{
    FontFamily, FontWeight, HorizontalAlignment, ImageSampling, Presentation,
    PresentationPrimitive, PresentationRect, TextStyle, VerticalAlignment,
};
use lotus_ui::theme::Color;

use super::{
    OnboardingModule, OnboardingStep, SettingsControl, SettingsLayout, SettingsPage,
    SettingsRect, SettingsScene, SettingsSlider, SettingsToggle, SettingsUpdateActivity,
    UPDATE_PROMPT_HEIGHT_DIP, UPDATE_PROMPT_INSET_DIP, UPDATE_PROMPT_WIDTH_DIP, WIDTH_DIP,
    is_page_content,
};
use crate::appearance::{AccentPreset, ForegroundPreset, SurfacePreset};

#[derive(Clone)]
pub struct SettingsAssets<Asset> {
    pub lotus: Asset,
    pub search: Asset,
}

impl SettingsScene {
    pub fn presentation<Asset: Clone>(
        &self,
        assets: &SettingsAssets<Asset>,
        translucent: bool,
    ) -> Presentation<Asset> {
        let layout = self.layout();
        let theme = self.theme();
        let palette = SettingsPalette::new(&theme, translucent);
        let mut output = Presentation::new(theme.canvas.with_alpha(0.0));

        output.push(fill(
            rect(0, 0, layout.size.width(), layout.size.height()),
            0.0,
            palette.panel,
        ));
        if let Some(step) = self.onboarding_step() {
            self.present_onboarding(&mut output, &layout, step, assets, palette);
        } else {
            output.push(fill(
                rect(0, 0, scale(self, 209), layout.size.height()),
                0.0,
                palette.sidebar,
            ));
            self.present_navigation(&mut output, &layout, palette);
            self.present_content(&mut output, &layout, assets);
            self.present_footer(&mut output, &layout);
        }
        if self.update_prompt().is_some() {
            self.present_update_prompt(&mut output, &layout);
        }
        output
    }

    fn present_update_prompt<Asset>(
        &self,
        output: &mut Presentation<Asset>,
        layout: &SettingsLayout,
    ) {
        let Some(prompt) = self.update_prompt() else {
            return;
        };
        let theme = self.theme();
        let card_left = scale(self, (WIDTH_DIP - UPDATE_PROMPT_WIDTH_DIP) / 2);
        let card_top = scale(self, (super::HEIGHT_DIP - UPDATE_PROMPT_HEIGHT_DIP) / 2);
        let card_width = scale(self, UPDATE_PROMPT_WIDTH_DIP);
        let card_height = scale(self, UPDATE_PROMPT_HEIGHT_DIP);
        let inset = scale(self, UPDATE_PROMPT_INSET_DIP);

        output.push(fill(
            rect(0, 0, layout.size.width(), layout.size.height()),
            0.0,
            theme.canvas.with_alpha(0.72),
        ));
        output.push(fill(
            rect(card_left, card_top, card_width, card_height),
            scaled(self, theme.radii.panel),
            theme.elevated_surface,
        ));
        output.push(stroke(
            rect(card_left, card_top, card_width, card_height),
            scaled(self, theme.radii.panel),
            scaled(self, 1.0),
            theme.border_strong,
        ));
        output.push(text(
            if prompt.is_installed() {
                "Update Lotus"
            } else {
                "Install Lotus"
            },
            rect(
                card_left + inset,
                card_top + inset,
                card_width - inset * 2,
                scale(self, 30),
            ),
            title(self, false),
            theme.text,
        ));
        output.push(text(
            format!("Lotus {} is ready.", prompt.version()),
            rect(
                card_left + inset,
                card_top + scale(self, 72),
                card_width - inset * 2,
                scale(self, 26),
            ),
            body(self, false),
            theme.text,
        ));
        output.push(text(
            "Lotus will restart when installation is ready.",
            rect(
                card_left + inset,
                card_top + scale(self, 102),
                card_width - inset * 2,
                scale(self, 24),
            ),
            small(self, false),
            theme.text_muted,
        ));
        if let Some(bounds) = layout.bounds(SettingsControl::CancelUpdate) {
            self.present_button(
                output,
                bounds,
                SettingsControl::CancelUpdate,
                "Not now",
                true,
                false,
            );
        }
        if let Some(bounds) = layout.bounds(SettingsControl::AcceptUpdate) {
            self.present_button(
                output,
                bounds,
                SettingsControl::AcceptUpdate,
                if prompt.is_installed() {
                    "Update Lotus"
                } else {
                    "Install Lotus"
                },
                true,
                true,
            );
        }
    }

    fn present_navigation<Asset>(
        &self,
        output: &mut Presentation<Asset>,
        layout: &SettingsLayout,
        palette: SettingsPalette,
    ) {
        let theme = self.theme();
        output.push(text(
            "lotus",
            rect(
                scale(self, 34),
                scale(self, 18),
                scale(self, 160),
                scale(self, 44),
            ),
            brand_leading(self, 22.0),
            theme.text,
        ));
        for page in SettingsPage::ALL {
            let control = SettingsControl::Navigate(page);
            let Some(bounds) = layout.bounds(control) else {
                continue;
            };
            if self.page() == page {
                output.push(fill(
                    settings_rect(bounds),
                    scaled(self, theme.radii.control),
                    palette.sidebar_selected,
                ));
                output.push(fill(
                    rect(
                        bounds.left + scale(self, 3),
                        bounds.top + scale(self, 12),
                        scale(self, 3),
                        bounds.height.saturating_sub(scale(self, 24)),
                    ),
                    scaled(self, 1.5),
                    theme.text,
                ));
            }
            output.push(text(
                page.title(),
                if page == SettingsPage::About {
                    settings_rect(bounds)
                } else {
                    settings_rect(inset(bounds, scale(self, 20), 0))
                },
                body(self, page == SettingsPage::About),
                theme.text,
            ));
            self.present_focus(output, control, bounds);
        }
        if let (Some(apps), Some(taskbar)) = (
            layout.bounds(SettingsControl::Navigate(SettingsPage::Apps)),
            layout.bounds(SettingsControl::Navigate(SettingsPage::Taskbar)),
        ) {
            let top = apps
                .top
                .saturating_add(apps.height)
                .saturating_add(taskbar.top.saturating_sub(apps.top + apps.height) / 2);
            output.push(fill(
                rect(
                    apps.left + scale(self, 20),
                    top,
                    apps.width.saturating_sub(scale(self, 40)),
                    scale(self, 1),
                ),
                0.0,
                theme.divider,
            ));
        }
    }

    fn present_content<Asset: Clone>(
        &self,
        output: &mut Presentation<Asset>,
        layout: &SettingsLayout,
        assets: &SettingsAssets<Asset>,
    ) {
        output.push(PresentationPrimitive::PushClip {
            bounds: settings_rect(layout.content_viewport),
        });
        let translation = -as_f32(layout.content_scroll_offset);
        for section in layout
            .sections
            .iter()
            .filter(|section| layout.content_intersects_viewport(section.bounds))
        {
            let first = output.primitives.len();
            output.push(text(
                section.section.title(),
                settings_rect(inset(section.bounds, scale(self, 16), 0)),
                small(self, false),
                self.theme().text_muted,
            ));
            output.translate_y_from(first, translation);
        }
        let mut previous: Option<SettingsRect> = None;
        for entry in layout
            .controls
            .iter()
            .filter(|entry| is_page_content(entry.control))
        {
            let visible = layout.content_intersects_viewport(entry.bounds);
            let first = output.primitives.len();
            if grouped_control(entry.control) {
                if let Some(before) = previous
                    && visible
                    && entry
                        .bounds
                        .top
                        .saturating_sub(before.top.saturating_add(before.height))
                        <= scale(self, 8)
                {
                    output.push(fill(
                        rect(
                            entry.bounds.left + scale(self, 16),
                            entry.bounds.top.saturating_sub(scale(self, 2)),
                            entry.bounds.width.saturating_sub(scale(self, 32)),
                            scale(self, 1),
                        ),
                        0.0,
                        self.theme().divider,
                    ));
                }
                previous = Some(entry.bounds);
            }
            if !visible {
                continue;
            }
            self.present_control(output, entry.control, entry.bounds, assets);
            output.translate_y_from(first, translation);
        }
        if self.page() == SettingsPage::About {
            let first = output.primitives.len();
            self.present_about(output);
            output.translate_y_from(first, translation);
        }
        output.push(PresentationPrimitive::PopClip);
        if let Some(thumb) = layout.scrollbar_thumb {
            output.push(fill(
                settings_rect(thumb),
                scaled(self, 1.5),
                self.theme().text_muted,
            ));
        }
        for entry in layout
            .controls
            .iter()
            .filter(|entry| !is_page_content(entry.control))
        {
            self.present_control(output, entry.control, entry.bounds, assets);
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "the exhaustive settings-control projection is clearer as one dispatcher"
    )]
    fn present_control<Asset: Clone>(
        &self,
        output: &mut Presentation<Asset>,
        control: SettingsControl,
        bounds: SettingsRect,
        assets: &SettingsAssets<Asset>,
    ) {
        match control {
            SettingsControl::Toggle(toggle) => self.present_toggle(output, bounds, toggle),
            SettingsControl::Slider(slider) => self.present_slider(output, bounds, slider),
            SettingsControl::DockZone => self.present_segments(
                output,
                bounds,
                control,
                "Main dock position",
                zone_options(self.draft().dock_zone),
            ),
            SettingsControl::SystemStatusZone => self.present_segments(
                output,
                bounds,
                control,
                "System status position",
                zone_options(self.draft().system_status_zone),
            ),
            SettingsControl::MediaZone => self.present_segments(
                output,
                bounds,
                control,
                "Media position",
                zone_options(self.draft().media_zone),
            ),
            SettingsControl::NotificationBadgeStyle => self.present_segments(
                output,
                bounds,
                control,
                "Notification badges",
                vec![
                    (
                        "Off",
                        self.draft().notification_badge_style
                            == NotificationBadgeStyle::Off,
                    ),
                    (
                        "Dot",
                        self.draft().notification_badge_style
                            == NotificationBadgeStyle::Dot,
                    ),
                    (
                        "Number",
                        self.draft().notification_badge_style
                            == NotificationBadgeStyle::Count,
                    ),
                ],
            ),
            SettingsControl::UpdateChannel => self.present_segments(
                output,
                bounds,
                control,
                "Update channel",
                vec![
                    (
                        "Stable",
                        self.draft().update_channel == UpdateChannel::Stable,
                    ),
                    ("Alpha", self.draft().update_channel == UpdateChannel::Alpha),
                ],
            ),
            SettingsControl::SurfacePreset => self.present_surface_picker(output, bounds),
            SettingsControl::AccentPreset => self.present_accent_picker(output, bounds),
            SettingsControl::ForegroundPreset => {
                self.present_foreground_picker(output, bounds);
            }
            SettingsControl::ChooseMascotImage => self.present_row_action(
                output,
                bounds,
                "Dock image",
                if self.draft().mascot_image_path.is_some() {
                    "Change image"
                } else {
                    "Choose image"
                },
                control,
            ),
            SettingsControl::ResetMascotImage => {
                self.present_row_action(output, bounds, "Restore lotus icon", "", control);
            }
            SettingsControl::ApplicationSearch => {
                self.present_application_search(output, bounds, assets);
            }
            SettingsControl::ApplicationRow(index) => {
                self.present_application(output, bounds, index);
            }
            SettingsControl::ChooseApplicationIcon(index) => self.present_row_action(
                output,
                bounds,
                "",
                "Choose image",
                SettingsControl::ChooseApplicationIcon(index),
            ),
            SettingsControl::ResetApplicationIcon(index) => self.present_row_action(
                output,
                bounds,
                "",
                "Reset",
                SettingsControl::ResetApplicationIcon(index),
            ),
            SettingsControl::CheckForUpdates => self.present_button(
                output,
                bounds,
                control,
                update_label(self),
                self.update_activity() == SettingsUpdateActivity::Idle,
                false,
            ),
            SettingsControl::RestartIntegration => self.present_button(
                output,
                bounds,
                control,
                "Restart Lotus integration",
                true,
                false,
            ),
            SettingsControl::ReplaySetup => self.present_button(
                output,
                bounds,
                control,
                "Run first setup again",
                true,
                false,
            ),
            SettingsControl::ExportSettings => {
                self.present_button(
                    output,
                    bounds,
                    control,
                    "Export settings",
                    true,
                    false,
                );
            }
            SettingsControl::ExportDiagnostics => self.present_button(
                output,
                bounds,
                control,
                "Export diagnostics",
                true,
                false,
            ),
            SettingsControl::ResetLotus => self.present_button(
                output,
                bounds,
                control,
                "Reset Lotus safely",
                true,
                false,
            ),
            SettingsControl::Revert => self.present_button(
                output,
                bounds,
                control,
                "Revert",
                self.is_dirty(),
                false,
            ),
            SettingsControl::Apply => {
                self.present_button(
                    output,
                    bounds,
                    control,
                    "Apply",
                    self.is_dirty(),
                    true,
                );
            }
            SettingsControl::Close => output.push(text(
                "×",
                settings_rect(bounds),
                title(self, true),
                self.theme().text_muted,
            )),
            SettingsControl::Navigate(_)
            | SettingsControl::CancelUpdate
            | SettingsControl::AcceptUpdate
            | SettingsControl::OnboardingModule(_)
            | SettingsControl::OnboardingZone(_)
            | SettingsControl::OnboardingBack
            | SettingsControl::OnboardingNext
            | SettingsControl::OnboardingFinish => {}
        }
    }

    fn present_toggle<Asset>(
        &self,
        output: &mut Presentation<Asset>,
        bounds: SettingsRect,
        toggle: SettingsToggle,
    ) {
        let theme = self.theme();
        output.push(text(
            toggle_label(toggle),
            settings_rect(inset(bounds, scale(self, 16), 0)),
            body(self, false),
            theme.text,
        ));
        let switch = SettingsRect {
            left: bounds.left + bounds.width - scale(self, 58),
            top: bounds.top + scale(self, 11),
            width: scale(self, 42),
            height: scale(self, 24),
        };
        let on = self.toggle(toggle);
        output.push(fill(
            settings_rect(switch),
            as_f32(switch.height) * 0.5,
            if on {
                theme.accent
            } else {
                theme.border_strong
            },
        ));
        let knob = scale(self, 18);
        output.push(fill(
            rect(
                if on {
                    switch.left + switch.width - knob - scale(self, 3)
                } else {
                    switch.left + scale(self, 3)
                },
                switch.top + scale(self, 3),
                knob,
                knob,
            ),
            as_f32(knob) * 0.5,
            if on {
                theme.on_accent
            } else {
                theme.text
            },
        ));
        self.present_focus(output, SettingsControl::Toggle(toggle), bounds);
    }

    fn present_surface_picker<Asset>(
        &self,
        output: &mut Presentation<Asset>,
        bounds: SettingsRect,
    ) {
        output.push(text(
            "Surface",
            settings_rect(inset(bounds, scale(self, 16), 0)),
            body(self, false),
            self.theme().text,
        ));
        let picker = self.control_column(bounds);
        let width = picker.width / 4;
        let selected = SurfacePreset::selected(self.draft());
        for index in 0_u32..4 {
            let segment = SettingsRect {
                left: picker.left + index * width,
                top: picker.top,
                width: if index == 3 {
                    picker.width - width * 3
                } else {
                    width
                },
                height: picker.height,
            };
            let preset = usize::try_from(index)
                .ok()
                .and_then(|index| SurfacePreset::ALL.get(index));
            let color = preset.map_or_else(
                || {
                    Color::from_hex(&self.draft().background_color)
                        .unwrap_or(self.theme().canvas)
                },
                |preset| Color::from_hex(preset.color()).unwrap_or(self.theme().canvas),
            );
            let surface = inset_all(segment, scale(self, 2));
            output.push(fill(
                settings_rect(surface),
                scaled(self, self.theme().radii.compact),
                color,
            ));
            if preset.map_or(selected.is_none(), |preset| selected == Some(*preset)) {
                output.push(stroke(
                    settings_rect(surface),
                    scaled(self, self.theme().radii.compact),
                    scaled(self, 1.5),
                    self.theme().accent,
                ));
            }
            output.push(text(
                preset.map_or("Custom", |preset| preset.name()),
                settings_rect(segment),
                small(self, true),
                self.theme().text,
            ));
        }
        self.present_focus(output, SettingsControl::SurfacePreset, bounds);
    }

    fn present_accent_picker<Asset>(
        &self,
        output: &mut Presentation<Asset>,
        bounds: SettingsRect,
    ) {
        let selected = AccentPreset::selected(self.draft());
        let colors = AccentPreset::ALL
            .iter()
            .map(|preset| {
                (
                    Color::from_hex(preset.color()).unwrap_or(self.theme().accent),
                    selected == Some(*preset),
                )
            })
            .chain(std::iter::once((
                Color::from_hex(&self.draft().accent_color).unwrap_or(self.theme().accent),
                selected.is_none(),
            )))
            .collect();
        self.present_swatches(
            output,
            bounds,
            "Accent",
            SettingsControl::AccentPreset,
            colors,
        );
    }

    fn present_foreground_picker<Asset>(
        &self,
        output: &mut Presentation<Asset>,
        bounds: SettingsRect,
    ) {
        let selected = ForegroundPreset::selected(self.draft());
        let colors = ForegroundPreset::ALL
            .iter()
            .map(|preset| {
                (
                    Color::from_hex(preset.color()).unwrap_or(self.theme().text),
                    selected == Some(*preset),
                )
            })
            .chain(std::iter::once((
                Color::from_hex(&self.draft().foreground_color)
                    .unwrap_or(self.theme().text),
                selected.is_none(),
            )))
            .collect();
        self.present_swatches(
            output,
            bounds,
            "Text & icons",
            SettingsControl::ForegroundPreset,
            colors,
        );
    }

    fn present_swatches<Asset>(
        &self,
        output: &mut Presentation<Asset>,
        bounds: SettingsRect,
        label: &str,
        control: SettingsControl,
        colors: Vec<(Color, bool)>,
    ) {
        output.push(text(
            label,
            settings_rect(inset(bounds, scale(self, 16), 0)),
            body(self, false),
            self.theme().text,
        ));
        let picker = self.control_column(bounds);
        let count = u32::try_from(colors.len()).unwrap_or(1).max(1);
        let slot = picker.width / count;
        let diameter = scale(self, 18);
        for (index, (color, selected)) in colors.into_iter().enumerate() {
            let index = u32::try_from(index).unwrap_or_default();
            let swatch = SettingsRect {
                left: picker.left + index * slot + slot.saturating_sub(diameter) / 2,
                top: bounds.top + bounds.height.saturating_sub(diameter) / 2,
                width: diameter,
                height: diameter,
            };
            output.push(fill(settings_rect(swatch), as_f32(diameter) * 0.5, color));
            if selected {
                output.push(stroke(
                    outset(settings_rect(swatch), scaled(self, 3.0)),
                    as_f32(diameter) * 0.5 + scaled(self, 3.0),
                    scaled(self, 1.0),
                    self.theme().text,
                ));
            }
            if index + 1 == count {
                output.push(text(
                    "+",
                    settings_rect(swatch),
                    small(self, true),
                    if color.relative_luminance() > 0.5 {
                        Color::rgb(0x18, 0x1A, 0x20)
                    } else {
                        Color::rgb(0xF7, 0xF8, 0xFB)
                    },
                ));
            }
        }
        self.present_focus(output, control, bounds);
    }

    fn present_slider<Asset>(
        &self,
        output: &mut Presentation<Asset>,
        bounds: SettingsRect,
        slider: SettingsSlider,
    ) {
        let theme = self.theme();
        output.push(text(
            slider_label(slider),
            settings_rect(inset(bounds, scale(self, 16), 0)),
            body(self, false),
            theme.text,
        ));
        let (left, width) = self.slider_track(bounds);
        let track = SettingsRect {
            left,
            top: bounds.top + scale(self, 21),
            width,
            height: scale(self, 4),
        };
        let (minimum, maximum) = slider.range();
        let value = self.slider_value(slider);
        let filled = track.width.saturating_mul(value - minimum) / (maximum - minimum);
        output.push(fill(
            settings_rect(track),
            as_f32(track.height) * 0.5,
            theme.border_strong,
        ));
        output.push(fill(
            rect(track.left, track.top, filled, track.height),
            as_f32(track.height) * 0.5,
            theme.accent,
        ));
        output.push(fill(
            rect(
                track.left + filled.saturating_sub(scale(self, 7)),
                track.top.saturating_sub(scale(self, 5)),
                scale(self, 14),
                scale(self, 14),
            ),
            scaled(self, 7.0),
            theme.accent,
        ));
        let value_bounds = self.slider_value_bounds(bounds);
        output.push(fill(
            settings_rect(value_bounds),
            scaled(self, theme.radii.compact),
            theme.control,
        ));
        output.push(stroke(
            settings_rect(value_bounds),
            scaled(self, theme.radii.compact),
            scaled(self, 1.0),
            theme.divider,
        ));
        output.push(text(
            if slider == SettingsSlider::BackgroundOpacity {
                format!("{value}%")
            } else {
                value.to_string()
            },
            settings_rect(value_bounds),
            small(self, true),
            theme.text_muted,
        ));
    }

    fn present_segments<Asset>(
        &self,
        output: &mut Presentation<Asset>,
        bounds: SettingsRect,
        control: SettingsControl,
        label: &str,
        options: Vec<(&str, bool)>,
    ) {
        let theme = self.theme();
        output.push(text(
            label,
            settings_rect(inset(bounds, scale(self, 16), 0)),
            body(self, false),
            theme.text,
        ));
        let picker = self.control_column(bounds);
        let count = u32::try_from(options.len()).unwrap_or(1).max(1);
        let segment_width = picker.width / count;
        for (index, (label, selected)) in options.into_iter().enumerate() {
            let index = u32::try_from(index).unwrap_or_default();
            let segment = SettingsRect {
                left: picker.left + index * segment_width,
                top: picker.top,
                width: if index + 1 == count {
                    picker.width - segment_width * index
                } else {
                    segment_width
                },
                height: picker.height,
            };
            let surface = inset_all(segment, scale(self, 2));
            output.push(fill(
                settings_rect(surface),
                scaled(self, theme.radii.compact),
                if selected {
                    theme.control_selected
                } else {
                    theme.control
                },
            ));
            if selected {
                output.push(stroke(
                    settings_rect(surface),
                    scaled(self, theme.radii.compact),
                    scaled(self, 1.0),
                    theme.border_strong,
                ));
            }
            output.push(text(
                label,
                settings_rect(segment),
                small(self, true),
                theme.text,
            ));
        }
        self.present_focus(output, control, bounds);
    }

    fn present_row_action<Asset>(
        &self,
        output: &mut Presentation<Asset>,
        bounds: SettingsRect,
        label: &str,
        action: &str,
        control: SettingsControl,
    ) {
        if !label.is_empty() {
            output.push(text(
                label,
                settings_rect(inset(bounds, scale(self, 16), 0)),
                body(self, false),
                self.theme().text,
            ));
        }
        if !action.is_empty() {
            output.push(text(
                action,
                rect(
                    bounds.left + bounds.width.saturating_sub(scale(self, 142)),
                    bounds.top,
                    scale(self, 126),
                    bounds.height,
                ),
                small(self, true),
                self.theme().accent,
            ));
        }
        self.present_focus(output, control, bounds);
    }

    fn present_application_search<Asset: Clone>(
        &self,
        output: &mut Presentation<Asset>,
        bounds: SettingsRect,
        assets: &SettingsAssets<Asset>,
    ) {
        output.push(fill(
            settings_rect(bounds),
            scaled(self, self.theme().radii.control),
            self.theme().control,
        ));
        output.push(PresentationPrimitive::Icon {
            icon: Icon::Embedded(assets.search.clone()),
            bounds: rect(
                bounds.left + scale(self, 14),
                bounds.top + scale(self, 13),
                scale(self, 18),
                scale(self, 18),
            ),
            tint: self.theme().text_muted,
            opacity: 1.0,
            sampling: ImageSampling::Smooth,
            radius: 0.0,
        });
        output.push(text(
            if self.application_query().is_empty() {
                "Search applications"
            } else {
                self.application_query()
            },
            rect(
                bounds.left + scale(self, 42),
                bounds.top,
                bounds.width.saturating_sub(scale(self, 56)),
                bounds.height,
            ),
            body(self, false),
            if self.application_query().is_empty() {
                self.theme().text_muted
            } else {
                self.theme().text
            },
        ));
    }

    fn present_application<Asset>(
        &self,
        output: &mut Presentation<Asset>,
        bounds: SettingsRect,
        index: usize,
    ) {
        let Some(app) = self.applications().get(index) else {
            return;
        };
        if self.application_actions_visible(index) {
            output.push(fill(
                settings_rect(bounds),
                scaled(self, self.theme().radii.control),
                self.theme().control_hover,
            ));
        }
        if let Some(icon) = &app.icon {
            output.push(PresentationPrimitive::Icon {
                icon: Icon::Raster(icon.clone()),
                bounds: rect(
                    bounds.left + scale(self, 14),
                    bounds.top + scale(self, 10),
                    scale(self, 28),
                    scale(self, 28),
                ),
                tint: self.theme().text,
                opacity: 1.0,
                sampling: ImageSampling::Smooth,
                radius: scaled(self, 4.0),
            });
        }
        output.push(text(
            &app.name,
            rect(
                bounds.left + scale(self, 54),
                bounds.top,
                bounds.width.saturating_sub(scale(self, 180)),
                bounds.height,
            ),
            body(self, false),
            self.theme().text,
        ));
    }

    fn present_about<Asset>(&self, output: &mut Presentation<Asset>) {
        output.push(text(
            concat!("lotus ", env!("CARGO_PKG_VERSION")),
            rect(
                scale(self, 260),
                scale(self, 106),
                scale(self, 600),
                scale(self, 32),
            ),
            title(self, false),
            self.theme().text,
        ));
        output.push(text(
            "<3 broccoli",
            rect(
                scale(self, 260),
                scale(self, 148),
                scale(self, 600),
                scale(self, 32),
            ),
            body(self, false),
            self.theme().accent,
        ));
    }

    fn present_footer<Asset>(
        &self,
        output: &mut Presentation<Asset>,
        layout: &SettingsLayout,
    ) {
        if self.is_dirty() {
            output.push(text(
                "Unsaved changes",
                rect(
                    scale(self, 244),
                    layout.size.height().saturating_sub(scale(self, 72)),
                    scale(self, 240),
                    scale(self, 72),
                ),
                small(self, false),
                self.theme().text_muted,
            ));
        }
    }

    fn present_button<Asset>(
        &self,
        output: &mut Presentation<Asset>,
        bounds: SettingsRect,
        control: SettingsControl,
        label: &str,
        enabled: bool,
        primary: bool,
    ) {
        let hovered = enabled && self.hovered() == Some(control);
        let color = if primary && enabled {
            self.theme().accent
        } else if hovered {
            self.theme().control_hover
        } else {
            self.theme().control
        };
        output.push(fill(
            settings_rect(bounds),
            scaled(self, self.theme().radii.control),
            color,
        ));
        output.push(stroke(
            settings_rect(bounds),
            scaled(self, self.theme().radii.control),
            scaled(self, 1.0),
            if primary && enabled {
                self.theme().accent
            } else {
                self.theme().divider
            },
        ));
        output.push(text(
            label,
            settings_rect(bounds),
            button(self),
            if !enabled {
                self.theme().text_disabled
            } else if primary {
                self.theme().on_accent
            } else {
                self.theme().text
            },
        ));
        self.present_focus(output, control, bounds);
    }

    fn present_focus<Asset>(
        &self,
        output: &mut Presentation<Asset>,
        control: SettingsControl,
        bounds: SettingsRect,
    ) {
        if self.focus_visible() && self.focused() == Some(control) {
            output.push(stroke(
                settings_rect(inset_all(bounds, scale(self, 2))),
                scaled(self, self.theme().radii.compact),
                scaled(self, 1.0),
                self.theme().accent_soft,
            ));
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "onboarding is a finite five-step visual projection kept together"
    )]
    fn present_onboarding<Asset: Clone>(
        &self,
        output: &mut Presentation<Asset>,
        layout: &SettingsLayout,
        step: OnboardingStep,
        assets: &SettingsAssets<Asset>,
        palette: SettingsPalette,
    ) {
        let title_bounds = match step {
            OnboardingStep::Welcome => rect(
                scale(self, 72),
                scale(self, 220),
                scale(self, 756),
                scale(self, 130),
            ),
            OnboardingStep::Ready => rect(
                scale(self, 72),
                scale(self, 136),
                scale(self, 756),
                scale(self, 72),
            ),
            _ => rect(
                scale(self, 72),
                scale(self, 110),
                scale(self, 756),
                scale(self, 58),
            ),
        };
        output.push(text(
            onboarding_title(step),
            title_bounds,
            if step == OnboardingStep::Welcome {
                brand(self, 88.0)
            } else {
                brand_regular(self, 30.0)
            },
            if step == OnboardingStep::Welcome {
                self.theme().accent
            } else {
                self.theme().text
            },
        ));
        if step == OnboardingStep::Welcome {
            output.push(PresentationPrimitive::Icon {
                icon: Icon::Embedded(assets.lotus.clone()),
                bounds: rect(
                    scale(self, 418),
                    scale(self, 180),
                    scale(self, 64),
                    scale(self, 64),
                ),
                tint: self.theme().text,
                opacity: 1.0,
                sampling: ImageSampling::PixelAligned,
                radius: 0.0,
            });
        }
        if step != OnboardingStep::Welcome {
            let track = rect(
                scale(self, 390),
                scale(self, 28),
                scale(self, 120),
                scale(self, 3),
            );
            output.push(fill(track, scaled(self, 1.5), self.theme().divider));
            output.push(fill(
                PresentationRect::new(
                    track.left,
                    track.top,
                    track.left
                        + track.width()
                            * f32::from(u16::try_from(step.number()).unwrap_or(u16::MAX))
                            / 4.0,
                    track.bottom,
                ),
                scaled(self, 1.5),
                self.theme().accent,
            ));
        }
        if step == OnboardingStep::Ready {
            output.push(text(
                "you can change these choices and much more in lotus settings.",
                rect(
                    scale(self, 170),
                    scale(self, 226),
                    scale(self, 560),
                    scale(self, 28),
                ),
                body(self, true),
                self.theme().text,
            ));
            output.push(text(
                "right-click the lotus icon or search >settings.",
                rect(
                    scale(self, 210),
                    scale(self, 262),
                    scale(self, 480),
                    scale(self, 26),
                ),
                body(self, true),
                self.theme().text_muted,
            ));
        }
        for entry in &layout.controls {
            match entry.control {
                SettingsControl::OnboardingModule(module) => {
                    self.present_onboarding_module(output, entry.bounds, module, palette);
                }
                SettingsControl::OnboardingZone(module) => {
                    self.present_onboarding_zone(output, entry.bounds, module);
                }
                SettingsControl::Toggle(toggle) => {
                    self.present_toggle(output, entry.bounds, toggle);
                }
                SettingsControl::OnboardingBack => self.present_button(
                    output,
                    entry.bounds,
                    entry.control,
                    "back",
                    true,
                    false,
                ),
                SettingsControl::OnboardingNext => self.present_button(
                    output,
                    entry.bounds,
                    entry.control,
                    if step == OnboardingStep::Welcome {
                        "begin"
                    } else {
                        "continue"
                    },
                    true,
                    true,
                ),
                SettingsControl::OnboardingFinish => self.present_button(
                    output,
                    entry.bounds,
                    entry.control,
                    "start lotus",
                    true,
                    true,
                ),
                SettingsControl::Close => output.push(text(
                    "×",
                    settings_rect(entry.bounds),
                    title(self, true),
                    self.theme().text_muted,
                )),
                _ => {}
            }
        }
    }

    fn present_onboarding_module<Asset>(
        &self,
        output: &mut Presentation<Asset>,
        bounds: SettingsRect,
        module: OnboardingModule,
        palette: SettingsPalette,
    ) {
        let enabled = self.onboarding_module_enabled(module);
        output.push(fill(
            settings_rect(bounds),
            scaled(self, self.theme().radii.panel),
            if enabled {
                self.theme().control_selected
            } else if self.hovered() == Some(SettingsControl::OnboardingModule(module)) {
                self.theme().control_hover
            } else {
                palette.group
            },
        ));
        output.push(text(
            module.title(),
            rect(
                bounds.left + scale(self, 18),
                bounds.top + scale(self, 7),
                bounds.width - scale(self, 36),
                scale(self, 28),
            ),
            body(self, false),
            self.theme().text,
        ));
        output.push(text(
            module.description(),
            rect(
                bounds.left + scale(self, 18),
                bounds.top + scale(self, 34),
                bounds.width - scale(self, 36),
                scale(self, 24),
            ),
            small(self, false),
            self.theme().text_muted,
        ));
        self.present_focus(output, SettingsControl::OnboardingModule(module), bounds);
    }

    fn present_onboarding_zone<Asset>(
        &self,
        output: &mut Presentation<Asset>,
        bounds: SettingsRect,
        module: OnboardingModule,
    ) {
        output.push(text(
            module.title(),
            rect(
                bounds.left.saturating_sub(scale(self, 260)),
                bounds.top,
                scale(self, 230),
                bounds.height,
            ),
            body(self, false),
            self.theme().text,
        ));
        let selected = self.onboarding_zone(module);
        let width = bounds.width / 3;
        for (index, (label, zone)) in [
            ("LEFT", DockZone::Left),
            ("MIDDLE", DockZone::Center),
            ("RIGHT", DockZone::Right),
        ]
        .into_iter()
        .enumerate()
        {
            let index = u32::try_from(index).unwrap_or_default();
            let segment = SettingsRect {
                left: bounds.left + index * width,
                top: bounds.top,
                width: if index == 2 {
                    bounds.width - width * 2
                } else {
                    width
                },
                height: bounds.height,
            };
            if selected == zone {
                output.push(fill(
                    settings_rect(inset_all(segment, scale(self, 3))),
                    scaled(self, self.theme().radii.compact),
                    self.theme().control_selected,
                ));
            }
            output.push(text(
                label,
                settings_rect(segment),
                small(self, true),
                if selected == zone {
                    self.theme().accent
                } else {
                    self.theme().text_muted
                },
            ));
        }
        self.present_focus(output, SettingsControl::OnboardingZone(module), bounds);
    }
}

#[derive(Clone, Copy)]
struct SettingsPalette {
    panel: Color,
    sidebar: Color,
    sidebar_selected: Color,
    group: Color,
}

impl SettingsPalette {
    fn new(theme: &lotus_ui::theme::Theme, translucent: bool) -> Self {
        if translucent {
            Self {
                panel: theme.chrome_overlay,
                sidebar: theme.accent.with_alpha(0.38),
                sidebar_selected: theme.text.with_alpha(0.14),
                group: theme.control,
            }
        } else {
            Self {
                panel: theme.canvas,
                sidebar: theme.canvas.blend(theme.accent, 0.18),
                sidebar_selected: theme.text.with_alpha(0.14),
                group: theme.surface,
            }
        }
    }
}

fn update_label(scene: &SettingsScene) -> &'static str {
    match scene.update_activity() {
        SettingsUpdateActivity::Idle if scene.is_installed() => "Check for updates",
        SettingsUpdateActivity::Idle => "Install lotus",
        SettingsUpdateActivity::Checking => "Checking…",
        SettingsUpdateActivity::Installing => "Installing…",
    }
}
fn grouped_control(control: SettingsControl) -> bool {
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
    )
}
fn zone_options(selected: DockZone) -> Vec<(&'static str, bool)> {
    vec![
        ("Left", selected == DockZone::Left),
        ("Centre", selected == DockZone::Center),
        ("Right", selected == DockZone::Right),
    ]
}
fn toggle_label(value: SettingsToggle) -> &'static str {
    match value {
        SettingsToggle::UseAcrylic => "Acrylic",
        SettingsToggle::ShowAppDock => "Application dock",
        SettingsToggle::ShowUnpinnedRunningApps => "Show unpinned running applications",
        SettingsToggle::ShowRunningIndicators => "Show indicators for open applications",
        SettingsToggle::ShowOnAllMonitors => "Show lotus on every monitor",
        SettingsToggle::ShowDesktopButton => "Show a desktop button at the right edge",
        SettingsToggle::ShowSystemStatus => "Show system status",
        SettingsToggle::ShowVolumeStatus => "Show volume",
        SettingsToggle::ShowHdrStatus => "Show HDR toggle",
        SettingsToggle::ShowNetworkStatus => "Show network",
        SettingsToggle::ShowBackgroundAppsStatus => "Show background applications",
        SettingsToggle::ShowDateTimeStatus => "Show time",
        SettingsToggle::ShowDateInStatus => "Show date below the time",
        SettingsToggle::Use24HourTime => "Use 24-hour time",
        SettingsToggle::ShowMediaControls => "Show media controls",
        SettingsToggle::ShowMediaMetadata => "Show track information",
        SettingsToggle::StartWithWindows => "Start lotus when you sign in",
        SettingsToggle::ReplaceWindowsTaskbar => "Replace the Windows taskbar",
        SettingsToggle::HideWhenFullscreen => "Hide while an app is fullscreen",
        SettingsToggle::SearchEnabled => "Application search",
        SettingsToggle::SearchOpenWithWindowsKey => "Open search with the Windows key",
        SettingsToggle::AltTabEnabled => "Replace Alt+Tab with lotus",
    }
}
fn slider_label(value: SettingsSlider) -> &'static str {
    match value {
        SettingsSlider::IconSize => "Icon size",
        SettingsSlider::ItemSpacing => "Item spacing",
        SettingsSlider::HorizontalPadding => "Horizontal padding",
        SettingsSlider::VerticalPadding => "Vertical padding",
        SettingsSlider::BottomOffset => "Bottom offset",
        SettingsSlider::ScreenEdgeInset => "Screen edge inset",
        SettingsSlider::CornerRadius => "Corner radius",
        SettingsSlider::BackgroundOpacity => "Material opacity",
        SettingsSlider::SearchResultLimit => "Number of search results",
    }
}
fn onboarding_title(step: OnboardingStep) -> &'static str {
    match step {
        OnboardingStep::Welcome => "lotus",
        OnboardingStep::Modules => "choose what belongs",
        OnboardingStep::Layout => "place everything",
        OnboardingStep::Integration => "integrate with windows",
        OnboardingStep::Ready => "thank you!",
    }
}
fn text<Asset>(
    value: impl Into<String>,
    bounds: PresentationRect,
    style: TextStyle,
    color: Color,
) -> PresentationPrimitive<Asset> {
    PresentationPrimitive::Text {
        value: value.into(),
        bounds,
        style,
        color,
    }
}
fn fill<Asset>(
    bounds: PresentationRect,
    radius: f32,
    color: Color,
) -> PresentationPrimitive<Asset> {
    PresentationPrimitive::FillRoundedRect {
        bounds,
        radius,
        color,
    }
}
fn stroke<Asset>(
    bounds: PresentationRect,
    radius: f32,
    width: f32,
    color: Color,
) -> PresentationPrimitive<Asset> {
    PresentationPrimitive::StrokeRoundedRect {
        bounds,
        radius,
        width,
        color,
    }
}
fn style(size: f32, family: FontFamily, weight: FontWeight, centered: bool) -> TextStyle {
    TextStyle {
        size,
        family,
        weight,
        horizontal: if centered {
            HorizontalAlignment::Center
        } else {
            HorizontalAlignment::Leading
        },
        vertical: VerticalAlignment::Center,
    }
}
fn brand(scene: &SettingsScene, size: f32) -> TextStyle {
    style(
        size * scale_factor(scene),
        FontFamily::Brand,
        FontWeight::Semibold,
        true,
    )
}
fn brand_leading(scene: &SettingsScene, size: f32) -> TextStyle {
    style(
        size * scale_factor(scene),
        FontFamily::Brand,
        FontWeight::Semibold,
        false,
    )
}
fn brand_regular(scene: &SettingsScene, size: f32) -> TextStyle {
    style(
        size * scale_factor(scene),
        FontFamily::Brand,
        FontWeight::Normal,
        true,
    )
}
fn body(scene: &SettingsScene, centered: bool) -> TextStyle {
    style(
        14.0 * scale_factor(scene),
        FontFamily::Interface,
        FontWeight::Normal,
        centered,
    )
}
fn small(scene: &SettingsScene, centered: bool) -> TextStyle {
    style(
        12.5 * scale_factor(scene),
        FontFamily::Interface,
        FontWeight::Normal,
        centered,
    )
}
fn title(scene: &SettingsScene, centered: bool) -> TextStyle {
    style(
        18.0 * scale_factor(scene),
        FontFamily::Interface,
        FontWeight::Semibold,
        centered,
    )
}
fn button(scene: &SettingsScene) -> TextStyle {
    style(
        13.5 * scale_factor(scene),
        FontFamily::Interface,
        FontWeight::Semibold,
        true,
    )
}
fn rect(left: u32, top: u32, width: u32, height: u32) -> PresentationRect {
    PresentationRect::new(
        as_f32(left),
        as_f32(top),
        as_f32(left.saturating_add(width)),
        as_f32(top.saturating_add(height)),
    )
}
fn settings_rect(value: SettingsRect) -> PresentationRect {
    rect(value.left, value.top, value.width, value.height)
}
fn outset(mut value: PresentationRect, amount: f32) -> PresentationRect {
    value.left -= amount;
    value.top -= amount;
    value.right += amount;
    value.bottom += amount;
    value
}
fn inset(mut value: SettingsRect, horizontal: u32, vertical: u32) -> SettingsRect {
    value.left = value.left.saturating_add(horizontal);
    value.top = value.top.saturating_add(vertical);
    value.width = value.width.saturating_sub(horizontal.saturating_mul(2));
    value.height = value.height.saturating_sub(vertical.saturating_mul(2));
    value
}
fn inset_all(value: SettingsRect, amount: u32) -> SettingsRect {
    inset(value, amount, amount)
}
fn scale(scene: &SettingsScene, dips: u32) -> u32 {
    u32::try_from((u64::from(dips) * u64::from(scene.effective_dpi()) + 48) / 96)
        .unwrap_or(u32::MAX)
}
fn scaled(scene: &SettingsScene, dips: f32) -> f32 {
    dips * scale_factor(scene)
}
fn scale_factor(scene: &SettingsScene) -> f32 {
    f32::from(u16::try_from(scene.effective_dpi()).unwrap_or(u16::MAX)) / 96.0
}
#[allow(
    clippy::cast_precision_loss,
    reason = "settings dimensions remain below f32 exact range"
)]
fn as_f32(value: u32) -> f32 {
    value as f32
}
