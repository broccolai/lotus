use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use lotus_ui::theme::Theme;
use thiserror::Error;
use windows::Win32::Foundation::D2DERR_RECREATE_TARGET;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
    D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_DRAW_TEXT_OPTIONS_CLIP,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_INTERPOLATION_MODE,
    D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
    D2D1_LAYER_OPTIONS1_NONE, D2D1_LAYER_PARAMETERS1, D2D1_ROUNDED_RECT, D2D1CreateFactory,
    ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext, ID2D1Factory1, ID2D1Geometry,
    ID2D1Image, ID2D1Layer, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory,
    IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::{IDXGISurface, IDXGISwapChain1};
use windows::core::{Error as WindowsError, Interface, w};

use super::assets::{AssetError, RasterImage, RasterSize, SvgAsset, SvgAssetCache};
use super::device::GraphicsDevice;
use super::scene::{
    DockAnchor, DockBadge, DockDragState, DockHitTarget, DockIcon, DockInteractionState,
    DockLayout, DockScene, LaidOutItem, LaidOutMedia, LaidOutStatusItem, PixelRect,
    RasterIcon, RasterIconId,
};
use super::surface::SurfaceSize;
use super::theme;

const TARGET_DPI: f32 = 96.0;
const DIVIDER_CORNER_RADIUS: f32 = 1.0;
const HOVER_DURATION: Duration = Duration::from_millis(145);
const PRESS_DURATION: Duration = Duration::from_millis(80);
const REORDER_DURATION: Duration = Duration::from_millis(180);
const CHROME_RESIZE_DURATION: Duration = Duration::from_millis(90);
const CHROME_RESIZE_DISTANCE_DIP: f32 = 10.0;
const EXIT_DURATION: Duration = Duration::from_millis(80);

const TRANSPARENT: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.0,
};

pub(super) enum DrawResult {
    Complete { needs_animation: bool },
    RecreateTarget,
}

#[derive(Clone)]
struct StatusTextFormats {
    time: IDWriteTextFormat,
    date: IDWriteTextFormat,
}

#[derive(Clone)]
struct MediaTextFormats {
    title: IDWriteTextFormat,
    artist: IDWriteTextFormat,
}

pub(super) struct Direct2DRenderer {
    factory: ID2D1Factory1,
    _device: ID2D1Device,
    context: ID2D1DeviceContext,
    target: Option<ID2D1Bitmap1>,
    dock_brush: ID2D1SolidColorBrush,
    divider_brush: ID2D1SolidColorBrush,
    show_desktop_brush: ID2D1SolidColorBrush,
    badge_brush: ID2D1SolidColorBrush,
    badge_text_brush: ID2D1SolidColorBrush,
    status_text_brush: ID2D1SolidColorBrush,
    status_muted_text_brush: ID2D1SolidColorBrush,
    write_factory: IDWriteFactory,
    badge_formats: HashMap<u32, IDWriteTextFormat>,
    status_formats: HashMap<u32, StatusTextFormats>,
    media_formats: HashMap<u32, MediaTextFormats>,
    interaction: InteractionAnimator,
    reorder: ReorderAnimator,
    chrome: ChromeAnimator,
    exit: ExitAnimator,
    assets: SvgAssetCache,
    embedded_bitmaps: HashMap<(SvgAsset, NonZeroU32), ID2D1Bitmap1>,
    raster_bitmaps: HashMap<(RasterIconId, u32, u32), ID2D1Bitmap1>,
}

impl Direct2DRenderer {
    pub(super) fn create(
        graphics: &GraphicsDevice,
        swap_chain: &IDXGISwapChain1,
    ) -> Result<Self, RendererError> {
        let dxgi_device = graphics.dxgi_device()?;

        // SAFETY: The requested factory interface is supported by Direct2D 1.1;
        // no debug options pointer is supplied.
        let factory: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        // SAFETY: `dxgi_device` is a live typed COM interface created from the
        // same D3D11 device that owns the swap chain.
        let device = unsafe { factory.CreateDevice(&dxgi_device)? };
        // SAFETY: The Direct2D device is live and returns an owned context.
        let context =
            unsafe { device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)? };
        // SAFETY: DirectWrite returns an owned shared factory.
        let write_factory: IDWriteFactory =
            unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let default_theme = Theme::default();
        let dock_tint = theme::d2d(default_theme.chrome_overlay);
        let divider_tint = theme::d2d(default_theme.divider);
        let show_desktop_tint = theme::d2d(default_theme.control_hover);
        let badge_tint = theme::d2d(default_theme.accent);
        let badge_text_tint = theme::d2d(default_theme.on_accent);
        let status_text_tint = theme::d2d(default_theme.text);
        let status_muted_text_tint = theme::d2d(default_theme.text_muted);
        // SAFETY: The context is live, and all color pointers remain valid for
        // their synchronous brush-creation calls.
        let (
            dock_brush,
            divider_brush,
            show_desktop_brush,
            badge_brush,
            badge_text_brush,
            status_text_brush,
            status_muted_text_brush,
        ) = unsafe {
            (
                context.CreateSolidColorBrush(&raw const dock_tint, None)?,
                context.CreateSolidColorBrush(&raw const divider_tint, None)?,
                context.CreateSolidColorBrush(&raw const show_desktop_tint, None)?,
                context.CreateSolidColorBrush(&raw const badge_tint, None)?,
                context.CreateSolidColorBrush(&raw const badge_text_tint, None)?,
                context.CreateSolidColorBrush(&raw const status_text_tint, None)?,
                context.CreateSolidColorBrush(&raw const status_muted_text_tint, None)?,
            )
        };

