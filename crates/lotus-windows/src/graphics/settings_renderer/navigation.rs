use super::{
    D2D_RECT_F, SettingsControl, SettingsLayout, SettingsPage, SettingsRect,
    SettingsRenderer, SettingsScene, as_f32, inset, rect, rounded, scale, scale_f32,
};

impl SettingsRenderer {
    pub(super) fn draw_navigation(&self, scene: &SettingsScene, layout: &SettingsLayout) {
        self.draw_text(
            "lotus",
            SettingsRect {
                left: scale(scene, 34),
                top: scale(scene, 18),
                width: scale(scene, 160),
                height: scale(scene, 44),
            },
            &self.brand_format,
            &self.text,
            false,
        );
        for page in SettingsPage::ALL {
            let control = SettingsControl::Navigate(page);
            let Some(bounds) = layout.bounds(control) else {
                continue;
            };
            let selected = scene.page() == page;
            if selected {
                let surface =
                    rounded(rect(bounds), scale_f32(scene, scene.theme().radii.control));
                let marker = SettingsRect {
                    left: bounds.left + scale(scene, 3),
                    top: bounds.top + scale(scene, 12),
                    width: scale(scene, 3),
                    height: bounds.height.saturating_sub(scale(scene, 24)),
                };
                let marker = rounded(rect(marker), scale_f32(scene, 1.5));
                unsafe {
                    self.context
                        .FillRoundedRectangle(&raw const surface, &self.sidebar_selected);
                    self.context
                        .FillRoundedRectangle(&raw const marker, &self.text);
                };
            }
            self.draw_text(
                page.title(),
                if page == SettingsPage::About {
                    bounds
                } else {
                    inset(bounds, scale(scene, 20), 0)
                },
                &self.body_format,
                &self.text,
                page == SettingsPage::About,
            );
        }

        let Some(appearance) =
            layout.bounds(SettingsControl::Navigate(SettingsPage::Appearance))
        else {
            return;
        };
        let Some(taskbar) = layout.bounds(SettingsControl::Navigate(SettingsPage::Taskbar))
        else {
            return;
        };
        let top = appearance
            .top
            .saturating_add(appearance.height)
            .saturating_add(
                taskbar
                    .top
                    .saturating_sub(appearance.top + appearance.height)
                    / 2,
            );
        let divider = D2D_RECT_F {
            left: as_f32(appearance.left.saturating_add(scale(scene, 20))),
            top: as_f32(top),
            right: as_f32(
                appearance
                    .left
                    .saturating_add(appearance.width)
                    .saturating_sub(scale(scene, 20)),
            ),
            bottom: as_f32(top.saturating_add(scale(scene, 1))),
        };
        unsafe {
            self.context
                .FillRectangle(&raw const divider, &self.divider);
        }
    }
}
