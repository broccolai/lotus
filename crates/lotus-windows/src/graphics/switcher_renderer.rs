use std::collections::HashMap;
use std::ffi::c_void;
use std::num::NonZeroU32;

use lotus_ui::geometry::DpiScale;
use lotus_ui::theme::Theme;
use thiserror::Error;
use windows::Win32::Foundation::D2DERR_RECREATE_TARGET;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_OPTIONS_TARGET,
    D2D1_BITMAP_PROPERTIES1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_DRAW_TEXT_OPTIONS_CLIP,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
    D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_ROUNDED_RECT, D2D1CreateFactory,
    ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext, ID2D1Factory1, ID2D1Image,
    ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::{IDXGISurface, IDXGISwapChain1};
use windows::core::{Error as WindowsError, w};

use super::assets::{AssetError, RasterImage, RasterSize, SvgAsset, SvgAssetCache};
use super::device::GraphicsDevice;
use super::scene::{DockIcon, RasterIcon, RasterIconId};
use super::surface::SurfaceSize;
use super::switcher_scene::SwitcherScene;
use super::theme;

const TARGET_DPI: f32 = 96.0;
const TRANSPARENT: D2D1_COLOR_F = rgba(0, 0, 0, 0);

pub(super) enum DrawResult {
    Complete,
    RecreateTarget,
}

pub(super) struct SwitcherRenderer {
    _factory: ID2D1Factory1,
    _device: ID2D1Device,
    context: ID2D1DeviceContext,
    target: Option<ID2D1Bitmap1>,
    panel: ID2D1SolidColorBrush,
    selected: ID2D1SolidColorBrush,
    icon: ID2D1SolidColorBrush,
    text: ID2D1SolidColorBrush,
    icon_format: IDWriteTextFormat,
    text_format: IDWriteTextFormat,
    assets: SvgAssetCache,
    embedded_bitmaps: HashMap<(SvgAsset, NonZeroU32), ID2D1Bitmap1>,
    raster_bitmaps: HashMap<(RasterIconId, u32, u32), ID2D1Bitmap1>,
}

impl SwitcherRenderer {
    pub(super) fn create(
        graphics: &GraphicsDevice,
        swap_chain: &IDXGISwapChain1,
    ) -> Result<Self, RendererError> {
        let dxgi = graphics.dxgi_device()?;
        // SAFETY: A typed factory is requested without retained options.
        let factory: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        // SAFETY: The live DXGI device is compatible with this factory.
        let device = unsafe { factory.CreateDevice(&dxgi)? };
        // SAFETY: The live device returns an owned drawing context.
        let context =
            unsafe { device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)? };
        // SAFETY: DirectWrite returns an owned shared factory.
        let write: IDWriteFactory =
            unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let icon_format = text_format(&write, 26.0, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
        let text_format = text_format(&write, 13.0, DWRITE_FONT_WEIGHT_NORMAL)?;
        let theme = Theme::default();
        let mut renderer = Self {
            _factory: factory,
            _device: device,
            context: context.clone(),
            target: None,
            panel: brush(&context, &theme::d2d(theme.chrome_overlay))?,
            selected: brush(&context, &theme::d2d(theme.control_selected))?,
            icon: brush(&context, &theme::d2d(theme.accent))?,
            text: brush(&context, &theme::d2d(theme.text))?,
            icon_format,
            text_format,
            assets: SvgAssetCache::create()?,
            embedded_bitmaps: HashMap::new(),
            raster_bitmaps: HashMap::new(),
        };
        renderer.attach_target(swap_chain)?;
        Ok(renderer)
    }

    pub(super) fn detach_target(&mut self) {
        // SAFETY: A null target releases the swap-chain buffer before resize.
        unsafe { self.context.SetTarget(None::<&ID2D1Image>) };
        self.target = None;
    }

    pub(super) fn attach_target(
        &mut self,
        chain: &IDXGISwapChain1,
    ) -> Result<(), WindowsError> {
        self.detach_target();
        // SAFETY: Buffer zero exists on the initialized swap chain.
        let surface: IDXGISurface = unsafe { chain.GetBuffer(0)? };
        let properties = target_properties();
        // SAFETY: Surface and properties remain live through bitmap creation.
        let target = unsafe {
            self.context
                .CreateBitmapFromDxgiSurface(&surface, Some(&raw const properties))?
        };
        // SAFETY: The bitmap belongs to this context and is target-capable.
        unsafe { self.context.SetTarget(&target) };
        self.target = Some(target);
        Ok(())
    }

