use std::num::NonZeroU32;

use super::{
    ButtonEmphasis, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, DockZone, OnboardingModule,
    OnboardingStep, SettingsControl, SettingsLayout, SettingsRect, SettingsRenderer,
    SettingsRendererError, SettingsScene, SvgAsset, inset_all, onboarding_title, rect,
    rounded, scale, scale_f32,
};

impl SettingsRenderer {
    pub(super) fn draw_onboarding(
        &self,
        scene: &SettingsScene,
        layout: &SettingsLayout,
        step: OnboardingStep,
        welcome_icon_size: Option<NonZeroU32>,
    ) -> Result<(), SettingsRendererError> {
        let title = onboarding_title(step);
        let title_bounds = match step {
            OnboardingStep::Welcome => SettingsRect {
                left: scale(scene, 72),
                top: scale(scene, 220),
                width: scale(scene, 756),
                height: scale(scene, 130),
            },
            OnboardingStep::Ready => SettingsRect {
                left: scale(scene, 72),
                top: scale(scene, 136),
                width: scale(scene, 756),
                height: scale(scene, 72),
            },
            _ => SettingsRect {
                left: scale(scene, 72),
                top: scale(scene, 110),
                width: scale(scene, 756),
                height: scale(scene, 58),
            },
        };
        self.draw_text(
            title,
            title_bounds,
            if step == OnboardingStep::Welcome {
                &self.hero_format
            } else {
                &self.onboarding_format
            },
            if step == OnboardingStep::Welcome {
                &self.accent
            } else {
                &self.text
            },
            true,
        );
        if let Some(icon_size) = welcome_icon_size {
            self.draw_welcome_icon(scene, icon_size)?;
        }
        if step == OnboardingStep::Ready {
            self.draw_onboarding_ready(scene);
        }
        if step != OnboardingStep::Welcome {
            self.draw_onboarding_progress(scene, step);
        }

        for entry in &layout.controls {
            match entry.control {
                SettingsControl::OnboardingModule(module) => {
                    self.draw_onboarding_module(scene, entry.bounds, module);
                }
                SettingsControl::OnboardingZone(module) => {
                    self.draw_onboarding_zone(scene, entry.bounds, module);
                }
                SettingsControl::Toggle(toggle) => {
                    self.draw_toggle(scene, entry.bounds, toggle);
                }
                SettingsControl::OnboardingBack => self.draw_button(
                    scene,
                    entry.bounds,
                    entry.control,
                    "back",
                    true,
                    ButtonEmphasis::Outline,
                ),
                SettingsControl::OnboardingNext => self.draw_button(
                    scene,
                    entry.bounds,
                    entry.control,
                    if step == OnboardingStep::Welcome {
                        "begin"
                    } else {
                        "continue"
                    },
                    true,
                    ButtonEmphasis::Primary,
                ),
                SettingsControl::OnboardingFinish => self.draw_button(
                    scene,
                    entry.bounds,
                    entry.control,
                    "start lotus",
                    true,
                    ButtonEmphasis::Primary,
                ),
                SettingsControl::Close => self.draw_close(scene, entry.bounds),
                _ => {}
            }
        }
        Ok(())
    }

    pub(super) fn draw_welcome_icon(
        &self,
        scene: &SettingsScene,
        size: NonZeroU32,
    ) -> Result<(), SettingsRendererError> {
        let bitmap = self.embedded_bitmap(SvgAsset::LotusPixel, size)?;
        let destination = rect(SettingsRect {
            left: scale(scene, 418),
            top: scale(scene, 180),
            width: size.get(),
            height: size.get(),
        });
        unsafe {
            self.context.DrawBitmap(
                bitmap,
                Some(&raw const destination),
                1.0,
                D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                None,
                None,
            );
        }
        Ok(())
    }