        let mut renderer = Self {
            factory,
            _device: device,
            context,
            target: None,
            dock_brush,
            divider_brush,
            show_desktop_brush,
            badge_brush,
            badge_text_brush,
            status_text_brush,
            status_muted_text_brush,
            write_factory,
            badge_formats: HashMap::new(),
            status_formats: HashMap::new(),
            media_formats: HashMap::new(),
            interaction: InteractionAnimator::default(),
            reorder: ReorderAnimator::default(),
            chrome: ChromeAnimator::default(),
            exit: ExitAnimator::default(),
            assets: SvgAssetCache::create()?,
            embedded_bitmaps: HashMap::new(),
            raster_bitmaps: HashMap::new(),
        };
        renderer.attach_target(swap_chain)?;
        Ok(renderer)
    }

    pub(super) fn detach_target(&mut self) {
        // SAFETY: The context is live. A null image is the documented way to
        // release its target reference before resizing DXGI buffers.
        unsafe { self.context.SetTarget(None::<&ID2D1Image>) };
        self.target = None;
    }

    pub(super) const fn is_target_attached(&self) -> bool {
        self.target.is_some()
    }

    pub(super) fn attach_target(
        &mut self,
        swap_chain: &IDXGISwapChain1,
    ) -> Result<(), WindowsError> {
        self.detach_target();

        // SAFETY: Buffer zero exists on this initialized two-buffer swap chain.
        // The returned `IDXGISurface` owns its COM reference.
        let surface: IDXGISurface = unsafe { swap_chain.GetBuffer(0)? };
        let properties = target_properties();
        // SAFETY: The DXGI surface and context are live, and `properties`
        // remains valid for the duration of the synchronous creation call.
        let target = unsafe {
            self.context
                .CreateBitmapFromDxgiSurface(&surface, Some(&raw const properties))?
        };
        // SAFETY: `target` is a bitmap created by this context with TARGET set.
        unsafe { self.context.SetTarget(&target) };
        self.target = Some(target);
        Ok(())
    }

    pub(super) fn draw(
        &mut self,
        size: SurfaceSize,
        scene: &DockScene,
    ) -> Result<DrawResult, RendererError> {
        debug_assert!(self.target.is_some(), "drawing requires an attached target");
        let theme = scene.theme();
        self.apply_theme(&theme);
        let layout = scene.layout(size.width(), size.height());
        let now = Instant::now();
        let (visuals, jirachi_visual, needs_animation) =
            self.interaction
                .sample(now, scene.interaction(), &layout.items);
        let (reorder_offsets, reorder_animating) =
            self.reorder.sample(now, scene.drag(), &layout.items);
        let (dock, chrome_animating) =
            self.chrome_geometry(now, size, scene, theme.radii.window);
        let (exit_opacity, exit_animating) = self.exit.sample(now, &layout.items);
        self.ensure_scene_icons(scene, &layout)?;
        let badge_format = self.badge_format(scene.dpi())?;
        let status_formats = self.status_formats(scene.dpi())?;
        let media_formats = self.media_formats(scene.dpi())?;
        let mut item_draws =
            self.item_draws(scene, &layout, visuals, reorder_offsets, exit_opacity, size)?;
        item_draws.sort_by_key(|item| item.is_dragged);
        let jirachi = layout
            .launcher_button_visible
            .then(|| self.bitmap(scene.mascot(), layout.icon_size))
            .transpose()?;
        let divider = rounded_pixel_rectangle(layout.divider, DIVIDER_CORNER_RADIUS);
        let jirachi_bounds = translated_scaled_pixel_rectangle(
            layout.jirachi,
            jirachi_visual.scale,
            0.0,
            scale_dip_offset(jirachi_visual.translate_y, scene.dpi()),
        );
        let mascot_bounds = fitted_mascot_bounds(scene.mascot(), jirachi_bounds);
        let status_bitmaps = self.status_bitmaps(&layout)?;
        let media_artwork_clip = self.media_artwork_clip(&layout, scene)?;
        let transparent = TRANSPARENT;

        // SAFETY: The context has a live target. Geometry and color pointers
        // remain valid through their calls, and every BeginDraw is paired with
        // EndDraw before the result is inspected.
        let result = unsafe {
            self.context.BeginDraw();
            self.context.Clear(Some(&raw const transparent));
            self.context
                .FillRoundedRectangle(&raw const dock, &self.dock_brush);
            self.draw_items(&item_draws, scene.dpi(), &badge_format);
            if layout.launcher_button_visible {
                self.context
                    .FillRoundedRectangle(&raw const divider, &self.divider_brush);
            }
            if let Some(media_divider) = layout.media_divider {
                let media_divider =
                    rounded_pixel_rectangle(media_divider, DIVIDER_CORNER_RADIUS);
                self.context
                    .FillRoundedRectangle(&raw const media_divider, &self.divider_brush);
            }
            self.draw_media(
                &layout,
                scene.interaction(),
                &media_formats,
                media_artwork_clip.as_ref(),
            );
            if let Some(status_divider) = layout.status_divider {
                let status_divider =
                    rounded_pixel_rectangle(status_divider, DIVIDER_CORNER_RADIUS);
                self.context
                    .FillRoundedRectangle(&raw const status_divider, &self.divider_brush);
            }
            self.draw_status_items(
                &layout,
                &status_bitmaps,
                scene.interaction(),
                &status_formats,
            );
            self.draw_show_desktop(&layout, scene.interaction());
            if let Some(jirachi) = jirachi {
                self.context.DrawBitmap(
                    jirachi,
                    Some(&raw const mascot_bounds),
                    jirachi_visual.icon_opacity,
                    mascot_interpolation(scene.mascot()),
                    None,
                    None,
                );
            }
            self.context.EndDraw(None, None)
        };

        map_draw_result(
            result,
            needs_animation || reorder_animating || chrome_animating || exit_animating,
        )
    }

    fn item_draws<'a>(
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

    fn apply_theme(&self, value: &Theme) {
        theme::set(&self.dock_brush, value.chrome_overlay);
        theme::set(&self.divider_brush, value.divider);
        theme::set(&self.show_desktop_brush, value.control_hover);
        theme::set(&self.badge_brush, value.accent);
        theme::set(&self.badge_text_brush, value.on_accent);
        theme::set(&self.status_text_brush, value.text);
        theme::set(&self.status_muted_text_brush, value.text_muted);
    }

    fn ensure_scene_icons(
        &mut self,
        scene: &DockScene,
        layout: &DockLayout,
    ) -> Result<(), RendererError> {
        for item in &layout.items {
            self.ensure_icon(&item.icon, layout.icon_size)?;
        }
        for item in &layout.status_items {
            if let Some(icon) = &item.icon {
                self.ensure_icon(&icon.icon, nonzero_or_one(icon.bounds.width))?;
            }
        }
        if let Some(media) = &layout.media {
            self.ensure_icon(
                &media.artwork.icon,
                nonzero_or_one(media.artwork.bounds.width),
            )?;
            for control in &media.controls {
                self.ensure_icon(&control.icon, nonzero_or_one(control.bounds.width))?;
            }
        }
        if layout.launcher_button_visible {
            self.ensure_icon(scene.mascot(), layout.icon_size)?;
        }
        Ok(())
    }

    fn chrome_geometry(
        &mut self,
        now: Instant,
        size: SurfaceSize,
        scene: &DockScene,
        radius: f32,
    ) -> (D2D1_ROUNDED_RECT, bool) {
        let (width, animating) = self.chrome.sample(now, size.width(), scene.dpi());
        let dpi = f32::from(u16::try_from(scene.dpi()).unwrap_or(u16::MAX));
        (
            dock_rectangle(size, radius * dpi / TARGET_DPI, width, scene.anchor()),
            animating,
        )
    }

    fn status_formats(&mut self, dpi: u32) -> Result<StatusTextFormats, WindowsError> {
        if let Some(formats) = self.status_formats.get(&dpi) {
            return Ok(formats.clone());
        }

        let scale = f32::from(u16::try_from(dpi).unwrap_or(u16::MAX)) / TARGET_DPI;
        let time = centered_text_format(&self.write_factory, 12.5 * scale)?;
        let date = centered_text_format(&self.write_factory, 10.5 * scale)?;
        let formats = StatusTextFormats { time, date };
        self.status_formats.insert(dpi, formats.clone());
        Ok(formats)
    }

    fn media_formats(&mut self, dpi: u32) -> Result<MediaTextFormats, WindowsError> {
        if let Some(formats) = self.media_formats.get(&dpi) {
            return Ok(formats.clone());
        }

        let scale = f32::from(u16::try_from(dpi).unwrap_or(u16::MAX)) / TARGET_DPI;
        let title = media_text_format(&self.write_factory, 12.5 * scale)?;
        let artist = media_text_format(&self.write_factory, 10.5 * scale)?;
        let formats = MediaTextFormats { title, artist };
        self.media_formats.insert(dpi, formats.clone());
        Ok(formats)
    }

    fn badge_format(&mut self, dpi: u32) -> Result<IDWriteTextFormat, WindowsError> {
        if let Some(format) = self.badge_formats.get(&dpi) {
            return Ok(format.clone());
        }
        let dpi_value = f32::from(u16::try_from(dpi).unwrap_or(u16::MAX));
        let size = 10.5 * dpi_value / TARGET_DPI;
        // SAFETY: Static family and locale strings are NUL terminated.
        let format = unsafe {
            self.write_factory.CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("en-us"),
            )?
        };
        // SAFETY: The newly created format accepts these documented layout values.
        unsafe {
            format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        }
        self.badge_formats.insert(dpi, format.clone());
        Ok(format)
    }

    fn draw_badge(
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

    fn draw_items(
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

    fn draw_show_desktop(&self, layout: &DockLayout, interaction: DockInteractionState) {
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

    fn draw_status_items(
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
            } else {
                self.draw_status_clock(item, opacity, formats);
            }
        }
    }

    fn draw_media(
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

    fn media_artwork_clip(
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

    fn draw_media_text(
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

    fn status_bitmaps(
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

    fn draw_status_clock(
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

    fn draw_status_text(
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

    fn ensure_icon(
        &mut self,
        icon: &DockIcon,
        size: NonZeroU32,
    ) -> Result<(), RendererError> {
        match icon {
            DockIcon::Embedded(asset) => self.ensure_embedded_bitmap(*asset, size),
            DockIcon::Raster(raster) => self.ensure_raster_bitmap(raster),
        }
    }

    fn ensure_embedded_bitmap(
        &mut self,
        asset: SvgAsset,
        size: NonZeroU32,
    ) -> Result<(), RendererError> {
        let key = (asset, size);
        if self.embedded_bitmaps.contains_key(&key) {
            return Ok(());
        }

        let raster = self.assets.rasterize(asset, RasterSize::square(size))?;
        let bitmap = upload_bitmap(&self.context, raster)?;
        self.embedded_bitmaps.insert(key, bitmap);
        Ok(())
    }

    fn ensure_raster_bitmap(&mut self, raster: &RasterIcon) -> Result<(), RendererError> {
        let key = raster_key(raster);
        if self.raster_bitmaps.contains_key(&key) {
            return Ok(());
        }

        let bitmap = upload_raster_icon(&self.context, raster)?;
        self.raster_bitmaps.insert(key, bitmap);
        Ok(())
    }

    fn bitmap(
        &self,
        icon: &DockIcon,
        embedded_size: NonZeroU32,
    ) -> Result<&ID2D1Bitmap1, RendererError> {
        match icon {
            DockIcon::Embedded(asset) => self
                .embedded_bitmaps
                .get(&(*asset, embedded_size))
                .ok_or(RendererError::BitmapCacheInvariant),
            DockIcon::Raster(raster) => self
                .raster_bitmaps
                .get(&raster_key(raster))
                .ok_or(RendererError::BitmapCacheInvariant),
        }
    }
}

fn map_draw_result(
    result: windows::core::Result<()>,
    needs_animation: bool,
) -> Result<DrawResult, RendererError> {
    match result {
        Ok(()) => Ok(DrawResult::Complete { needs_animation }),
        Err(error) if error.code() == D2DERR_RECREATE_TARGET => {
            Ok(DrawResult::RecreateTarget)
        }
        Err(error) => Err(error.into()),
    }
}

#[derive(Default)]
struct InteractionAnimator {
    items: HashMap<usize, ItemMotion>,
    jirachi: Option<ItemMotion>,
}

impl InteractionAnimator {
    fn sample(
        &mut self,
        now: Instant,
        state: DockInteractionState,
        items: &[LaidOutItem],
    ) -> (Vec<ItemVisual>, ItemVisual, bool) {
        self.items.retain(|source_index, _| {
            items.iter().any(|item| item.source_index == *source_index)
        });
        let mut needs_animation = false;
        let visuals = items
            .iter()
            .map(|item| {
                let motion = self
                    .items
                    .entry(item.source_index)
                    .or_insert_with(|| ItemMotion::new(now));
                let target = DockHitTarget::Item(item.source_index);
                let (visual, animating) = motion.sample(
                    now,
                    state.hovered == Some(target),
                    state.pressed == Some(target),
                );
                needs_animation |= animating;
                visual
            })
            .collect();
        let jirachi = self.jirachi.get_or_insert_with(|| ItemMotion::new(now));
        let (jirachi_visual, jirachi_animating) = jirachi.sample(
            now,
            state.hovered == Some(DockHitTarget::Jirachi),
            state.pressed == Some(DockHitTarget::Jirachi),
        );
        needs_animation |= jirachi_animating;
        (visuals, jirachi_visual, needs_animation)
    }
}

#[derive(Default)]
struct ReorderAnimator {
    items: HashMap<usize, OffsetMotion>,
    was_dragging: bool,
}

#[derive(Default)]
struct ChromeAnimator {
    from: f32,
    target: f32,
    started: Option<Instant>,
}

#[derive(Default)]
struct ExitAnimator {
    started: Option<Instant>,
}

impl ExitAnimator {
    fn sample(&mut self, now: Instant, items: &[LaidOutItem]) -> (f32, bool) {
        if !items.iter().any(|item| item.exiting) {
            self.started = None;
            return (1.0, false);
        }

        let started = *self.started.get_or_insert(now);
        let progress = (now.saturating_duration_since(started).as_secs_f32()
            / EXIT_DURATION.as_secs_f32())
        .clamp(0.0, 1.0);
        (1.0 - progress, progress < 1.0)
    }
}

impl ChromeAnimator {
    fn sample(&mut self, now: Instant, width: u32, dpi: u32) -> (f32, bool) {
        let width = pixels_to_f32(width);
        if self.target == 0.0 {
            self.from = width;
            self.target = width;
            return (width, false);
        }
        if width > self.target {
            let scale = f32::from(u16::try_from(dpi).unwrap_or(u16::MAX)) / TARGET_DPI;
            self.from = (width - CHROME_RESIZE_DISTANCE_DIP * scale).max(self.target);
            self.target = width;
            self.started = Some(now);
        } else if width < self.target {
            self.from = width;
            self.target = width;
            self.started = None;
        }
        let Some(started) = self.started else {
            return (self.target, false);
        };
        let progress = (now.saturating_duration_since(started).as_secs_f32()
            / CHROME_RESIZE_DURATION.as_secs_f32())
        .clamp(0.0, 1.0);
        let width = self.from + (self.target - self.from) * ease_out_cubic(progress);
        let moving = progress < 1.0;
        if !moving {
            self.started = None;
        }
        (width, moving)
    }
}

impl ReorderAnimator {
    fn sample(
        &mut self,
        now: Instant,
        drag: Option<DockDragState>,
        items: &[LaidOutItem],
    ) -> (Vec<f32>, bool) {
        self.items.retain(|source_index, _| {
            items.iter().any(|item| item.source_index == *source_index)
        });

        let targets = drag.map_or_else(
            || vec![0.0; items.len()],
            |drag| reorder_targets(items, drag),
        );
        let released = self.was_dragging && drag.is_none();
        let mut animating = false;
        let offsets = items
            .iter()
            .zip(targets)
            .map(|(item, target)| {
                let motion = self
                    .items
                    .entry(item.source_index)
                    .or_insert_with(|| OffsetMotion::new(now));
                if released {
                    motion.snap(target, now);
                } else {
                    motion.retarget(target, now);
                }
                animating |= motion.is_animating(now);
                motion.sample(now)
            })
            .collect();

        self.was_dragging = drag.is_some();
        (offsets, animating)
    }
}

fn reorder_targets(items: &[LaidOutItem], drag: DockDragState) -> Vec<f32> {
    let Some(source_position) = items
        .iter()
        .position(|item| item.source_index == drag.source_index)
    else {
        return vec![0.0; items.len()];
    };
    let insertion_slot = DockLayoutView(items).insertion_slot(drag.pointer_x);
    let destination = if insertion_slot == items.len() {
        items.len().saturating_sub(1)
    } else if source_position < insertion_slot {
        insertion_slot.saturating_sub(1)
    } else {
        insertion_slot
    };
    let slot_width = pixels_to_f32(items[source_position].hit_bounds.width);

    (0..items.len())
        .map(|index| {
            if destination > source_position
                && index > source_position
                && index <= destination
            {
                -slot_width
            } else if destination < source_position
                && index >= destination
                && index < source_position
            {
                slot_width
            } else {
                0.0
            }
        })
        .collect()
}

struct DockLayoutView<'a>(&'a [LaidOutItem]);

impl DockLayoutView<'_> {
    fn insertion_slot(&self, x: i32) -> usize {
        self.0
            .iter()
            .position(|item| {
                i64::from(x)
                    < i64::from(item.bounds.left.saturating_add(item.bounds.width / 2))
            })
            .unwrap_or(self.0.len())
    }
}

struct OffsetMotion {
    from: f32,
    target: f32,
    started: Instant,
    moving: bool,
}

impl OffsetMotion {
    const fn new(now: Instant) -> Self {
        Self {
            from: 0.0,
            target: 0.0,
            started: now,
            moving: false,
        }
    }

    fn retarget(&mut self, target: f32, now: Instant) {
        if self.target.to_bits() == target.to_bits() {
            return;
        }
        self.from = self.sample(now);
        self.target = target;
        self.started = now;
        self.moving = true;
    }

    const fn snap(&mut self, target: f32, now: Instant) {
        self.from = target;
        self.target = target;
        self.started = now;
        self.moving = false;
    }

    fn sample(&self, now: Instant) -> f32 {
        if !self.moving {
            return self.target;
        }
        let progress = (now.saturating_duration_since(self.started).as_secs_f32()
            / REORDER_DURATION.as_secs_f32())
        .clamp(0.0, 1.0);
        self.from + (self.target - self.from) * ease_out_cubic(progress)
    }

    fn is_animating(&self, now: Instant) -> bool {
        self.moving && now.saturating_duration_since(self.started) < REORDER_DURATION
    }
}

struct ItemMotion {
    hover: AnimationTrack,
    press: AnimationTrack,
}

impl ItemMotion {
    fn new(now: Instant) -> Self {
        Self {
            hover: AnimationTrack::new(now, HOVER_DURATION),
            press: AnimationTrack::new(now, PRESS_DURATION),
        }
    }

    fn sample(&mut self, now: Instant, hovered: bool, pressed: bool) -> (ItemVisual, bool) {
        self.hover.retarget(hovered, now);
        self.press.retarget(pressed, now);
        let hover = self.hover.sample(now);
        let press = self.press.sample(now);
        let hover_translate_y = hover * -2.5;
        (
            ItemVisual {
                scale: 1.0 + (0.95 - 1.0) * press,
                translate_y: hover_translate_y + (1.0 - hover_translate_y) * press,
                icon_opacity: 1.0 - press * 0.10,
            },
            self.hover.is_animating(now) || self.press.is_animating(now),
        )
    }
}

struct AnimationTrack {
    from: f32,
    active: bool,
    moving: bool,
    started: Instant,
    duration: Duration,
}

impl AnimationTrack {
    const fn new(now: Instant, duration: Duration) -> Self {
        Self {
            from: 0.0,
            active: false,
            moving: false,
            started: now,
            duration,
        }
    }

    fn retarget(&mut self, active: bool, now: Instant) {
        if self.active == active {
            return;
        }
        self.from = self.sample(now);
        self.active = active;
        self.moving = true;
        self.started = now;
    }

    fn sample(&self, now: Instant) -> f32 {
        let target = if self.active {
            1.0
        } else {
            0.0
        };
        if !self.moving {
            return target;
        }
        let elapsed = now.saturating_duration_since(self.started);
        let progress =
            (elapsed.as_secs_f32() / self.duration.as_secs_f32()).clamp(0.0, 1.0);
        self.from + (target - self.from) * ease_out_cubic(progress)
    }

    fn is_animating(&self, now: Instant) -> bool {
        self.moving && now.saturating_duration_since(self.started) < self.duration
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ItemVisual {
    scale: f32,
    translate_y: f32,
    icon_opacity: f32,
}

struct ItemDraw<'a> {
    is_dragged: bool,
    bitmap: &'a ID2D1Bitmap1,
    visual: ItemVisual,
    bounds: D2D_RECT_F,
    interpolation: D2D1_INTERPOLATION_MODE,
    badge: Option<DockBadge>,
    running: Option<D2D1_ROUNDED_RECT>,
}

fn running_indicator(
    bounds: D2D_RECT_F,
    surface_height: u32,
    dpi: u32,
) -> D2D1_ROUNDED_RECT {
    let scale = f32::from(u16::try_from(dpi).unwrap_or(u16::MAX)) / TARGET_DPI;
    let width = 8.0 * scale;
    let height = 2.0 * scale;
    let center = f32::midpoint(bounds.left, bounds.right);
    let bottom = pixels_to_f32(surface_height);

    D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left: center - width * 0.5,
            top: bottom - height,
            right: center + width * 0.5,
            bottom,
        },
        radiusX: height * 0.5,
        radiusY: height * 0.5,
    }
}

fn ease_out_cubic(value: f32) -> f32 {
    1.0 - (1.0 - value).powi(3)
}

#[derive(Debug, Error)]
pub(super) enum RendererError {
    #[error(transparent)]
    Asset(#[from] AssetError),
    #[error("uploaded bitmap disappeared from the graphics cache")]
    BitmapCacheInvariant,
    #[error(transparent)]
    Windows(#[from] WindowsError),
}

fn target_properties() -> D2D1_BITMAP_PROPERTIES1 {
    D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: TARGET_DPI,
        dpiY: TARGET_DPI,
        bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
        ..D2D1_BITMAP_PROPERTIES1::default()
    }
}

fn source_bitmap_properties() -> D2D1_BITMAP_PROPERTIES1 {
    D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: TARGET_DPI,
        dpiY: TARGET_DPI,
        bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
        ..D2D1_BITMAP_PROPERTIES1::default()
    }
}

fn upload_bitmap(
    context: &ID2D1DeviceContext,
    raster: &RasterImage,
) -> Result<ID2D1Bitmap1, RendererError> {
    let size = raster.size();
    let properties = source_bitmap_properties();
    // SAFETY: The source slice contains `stride * height` premultiplied BGRA
    // bytes and remains alive for the synchronous copy. The dimensions and
    // pixel format exactly match the CPU raster.
    unsafe {
        Ok(context.CreateBitmap(
            D2D_SIZE_U {
                width: size.width(),
                height: size.height(),
            },
            Some(raster.pixels().as_ptr().cast::<c_void>()),
            raster.stride()?,
            &raw const properties,
        )?)
    }
}

fn upload_raster_icon(
    context: &ID2D1DeviceContext,
    raster: &RasterIcon,
) -> Result<ID2D1Bitmap1, RendererError> {
    upload_pixels(
        context,
        raster.width(),
        raster.height(),
        raster.pixels(),
        raster.stride(),
    )
}

fn upload_pixels(
    context: &ID2D1DeviceContext,
    width: u32,
    height: u32,
    pixels: &[u8],
    stride: u32,
) -> Result<ID2D1Bitmap1, RendererError> {
    let properties = source_bitmap_properties();
    // SAFETY: The validated source slice contains tightly packed premultiplied
    // BGRA bytes and remains alive for the synchronous Direct2D copy.
    unsafe {
        Ok(context.CreateBitmap(
            D2D_SIZE_U { width, height },
            Some(pixels.as_ptr().cast::<c_void>()),
            stride,
            &raw const properties,
        )?)
    }
}

fn raster_key(raster: &RasterIcon) -> (RasterIconId, u32, u32) {
    (raster.id().clone(), raster.width(), raster.height())
}

fn mascot_interpolation(icon: &DockIcon) -> D2D1_INTERPOLATION_MODE {
    match icon {
        DockIcon::Embedded(_) => D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
        DockIcon::Raster(_) => D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
    }
}

fn icon_interpolation(icon: &DockIcon, target_size: NonZeroU32) -> D2D1_INTERPOLATION_MODE {
    match icon {
        DockIcon::Raster(raster)
            if raster.width() != target_size.get()
                || raster.height() != target_size.get() =>
        {
            D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC
        }
        DockIcon::Embedded(_) | DockIcon::Raster(_) => {
            D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR
        }
    }
}

fn fitted_mascot_bounds(icon: &DockIcon, bounds: D2D_RECT_F) -> D2D_RECT_F {
    let DockIcon::Raster(raster) = icon else {
        return bounds;
    };
    let available_width = bounds.right - bounds.left;
    let available_height = bounds.bottom - bounds.top;
    let aspect = pixels_to_f32(raster.width()) / pixels_to_f32(raster.height());
    let (width, height) = if available_width / available_height > aspect {
        (available_height * aspect, available_height)
    } else {
        (available_width, available_width / aspect)
    };
    let center_x = f32::midpoint(bounds.left, bounds.right);
    let center_y = f32::midpoint(bounds.top, bounds.bottom);
    D2D_RECT_F {
        left: center_x - width * 0.5,
        top: center_y - height * 0.5,
        right: center_x + width * 0.5,
        bottom: center_y + height * 0.5,
    }
}

fn dock_rectangle(
    size: SurfaceSize,
    radius: f32,
    width: f32,
    anchor: DockAnchor,
) -> D2D1_ROUNDED_RECT {
    let surface_width = pixels_to_f32(size.width());
    let left = match anchor {
        DockAnchor::Left => 0.0,
        DockAnchor::Center => (surface_width - width) * 0.5,
        DockAnchor::Right => surface_width - width,
    };
    D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left,
            top: 0.0,
            right: left + width,
            bottom: pixels_to_f32(size.height()),
        },
        radiusX: radius,
        radiusY: radius,
    }
}

fn pixel_rectangle(rectangle: PixelRect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: pixels_to_f32(rectangle.left),
        top: pixels_to_f32(rectangle.top),
        right: pixels_to_f32(rectangle.left.saturating_add(rectangle.width)),
        bottom: pixels_to_f32(rectangle.top.saturating_add(rectangle.height)),
    }
}

