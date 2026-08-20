use std::num::NonZeroU32;

use lotus_core::settings::UpdateChannel;
use windows::Win32::Graphics::Direct2D::D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC;
use windows_numerics::Matrix3x2;

use super::{
    AccentPreset, ButtonEmphasis, Color, D2D_RECT_F, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
    DockZone, ForegroundPreset, NotificationBadgeStyle, SettingsControl, SettingsLayout,
    SettingsPage, SettingsRect, SettingsRenderer, SettingsScene, SettingsSlider,
    SettingsToggle, SurfacePreset, SvgAsset, as_f32, inset, inset_all, inset_rect,
    is_page_content, outset_rect, picker_bounds, rect, rounded, scale, scale_f32,
    slider_label, slider_value_text, theme, toggle_label,
};

impl SettingsRenderer {
    pub(super) fn draw_content(&self, scene: &SettingsScene, layout: &SettingsLayout) {
        let viewport = rect(layout.content_viewport);
        let mut previous_transform = Matrix3x2::default();
        let content_transform = Matrix3x2 {
            M11: 1.0,
            M22: 1.0,
            M32: -as_f32(layout.content_scroll_offset),
            ..Matrix3x2::default()
        };
        // PushAxisAlignedClip/PopAxisAlignedClip pair below.
        unsafe {
            self.context.GetTransform(&raw mut previous_transform);
            self.context.PushAxisAlignedClip(
                &raw const viewport,
                D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            );
            self.context.SetTransform(&raw const content_transform);
        }
        for entry in &layout.sections {
            self.draw_text(
                entry.section.title(),
                inset(entry.bounds, scale(scene, 16), 0),
                &self.small_format,
                &self.muted,
                false,
            );
        }
        self.draw_content_group(scene, layout);
        if scene.page() == SettingsPage::About {
            self.draw_about(scene);
        }
        for entry in layout
            .controls
            .iter()
            .filter(|entry| is_page_content(entry.control))
        {
            self.draw_settings_control(scene, entry.control, entry.bounds);
        }
        unsafe {
            self.context.SetTransform(&raw const previous_transform);
            self.context.PopAxisAlignedClip();
        }

        if let Some(thumb) = layout.scrollbar_thumb {
            let thumb = rounded(rect(thumb), scale_f32(scene, 1.5));
            unsafe {
                self.context
                    .FillRoundedRectangle(&raw const thumb, &self.muted);
            }
        }

        for entry in layout
            .controls
            .iter()
            .filter(|entry| !is_page_content(entry.control))
        {
            self.draw_settings_control(scene, entry.control, entry.bounds);
        }
    }

    pub(super) fn draw_settings_control(
        &self,
        scene: &SettingsScene,
        control: SettingsControl,
        bounds: SettingsRect,
    ) {
        match control {
            SettingsControl::SurfacePreset => self.draw_surface_picker(scene, bounds),
            SettingsControl::AccentPreset => self.draw_accent_picker(scene, bounds),
            SettingsControl::ForegroundPreset => {
                self.draw_foreground_picker(scene, bounds);
            }
            SettingsControl::NotificationBadgeStyle => {
                self.draw_notification_badge_style(scene, bounds);
            }
            SettingsControl::UpdateChannel => self.draw_update_channel(scene, bounds),
            SettingsControl::DockZone => self.draw_zone_picker(scene, bounds, false),
            SettingsControl::SystemStatusZone => self.draw_zone_picker(scene, bounds, true),
            SettingsControl::MediaZone => self.draw_media_zone_picker(scene, bounds),
            SettingsControl::Toggle(toggle) => self.draw_toggle(scene, bounds, toggle),
            SettingsControl::Slider(slider) => self.draw_slider(scene, bounds, slider),
            SettingsControl::ChooseMascotImage => self.draw_mascot_image(scene, bounds),
            SettingsControl::ResetMascotImage => self.draw_reset_mascot(scene, bounds),
            SettingsControl::ApplicationSearch => {
                self.draw_application_search(scene, bounds);
            }
            SettingsControl::ApplicationRow(index) => {
                self.draw_application_row(scene, index, bounds);
            }
            SettingsControl::ChooseApplicationIcon(index) => {
                self.draw_application_icon_action(scene, index, bounds, false);
            }
            SettingsControl::ResetApplicationIcon(index) => {
                self.draw_application_icon_action(scene, index, bounds, true);
            }
            SettingsControl::CheckForUpdates => self.draw_check_for_updates(scene, bounds),
            SettingsControl::ReplaySetup => self.draw_button(
                scene,
                bounds,
                SettingsControl::ReplaySetup,
                "Run first setup again",
                true,
                ButtonEmphasis::Secondary,
            ),
            SettingsControl::Revert => self.draw_revert(scene, bounds),
            SettingsControl::Apply => self.draw_apply(scene, bounds),
            SettingsControl::Close => self.draw_close(scene, bounds),
            SettingsControl::Navigate(_)
            | SettingsControl::OnboardingModule(_)
            | SettingsControl::OnboardingZone(_)
            | SettingsControl::OnboardingBack
            | SettingsControl::OnboardingNext
            | SettingsControl::OnboardingFinish => {}
        }
    }