    pub(super) fn draw_onboarding_progress(
        &self,
        scene: &SettingsScene,
        step: OnboardingStep,
    ) {
        let track_bounds = SettingsRect {
            left: scale(scene, 390),
            top: scale(scene, 28),
            width: scale(scene, 120),
            height: scale(scene, 3),
        };
        let progress_bounds = SettingsRect {
            width: track_bounds.width.saturating_mul(step.number()) / 4,
            ..track_bounds
        };
        let track = rounded(rect(track_bounds), scale_f32(scene, 1.5));
        let progress = rounded(rect(progress_bounds), scale_f32(scene, 1.5));
        unsafe {
            self.context
                .FillRoundedRectangle(&raw const track, &self.divider);
            self.context
                .FillRoundedRectangle(&raw const progress, &self.accent);
        }
    }

    pub(super) fn draw_onboarding_ready(&self, scene: &SettingsScene) {
        self.draw_text(
            "you can change these choices and much more in lotus settings.",
            SettingsRect {
                left: scale(scene, 170),
                top: scale(scene, 226),
                width: scale(scene, 560),
                height: scale(scene, 28),
            },
            &self.onboarding_body_format,
            &self.text,
            true,
        );
        self.draw_text(
            "right-click the lotus icon or search >settings.",
            SettingsRect {
                left: scale(scene, 210),
                top: scale(scene, 262),
                width: scale(scene, 480),
                height: scale(scene, 26),
            },
            &self.onboarding_body_format,
            &self.muted,
            true,
        );
    }

    pub(super) fn draw_onboarding_module(
        &self,
        scene: &SettingsScene,
        bounds: SettingsRect,
        module: OnboardingModule,
    ) {
        let enabled = scene.onboarding_module_enabled(module);
        let surface = rounded(rect(bounds), scale_f32(scene, scene.theme().radii.panel));
        let brush = if enabled {
            &self.selected
        } else if scene.hovered() == Some(SettingsControl::OnboardingModule(module)) {
            &self.hover
        } else {
            &self.group
        };
        unsafe {
            self.context.FillRoundedRectangle(&raw const surface, brush);
        }
        self.draw_text(
            module.title(),
            SettingsRect {
                left: bounds.left + scale(scene, 18),
                top: bounds.top + scale(scene, 7),
                width: bounds.width - scale(scene, 36),
                height: scale(scene, 28),
            },
            &self.onboarding_body_format,
            &self.text,
            false,
        );
        self.draw_text(
            module.description(),
            SettingsRect {
                left: bounds.left + scale(scene, 18),
                top: bounds.top + scale(scene, 34),
                width: bounds.width - scale(scene, 36),
                height: scale(scene, 24),
            },
            &self.onboarding_small_format,
            &self.muted,
            false,
        );
        self.draw_focus(scene, SettingsControl::OnboardingModule(module), bounds);
    }

    pub(super) fn draw_onboarding_zone(
        &self,
        scene: &SettingsScene,
        bounds: SettingsRect,
        module: OnboardingModule,
    ) {
        self.draw_text(
            module.title(),
            SettingsRect {
                left: bounds.left.saturating_sub(scale(scene, 260)),
                top: bounds.top,
                width: scale(scene, 230),
                height: bounds.height,
            },
            &self.onboarding_body_format,
            &self.text,
            false,
        );
        let selector = rounded(rect(bounds), scale_f32(scene, scene.theme().radii.control));
        unsafe {
            self.context
                .FillRoundedRectangle(&raw const selector, &self.group);
        };
        let selected = scene.onboarding_zone(module);
        let segment_width = bounds.width / 3;
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
                left: bounds.left + index * segment_width,
                top: bounds.top,
                width: if index == 2 {
                    bounds.width - segment_width * 2
                } else {
                    segment_width
                },
                height: bounds.height,
            };
            if selected == zone {
                let selected_surface = rounded(
                    rect(inset_all(segment, scale(scene, 3))),
                    scale_f32(scene, scene.theme().radii.compact),
                );
                unsafe {
                    self.context
                        .FillRoundedRectangle(&raw const selected_surface, &self.selected);
                }
            }
            self.draw_text(
                label,
                segment,
                &self.onboarding_small_format,
                if selected == zone {
                    &self.accent
                } else {
                    &self.muted
                },
                true,
            );
        }
        self.draw_focus(scene, SettingsControl::OnboardingZone(module), bounds);
    }
}
