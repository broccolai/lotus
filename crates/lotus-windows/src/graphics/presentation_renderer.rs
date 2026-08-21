use std::mem::ManuallyDrop;

use lotus_ui::icon::{Icon, RasterIcon};
use lotus_ui::presentation::{
    FontFamily, FontWeight, HorizontalAlignment, ImageSampling, Presentation,
    PresentationPrimitive, PresentationRect, TextStyle, VerticalAlignment,
};
use lotus_ui::theme::Color;
use thiserror::Error;
use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, E_FAIL};
use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D1_COLOR_F};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_DEVICE_CONTEXT_OPTIONS_NONE,
    D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, D2D1_LAYER_OPTIONS1_NONE,
    D2D1_LAYER_PARAMETERS1, D2D1_ROUNDED_RECT, D2D1CreateFactory, ID2D1Bitmap1,
    ID2D1Device, ID2D1DeviceContext, ID2D1Factory1, ID2D1Image, ID2D1Layer,
    ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_PARAGRAPH_ALIGNMENT_FAR,
    DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory,
    IDWriteFontCollection, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::{IDXGISurface, IDXGISwapChain1};
use windows::core::{Error as WindowsError, Interface, w};

use super::assets::{AssetError, IconTint, RasterSize, SvgAsset, SvgAssetCache};
use super::device::GraphicsDevice;
use super::resources::{raster_key, target_bitmap_properties, upload_bgra_pixels};
use super::surface::SurfaceError;
use crate::font::BundledFontCollection;
use crate::resource_cache::BoundedResourceCache;

const BITMAP_CACHE_BYTES: usize = 16 * 1024 * 1024;

pub(super) enum PresentationDrawResult {
    Complete,
    RecreateTarget,
}

pub(super) struct PresentationRenderer {
    factory: ID2D1Factory1,
    _device: ID2D1Device,
    context: ID2D1DeviceContext,
    target: Option<ID2D1Bitmap1>,
    write_factory: IDWriteFactory,
    _bundled_fonts: BundledFontCollection,
    brand_collection: IDWriteFontCollection,
    modern_symbol_font: bool,
    brushes: BoundedResourceCache<ColorKey, ID2D1SolidColorBrush>,
    text_formats: BoundedResourceCache<TextFormatKey, IDWriteTextFormat>,
    assets: SvgAssetCache,
    embedded: BoundedResourceCache<EmbeddedKey, ID2D1Bitmap1>,
    rasters: BoundedResourceCache<(lotus_ui::icon::RasterIconId, u32, u32), ID2D1Bitmap1>,
}