    pub(super) fn draw_content_group(
        &self,
        scene: &SettingsScene,
        layout: &SettingsLayout,
    ) {
        let controls: Vec<_> = layout
            .controls
            .iter()
            .filter(|entry| {
                matches!(
                    entry.control,
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
            })
            .collect();
        if controls.is_empty() {
            return;
        }
        for pair in controls.windows(2) {
            let [entry, next] = pair else {
                continue;
            };
            let gap = next
                .bounds
                .top
                .saturating_sub(entry.bounds.top.saturating_add(entry.bounds.height));
            if gap > scale(scene, 8) {
                continue;
            }
            let y = entry
                .bounds
                .top
                .saturating_add(entry.bounds.height)
                .saturating_add(scale(scene, 2));
            let divider = D2D_RECT_F {
                left: as_f32(entry.bounds.left.saturating_add(scale(scene, 16))),
                top: as_f32(y),
                right: as_f32(
                    entry
                        .bounds
                        .left
                        .saturating_add(entry.bounds.width)
                        .saturating_sub(scale(scene, 16)),
                ),
                bottom: as_f32(y.saturating_add(scale(scene, 1))),
            };
            unsafe {
                self.context
                    .FillRectangle(&raw const divider, &self.divider);
            };
        }
    }

    pub(super) fn draw_surface_picker(&self, scene: &SettingsScene, bounds: SettingsRect) {
        self.draw_text(
            "Surface",
            inset(bounds, scale(scene, 16), 0),
            &self.body_format,
            &self.text,
            false,
        );
        let picker = picker_bounds(scene, bounds);
        let segment_width = picker.width / 4;
        let selected = SurfacePreset::selected(scene.draft());
        for index in 0_u32..4 {
            let segment = SettingsRect {
                left: picker
                    .left
                    .saturating_add(index.saturating_mul(segment_width)),
                top: picker.top,
                width: if index == 3 {
                    picker.width.saturating_sub(segment_width.saturating_mul(3))
                } else {
                    segment_width
                },
                height: picker.height,
            };
            let preset = usize::try_from(index)
                .ok()
                .and_then(|index| SurfacePreset::ALL.get(index));
            let color = preset.map_or_else(
                || {
                    Color::from_hex(&scene.draft().background_color)
                        .unwrap_or(scene.theme().canvas)
                },
                |preset| Color::from_hex(preset.color()).unwrap_or(scene.theme().canvas),
            );
            let label = preset.map_or("Custom", |preset| preset.name());
            let is_selected =
                preset.map_or(selected.is_none(), |preset| selected == Some(*preset));
            let surface = rounded(
                rect(inset_all(segment, scale(scene, 2))),
                scale_f32(scene, scene.theme().radii.compact),
            );
            theme::set(&self.row, color);
            unsafe {
                self.context
                    .FillRoundedRectangle(&raw const surface, &self.row);
            };
            if is_selected {
                let outline = rounded(
                    inset_rect(surface.rect, scale_f32(scene, 0.5)),
                    scale_f32(scene, (scene.theme().radii.compact - 0.5).max(1.0)),
                );
                unsafe {
                    self.context.DrawRoundedRectangle(
                        &raw const outline,
                        &self.accent,
                        scale_f32(scene, 1.5),
                        None,
                    );
                }
            }
            self.draw_text(label, segment, &self.small_format, &self.text, true);
        }
        theme::set(&self.row, scene.theme().control);
        self.draw_focus(scene, SettingsControl::SurfacePreset, bounds);
    }

    fn draw_application_row(
        &self,
        scene: &SettingsScene,
        index: usize,
        bounds: SettingsRect,
    ) {
        let Some(application) = scene.applications().get(index) else {
            return;
        };
        let interactive = scene.application_actions_visible(index);
        if interactive {
            let surface = rounded(
                rect(inset_all(bounds, scale(scene, 2))),
                scale_f32(scene, scene.theme().radii.compact),
            );
            unsafe {
                self.context
                    .FillRoundedRectangle(&raw const surface, &self.row);
            }
        }
        if let Some(icon) = application
            .icon
            .as_ref()
            .and_then(|icon| self.raster_bitmap(icon))
        {
            let size = scale(scene, 28);
            let destination = rect(SettingsRect {
                left: bounds.left.saturating_add(scale(scene, 12)),
                top: bounds
                    .top
                    .saturating_add(bounds.height.saturating_sub(size) / 2),
                width: size,
                height: size,
            });
            unsafe {
                self.context.DrawBitmap(
                    icon,
                    Some(&raw const destination),
                    1.0,
                    D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                    None,
                    None,
                );
            }
        }
        self.draw_text(
            &application.name,
            SettingsRect {
                left: bounds.left.saturating_add(scale(scene, 52)),
                top: bounds.top,
                width: bounds.width.saturating_sub(scale(scene, 196)),
                height: bounds.height,
            },
            &self.body_format,
            &self.text,
            false,
        );
        if application.customized && !interactive {
            self.draw_text(
                if application.missing_icon {
                    "missing"
                } else {
                    "custom"
                },
                SettingsRect {
                    left: bounds
                        .left
                        .saturating_add(bounds.width.saturating_sub(scale(scene, 92))),
                    top: bounds.top,
                    width: scale(scene, 76),
                    height: bounds.height,
                },
                &self.small_format,
                &self.muted,
                true,
            );
        }
        if !interactive {
            let divider = D2D_RECT_F {
                left: as_f32(bounds.left.saturating_add(scale(scene, 52))),
                top: as_f32(
                    bounds
                        .top
                        .saturating_add(bounds.height)
                        .saturating_sub(scale(scene, 1)),
                ),
                right: as_f32(
                    bounds
                        .left
                        .saturating_add(bounds.width)
                        .saturating_sub(scale(scene, 12)),
                ),
                bottom: as_f32(bounds.top.saturating_add(bounds.height)),
            };
            unsafe {
                self.context
                    .FillRectangle(&raw const divider, &self.divider);
            }
        }
        self.draw_focus(scene, SettingsControl::ApplicationRow(index), bounds);
    }

    fn draw_application_icon_action(
        &self,
        scene: &SettingsScene,
        index: usize,
        bounds: SettingsRect,
        reset: bool,
    ) {
        let control = if reset {
            SettingsControl::ResetApplicationIcon(index)
        } else {
            SettingsControl::ChooseApplicationIcon(index)
        };
        let label = if reset {
            "Reset"
        } else if scene
            .applications()
            .get(index)
            .is_some_and(|application| application.customized)
        {
            "Change image"
        } else {
            "Choose image"
        };
        let brush = if scene.hovered() == Some(control) {
            &self.accent
        } else {
            &self.muted
        };
        self.draw_text(label, bounds, &self.small_format, brush, true);
        self.draw_focus(scene, control, bounds);
    }

    fn draw_application_search(&self, scene: &SettingsScene, bounds: SettingsRect) {
        let surface = rounded(
            rect(inset_all(bounds, scale(scene, 2))),
            scale_f32(scene, scene.theme().radii.compact),
        );
        unsafe {
            self.context
                .FillRoundedRectangle(&raw const surface, &self.row);
        }
        let size = scale(scene, 18);
        let icon_size = NonZeroU32::new(size).expect("the scaled search icon is nonzero");
        if let Ok(icon) = self.embedded_bitmap(SvgAsset::FluentSearch, icon_size) {
            let destination = rect(SettingsRect {
                left: bounds.left.saturating_add(scale(scene, 16)),
                top: bounds
                    .top
                    .saturating_add(bounds.height.saturating_sub(size) / 2),
                width: size,
                height: size,
            });
            unsafe {
                self.context.DrawBitmap(
                    icon,
                    Some(&raw const destination),
                    1.0,
                    D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                    None,
                    None,
                );
            }
        }
        let query = scene.application_query();
        self.draw_text(
            if query.is_empty() {
                "Search applications"
            } else {
                query
            },
            SettingsRect {
                left: bounds.left.saturating_add(scale(scene, 48)),
                top: bounds.top,
                width: bounds.width.saturating_sub(scale(scene, 64)),
                height: bounds.height,
            },
            &self.body_format,
            if query.is_empty() {
                &self.muted
            } else {
                &self.text
            },
            false,
        );
        self.draw_focus(scene, SettingsControl::ApplicationSearch, bounds);
    }

    pub(super) fn draw_accent_picker(&self, scene: &SettingsScene, bounds: SettingsRect) {
        self.draw_text(
            "Accent",
            inset(bounds, scale(scene, 16), 0),
            &self.body_format,
            &self.text,
            false,
        );
        let picker = picker_bounds(scene, bounds);
        let segment_width = picker.width / 6;
        let selected = AccentPreset::selected(scene.draft());
        for index in 0_u32..6 {
            let diameter = scale(scene, 18);
            let left = picker
                .left
                .saturating_add(index.saturating_mul(segment_width))
                .saturating_add(segment_width.saturating_sub(diameter) / 2);
            let swatch_bounds = SettingsRect {
                left,
                top: bounds
                    .top
                    .saturating_add(bounds.height.saturating_sub(diameter) / 2),
                width: diameter,
                height: diameter,
            };
            let swatch = rounded(rect(swatch_bounds), as_f32(diameter) * 0.5);
            let preset = usize::try_from(index)
                .ok()
                .and_then(|index| AccentPreset::ALL.get(index));
            let color = preset.map_or_else(
                || {
                    Color::from_hex(&scene.draft().accent_color)
                        .unwrap_or(scene.theme().accent)
                },
                |preset| Color::from_hex(preset.color()).unwrap_or(scene.theme().accent),
            );
            theme::set(&self.row, color);
            unsafe {
                self.context
                    .FillRoundedRectangle(&raw const swatch, &self.row);
            };
            let is_selected =
                preset.map_or(selected.is_none(), |preset| selected == Some(*preset));
            if is_selected {
                let outline = rounded(
                    outset_rect(swatch.rect, scale_f32(scene, 3.0)),
                    as_f32(diameter) * 0.5 + scale_f32(scene, 3.0),
                );
                unsafe {
                    self.context.DrawRoundedRectangle(
                        &raw const outline,
                        &self.text,
                        scale_f32(scene, 1.0),
                        None,
                    );
                }
            }
            if preset.is_none() {
                self.draw_text(
                    "+",
                    swatch_bounds,
                    &self.small_format,
                    &self.accent_dark,
                    true,
                );
            }
        }
        theme::set(&self.row, scene.theme().control);
        self.draw_focus(scene, SettingsControl::AccentPreset, bounds);
    }

    pub(super) fn draw_foreground_picker(
        &self,
        scene: &SettingsScene,
        bounds: SettingsRect,
    ) {
        self.draw_text(
            "Text & icons",
            inset(bounds, scale(scene, 16), 0),
            &self.body_format,
            &self.text,
            false,
        );
        let picker = picker_bounds(scene, bounds);
        let segment_width = picker.width / 3;
        let selected = ForegroundPreset::selected(scene.draft());
        for index in 0_u32..3 {
            let diameter = scale(scene, 18);
            let left = picker
                .left
                .saturating_add(index.saturating_mul(segment_width))
                .saturating_add(segment_width.saturating_sub(diameter) / 2);
            let swatch_bounds = SettingsRect {
                left,
                top: bounds
                    .top
                    .saturating_add(bounds.height.saturating_sub(diameter) / 2),
                width: diameter,
                height: diameter,
            };
            let swatch = rounded(rect(swatch_bounds), as_f32(diameter) * 0.5);
            let preset = usize::try_from(index)
                .ok()
                .and_then(|index| ForegroundPreset::ALL.get(index));
            let color = preset.map_or_else(
                || {
                    Color::from_hex(&scene.draft().foreground_color)
                        .unwrap_or(scene.theme().text)
                },
                |preset| Color::from_hex(preset.color()).unwrap_or(scene.theme().text),
            );
            theme::set(&self.row, color);
            unsafe {
                self.context
                    .FillRoundedRectangle(&raw const swatch, &self.row);
            };
            let is_selected =
                preset.map_or(selected.is_none(), |preset| selected == Some(*preset));
            if is_selected {
                let outline = rounded(
                    outset_rect(swatch.rect, scale_f32(scene, 3.0)),
                    as_f32(diameter) * 0.5 + scale_f32(scene, 3.0),
                );
                unsafe {
                    self.context.DrawRoundedRectangle(
                        &raw const outline,
                        &self.accent,
                        scale_f32(scene, 1.0),
                        None,
                    );
                }
            }
            if preset.is_none() {
                let contrast = if color.relative_luminance() > 0.5 {
                    Color::rgb(0x18, 0x1A, 0x20)
                } else {
                    Color::rgb(0xF7, 0xF8, 0xFB)
                };
                theme::set(&self.disabled, contrast);
                self.draw_text(
                    "+",
                    swatch_bounds,
                    &self.small_format,
                    &self.disabled,
                    true,
                );
                theme::set(&self.disabled, scene.theme().text_disabled);
            }
        }
        theme::set(&self.row, scene.theme().control);
        self.draw_focus(scene, SettingsControl::ForegroundPreset, bounds);
    }

    pub(super) fn draw_notification_badge_style(
        &self,
        scene: &SettingsScene,
        bounds: SettingsRect,
    ) {
        let selected = scene.draft().notification_badge_style;
        self.draw_text_segments(
            scene,
            bounds,
            SettingsControl::NotificationBadgeStyle,
            "Notification badges",
            &[
                ("Off", selected == NotificationBadgeStyle::Off),
                ("Dot", selected == NotificationBadgeStyle::Dot),
                ("Number", selected == NotificationBadgeStyle::Count),
            ],
        );
    }

    pub(super) fn draw_update_channel(&self, scene: &SettingsScene, bounds: SettingsRect) {
        let selected = scene.draft().update_channel;
        self.draw_text_segments(
            scene,
            bounds,
            SettingsControl::UpdateChannel,
            "Update channel",
            &[
                ("Stable", selected == UpdateChannel::Stable),
                ("Alpha", selected == UpdateChannel::Alpha),
            ],
        );
    }

    pub(super) fn draw_zone_picker(
        &self,
        scene: &SettingsScene,
        bounds: SettingsRect,
        status: bool,
    ) {
        let selected = if status {
            scene.draft().system_status_zone
        } else {
            scene.draft().dock_zone
        };
        self.draw_text_segments(
            scene,
            bounds,
            if status {
                SettingsControl::SystemStatusZone
            } else {
                SettingsControl::DockZone
            },
            if status {
                "System status position"
            } else {
                "Main dock position"
            },
            &[
                ("Left", selected == DockZone::Left),
                ("Centre", selected == DockZone::Center),
                ("Right", selected == DockZone::Right),
            ],
        );
    }

    pub(super) fn draw_media_zone_picker(
        &self,
        scene: &SettingsScene,
        bounds: SettingsRect,
    ) {
        let selected = scene.draft().media_zone;
        self.draw_text_segments(
            scene,
            bounds,
            SettingsControl::MediaZone,
            "Media position",
            &[
                ("Left", selected == DockZone::Left),
                ("Centre", selected == DockZone::Center),
                ("Right", selected == DockZone::Right),
            ],
        );
    }

    pub(super) fn draw_text_segments(
        &self,
        scene: &SettingsScene,
        bounds: SettingsRect,
        control: SettingsControl,
        label: &str,
        options: &[(&str, bool)],
    ) {
        self.draw_text(
            label,
            inset(bounds, scale(scene, 16), 0),
            &self.body_format,
            &self.text,
            false,
        );
        let picker = picker_bounds(scene, bounds);
        let count = u32::try_from(options.len()).unwrap_or(1).max(1);
        let segment_width = picker.width / count;
        for (index, (label, selected)) in options.iter().enumerate() {
            let index = u32::try_from(index).unwrap_or_default();
            let segment = SettingsRect {
                left: picker
                    .left
                    .saturating_add(index.saturating_mul(segment_width)),
                top: picker.top,
                width: if index + 1 == count {
                    picker
                        .width
                        .saturating_sub(segment_width.saturating_mul(index))
                } else {
                    segment_width
                },
                height: picker.height,
            };
            let surface = rounded(
                rect(inset_all(segment, scale(scene, 2))),
                scale_f32(scene, scene.theme().radii.compact),
            );
            unsafe {
                self.context.FillRoundedRectangle(
                    &raw const surface,
                    if *selected {
                        &self.selected
                    } else {
                        &self.row
                    },
                );
                if *selected {
                    self.context.DrawRoundedRectangle(
                        &raw const surface,
                        &self.track,
                        scale_f32(scene, 1.0),
                        None,
                    );
                }
            }
            self.draw_text(label, segment, &self.small_format, &self.text, true);
        }
        self.draw_focus(scene, control, bounds);
    }

    pub(super) fn draw_toggle(
        &self,
        scene: &SettingsScene,
        bounds: SettingsRect,
        toggle: SettingsToggle,
    ) {
        let label_format = if scene.onboarding_step().is_some() {
            &self.onboarding_body_format
        } else {
            &self.body_format
        };
        self.draw_text(
            toggle_label(toggle),
            inset(bounds, scale(scene, 16), 0),
            label_format,
            &self.text,
            false,
        );
        let switch = SettingsRect {
            left: bounds.left + bounds.width - scale(scene, 58),
            top: bounds.top + scale(scene, 11),
            width: scale(scene, 42),
            height: scale(scene, 24),
        };
        let switch_rect = rounded(rect(switch), as_f32(switch.height) * 0.5);
        let on = scene.toggle(toggle);
        let knob_size = scale(scene, 18);
        let knob = SettingsRect {
            left: if on {
                switch.left + switch.width - knob_size - scale(scene, 3)
            } else {
                switch.left + scale(scene, 3)
            },
            top: switch.top + scale(scene, 3),
            width: knob_size,
            height: knob_size,
        };
        let knob_rect = rounded(rect(knob), as_f32(knob_size) * 0.5);
        unsafe {
            self.context.FillRoundedRectangle(
                &raw const switch_rect,
                if on {
                    &self.accent
                } else {
                    &self.track
                },
            );
            self.context.FillRoundedRectangle(
                &raw const knob_rect,
                if on {
                    &self.accent_dark
                } else {
                    &self.text
                },
            );
        }
    }

    pub(super) fn draw_slider(
        &self,
        scene: &SettingsScene,
        bounds: SettingsRect,
        slider: SettingsSlider,
    ) {
        self.draw_text(
            slider_label(slider),
            inset(bounds, scale(scene, 16), 0),
            &self.body_format,
            &self.text,
            false,
        );
        let (track_left, track_width) = scene.slider_track(bounds);
        let track = SettingsRect {
            left: track_left,
            top: bounds.top + scale(scene, 21),
            width: track_width,
            height: scale(scene, 4),
        };
        let (minimum, maximum) = slider.range();
        let value = scene.slider_value(slider);
        let filled = track.width.saturating_mul(value - minimum) / (maximum - minimum);
        let fill = SettingsRect {
            width: filled,
            ..track
        };
        let knob = SettingsRect {
            left: track.left + filled.saturating_sub(scale(scene, 7)),
            top: track.top.saturating_sub(scale(scene, 5)),
            width: scale(scene, 14),
            height: scale(scene, 14),
        };
        let track_round = rounded(rect(track), as_f32(track.height) * 0.5);
        let fill_round = rounded(rect(fill), as_f32(fill.height) * 0.5);
        let knob_round = rounded(rect(knob), as_f32(knob.height) * 0.5);
        unsafe {
            self.context
                .FillRoundedRectangle(&raw const track_round, &self.track);
            self.context
                .FillRoundedRectangle(&raw const fill_round, &self.accent);
            self.context
                .FillRoundedRectangle(&raw const knob_round, &self.accent);
        }
        let value_bounds = scene.slider_value_bounds(bounds);
        let value_surface = rounded(
            rect(value_bounds),
            scale_f32(scene, scene.theme().radii.compact),
        );
        unsafe {
            self.context
                .FillRoundedRectangle(&raw const value_surface, &self.row);
            self.context.DrawRoundedRectangle(
                &raw const value_surface,
                &self.divider,
                scale_f32(scene, 1.0),
                None,
            );
        }
        let value_text = slider_value_text(scene, slider);
        self.draw_text(
            &value_text,
            value_bounds,
            &self.small_format,
            &self.muted,
            true,
        );
    }
}