    pub(super) fn draw(
        &mut self,
        size: SurfaceSize,
        scene: &SwitcherScene,
    ) -> Result<DrawResult, RendererError> {
        let theme = scene.theme();
        theme::set(&self.panel, theme.chrome_overlay);
        theme::set(&self.selected, theme.control_selected);
        theme::set(&self.icon, theme.accent);
        theme::set(&self.text, theme.text);
        let panel = rounded(
            D2D_RECT_F {
                left: 0.5,
                top: 0.5,
                right: as_f32(size.width()) - 0.5,
                bottom: as_f32(size.height()) - 0.5,
            },
            scaled(scene, theme.radii.panel),
        );
        let layout = scene.layout();
        let icon_size = NonZeroU32::new(DpiScale::from_system(scene.dpi()).physical(38))
            .expect("switcher icon size is nonzero");
        for item in &layout.items {
            if let Some(icon) = &item.item.icon {
                self.ensure_icon(icon, icon_size)?;
            }
        }
        let transparent = TRANSPARENT;
        // SAFETY: All targets, brushes, formats, strings, and geometry remain live through EndDraw.
        let result = unsafe {
            self.context.BeginDraw();
            self.context.Clear(Some(&raw const transparent));
            self.context
                .FillRoundedRectangle(&raw const panel, &self.panel);
            for item in layout.items {
                let bounds = rect(item.bounds);
                if item.source_index == scene.selected() {
                    let selected_bounds =
                        rounded(bounds, scaled(scene, theme.radii.control));
                    self.context
                        .FillRoundedRectangle(&raw const selected_bounds, &self.selected);
                }
                let icon_bounds = D2D_RECT_F {
                    bottom: bounds.top + (bounds.bottom - bounds.top) * 0.62,
                    ..bounds
                };
                if let Some(icon) = &item.item.icon {
                    let bitmap = self.bitmap(icon, icon_size)?;
                    let icon_width = as_f32(icon_size.get());
                    let center_x = bounds.left.midpoint(bounds.right);
                    let icon_left = match icon {
                        DockIcon::Raster(_) => (center_x - icon_width / 2.0).round(),
                        DockIcon::Embedded(_) => center_x - icon_width / 2.0,
                    };
                    let icon_rectangle = D2D_RECT_F {
                        left: icon_left,
                        top: bounds.top + scaled(scene, 12.0),
                        right: icon_left + icon_width,
                        bottom: bounds.top + scaled(scene, 12.0) + icon_width,
                    };
                    self.context.DrawBitmap(
                        bitmap,
                        Some(&raw const icon_rectangle),
                        1.0,
                        icon_interpolation(icon, icon_size),
                        None,
                        None,
                    );
                } else {
                    let initial = item
                        .item
                        .title
                        .chars()
                        .next()
                        .unwrap_or('?')
                        .to_uppercase()
                        .to_string();
                    let initial = initial.encode_utf16().collect::<Vec<_>>();
                    self.context.DrawText(
                        &initial,
                        &self.icon_format,
                        &raw const icon_bounds,
                        &self.icon,
                        D2D1_DRAW_TEXT_OPTIONS_CLIP,
                        DWRITE_MEASURING_MODE_NATURAL,
                    );
                }
                let title = item.item.title.encode_utf16().collect::<Vec<_>>();
                let title_bounds = D2D_RECT_F {
                    top: icon_bounds.bottom - scaled(scene, 4.0),
                    ..bounds
                };
                self.context.DrawText(
                    &title,
                    &self.text_format,
                    &raw const title_bounds,
                    &self.text,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
            }
            self.context.EndDraw(None, None)
        };
        match result {
            Ok(()) => Ok(DrawResult::Complete),
            Err(error) if error.code() == D2DERR_RECREATE_TARGET => {
                Ok(DrawResult::RecreateTarget)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn ensure_icon(
        &mut self,
        icon: &DockIcon,
        size: NonZeroU32,
    ) -> Result<(), RendererError> {
        match icon {
            DockIcon::Embedded(asset) => {
                let key = (*asset, size);
                if !self.embedded_bitmaps.contains_key(&key) {
                    let raster = self.assets.rasterize(*asset, RasterSize::square(size))?;
                    let bitmap = upload_bitmap(&self.context, raster)?;
                    self.embedded_bitmaps.insert(key, bitmap);
                }
            }
            DockIcon::Raster(raster) => {
                let key = raster_key(raster);
                if !self.raster_bitmaps.contains_key(&key) {
                    let bitmap = upload_pixels(
                        &self.context,
                        raster.width(),
                        raster.height(),
                        raster.pixels(),
                        raster.stride(),
                    )?;
                    self.raster_bitmaps.insert(key, bitmap);
                }
            }
        }
        Ok(())
    }

    fn bitmap(
        &self,
        icon: &DockIcon,
        size: NonZeroU32,
    ) -> Result<&ID2D1Bitmap1, RendererError> {
        match icon {
            DockIcon::Embedded(asset) => self
                .embedded_bitmaps
                .get(&(*asset, size))
                .ok_or(RendererError::BitmapCacheInvariant),
            DockIcon::Raster(raster) => self
                .raster_bitmaps
                .get(&raster_key(raster))
                .ok_or(RendererError::BitmapCacheInvariant),
        }
    }
}

#[derive(Debug, Error)]
pub(super) enum RendererError {
    #[error(transparent)]
    Windows(#[from] WindowsError),
    #[error(transparent)]
    Asset(#[from] AssetError),
    #[error("switcher bitmap cache lost a prepared icon")]
    BitmapCacheInvariant,
}

fn text_format(
    factory: &IDWriteFactory,
    size: f32,
    weight: windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT,
) -> Result<IDWriteTextFormat, WindowsError> {
    // SAFETY: Static family and locale are NUL terminated.
    let format = unsafe {
        factory.CreateTextFormat(
            w!("Segoe UI Variable Text"),
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("en-us"),
        )?
    };
    // SAFETY: These are valid layout properties for the retained format.
    unsafe {
        format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
    }
    Ok(format)
}

fn brush(
    context: &ID2D1DeviceContext,
    color: &D2D1_COLOR_F,
) -> Result<ID2D1SolidColorBrush, WindowsError> {
    // SAFETY: Direct2D copies the color synchronously.
    unsafe { context.CreateSolidColorBrush(color, None) }
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

fn source_properties() -> D2D1_BITMAP_PROPERTIES1 {
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
    upload_pixels(
        context,
        size.width(),
        size.height(),
        raster.pixels(),
        raster.stride()?,
    )
}

fn upload_pixels(
    context: &ID2D1DeviceContext,
    width: u32,
    height: u32,
    pixels: &[u8],
    stride: u32,
) -> Result<ID2D1Bitmap1, RendererError> {
    let properties = source_properties();
    // SAFETY: Both asset and native icon types validate tightly packed premultiplied BGRA data.
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

fn rect(value: lotus_ui::geometry::PhysicalRect) -> D2D_RECT_F {
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

fn scaled(scene: &SwitcherScene, dips: f32) -> f32 {
    as_f32(scene.dpi()) * dips / TARGET_DPI
}

fn icon_interpolation(
    icon: &DockIcon,
    target_size: NonZeroU32,
) -> windows::Win32::Graphics::Direct2D::D2D1_INTERPOLATION_MODE {
    match icon {
        DockIcon::Raster(raster)
            if raster.width() == target_size.get()
                && raster.height() == target_size.get() =>
        {
            D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR
        }
        DockIcon::Raster(_) | DockIcon::Embedded(_) => {
            D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC
        }
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "window dimensions remain below f32 exact range"
)]
const fn as_f32(value: u32) -> f32 {
    value as f32
}

const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> D2D1_COLOR_F {
    const MAX: f32 = 255.0;
    D2D1_COLOR_F {
        r: red as f32 / MAX,
        g: green as f32 / MAX,
        b: blue as f32 / MAX,
        a: alpha as f32 / MAX,
    }
}
