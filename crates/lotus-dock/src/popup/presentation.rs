use lotus_ui::presentation::{
    FontWeight, HorizontalAlignment, ImageSampling, Presentation, PresentationPrimitive,
    PresentationRect, TextStyle, VerticalAlignment,
};

use super::{
    DockPopup, Icon, PhysicalRect, PopupEntry, PopupIcon, PopupSymbol, Theme, physical_rect,
};

impl<Asset: Clone> DockPopup<Asset> {
    pub fn presentation(
        &self,
        asset_for: impl Fn(PopupSymbol) -> Asset,
    ) -> Presentation<Asset> {
        let theme = self.theme();
        let size = self.desired_size();
        let mut presentation = Presentation::new(Theme::default().canvas.with_alpha(0.0));
        presentation.push(PresentationPrimitive::FillRoundedRect {
            bounds: PresentationRect::new(
                0.5,
                0.5,
                as_f32(size.width()) - 0.5,
                as_f32(size.height()) - 0.5,
            ),
            radius: self.scale_f32(theme.radii.window),
            color: theme.chrome_overlay,
        });

        let icon_size = (20 * self.dpi()).div_ceil(96);
        let fallback_size = (42 * self.dpi()).div_ceil(96);
        for entry in &self.entries() {
            self.present_entry(
                &mut presentation,
                entry,
                icon_size,
                fallback_size,
                &asset_for,
            );
        }
        self.present_picker_navigation(&mut presentation, icon_size, &asset_for);
        presentation
    }

    fn present_entry(
        &self,
        presentation: &mut Presentation<Asset>,
        entry: &PopupEntry<Asset>,
        icon_size: u32,
        fallback_size: u32,
        asset_for: &impl Fn(PopupSymbol) -> Asset,
    ) {
        let theme = self.theme();
        let bounds = presentation_rect(entry.bounds);
        let radius = self.scale_f32(theme.radii.control);
        if entry.highlighted {
            presentation.push(PresentationPrimitive::FillRoundedRect {
                bounds,
                radius,
                color: theme.control_hover,
            });
        }
        if entry.active {
            presentation.push(PresentationPrimitive::StrokeRoundedRect {
                bounds,
                radius,
                width: 1.0,
                color: theme.control_selected,
            });
        }

        let artwork_size = if entry.preview.is_some() {
            fallback_size
        } else {
            icon_size
        };
        let icon = match &entry.icon {
            PopupIcon::Symbol(symbol) => Icon::Embedded(asset_for(*symbol)),
            PopupIcon::Artwork(icon) => icon.clone(),
        };
        presentation.push(PresentationPrimitive::Icon {
            icon,
            bounds: popup_icon_bounds(entry, artwork_size),
            tint: theme.text,
            opacity: 1.0,
            sampling: ImageSampling::Smooth,
            radius: 0.0,
        });
        self.present_label(presentation, entry);
        self.present_close(presentation, entry, icon_size, asset_for);
    }

    fn present_label(
        &self,
        presentation: &mut Presentation<Asset>,
        entry: &PopupEntry<Asset>,
    ) {
        if entry.label.is_empty() {
            return;
        }

        let mut bounds = presentation_rect(entry.bounds);
        if let Some(preview) = entry.preview {
            bounds.left += 12.0;
            bounds.bottom = as_f32(preview.min_y());
        } else {
            bounds.left += bounds.height();
        }
        if let Some(close) = entry.close {
            bounds.right = as_f32(close.min_x().saturating_sub(4));
        }
        presentation.push(PresentationPrimitive::Text {
            value: entry.label.clone(),
            bounds,
            style: TextStyle {
                size: 13.5 * as_f32(self.dpi()) / 96.0,
                family: lotus_ui::presentation::FontFamily::Interface,
                weight: FontWeight::Normal,
                horizontal: HorizontalAlignment::Leading,
                vertical: VerticalAlignment::Center,
            },
            color: self.theme().text,
        });
    }

