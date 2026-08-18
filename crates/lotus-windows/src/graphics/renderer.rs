use std::collections::HashMap;
use std::num::NonZeroU32;
use std::time::Instant;

use lotus_ui::theme::Theme;
use thiserror::Error;
use windows::Win32::Foundation::D2DERR_RECREATE_TARGET;
use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D1_COLOR_F};
use windows::Win32::Graphics::Direct2D::{
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_INTERPOLATION_MODE, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
    D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_ROUNDED_RECT, D2D1CreateFactory,
    ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext, ID2D1Factory1, ID2D1Image,
    ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory,
    IDWriteFactory, IDWriteFontCollection, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::{IDXGISurface, IDXGISwapChain1};
use windows::core::{Error as WindowsError, w};

use super::assets::{AssetError, IconTint, SvgAsset, SvgAssetCache};
use super::device::GraphicsDevice;
use super::resources::target_bitmap_properties;
use super::scene::{
    DockBadge, DockHitTarget, DockIcon, DockInteractionState, DockLayout, DockScene,
    RasterIconId,
};
use super::surface::SurfaceSize;
use super::theme;

mod animation;
mod content;
mod geometry;
mod resources;

use animation::{
    ChromeAnimator, ExitAnimator, InteractionAnimator, ItemVisual, ReorderAnimator,
};
use geometry::{
    dock_rectangle, fitted_mascot_bounds, rounded_pixel_rectangle, scale_dip_offset,
    translated_scaled_pixel_rectangle,
};

const TARGET_DPI: f32 = 96.0;
const DIVIDER_CORNER_RADIUS: f32 = 1.0;
const HOVER_DURATION: std::time::Duration = std::time::Duration::from_millis(145);
const PRESS_DURATION: std::time::Duration = std::time::Duration::from_millis(80);
const REORDER_DURATION: std::time::Duration = std::time::Duration::from_millis(180);
const CHROME_RESIZE_DURATION: std::time::Duration = std::time::Duration::from_millis(90);
const CHROME_RESIZE_DISTANCE_DIP: f32 = 10.0;
const EXIT_DURATION: std::time::Duration = std::time::Duration::from_millis(80);

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
    symbol: IDWriteTextFormat,
}

#[derive(Clone)]
struct MediaTextFormats {
    title: IDWriteTextFormat,
    artist: IDWriteTextFormat,
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
    icon_tint: IconTint,
    embedded_bitmaps: HashMap<(SvgAsset, NonZeroU32), ID2D1Bitmap1>,
    raster_bitmaps: HashMap<(RasterIconId, u32, u32), ID2D1Bitmap1>,
}

impl Direct2DRenderer {
    pub(super) fn create(
        graphics: &GraphicsDevice,
        swap_chain: &IDXGISwapChain1,
    ) -> Result<Self, RendererError> {
        let dxgi_device = graphics.dxgi_device()?;

        let factory: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let device = unsafe { factory.CreateDevice(&dxgi_device)? };
        let context =
            unsafe { device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)? };
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
            icon_tint: IconTint::from_color(default_theme.text),
            embedded_bitmaps: HashMap::new(),
            raster_bitmaps: HashMap::new(),
        };
        renderer.attach_target(swap_chain)?;
        Ok(renderer)
    }

    pub(super) fn detach_target(&mut self) {
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

        let surface: IDXGISurface = unsafe { swap_chain.GetBuffer(0)? };
        let properties = target_bitmap_properties();
        let target = unsafe {
            self.context
                .CreateBitmapFromDxgiSurface(&surface, Some(&raw const properties))?
        };
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
        self.sync_icon_tint(theme.text);
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

    fn apply_theme(&self, value: &Theme) {
        theme::set(&self.dock_brush, value.chrome_overlay);
        theme::set(&self.divider_brush, value.divider);
        theme::set(&self.show_desktop_brush, value.control_hover);
        theme::set(&self.badge_brush, value.accent);
        theme::set(&self.badge_text_brush, value.on_accent);
        theme::set(&self.status_text_brush, value.text);
        theme::set(&self.status_muted_text_brush, value.text_muted);
    }

    fn sync_icon_tint(&mut self, color: lotus_ui::theme::Color) {
        let tint = IconTint::from_color(color);
        if self.icon_tint != tint {
            self.icon_tint = tint;
            self.embedded_bitmaps.clear();
        }
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
        let symbol = centered_symbol_format(&self.write_factory, 18.0 * scale)?;
        let formats = StatusTextFormats { time, date, symbol };
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
        unsafe {
            format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        }
        self.badge_formats.insert(dpi, format.clone());
        Ok(format)
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

#[derive(Debug, Error)]
pub(super) enum RendererError {
    #[error(transparent)]
    Asset(#[from] AssetError),
    #[error("uploaded bitmap disappeared from the graphics cache")]
    BitmapCacheInvariant,
    #[error(transparent)]
    Windows(#[from] WindowsError),
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

fn centered_text_format(
    factory: &IDWriteFactory,
    size: f32,
) -> Result<IDWriteTextFormat, WindowsError> {
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
    unsafe {
        format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
    }
    Ok(format)
}

fn centered_symbol_format(
    factory: &IDWriteFactory,
    size: f32,
) -> Result<IDWriteTextFormat, WindowsError> {
    let mut collection = None;
    unsafe { factory.GetSystemFontCollection(&raw mut collection, false)? };
    let collection = collection.ok_or_else(|| {
        WindowsError::new(
            windows::Win32::Foundation::E_FAIL,
            "DirectWrite returned no system font collection",
        )
    })?;
    let family = if system_font_family_exists(&collection, w!("Segoe Fluent Icons"))? {
        w!("Segoe Fluent Icons")
    } else {
        w!("Segoe MDL2 Assets")
    };
    let format = unsafe {
        factory.CreateTextFormat(
            family,
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("en-us"),
        )?
    };
    unsafe {
        format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
    }
    Ok(format)
}

fn system_font_family_exists(
    collection: &IDWriteFontCollection,
    family: windows::core::PCWSTR,
) -> Result<bool, WindowsError> {
    let mut index = 0;
    let mut exists = windows::core::BOOL(0);
    unsafe { collection.FindFamilyName(family, &raw mut index, &raw mut exists) }?;
    Ok(exists.as_bool())
}

fn media_text_format(
    factory: &IDWriteFactory,
    size: f32,
) -> Result<IDWriteTextFormat, WindowsError> {
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
