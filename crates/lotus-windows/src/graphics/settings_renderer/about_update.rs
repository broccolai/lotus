use super::{
    ButtonEmphasis, SettingsControl, SettingsRect, SettingsRenderer, SettingsScene,
    SettingsUpdateActivity, inset, rect, rounded, scale, scale_f32,
};

impl SettingsRenderer {
    pub(super) fn draw_about(&self, scene: &SettingsScene) {
        self.draw_text(
            concat!("lotus ", env!("CARGO_PKG_VERSION")),
            SettingsRect {
                left: scale(scene, 260),
                top: scale(scene, 106),
                width: scale(scene, 600),
                height: scale(scene, 32),
            },
            &self.title_format,
            &self.text,
            false,
        );
        self.draw_text(
            "<3 broccoli",
            SettingsRect {
                left: scale(scene, 260),
                top: scale(scene, 148),
                width: scale(scene, 600),
                height: scale(scene, 32),
            },
            &self.body_format,
            &self.accent,
            false,
        );
    }

    pub(super) fn draw_mascot_image(&self, scene: &SettingsScene, bounds: SettingsRect) {
        self.draw_text(
            "Dock image",
            inset(bounds, scale(scene, 16), 0),
            &self.body_format,
            &self.text,
            false,
        );
        let value = if scene.draft().mascot_image_path.is_some() {
            "Change image"
        } else {
            "Choose image"
        };
        self.draw_text(
            value,
            SettingsRect {
                left: bounds
                    .left
                    .saturating_add(bounds.width)
                    .saturating_sub(scale(scene, 142)),
                top: bounds.top,
                width: scale(scene, 126),
                height: bounds.height,
            },
            &self.small_format,
            &self.accent,
            true,
        );
    }

    pub(super) fn draw_reset_mascot(&self, scene: &SettingsScene, bounds: SettingsRect) {
        self.draw_text(
            "Restore lotus icon",
            inset(bounds, scale(scene, 16), 0),
            &self.body_format,
            &self.muted,
            false,
        );
    }

    pub(super) fn draw_check_for_updates(
        &self,
        scene: &SettingsScene,
        bounds: SettingsRect,
    ) {
        let enabled = scene.update_activity() == SettingsUpdateActivity::Idle;
        let label = match scene.update_activity() {
            SettingsUpdateActivity::Idle if scene.is_installed() => "Check for updates",
            SettingsUpdateActivity::Idle => "Install lotus",
            SettingsUpdateActivity::Checking => "Checking…",
            SettingsUpdateActivity::Installing => "Installing…",
        };
        self.draw_button(
            scene,
            bounds,
            SettingsControl::CheckForUpdates,
            label,
            enabled,
            ButtonEmphasis::Secondary,
        );
    }

    pub(super) fn draw_apply(&self, scene: &SettingsScene, bounds: SettingsRect) {
        self.draw_button(
            scene,
            bounds,
            SettingsControl::Apply,
            "Apply",
            scene.is_dirty(),
            ButtonEmphasis::Primary,
        );
    }

    pub(super) fn draw_revert(&self, scene: &SettingsScene, bounds: SettingsRect) {
        self.draw_button(
            scene,
            bounds,
            SettingsControl::Revert,
            "Revert",
            scene.is_dirty(),
            ButtonEmphasis::Outline,
        );
    }

    pub(super) fn draw_button(
        &self,
        scene: &SettingsScene,
        bounds: SettingsRect,
        control: SettingsControl,
        label: &str,
        enabled: bool,
        emphasis: ButtonEmphasis,
    ) {
        let hovered = enabled && scene.hovered() == Some(control);
        let onboarding = matches!(
            control,
            SettingsControl::OnboardingBack
                | SettingsControl::OnboardingNext
                | SettingsControl::OnboardingFinish
        );
        let radius = if onboarding {
            10.0
        } else {
            scene.theme().radii.control
        };
        let surface = rounded(rect(bounds), scale_f32(scene, radius));
        let fill = match (emphasis, enabled, hovered) {
            (ButtonEmphasis::Primary, true, _) => Some(&self.accent),
            (ButtonEmphasis::Secondary | ButtonEmphasis::Outline, true, true) => {
                Some(&self.selected)
            }
            (ButtonEmphasis::Secondary, _, _) => Some(&self.row),
            (ButtonEmphasis::Primary | ButtonEmphasis::Outline, _, _) => None,
        };
        let border = if emphasis == ButtonEmphasis::Primary && enabled {
            &self.accent
        } else {
            &self.divider
        };
        unsafe {
            if let Some(fill) = fill {
                self.context.FillRoundedRectangle(&raw const surface, fill);
            }
            if !(onboarding && emphasis == ButtonEmphasis::Outline) {
                self.context.DrawRoundedRectangle(
                    &raw const surface,
                    border,
                    scale_f32(scene, 1.0),
                    None,
                );
            }
        }
        let text = match (emphasis, enabled) {
            (_, false) => &self.disabled,
            (ButtonEmphasis::Primary, true) => &self.accent_dark,
            (ButtonEmphasis::Secondary | ButtonEmphasis::Outline, true) => &self.text,
        };
        let format = if onboarding {
            &self.onboarding_button_format
        } else {
            &self.button_format
        };
        self.draw_text(label, bounds, format, text, true);
        self.draw_focus(scene, control, bounds);
    }
}