impl PresentationRenderer {
    pub(super) fn create(
        graphics: &GraphicsDevice,
        swap_chain: &IDXGISwapChain1,
    ) -> Result<Self, PresentationRendererError> {
        let dxgi = graphics.dxgi_device()?;
        let factory: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let device = unsafe { factory.CreateDevice(&dxgi)? };
        let context =
            unsafe { device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)? };
        let write_factory: IDWriteFactory =
            unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let write_factory6 = write_factory.cast()?;
        let bundled_fonts = BundledFontCollection::create(&write_factory6)?;
        let brand_collection = bundled_fonts.collection().cast()?;
        let modern_symbol_font =
            system_font_family_exists(&write_factory, w!("Segoe Fluent Icons"))?;
        let mut renderer = Self {
            factory,
            _device: device,
            context,
            target: None,
            write_factory,
            _bundled_fonts: bundled_fonts,
            brand_collection,
            modern_symbol_font,
            brushes: BoundedResourceCache::new(64),
            text_formats: BoundedResourceCache::new(32),
            assets: SvgAssetCache::create()?,
            embedded: BoundedResourceCache::new(BITMAP_CACHE_BYTES),
            rasters: BoundedResourceCache::new(BITMAP_CACHE_BYTES),
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
        presentation: &Presentation<SvgAsset>,
    ) -> Result<PresentationDrawResult, PresentationRendererError> {
        debug_assert!(self.target.is_some());
        let clear = d2d(presentation.clear);
        unsafe {
            self.context.BeginDraw();
            self.context.Clear(Some(&raw const clear));
        }
        for primitive in &presentation.primitives {
            self.draw_primitive(primitive)?;
        }
        let result = unsafe { self.context.EndDraw(None, None) };
        match result {
            Ok(()) => Ok(PresentationDrawResult::Complete),
            Err(error) if error.code() == D2DERR_RECREATE_TARGET => {
                Ok(PresentationDrawResult::RecreateTarget)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn draw_primitive(
        &mut self,
        primitive: &PresentationPrimitive<SvgAsset>,
    ) -> Result<(), PresentationRendererError> {
        match primitive {
            PresentationPrimitive::PushClip { bounds } => {
                let bounds = rect(*bounds);
                unsafe {
                    self.context.PushAxisAlignedClip(
                        &raw const bounds,
                        D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    );
                }
            }
            PresentationPrimitive::PopClip => unsafe {
                self.context.PopAxisAlignedClip();
            },
            PresentationPrimitive::FillRoundedRect {
                bounds,
                radius,
                color,
            } => {
                let brush = self.brush(*color)?;
                let rounded = rounded(*bounds, *radius);
                unsafe {
                    self.context
                        .FillRoundedRectangle(&raw const rounded, &brush);
                }
            }
            PresentationPrimitive::StrokeRoundedRect {
                bounds,
                radius,
                width,
                color,
            } => {
                let brush = self.brush(*color)?;
                let rounded = rounded(*bounds, *radius);
                unsafe {
                    self.context.DrawRoundedRectangle(
                        &raw const rounded,
                        &brush,
                        *width,
                        None,
                    );
                }
            }
            PresentationPrimitive::Text {
                value,
                bounds,
                style,
                color,
            } => self.draw_text(value, *bounds, *style, *color)?,
            PresentationPrimitive::TextCaret {
                before,
                bounds,
                style,
                top_inset,
                bottom_inset,
                width,
                color,
            } => self.draw_text_caret(TextCaretDraw {
                before,
                bounds: *bounds,
                style: *style,
                top_inset: *top_inset,
                bottom_inset: *bottom_inset,
                width: *width,
                color: *color,
            })?,
            PresentationPrimitive::Icon {
                icon,
                bounds,
                tint,
                opacity,
                sampling,
                radius,
            } => self.draw_icon(icon, *bounds, *tint, *opacity, *sampling, *radius)?,
        }
        Ok(())
    }

    fn draw_text_caret(&mut self, caret: TextCaretDraw<'_>) -> Result<(), WindowsError> {
        let format = self.text_format(caret.style)?;
        let text = caret.before.encode_utf16().collect::<Vec<_>>();
        let layout = unsafe {
            self.write_factory.CreateTextLayout(
                &text,
                &format,
                caret.bounds.width(),
                caret.bounds.height(),
            )?
        };
        let mut metrics = DWRITE_TEXT_METRICS::default();
        unsafe { layout.GetMetrics(&raw mut metrics)? };
        let left = (caret.bounds.left + metrics.widthIncludingTrailingWhitespace + 1.0)
            .min(caret.bounds.right);
        let bounds = PresentationRect::new(
            left,
            caret.bounds.top + caret.top_inset,
            left + caret.width,
            caret.bounds.bottom - caret.bottom_inset,
        );
        let brush = self.brush(caret.color)?;
        let bounds = rect(bounds);
        unsafe { self.context.FillRectangle(&raw const bounds, &brush) };
        Ok(())
    }

    fn draw_text(
        &mut self,
        value: &str,
        bounds: PresentationRect,
        style: TextStyle,
        color: Color,
    ) -> Result<(), WindowsError> {
        let format = self.text_format(style)?;
        let brush = self.brush(color)?;
        let bounds = rect(bounds);
        let text = value.encode_utf16().collect::<Vec<_>>();
        unsafe {
            self.context.DrawText(
                &text,
                &format,
                &raw const bounds,
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
        Ok(())
    }

    fn draw_icon(
        &mut self,
        icon: &Icon<SvgAsset>,
        bounds: PresentationRect,
        tint: Color,
        opacity: f32,
        sampling: ImageSampling,
        radius: f32,
    ) -> Result<(), PresentationRendererError> {
        let bitmap = match icon {
            Icon::Embedded(asset) => {
                let size = raster_size(bounds)?;
                let tint = IconTint::from_color(tint);
                let key = EmbeddedKey {
                    asset: *asset,
                    size,
                    tint,
                };
                self.ensure_embedded(key)?;
                self.embedded
                    .peek(&key)
                    .ok_or(PresentationRendererError::BitmapCacheInvariant)?
            }
            Icon::Raster(raster) => {
                self.ensure_raster(raster)?;
                self.rasters
                    .peek(&raster_key(raster))
                    .ok_or(PresentationRendererError::BitmapCacheInvariant)?
            }
        };
        let destination = rect(bounds);
        let clipped = radius > 0.0;
        unsafe {
            if clipped {
                let clip = rounded(bounds, radius);
                let geometry = self
                    .factory
                    .CreateRoundedRectangleGeometry(&raw const clip)?;
                let geometry = geometry.cast()?;
                let mut layer = D2D1_LAYER_PARAMETERS1 {
                    contentBounds: destination,
                    geometricMask: ManuallyDrop::new(Some(geometry)),
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
                Some(&raw const destination),
                opacity.clamp(0.0, 1.0),
                match sampling {
                    ImageSampling::Smooth => D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                    ImageSampling::PixelAligned => {
                        windows::Win32::Graphics::Direct2D::D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR
                    }
                },
                None,
                None,
            );
            if clipped {
                self.context.PopLayer();
            }
        }
        Ok(())
    }

    fn brush(&mut self, color: Color) -> Result<ID2D1SolidColorBrush, WindowsError> {
        let key = ColorKey::from(color);
        if let Some(brush) = self.brushes.get(&key) {
            return Ok(brush.clone());
        }
        let color = d2d(color);
        let brush = unsafe { self.context.CreateSolidColorBrush(&raw const color, None)? };
        self.brushes.insert(key, brush.clone(), 1);
        Ok(brush)
    }

    fn text_format(&mut self, style: TextStyle) -> Result<IDWriteTextFormat, WindowsError> {
        let key = TextFormatKey::from(style);
        if let Some(format) = self.text_formats.get(&key) {
            return Ok(format.clone());
        }
        let format = unsafe {
            self.write_factory.CreateTextFormat(
                match style.family {
                    FontFamily::Interface => w!("Segoe UI Variable Text"),
                    FontFamily::SystemSymbols if self.modern_symbol_font => {
                        w!("Segoe Fluent Icons")
                    }
                    FontFamily::SystemSymbols => w!("Segoe MDL2 Assets"),
                    FontFamily::Brand => w!("Fraunces"),
                },
                (style.family == FontFamily::Brand).then_some(&self.brand_collection),
                match style.weight {
                    FontWeight::Normal => DWRITE_FONT_WEIGHT_NORMAL,
                    FontWeight::Semibold => {
                        windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT_SEMI_BOLD
                    }
                },
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                style.size,
                w!("en-us"),
            )?
        };
        unsafe {
            format.SetTextAlignment(match style.horizontal {
                HorizontalAlignment::Leading => DWRITE_TEXT_ALIGNMENT_LEADING,
                HorizontalAlignment::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                HorizontalAlignment::Trailing => DWRITE_TEXT_ALIGNMENT_TRAILING,
            })?;
            format.SetParagraphAlignment(match style.vertical {
                VerticalAlignment::Top => DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
                VerticalAlignment::Center => DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
                VerticalAlignment::Bottom => DWRITE_PARAGRAPH_ALIGNMENT_FAR,
            })?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        }
        self.text_formats.insert(key, format.clone(), 1);
        Ok(format)
    }

    fn ensure_embedded(
        &mut self,
        key: EmbeddedKey,
    ) -> Result<(), PresentationRendererError> {
        if self.embedded.get(&key).is_some() {
            return Ok(());
        }
        let raster = self.assets.rasterize(key.asset, key.size, key.tint)?;
        let bitmap = upload_bgra_pixels(
            &self.context,
            raster.size().width(),
            raster.size().height(),
            raster.pixels(),
            raster.stride()?,
        )?;
        self.embedded.insert(key, bitmap, bitmap_bytes(key.size));
        Ok(())
    }

    fn ensure_raster(
        &mut self,
        raster: &RasterIcon,
    ) -> Result<(), PresentationRendererError> {
        let key = raster_key(raster);
        if self.rasters.get(&key).is_some() {
            return Ok(());
        }
        let bitmap = upload_bgra_pixels(
            &self.context,
            raster.width(),
            raster.height(),
            raster.pixels(),
            raster.stride(),
        )?;
        let bytes = usize::try_from(raster.width())
            .unwrap_or(usize::MAX)
            .saturating_mul(usize::try_from(raster.height()).unwrap_or(usize::MAX))
            .saturating_mul(4);
        self.rasters.insert(key, bitmap, bytes);
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct TextCaretDraw<'a> {
    before: &'a str,
    bounds: PresentationRect,
    style: TextStyle,
    top_inset: f32,
    bottom_inset: f32,
    width: f32,
    color: Color,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ColorKey([u32; 4]);

impl From<Color> for ColorKey {
    fn from(color: Color) -> Self {
        Self([
            color.red.to_bits(),
            color.green.to_bits(),
            color.blue.to_bits(),
            color.alpha.to_bits(),
        ])
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct TextFormatKey {
    size: u32,
    horizontal: u8,
    vertical: u8,
    weight: u8,
    family: u8,
}

impl From<TextStyle> for TextFormatKey {
    fn from(style: TextStyle) -> Self {
        Self {
            size: style.size.to_bits(),
            weight: match style.weight {
                FontWeight::Normal => 0,
                FontWeight::Semibold => 1,
            },
            family: match style.family {
                FontFamily::Interface => 0,
                FontFamily::SystemSymbols => 1,
                FontFamily::Brand => 2,
            },
            horizontal: match style.horizontal {
                HorizontalAlignment::Leading => 0,
                HorizontalAlignment::Center => 1,
                HorizontalAlignment::Trailing => 2,
            },
            vertical: match style.vertical {
                VerticalAlignment::Top => 0,
                VerticalAlignment::Center => 1,
                VerticalAlignment::Bottom => 2,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct EmbeddedKey {
    asset: SvgAsset,
    size: RasterSize,
    tint: IconTint,
}

#[derive(Debug, Error)]
pub(super) enum PresentationRendererError {
    #[error(transparent)]
    Asset(#[from] AssetError),
    #[error("uploaded presentation icon disappeared from the graphics cache")]
    BitmapCacheInvariant,
    #[error("presentation requested an empty or oversized embedded icon")]
    InvalidIconSize,
    #[error(transparent)]
    Windows(#[from] WindowsError),
}

impl From<PresentationRendererError> for SurfaceError {
    fn from(error: PresentationRendererError) -> Self {
        match error {
            PresentationRendererError::Asset(error) => Self::Asset(error),
            PresentationRendererError::BitmapCacheInvariant
            | PresentationRendererError::InvalidIconSize => Self::BitmapCacheInvariant,
            PresentationRendererError::Windows(error) => Self::from(error),
        }
    }
}

fn rect(value: PresentationRect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: value.left,
        top: value.top,
        right: value.right,
        bottom: value.bottom,
    }
}

fn rounded(rect: PresentationRect, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect: self::rect(rect),
        radiusX: radius,
        radiusY: radius,
    }
}

fn d2d(color: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: color.red,
        g: color.green,
        b: color.blue,
        a: color.alpha,
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "presentation icon bounds are validated and capped by native window dimensions"
)]
fn raster_size(bounds: PresentationRect) -> Result<RasterSize, PresentationRendererError> {
    let width = bounds.width().ceil().max(0.0) as u32;
    let height = bounds.height().ceil().max(0.0) as u32;
    RasterSize::new(width, height).ok_or(PresentationRendererError::InvalidIconSize)
}

fn bitmap_bytes(size: RasterSize) -> usize {
    usize::try_from(size.width())
        .unwrap_or(usize::MAX)
        .saturating_mul(usize::try_from(size.height()).unwrap_or(usize::MAX))
        .saturating_mul(4)
}

fn system_font_family_exists(
    factory: &IDWriteFactory,
    family: windows::core::PCWSTR,
) -> Result<bool, WindowsError> {
    let mut collection = None;
    unsafe { factory.GetSystemFontCollection(&raw mut collection, false)? };
    let collection = collection.ok_or_else(|| {
        WindowsError::new(E_FAIL, "DirectWrite returned no system font collection")
    })?;
    let mut index = 0;
    let mut exists = windows::core::BOOL(0);
    unsafe { collection.FindFamilyName(family, &raw mut index, &raw mut exists)? };
    Ok(exists.as_bool())
}
