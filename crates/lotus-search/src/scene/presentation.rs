use lotus_ui::icon::Icon;
use lotus_ui::presentation::{
    FontWeight, HorizontalAlignment, ImageSampling, Presentation, PresentationPrimitive,
    PresentationRect, TextStyle, VerticalAlignment,
};

use super::{LauncherResultKind, LauncherScene, PixelRect};
use crate::controller::SearchMode;

impl<Asset: Clone> LauncherScene<Asset> {
    pub fn render_presentation(&self, search_asset: Asset) -> Presentation<Asset> {
        let theme = self.theme();
        let layout = self.layout();
        let mut presentation = Presentation::new(theme.canvas.with_alpha(0.0));
        presentation.push(fill(
            PresentationRect::new(
                0.0,
                0.0,
                as_f32(layout.size.width()),
                as_f32(layout.size.height()),
            ),
            0.0,
            theme.chrome_overlay,
        ));

        let query = rect(layout.query);
        let radius = self.control_radius();
        presentation.push(fill(query, radius, theme.control));
        presentation.push(PresentationPrimitive::StrokeRoundedRect {
            bounds: inset(query, 0.5),
            radius: radius - 0.5,
            width: 1.0,
            color: theme.border,
        });
        self.present_search_mode(&mut presentation, query, search_asset);
        self.present_query(&mut presentation, query);
        self.present_row_states(&mut presentation, &layout);
        self.present_results(&mut presentation, &layout);
        self.present_footer(&mut presentation, &layout);
        presentation
    }

    fn present_search_mode(
        &self,
        presentation: &mut Presentation<Asset>,
        query: PresentationRect,
        search_asset: Asset,
    ) {
        let bounds = search_glyph_rect(query);
        if self.mode() == SearchMode::Applications {
            presentation.push(PresentationPrimitive::Icon {
                icon: Icon::Embedded(search_asset),
                bounds,
                tint: self.theme().text,
                opacity: self.theme().text_muted.alpha,
                sampling: ImageSampling::Smooth,
                radius: 0.0,
            });
            return;
        }
        presentation.push(PresentationPrimitive::Text {
            value: match self.mode() {
                SearchMode::Commands => ">",
                SearchMode::Calculator => "#",
                SearchMode::Applications => "",
            }
            .to_owned(),
            bounds,
            style: text_style(17.0, FontWeight::Semibold, HorizontalAlignment::Center),
            color: self.theme().text_muted,
        });
    }

    fn present_query(
        &self,
        presentation: &mut Presentation<Asset>,
        query: PresentationRect,
    ) {
        let bounds = search_text_rect(query);
        let placeholder =
            self.mode() == SearchMode::Applications && self.query().is_empty();
        let value = if placeholder {
            "Search apps or type > for actions"
        } else {
            self.display_query()
        };
        let style = text_style(18.0, FontWeight::Normal, HorizontalAlignment::Leading);
        presentation.push(PresentationPrimitive::Text {
            value: value.to_owned(),
            bounds,
            style,
            color: if placeholder {
                self.theme().text_muted
            } else {
                self.theme().text
            },
        });
        presentation.push(PresentationPrimitive::TextCaret {
            before: self.display_query_before_cursor().to_owned(),
            bounds,
            style,
            top_inset: 13.0,
            bottom_inset: 13.0,
            width: 1.0,
            color: self.theme().accent,
        });
    }

    fn present_row_states(
        &self,
        presentation: &mut Presentation<Asset>,
        layout: &super::LauncherLayout,
    ) {
        let radius = self.control_radius();
        if let Some(hovered) = layout
            .hovered
            .filter(|hovered| Some(*hovered) != layout.selected)
            .and_then(|index| layout.row_surfaces.get(index))
        {
            presentation.push(fill(rect(*hovered), radius, self.theme().control_hover));
        }
        if let Some(selected) = layout
            .selected
            .and_then(|index| layout.row_surfaces.get(index))
        {
            let bounds = rect(*selected);
            presentation.push(fill(bounds, radius, self.theme().control_selected));
            presentation.push(PresentationPrimitive::StrokeRoundedRect {
                bounds: inset(bounds, 0.5),
                radius: radius - 0.5,
                width: 1.0,
                color: self.theme().border_strong,
            });
        }
    }

