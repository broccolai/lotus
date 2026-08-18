use std::mem::ManuallyDrop;

use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_DRAW_TEXT_OPTIONS_CLIP,
    D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, D2D1_LAYER_OPTIONS1_NONE,
    D2D1_LAYER_PARAMETERS1, D2D1_ROUNDED_RECT, ID2D1Bitmap1, ID2D1Geometry, ID2D1Layer,
    ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_MEASURING_MODE_NATURAL, IDWriteTextFormat,
};
use windows::core::Interface;

use super::super::scene::{
    DockBadge, DockHitTarget, DockIcon, DockInteractionState, DockLayout, DockScene,
    LaidOutMedia, LaidOutStatusItem, PixelRect, SystemStatusKind,
};
use super::super::surface::SurfaceSize;
use super::animation::ItemVisual;
use super::geometry::{
    dragged_rectangle, inset_rectangle, pixel_rectangle, rounded_pixel_rectangle,
    running_indicator, scale_dip_offset, translated_scaled_pixel_rectangle,
};
use super::{
    DIVIDER_CORNER_RADIUS, Direct2DRenderer, ItemDraw, MediaTextFormats, RendererError,
    StatusTextFormats, TARGET_DPI, icon_interpolation, nonzero_or_one, status_opacity,
};

impl Direct2DRenderer {
    pub(super) fn item_draws<'a>(
        &'a self,
        scene: &DockScene,
        layout: &DockLayout,
        visuals: Vec<ItemVisual>,
        reorder_offsets: Vec<f32>,
        exit_opacity: f32,
        size: SurfaceSize,
    ) -> Result<Vec<ItemDraw<'a>>, RendererError> {
        let drag = scene.drag();
        layout
            .items
            .iter()
            .zip(visuals.into_iter().zip(reorder_offsets))
            .map(|(item, (mut visual, reorder_offset))| {
                let is_dragged =
                    drag.is_some_and(|drag| drag.source_index == item.source_index);
                let native_raster = matches!(item.icon, DockIcon::Raster(_));
                if drag.is_some() {
                    visual = ItemVisual {
                        scale: 1.0,
                        translate_y: 0.0,
                        icon_opacity: 1.0,
                    };
                }
                if item.exiting {
                    visual.icon_opacity *= exit_opacity;
                }
                let bounds = if let Some(active_drag) =
                    drag.filter(|drag| drag.source_index == item.source_index)
                {
                    dragged_rectangle(
                        active_drag.pointer_x,
                        active_drag.pointer_y,
                        item.bounds.width,
                        visual.scale,
                        size,
                    )
                } else {
                    translated_scaled_pixel_rectangle(
                        item.bounds,
                        visual.scale,
                        if native_raster {
                            reorder_offset.round()
                        } else {
                            reorder_offset
                        },
                        scale_dip_offset(visual.translate_y, scene.dpi()),
                    )
                };
                self.bitmap(&item.icon, layout.icon_size)
                    .map(|bitmap| ItemDraw {
                        is_dragged,
                        bitmap,
                        visual,
                        bounds,
                        interpolation: icon_interpolation(&item.icon, layout.icon_size),
                        badge: item.badge,
                        running: item
                            .running
                            .then(|| running_indicator(bounds, size.height(), scene.dpi())),
                    })
            })
            .collect()
    }

    pub(super) fn draw_badge(
        &self,
        badge: DockBadge,
        icon: D2D_RECT_F,
        dpi: u32,
        format: &IDWriteTextFormat,
        opacity: f32,
    ) {
        let scale = f32::from(u16::try_from(dpi).unwrap_or(u16::MAX)) / TARGET_DPI;
        let (width, height) = match badge {
            DockBadge::Dot => (8.0 * scale, 8.0 * scale),
            DockBadge::Count(count) if count < 10 => (18.0 * scale, 18.0 * scale),
            DockBadge::Count(count) if count < 100 => (24.0 * scale, 18.0 * scale),
            DockBadge::Count(_) | DockBadge::AtLeast(_) => (30.0 * scale, 18.0 * scale),
        };
        let bounds = D2D_RECT_F {
            left: icon.right - width + 3.0 * scale,
            top: icon.top - 3.0 * scale,
            right: icon.right + 3.0 * scale,
            bottom: icon.top + height - 3.0 * scale,
        };
        let surface = D2D1_ROUNDED_RECT {
            rect: bounds,
            radiusX: height * 0.5,
            radiusY: height * 0.5,
        };
        // SAFETY: Drawing occurs between BeginDraw and EndDraw with retained resources.
        unsafe {
            self.badge_brush.SetOpacity(opacity);
            self.badge_text_brush.SetOpacity(opacity);
            self.context
                .FillRoundedRectangle(&raw const surface, &self.badge_brush);
            if badge != DockBadge::Dot {
                let label = match badge {
                    DockBadge::AtLeast(count) => format!("{count}+"),
                    DockBadge::Count(count) if count > 99 => "99+".to_owned(),
                    DockBadge::Count(count) => count.to_string(),
                    DockBadge::Dot => String::new(),
                };
                let text = label.encode_utf16().collect::<Vec<_>>();
                self.context.DrawText(
                    &text,
                    format,
                    &raw const bounds,
                    &self.badge_text_brush,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
            }
        }
    }

    pub(super) fn draw_items(
        &self,
        item_draws: &[ItemDraw<'_>],
        dpi: u32,
        badge_format: &IDWriteTextFormat,
    ) {
        // SAFETY: Called between BeginDraw and EndDraw with live retained resources.
        unsafe {
            for item in item_draws {
                self.context.DrawBitmap(
                    item.bitmap,
                    Some(&raw const item.bounds),
                    item.visual.icon_opacity,
                    item.interpolation,
                    None,
                    None,
                );
            }
            for item in item_draws {
                if let Some(running) = item.running {
                    self.badge_brush.SetOpacity(item.visual.icon_opacity * 0.72);
                    self.context
                        .FillRoundedRectangle(&raw const running, &self.badge_brush);
                }
            }
            for item in item_draws {
                if let Some(badge) = item.badge {
                    self.draw_badge(
                        badge,
                        item.bounds,
                        dpi,
                        badge_format,
                        item.visual.icon_opacity,
                    );
                }
            }
        }
    }

    pub(super) fn draw_show_desktop(
        &self,
        layout: &DockLayout,
        interaction: DockInteractionState,
    ) {
        let highlight = layout
            .show_desktop
            .map(|bounds| rounded_pixel_rectangle(bounds, DIVIDER_CORNER_RADIUS));
        let opacity = if interaction.pressed == Some(DockHitTarget::ShowDesktop) {
            1.0
        } else if interaction.hovered == Some(DockHitTarget::ShowDesktop) {
            0.7
        } else {
            0.0
        };
        // SAFETY: The renderer owns a live context and brushes. Both local
        // geometries remain valid through these synchronous drawing calls.
        unsafe {
            if let Some(highlight) = highlight {
                self.show_desktop_brush.SetOpacity(opacity);
                self.context
                    .FillRoundedRectangle(&raw const highlight, &self.show_desktop_brush);
            }
        }
    }

    pub(super) fn draw_status_items(
        &self,
        layout: &DockLayout,
        bitmaps: &[Option<ID2D1Bitmap1>],
        interaction: DockInteractionState,
        formats: &StatusTextFormats,
    ) {
        for (item, bitmap) in layout.status_items.iter().zip(bitmaps) {
            let target = DockHitTarget::SystemStatus(item.kind);
            let opacity = status_opacity(interaction, target);
            if let (Some(icon), Some(bitmap)) = (&item.icon, bitmap) {
                let bounds = pixel_rectangle(icon.bounds);
                // SAFETY: Drawing occurs between BeginDraw and EndDraw with retained resources.
                unsafe {
                    self.context.DrawBitmap(
                        bitmap,
                        Some(&raw const bounds),
                        opacity,
                        D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                        None,
                        None,
                    );
                }
            } else if item.kind == SystemStatusKind::DateTime {
                self.draw_status_clock(item, opacity, formats);
            } else {
                self.draw_status_text(
                    &item.primary_text,
                    pixel_rectangle(item.hit_bounds),
                    &formats.symbol,
                    &self.status_text_brush,
                    opacity,
                );
            }
        }
    }

    pub(super) fn draw_media(
        &self,
        layout: &DockLayout,
        interaction: DockInteractionState,
        formats: &MediaTextFormats,
        artwork_clip: Option<&ID2D1Geometry>,
    ) {
        let Some(media) = &layout.media else {
            return;
        };
        let metadata_target = DockHitTarget::Media(lotus_media::MediaHitTarget::Metadata);
        let metadata_opacity = status_opacity(interaction, metadata_target);
        if let Ok(bitmap) = self.bitmap(
            &media.artwork.icon,
            nonzero_or_one(media.artwork.bounds.width),
        ) {
            let bounds = pixel_rectangle(media.artwork.bounds);
            // SAFETY: Drawing occurs between BeginDraw and EndDraw with retained resources.
            unsafe {
                if let Some(clip) = artwork_clip {
                    let mut layer = D2D1_LAYER_PARAMETERS1 {
                        contentBounds: bounds,
                        geometricMask: ManuallyDrop::new(Some(clip.clone())),
                        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                        opacity: 1.0,
                        opacityBrush: ManuallyDrop::new(None),
                        layerOptions: D2D1_LAYER_OPTIONS1_NONE,
                        ..Default::default()
                    };
                    layer.maskTransform.M11 = 1.0;
                    layer.maskTransform.M22 = 1.0;
                    self.context
                        .PushLayer(&raw const layer, None::<&ID2D1Layer>);
                }
                self.context.DrawBitmap(
                    bitmap,
                    Some(&raw const bounds),
                    metadata_opacity,
                    D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                    None,
                    None,
                );
                if artwork_clip.is_some() {
                    self.context.PopLayer();
                }
            }
        }
        self.draw_media_text(media, metadata_opacity, formats);

        for control in &media.controls {
            let target = DockHitTarget::Media(control.target);
            let opacity = if control.enabled {
                status_opacity(interaction, target)
            } else {
                0.34
            };
            let Ok(bitmap) =
                self.bitmap(&control.icon, nonzero_or_one(control.bounds.width))
            else {
                continue;
            };
            let bounds = pixel_rectangle(inset_rectangle(control.bounds, 5));
            // SAFETY: Drawing occurs between BeginDraw and EndDraw with retained resources.
            unsafe {
                self.context.DrawBitmap(
                    bitmap,
                    Some(&raw const bounds),
                    opacity,
                    D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                    None,
                    None,
                );
            }
        }
    }

    pub(super) fn media_artwork_clip(
        &self,
        layout: &DockLayout,
        scene: &DockScene,
    ) -> Result<Option<ID2D1Geometry>, RendererError> {
        let Some(media) = &layout.media else {
            return Ok(None);
        };
        let rounded = rounded_pixel_rectangle(
            media.artwork.bounds,
            scale_dip_offset(scene.theme().radii.control, scene.dpi()),
        );
        // SAFETY: The factory is live and copies the local rounded-rectangle description.
        let geometry = unsafe {
            self.factory
                .CreateRoundedRectangleGeometry(&raw const rounded)?
        };
        Ok(Some(geometry.cast()?))
    }

    pub(super) fn draw_media_text(
        &self,
        media: &LaidOutMedia,
        opacity: f32,
        formats: &MediaTextFormats,
    ) {
        let midpoint = media
            .metadata
            .top
            .saturating_add(media.metadata.height.saturating_mul(11) / 20);
        let title = PixelRect {
            left: media.metadata.left,
            top: media.metadata.top,
            width: media.metadata.width,
            height: midpoint.saturating_sub(media.metadata.top),
        };
        let artist = PixelRect {
            left: media.metadata.left,
            top: midpoint,
            width: media.metadata.width,
            height: media
                .metadata
                .top
                .saturating_add(media.metadata.height)
                .saturating_sub(midpoint),
        };
        self.draw_status_text(
            &media.title,
            pixel_rectangle(title),
            &formats.title,
            &self.status_text_brush,
            opacity,
        );
        self.draw_status_text(
            &media.artist,
            pixel_rectangle(artist),
            &formats.artist,
            &self.status_muted_text_brush,
            opacity,
        );
    }

    pub(super) fn status_bitmaps(
        &self,
        layout: &DockLayout,
    ) -> Result<Vec<Option<ID2D1Bitmap1>>, RendererError> {
        layout
            .status_items
            .iter()
            .map(|item| {
                item.icon
                    .as_ref()
                    .map(|icon| {
                        self.bitmap(&icon.icon, nonzero_or_one(icon.bounds.width))
                            .cloned()
                    })
                    .transpose()
            })
            .collect()
    }

    pub(super) fn draw_status_clock(
        &self,
        item: &LaidOutStatusItem,
        opacity: f32,
        formats: &StatusTextFormats,
    ) {
        let bounds = item.hit_bounds;
        if item.secondary_text.is_empty() {
            self.draw_status_text(
                &item.primary_text,
                pixel_rectangle(bounds),
                &formats.time,
                &self.status_text_brush,
                opacity,
            );
            return;
        }

        let stack_height = bounds.height.saturating_mul(3) / 5;
        let stack_top = bounds
            .top
            .saturating_add(bounds.height.saturating_sub(stack_height) / 2);
        let midpoint = stack_top.saturating_add(stack_height / 2);
        let time_bounds = pixel_rectangle(PixelRect {
            left: bounds.left,
            top: stack_top,
            width: bounds.width,
            height: midpoint.saturating_sub(stack_top),
        });
        let date_bounds = pixel_rectangle(PixelRect {
            left: bounds.left,
            top: midpoint,
            width: bounds.width,
            height: stack_top
                .saturating_add(stack_height)
                .saturating_sub(midpoint),
        });
        self.draw_status_text(
            &item.primary_text,
            time_bounds,
            &formats.time,
            &self.status_text_brush,
            opacity,
        );
        self.draw_status_text(
            &item.secondary_text,
            date_bounds,
            &formats.date,
            &self.status_muted_text_brush,
            opacity,
        );
    }

    pub(super) fn draw_status_text(
        &self,
        value: &str,
        bounds: D2D_RECT_F,
        format: &IDWriteTextFormat,
        brush: &ID2D1SolidColorBrush,
        opacity: f32,
    ) {
        let text = value.encode_utf16().collect::<Vec<_>>();
        // SAFETY: Drawing occurs between BeginDraw and EndDraw with retained resources.
        unsafe {
            brush.SetOpacity(opacity);
            self.context.DrawText(
                &text,
                format,
                &raw const bounds,
                brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }
}