fn centered_text_format(
    factory: &IDWriteFactory,
    size: f32,
) -> Result<IDWriteTextFormat, WindowsError> {
    // SAFETY: Static family and locale strings are NUL terminated.
    let format = unsafe {
        factory.CreateTextFormat(
            w!("Segoe UI Variable Text"),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("en-us"),
        )?
    };
    // SAFETY: The newly created format accepts these documented layout values.
    unsafe {
        format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
    }
    Ok(format)
}

fn media_text_format(
    factory: &IDWriteFactory,
    size: f32,
) -> Result<IDWriteTextFormat, WindowsError> {
    // SAFETY: Static family and locale strings are NUL terminated.
    let format = unsafe {
        factory.CreateTextFormat(
            w!("Segoe UI Variable Text"),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("en-us"),
        )?
    };
    // SAFETY: The newly created format accepts these documented layout values.
    unsafe {
        format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING)?;
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
    }
    Ok(format)
}

fn status_opacity(interaction: DockInteractionState, target: DockHitTarget) -> f32 {
    if interaction.pressed == Some(target) {
        0.62
    } else if interaction.hovered == Some(target) {
        1.0
    } else {
        0.78
    }
}

fn nonzero_or_one(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap_or(NonZeroU32::MIN)
}

fn inset_rectangle(rectangle: PixelRect, numerator: u32) -> PixelRect {
    let inset = rectangle.width.saturating_mul(numerator) / 28;
    PixelRect {
        left: rectangle.left.saturating_add(inset),
        top: rectangle.top.saturating_add(inset),
        width: rectangle.width.saturating_sub(inset.saturating_mul(2)),
        height: rectangle.height.saturating_sub(inset.saturating_mul(2)),
    }
}