    fn present_results(
        &self,
        presentation: &mut Presentation<Asset>,
        layout: &super::LauncherLayout,
    ) {
        for (index, entry) in self.results().iter().enumerate() {
            if let Some(icon) = &entry.icon
                && let Some(bounds) = layout.row_icons[index]
            {
                presentation.push(PresentationPrimitive::Icon {
                    icon: icon.clone(),
                    bounds: rect(bounds),
                    tint: self.theme().text,
                    opacity: 1.0,
                    sampling: ImageSampling::Smooth,
                    radius: 0.0,
                });
            } else {
                presentation.push(PresentationPrimitive::Text {
                    value: entry.initial(),
                    bounds: rect(layout.row_icon_cells[index]),
                    style: text_style(
                        15.0,
                        FontWeight::Semibold,
                        HorizontalAlignment::Center,
                    ),
                    color: self.theme().text,
                });
            }
            presentation.push(PresentationPrimitive::Text {
                value: entry.title.clone(),
                bounds: rect(layout.row_texts[index]),
                style: text_style(14.5, FontWeight::Normal, HorizontalAlignment::Leading),
                color: self.theme().text,
            });
            if let Some(badge) = layout.action_badges[index] {
                let badge = rect(badge);
                presentation.push(fill(badge, 6.0, self.theme().control_selected));
                presentation.push(PresentationPrimitive::Text {
                    value: match entry.kind {
                        LauncherResultKind::Command => "RUN",
                        LauncherResultKind::Calculator => "COPY",
                        LauncherResultKind::Application => "",
                    }
                    .to_owned(),
                    bounds: badge,
                    style: text_style(
                        10.5,
                        FontWeight::Semibold,
                        HorizontalAlignment::Center,
                    ),
                    color: self.theme().accent,
                });
            }
        }
        if let Some(empty) = layout.empty_state {
            presentation.push(PresentationPrimitive::Text {
                value: if self.is_command_mode() {
                    "No matching actions"
                } else {
                    "No applications found"
                }
                .to_owned(),
                bounds: rect(empty),
                style: text_style(14.0, FontWeight::Normal, HorizontalAlignment::Center),
                color: self.theme().text_muted,
            });
        }
    }

    fn present_footer(
        &self,
        presentation: &mut Presentation<Asset>,
        layout: &super::LauncherLayout,
    ) {
        if let Some(thumb) = layout.scrollbar_thumb {
            presentation.push(fill(
                rect(thumb),
                as_f32(thumb.width) / 2.0,
                self.theme().text_muted,
            ));
        }
        presentation.push(fill(
            rect(layout.footer_separator),
            0.0,
            self.theme().border,
        ));
        presentation.push(PresentationPrimitive::Text {
            value: if self.is_command_mode() {
                "Lotus Actions"
            } else if self.is_calculator_mode() {
                "Lotus Calculator"
            } else {
                "Lotus"
            }
            .to_owned(),
            bounds: rect(layout.footer_label),
            style: text_style(12.5, FontWeight::Semibold, HorizontalAlignment::Leading),
            color: self.theme().accent,
        });
        presentation.push(PresentationPrimitive::Text {
            value: self.footer_time().to_owned(),
            bounds: rect(layout.footer_time),
            style: text_style(12.5, FontWeight::Normal, HorizontalAlignment::Trailing),
            color: self.theme().text_muted,
        });
    }

    fn control_radius(&self) -> f32 {
        as_f32(self.dpi()) * self.theme().radii.control / 96.0
    }
}

fn fill<Asset>(
    bounds: PresentationRect,
    radius: f32,
    color: lotus_ui::theme::Color,
) -> PresentationPrimitive<Asset> {
    PresentationPrimitive::FillRoundedRect {
        bounds,
        radius,
        color,
    }
}

fn text_style(size: f32, weight: FontWeight, horizontal: HorizontalAlignment) -> TextStyle {
    TextStyle {
        size,
        family: lotus_ui::presentation::FontFamily::Interface,
        weight,
        horizontal,
        vertical: VerticalAlignment::Center,
    }
}

fn rect(value: PixelRect) -> PresentationRect {
    PresentationRect::new(
        as_f32(value.left),
        as_f32(value.top),
        as_f32(value.left.saturating_add(value.width)),
        as_f32(value.top.saturating_add(value.height)),
    )
}

fn inset(mut rect: PresentationRect, amount: f32) -> PresentationRect {
    rect.left += amount;
    rect.top += amount;
    rect.right -= amount;
    rect.bottom -= amount;
    rect
}

fn search_text_rect(query: PresentationRect) -> PresentationRect {
    let scale = query.height() / 50.0;
    PresentationRect::new(
        query.left + 44.0 * scale,
        query.top,
        query.right - 14.0 * scale,
        query.bottom,
    )
}

fn search_glyph_rect(query: PresentationRect) -> PresentationRect {
    let scale = query.height() / 50.0;
    let size = 17.0 * scale;
    let top = query.top + (query.height() - size) / 2.0;
    PresentationRect::new(
        query.left + 14.0 * scale,
        top,
        query.left + 14.0 * scale + size,
        top + size,
    )
}

#[allow(
    clippy::cast_precision_loss,
    reason = "launcher dimensions remain below f32 exact range"
)]
const fn as_f32(value: u32) -> f32 {
    value as f32
}
