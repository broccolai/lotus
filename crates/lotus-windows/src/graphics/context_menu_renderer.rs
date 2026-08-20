use std::collections::HashMap;
use std::num::NonZeroU32;

use lotus_ui::geometry::{PhysicalRect, physical_rect};
use lotus_ui::icon::{Icon, RasterIcon, RasterIconId};
use lotus_ui::theme::Theme;
use thiserror::Error;
use windows::Win32::Foundation::D2DERR_RECREATE_TARGET;
use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D1_COLOR_F};
use windows::Win32::Graphics::Direct2D::{
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_DRAW_TEXT_OPTIONS_CLIP,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
    D2D1_ROUNDED_RECT, D2D1CreateFactory, ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext,
    ID2D1Factory1, ID2D1Image, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::{IDXGISurface, IDXGISwapChain1};
use windows::core::{Error as WindowsError, w};

use super::assets::{AssetError, IconTint, RasterSize, SvgAsset, SvgAssetCache};
use super::device::GraphicsDevice;
use super::resources::{raster_key, target_bitmap_properties, upload_bgra_pixels};
use super::surface::SurfaceSize;
use super::{ContextMenuScene, PopupEntry, PopupIcon, PopupSymbol, theme};

const TARGET_DPI: f32 = 96.0;
const TRANSPARENT: D2D1_COLOR_F = rgba8(0, 0, 0, 0);

pub(super) enum ContextMenuDrawResult {
    Complete,
    RecreateTarget,
}

pub(super) struct ContextMenuRenderer {
    _factory: ID2D1Factory1,
    _device: ID2D1Device,
    context: ID2D1DeviceContext,
    target: Option<ID2D1Bitmap1>,
    panel: ID2D1SolidColorBrush,
    highlight: ID2D1SolidColorBrush,
    active: ID2D1SolidColorBrush,
    text: ID2D1SolidColorBrush,
    write_factory: IDWriteFactory,
    text_formats: HashMap<u32, IDWriteTextFormat>,
    assets: SvgAssetCache,
    icon_tint: IconTint,
    embedded: HashMap<(SvgAsset, NonZeroU32), ID2D1Bitmap1>,
    rasters: HashMap<(RasterIconId, u32, u32), ID2D1Bitmap1>,
}

impl ContextMenuRenderer {
    pub(super) fn create(
        graphics: &GraphicsDevice,
        swap_chain: &IDXGISwapChain1,
    ) -> Result<Self, ContextMenuRendererError> {
        let dxgi = graphics.dxgi_device()?;
        let factory: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let device = unsafe { factory.CreateDevice(&dxgi)? };
        let context =
            unsafe { device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)? };
        let write_factory: IDWriteFactory =
            unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let theme = Theme::default();
        let mut renderer = Self {
            _factory: factory,
            _device: device,
            context: context.clone(),
            target: None,
            panel: brush(&context, &theme::d2d(theme.chrome_overlay))?,
            highlight: brush(&context, &theme::d2d(theme.control_hover))?,
            active: brush(&context, &theme::d2d(theme.control_selected))?,
            text: brush(&context, &theme::d2d(theme.text))?,
            write_factory,
            text_formats: HashMap::new(),
            assets: SvgAssetCache::create()?,
            icon_tint: IconTint::from_color(theme.text),
            embedded: HashMap::new(),
            rasters: HashMap::new(),
        };
        renderer.attach_target(swap_chain)?;
        Ok(renderer)
    }

    pub(super) fn detach_target(&mut self) {
        unsafe { self.context.SetTarget(None::<&ID2D1Image>) };
        self.target = None;
    }

    pub(super) fn attach_target(
        &mut self,
        chain: &IDXGISwapChain1,
    ) -> Result<(), WindowsError> {
        self.detach_target();
        let surface: IDXGISurface = unsafe { chain.GetBuffer(0)? };
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
        scene: &ContextMenuScene,
    ) -> Result<ContextMenuDrawResult, ContextMenuRendererError> {
        debug_assert!(self.target.is_some());
        self.apply_theme(&scene.theme());
        self.sync_icon_tint(scene.theme().text);
        let entries = scene.entries();
        let icon_size = nonzero((20 * scene.dpi()).div_ceil(96));
        let fallback_size = nonzero((42 * scene.dpi()).div_ceil(96));
        for entry in &entries {
            let size = if entry.preview.is_some() {
                fallback_size
            } else {
                icon_size
            };
            self.ensure_popup_icon(&entry.icon, size)?;
        }
        self.ensure_embedded(SvgAsset::FluentDismiss, icon_size)?;
        if scene.picker_navigation().is_some() {
            self.ensure_embedded(SvgAsset::FluentPrevious, icon_size)?;
            self.ensure_embedded(SvgAsset::FluentNext, icon_size)?;
        }
        let format = self.text_format(scene.dpi())?;
        let panel = rounded(
            D2D_RECT_F {
                left: 0.5,
                top: 0.5,
                right: as_f32(size.width()) - 0.5,
                bottom: as_f32(size.height()) - 0.5,
            },
            scale(scene, scene.theme().radii.panel),
        );
        let transparent = TRANSPARENT;

        let result = unsafe {
            self.context.BeginDraw();
            self.context.Clear(Some(&raw const transparent));
            self.context
                .FillRoundedRectangle(&raw const panel, &self.panel);
            for entry in &entries {
                self.draw_entry(entry, scene, &format, icon_size, fallback_size)?;
            }
            self.draw_picker_navigation(scene, size, icon_size)?;
            self.context.EndDraw(None, None)
        };
        match result {
            Ok(()) => Ok(ContextMenuDrawResult::Complete),
            Err(error) if error.code() == D2DERR_RECREATE_TARGET => {
                Ok(ContextMenuDrawResult::RecreateTarget)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn draw_picker_navigation(
        &self,
        scene: &ContextMenuScene,
        size: SurfaceSize,
        icon_size: NonZeroU32,
    ) -> Result<(), ContextMenuRendererError> {
        let Some((previous, next)) = scene.picker_navigation() else {
            return Ok(());
        };
        let diameter = (28 * scene.dpi()).div_ceil(96);
        let top = size.height().saturating_sub(diameter) / 2;
        if previous {
            self.draw_navigation_icon(
                SvgAsset::FluentPrevious,
                physical_rect(2, top, diameter, diameter),
                icon_size,
            )?;
        }
        if next {
            self.draw_navigation_icon(
                SvgAsset::FluentNext,
                physical_rect(
                    size.width().saturating_sub(diameter + 2),
                    top,
                    diameter,
                    diameter,
                ),
                icon_size,
            )?;
        }
        Ok(())
    }

    fn draw_navigation_icon(
        &self,
        asset: SvgAsset,
        bounds: PhysicalRect,
        icon_size: NonZeroU32,
    ) -> Result<(), ContextMenuRendererError> {
        let background = rounded(rect(bounds), as_f32(bounds.height()) / 2.0);
        unsafe {
            self.context
                .FillRoundedRectangle(&raw const background, &self.highlight);
        };
        let bitmap = self.embedded_bitmap(asset, icon_size)?;
        let destination = centered_rect(bounds, icon_size.get());
        unsafe {
            self.context.DrawBitmap(
                bitmap,
                Some(&raw const destination),
                1.0,
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                None,
                None,
            );
        };
        Ok(())
    }

    fn draw_entry(
        &self,
        entry: &PopupEntry<SvgAsset>,
        scene: &ContextMenuScene,
        format: &IDWriteTextFormat,
        icon_size: NonZeroU32,
        fallback_size: NonZeroU32,
    ) -> Result<(), ContextMenuRendererError> {
        let bounds = rect(entry.bounds);
        let radius = scale(scene, scene.theme().radii.control);
        if entry.highlighted {
            let highlight = rounded(bounds, radius);
            unsafe {
                self.context
                    .FillRoundedRectangle(&raw const highlight, &self.highlight);
            };
        }
        if entry.active {
            let active = rounded(bounds, radius);
            unsafe {
                self.context.DrawRoundedRectangle(
                    &raw const active,
                    &self.active,
                    1.0,
                    None,
                );
            };
        }

        let artwork_size = if entry.preview.is_some() {
            fallback_size
        } else {
            icon_size
        };
        let icon = self.popup_bitmap(&entry.icon, artwork_size)?;
        let icon_bounds = icon_bounds(entry, artwork_size);
        unsafe {
            self.context.DrawBitmap(
                icon,
                Some(&raw const icon_bounds),
                1.0,
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                None,
                None,
            );
        };
        self.draw_label(entry, format);
        self.draw_close(entry, icon_size)?;
        Ok(())
    }

    fn draw_label(&self, entry: &PopupEntry<SvgAsset>, format: &IDWriteTextFormat) {
        if entry.label.is_empty() {
            return;
        }
        let mut bounds = rect(entry.bounds);
        let height = bounds.bottom - bounds.top;
        if let Some(preview) = entry.preview {
            bounds.left += 12.0;
            bounds.bottom = as_f32(preview.min_y());
        } else {
            bounds.left += height;
        }
        if let Some(close) = entry.close {
            bounds.right = as_f32(close.min_x().saturating_sub(4));
        }
        let text = entry.label.encode_utf16().collect::<Vec<_>>();
        unsafe {
            self.context.DrawText(
                &text,
                format,
                &raw const bounds,
                &self.text,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        };
    }

    fn draw_close(
        &self,
        entry: &PopupEntry<SvgAsset>,
        icon_size: NonZeroU32,
    ) -> Result<(), ContextMenuRendererError> {
        let Some(close) = entry.close.filter(|_| entry.highlighted) else {
            return Ok(());
        };
        if entry.close_highlighted {
            let highlight = rounded(rect(close), as_f32(close.height()) * 0.25);
            unsafe {
                self.context
                    .FillRoundedRectangle(&raw const highlight, &self.highlight);
            };
        }
        let bitmap = self.embedded_bitmap(SvgAsset::FluentDismiss, icon_size)?;
        let bounds = centered_rect(close, icon_size.get());
        unsafe {
            self.context.DrawBitmap(
                bitmap,
                Some(&raw const bounds),
                1.0,
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                None,
                None,
            );
        };
        Ok(())
    }

    fn apply_theme(&self, value: &Theme) {
        theme::set(&self.panel, value.chrome_overlay);
        theme::set(&self.highlight, value.control_hover);
        theme::set(&self.active, value.control_selected);
        theme::set(&self.text, value.text);
    }

    fn sync_icon_tint(&mut self, color: lotus_ui::theme::Color) {
        let tint = IconTint::from_color(color);
        if self.icon_tint != tint {
            self.icon_tint = tint;
            self.embedded.clear();
        }
    }

    fn text_format(&mut self, dpi: u32) -> Result<IDWriteTextFormat, WindowsError> {
        if let Some(format) = self.text_formats.get(&dpi) {
            return Ok(format.clone());
        }
        let size = 13.5 * as_f32(dpi) / TARGET_DPI;
        let format = unsafe {
            self.write_factory.CreateTextFormat(
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
        self.text_formats.insert(dpi, format.clone());
        Ok(format)
    }

    fn ensure_popup_icon(
        &mut self,
        icon: &PopupIcon<SvgAsset>,
        size: NonZeroU32,
    ) -> Result<(), ContextMenuRendererError> {
        match icon {
            PopupIcon::Symbol(symbol) => self.ensure_embedded(symbol_asset(*symbol), size),
            PopupIcon::Artwork(Icon::Embedded(asset)) => self.ensure_embedded(*asset, size),
            PopupIcon::Artwork(Icon::Raster(raster)) => self.ensure_raster(raster),
        }
    }

    fn ensure_embedded(
        &mut self,
        asset: SvgAsset,
        size: NonZeroU32,
    ) -> Result<(), ContextMenuRendererError> {
        let key = (asset, size);
        if self.embedded.contains_key(&key) {
            return Ok(());
        }
        let raster =
            self.assets
                .rasterize(asset, RasterSize::square(size), self.icon_tint)?;
        let bitmap = upload_bgra_pixels(
            &self.context,
            raster.size().width(),
            raster.size().height(),
            raster.pixels(),
            raster.stride()?,
        )?;
        self.embedded.insert(key, bitmap);
        Ok(())
    }

    fn ensure_raster(
        &mut self,
        raster: &RasterIcon,
    ) -> Result<(), ContextMenuRendererError> {
        let key = raster_key(raster);
        if self.rasters.contains_key(&key) {
            return Ok(());
        }
        let bitmap = upload_bgra_pixels(
            &self.context,
            raster.width(),
            raster.height(),
            raster.pixels(),
            raster.stride(),
        )?;
        self.rasters.insert(key, bitmap);
        Ok(())
    }

    fn popup_bitmap(
        &self,
        icon: &PopupIcon<SvgAsset>,
        size: NonZeroU32,
    ) -> Result<&ID2D1Bitmap1, ContextMenuRendererError> {
        match icon {
            PopupIcon::Symbol(symbol) => self.embedded_bitmap(symbol_asset(*symbol), size),
            PopupIcon::Artwork(Icon::Embedded(asset)) => self.embedded_bitmap(*asset, size),
            PopupIcon::Artwork(Icon::Raster(raster)) => self
                .rasters
                .get(&raster_key(raster))
                .ok_or(ContextMenuRendererError::BitmapCacheInvariant),
        }
    }

    fn embedded_bitmap(
        &self,
        asset: SvgAsset,
        size: NonZeroU32,
    ) -> Result<&ID2D1Bitmap1, ContextMenuRendererError> {
        self.embedded
            .get(&(asset, size))
            .ok_or(ContextMenuRendererError::BitmapCacheInvariant)
    }
}

#[derive(Debug, Error)]
pub(super) enum ContextMenuRendererError {
    #[error(transparent)]
    Asset(#[from] AssetError),
    #[error("uploaded popup icon disappeared from the graphics cache")]
    BitmapCacheInvariant,
    #[error(transparent)]
    Windows(#[from] WindowsError),
}

fn brush(
    context: &ID2D1DeviceContext,
    color: &D2D1_COLOR_F,
) -> Result<ID2D1SolidColorBrush, WindowsError> {
    unsafe { context.CreateSolidColorBrush(color, None) }
}

fn icon_bounds(entry: &PopupEntry<SvgAsset>, size: NonZeroU32) -> D2D_RECT_F {
    if let Some(preview) = entry.preview {
        centered_rect(preview, size.get())
    } else if entry.label.is_empty() {
        centered_rect(entry.bounds, size.get())
    } else {
        let inset = entry.bounds.height().saturating_sub(size.get()) / 2;
        D2D_RECT_F {
            left: as_f32(entry.bounds.min_x().saturating_add(inset)),
            top: as_f32(entry.bounds.min_y().saturating_add(inset)),
            right: as_f32(entry.bounds.min_x().saturating_add(inset + size.get())),
            bottom: as_f32(entry.bounds.min_y().saturating_add(inset + size.get())),
        }
    }
}

fn centered_rect(bounds: PhysicalRect, size: u32) -> D2D_RECT_F {
    let left = bounds
        .min_x()
        .saturating_add(bounds.width().saturating_sub(size) / 2);
    let top = bounds
        .min_y()
        .saturating_add(bounds.height().saturating_sub(size) / 2);
    D2D_RECT_F {
        left: as_f32(left),
        top: as_f32(top),
        right: as_f32(left.saturating_add(size)),
        bottom: as_f32(top.saturating_add(size)),
    }
}

fn rect(value: PhysicalRect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: as_f32(value.min_x()),
        top: as_f32(value.min_y()),
        right: as_f32(value.max_x()),
        bottom: as_f32(value.max_y()),
    }
}

const fn rounded(rect: D2D_RECT_F, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect,
        radiusX: radius,
        radiusY: radius,
    }
}

const fn symbol_asset(symbol: PopupSymbol) -> SvgAsset {
    match symbol {
        PopupSymbol::Power => SvgAsset::FluentPower,
        PopupSymbol::Lock => SvgAsset::FluentLock,
        PopupSymbol::Restart => SvgAsset::FluentRestart,
        PopupSymbol::Settings => SvgAsset::FluentSettings,
        PopupSymbol::Quit | PopupSymbol::Close => SvgAsset::FluentDismiss,
        PopupSymbol::Open | PopupSymbol::Image => SvgAsset::FluentOpen,
        PopupSymbol::Pin => SvgAsset::FluentPin,
        PopupSymbol::Unpin => SvgAsset::FluentPinOff,
    }
}

fn scale(scene: &ContextMenuScene, dips: f32) -> f32 {
    as_f32(scene.dpi()) * dips / TARGET_DPI
}

fn nonzero(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap_or(NonZeroU32::MIN)
}

#[allow(
    clippy::cast_precision_loss,
    reason = "popup dimensions remain below f32 exact range"
)]
const fn as_f32(value: u32) -> f32 {
    value as f32
}

const fn rgba8(red: u8, green: u8, blue: u8, alpha: u8) -> D2D1_COLOR_F {
    const MAX: f32 = 255.0;
    D2D1_COLOR_F {
        r: red as f32 / MAX,
        g: green as f32 / MAX,
        b: blue as f32 / MAX,
        a: alpha as f32 / MAX,
    }
}