fn scaled_pixel_rectangle(rectangle: PixelRect, scale: f32) -> D2D_RECT_F {
    let original = pixel_rectangle(rectangle);
    let center_x = f32::midpoint(original.left, original.right);
    let center_y = f32::midpoint(original.top, original.bottom);
    let half_width = (original.right - original.left) * 0.5 * scale;
    let half_height = (original.bottom - original.top) * 0.5 * scale;
    D2D_RECT_F {
        left: center_x - half_width,
        top: center_y - half_height,
        right: center_x + half_width,
        bottom: center_y + half_height,
    }
}

fn translated_scaled_pixel_rectangle(
    rectangle: PixelRect,
    scale: f32,
    offset_x: f32,
    offset_y: f32,
) -> D2D_RECT_F {
    let mut translated = scaled_pixel_rectangle(rectangle, scale);
    translated.left += offset_x;
    translated.right += offset_x;
    translated.top += offset_y;
    translated.bottom += offset_y;
    translated
}

fn scale_dip_offset(offset: f32, dpi: u32) -> f32 {
    let bounded_dpi = u16::try_from(dpi).unwrap_or(u16::MAX);
    offset * f32::from(bounded_dpi) / TARGET_DPI
}

fn dragged_rectangle(
    pointer_x: i32,
    pointer_y: i32,
    side: u32,
    scale: f32,
    surface: SurfaceSize,
) -> D2D_RECT_F {
    let half = pixels_to_f32(side) * scale * 0.5;
    let center_x = clamped_drag_center(pointer_x, surface.width(), half);
    let center_y = clamped_drag_center(pointer_y, surface.height(), half);
    D2D_RECT_F {
        left: center_x - half,
        top: center_y - half,
        right: center_x + half,
        bottom: center_y + half,
    }
}

fn clamped_drag_center(pointer: i32, extent: u32, half_side: f32) -> f32 {
    let extent = pixels_to_f32(extent);
    if extent <= half_side * 2.0 {
        extent * 0.5
    } else {
        signed_pixels_to_f32(pointer).clamp(half_side, extent - half_side)
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "captured pointer coordinates remain below f32 exact range"
)]
const fn signed_pixels_to_f32(value: i32) -> f32 {
    value as f32
}

fn rounded_pixel_rectangle(rectangle: PixelRect, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect: pixel_rectangle(rectangle),
        radiusX: radius,
        radiusY: radius,
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "window dimensions are far below f32's exact integer range"
)]
const fn pixels_to_f32(value: u32) -> f32 {
    value as f32
}