    fn present_close(
        &self,
        presentation: &mut Presentation<Asset>,
        entry: &PopupEntry<Asset>,
        icon_size: u32,
        asset_for: &impl Fn(PopupSymbol) -> Asset,
    ) {
        let Some(close) = entry.close.filter(|_| entry.highlighted) else {
            return;
        };
        if entry.close_highlighted {
            presentation.push(PresentationPrimitive::FillRoundedRect {
                bounds: presentation_rect(close),
                radius: as_f32(close.height()) * 0.25,
                color: self.theme().control_hover,
            });
        }
        presentation.push(PresentationPrimitive::Icon {
            icon: Icon::Embedded(asset_for(PopupSymbol::Close)),
            bounds: centered_rect(close, icon_size),
            tint: self.theme().text,
            opacity: 1.0,
            sampling: ImageSampling::Smooth,
            radius: 0.0,
        });
    }

    fn present_picker_navigation(
        &self,
        presentation: &mut Presentation<Asset>,
        icon_size: u32,
        asset_for: &impl Fn(PopupSymbol) -> Asset,
    ) {
        let Some((previous, next)) = self.picker_navigation() else {
            return;
        };
        let size = self.desired_size();
        let diameter = (28 * self.dpi()).div_ceil(96);
        let top = size.height().saturating_sub(diameter) / 2;
        if previous {
            self.present_navigation(
                presentation,
                physical_rect(2, top, diameter, diameter),
                icon_size,
                PopupSymbol::Previous,
                asset_for,
            );
        }
        if next {
            self.present_navigation(
                presentation,
                physical_rect(
                    size.width().saturating_sub(diameter + 2),
                    top,
                    diameter,
                    diameter,
                ),
                icon_size,
                PopupSymbol::Next,
                asset_for,
            );
        }
    }

    fn present_navigation(
        &self,
        presentation: &mut Presentation<Asset>,
        bounds: PhysicalRect,
        icon_size: u32,
        symbol: PopupSymbol,
        asset_for: &impl Fn(PopupSymbol) -> Asset,
    ) {
        presentation.push(PresentationPrimitive::FillRoundedRect {
            bounds: presentation_rect(bounds),
            radius: as_f32(bounds.height()) / 2.0,
            color: self.theme().control_hover,
        });
        presentation.push(PresentationPrimitive::Icon {
            icon: Icon::Embedded(asset_for(symbol)),
            bounds: centered_rect(bounds, icon_size),
            tint: self.theme().text,
            opacity: 1.0,
            sampling: ImageSampling::Smooth,
            radius: 0.0,
        });
    }

    fn scale_f32(&self, dips: f32) -> f32 {
        as_f32(self.dpi()) * dips / 96.0
    }
}

fn popup_icon_bounds<Asset>(entry: &PopupEntry<Asset>, size: u32) -> PresentationRect {
    if let Some(preview) = entry.preview {
        centered_rect(preview, size)
    } else if entry.label.is_empty() {
        centered_rect(entry.bounds, size)
    } else {
        let inset = entry.bounds.height().saturating_sub(size) / 2;
        PresentationRect::new(
            as_f32(entry.bounds.min_x().saturating_add(inset)),
            as_f32(entry.bounds.min_y().saturating_add(inset)),
            as_f32(entry.bounds.min_x().saturating_add(inset + size)),
            as_f32(entry.bounds.min_y().saturating_add(inset + size)),
        )
    }
}

fn centered_rect(bounds: PhysicalRect, size: u32) -> PresentationRect {
    let left = bounds
        .min_x()
        .saturating_add(bounds.width().saturating_sub(size) / 2);
    let top = bounds
        .min_y()
        .saturating_add(bounds.height().saturating_sub(size) / 2);
    PresentationRect::new(
        as_f32(left),
        as_f32(top),
        as_f32(left.saturating_add(size)),
        as_f32(top.saturating_add(size)),
    )
}

fn presentation_rect(bounds: PhysicalRect) -> PresentationRect {
    PresentationRect::new(
        as_f32(bounds.min_x()),
        as_f32(bounds.min_y()),
        as_f32(bounds.max_x()),
        as_f32(bounds.max_y()),
    )
}

#[allow(
    clippy::cast_precision_loss,
    reason = "popup dimensions remain below f32 exact range"
)]
const fn as_f32(value: u32) -> f32 {
    value as f32
}
