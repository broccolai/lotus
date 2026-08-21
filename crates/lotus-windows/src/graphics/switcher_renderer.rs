use std::num::NonZeroU32;

use lotus_ui::geometry::DpiScale;
use lotus_ui::theme::Theme;
use thiserror::Error;
use windows::Win32::Foundation::D2DERR_RECREATE_TARGET;
use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D1_COLOR_F};
use windows::Win32::Graphics::Direct2D::{
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_DRAW_TEXT_OPTIONS_CLIP,
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
use windows::Win32::Graphics::Dxgi::{IDXGISurface, IDXGISwapChain1};
use windows::core::{Error as WindowsError, w};

use super::assets::{
    AssetError, IconTint, RasterImage, RasterSize, SvgAsset, SvgAssetCache,
};
use super::device::GraphicsDevice;
use super::resources::{raster_key, target_bitmap_properties, upload_bgra_pixels};
use super::scene::{DockIcon, RasterIconId};
use super::surface::SurfaceSize;
use super::{LaidOutItem, SwitcherHitTarget, SwitcherScene, theme};
use crate::resource_cache::BoundedResourceCache;

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
    hover: ID2D1SolidColorBrush,
    selected: ID2D1SolidColorBrush,
    selected_border: ID2D1SolidColorBrush,
    close_hover: ID2D1SolidColorBrush,
    icon: ID2D1SolidColorBrush,
    text: ID2D1SolidColorBrush,
    icon_format: IDWriteTextFormat,
    text_format: IDWriteTextFormat,
    assets: SvgAssetCache,
    icon_tint: IconTint,
    embedded_bitmaps: BoundedResourceCache<(SvgAsset, NonZeroU32), ID2D1Bitmap1>,
    raster_bitmaps: BoundedResourceCache<(RasterIconId, u32, u32), ID2D1Bitmap1>,
}

impl SwitcherRenderer {
    pub(super) fn create(
        graphics: &GraphicsDevice,
        swap_chain: &IDXGISwapChain1,
    ) -> Result<Self, RendererError> {
        let dxgi = graphics.dxgi_device()?;
        let factory: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let device = unsafe { factory.CreateDevice(&dxgi)? };
        let context =
            unsafe { device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)? };
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
            hover: brush(&context, &theme::d2d(theme.control_hover))?,
            selected: brush(&context, &theme::d2d(theme.control_selected))?,
            selected_border: brush(&context, &theme::d2d(theme.border_strong))?,
            close_hover: brush(&context, &theme::d2d(theme.control_hover))?,
            icon: brush(&context, &theme::d2d(theme.accent))?,
            text: brush(&context, &theme::d2d(theme.text))?,
            icon_format,
            text_format,
            assets: SvgAssetCache::create()?,
            icon_tint: IconTint::from_color(theme.text),
            embedded_bitmaps: BoundedResourceCache::new(16 * 1024 * 1024),
            raster_bitmaps: BoundedResourceCache::new(16 * 1024 * 1024),
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
        scene: &SwitcherScene,
    ) -> Result<DrawResult, RendererError> {
        let theme = scene.theme();
        let icon_tint = IconTint::from_color(theme.text);
        if self.icon_tint != icon_tint {
            self.icon_tint = icon_tint;
            self.embedded_bitmaps.clear();
        }
        theme::set(&self.panel, theme.chrome_overlay);
        theme::set(&self.hover, theme.control_hover);
        theme::set(&self.selected, theme.control_selected);
        theme::set(&self.selected_border, theme.border_strong);
        theme::set(&self.close_hover, theme.control_hover);
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
        let close_icon_size =
            NonZeroU32::new(DpiScale::from_system(scene.dpi()).physical(14))
                .expect("switcher close icon size is nonzero");
        for item in &layout.items {
            if let Some(icon) = &item.item.icon {
                self.ensure_icon(icon, icon_size)?;
            }
        }
        self.ensure_icon(
            &DockIcon::Embedded(SvgAsset::FluentDismiss),
            close_icon_size,
        )?;
        let bitmaps = layout
            .items
            .iter()
            .map(|item| {
                item.item
                    .icon
                    .as_ref()
                    .map(|icon| self.bitmap(icon, icon_size).cloned())
                    .transpose()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let close_bitmap = self
            .bitmap(
                &DockIcon::Embedded(SvgAsset::FluentDismiss),
                close_icon_size,
            )?
            .clone();
        let transparent = TRANSPARENT;
        unsafe {
            self.context.BeginDraw();
            self.context.Clear(Some(&raw const transparent));
            self.context
                .FillRoundedRectangle(&raw const panel, &self.panel);
        }
        for (item, bitmap) in layout.items.into_iter().zip(&bitmaps) {
            self.draw_item(
                scene,
                &item,
                bitmap.as_ref(),
                icon_size,
                &close_bitmap,
                close_icon_size,
            );
        }
        let result = unsafe { self.context.EndDraw(None, None) };
        match result {
            Ok(()) => Ok(DrawResult::Complete),
            Err(error) if error.code() == D2DERR_RECREATE_TARGET => {
                Ok(DrawResult::RecreateTarget)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn draw_item(
        &self,
        scene: &SwitcherScene,
        item: &LaidOutItem<'_, DockIcon>,
        bitmap: Option<&ID2D1Bitmap1>,
        icon_size: NonZeroU32,
        close_bitmap: &ID2D1Bitmap1,
        close_icon_size: NonZeroU32,
    ) {
        let bounds = rect(item.bounds);
        if item.source_index == scene.selected() {
            let selected = rounded(bounds, scaled(scene, scene.theme().radii.control));
            let outline_bounds = D2D_RECT_F {
                left: bounds.left + scaled(scene, 0.5),
                top: bounds.top + scaled(scene, 0.5),
                right: bounds.right - scaled(scene, 0.5),
                bottom: bounds.bottom - scaled(scene, 0.5),
            };
            let outline = rounded(
                outline_bounds,
                scaled(scene, (scene.theme().radii.control - 0.5).max(1.0)),
            );
            unsafe {
                self.context
                    .FillRoundedRectangle(&raw const selected, &self.selected);
                self.context.DrawRoundedRectangle(
                    &raw const outline,
                    &self.selected_border,
                    scaled(scene, 1.0),
                    None,
                );
            }
        } else if matches!(
            scene.hovered(),
            Some(SwitcherHitTarget::Item(window) | SwitcherHitTarget::Close(window))
                if window == item.item.window
        ) {
            let hovered = rounded(bounds, scaled(scene, scene.theme().radii.control));
            unsafe {
                self.context
                    .FillRoundedRectangle(&raw const hovered, &self.hover);
            }
        }

        let icon_bounds = D2D_RECT_F {
            bottom: bounds.top + (bounds.bottom - bounds.top) * 0.62,
            ..bounds
        };
        self.draw_artwork(scene, item, bitmap, icon_size, icon_bounds);

        let title = item.item.title.encode_utf16().collect::<Vec<_>>();
        let title_bounds = D2D_RECT_F {
            top: icon_bounds.bottom - scaled(scene, 4.0),
            ..bounds
        };
        unsafe {
            self.context.DrawText(
                &title,
                &self.text_format,
                &raw const title_bounds,
                &self.text,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
        self.draw_close(scene, item, close_bitmap, close_icon_size);
    }

    fn draw_artwork(
        &self,
        scene: &SwitcherScene,
        item: &LaidOutItem<'_, DockIcon>,
        bitmap: Option<&ID2D1Bitmap1>,
        icon_size: NonZeroU32,
        bounds: D2D_RECT_F,
    ) {
        let (Some(icon), Some(bitmap)) = (&item.item.icon, bitmap) else {
            let initial = item
                .item
                .title
                .chars()
                .next()
                .unwrap_or('?')
                .to_uppercase()
                .to_string()
                .encode_utf16()
                .collect::<Vec<_>>();
            unsafe {
                self.context.DrawText(
                    &initial,
                    &self.icon_format,
                    &raw const bounds,
                    &self.icon,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
            }
            return;
        };

        let width = as_f32(icon_size.get());
        let center_x = bounds.left.midpoint(bounds.right);
        let left = match icon {
            DockIcon::Raster(_) => (center_x - width / 2.0).round(),
            DockIcon::Embedded(_) => center_x - width / 2.0,
        };
        let destination = D2D_RECT_F {
            left,
            top: bounds.top + scaled(scene, 12.0),
            right: left + width,
            bottom: bounds.top + scaled(scene, 12.0) + width,
        };
        unsafe {
            self.context.DrawBitmap(
                bitmap,
                Some(&raw const destination),
                1.0,
                icon_interpolation(icon, icon_size),
                None,
                None,
            );
        }
    }

    fn draw_close(
        &self,
        scene: &SwitcherScene,
        item: &LaidOutItem<'_, DockIcon>,
        bitmap: &ID2D1Bitmap1,
        icon_size: NonZeroU32,
    ) {
        let hovered = scene.hovered();
        let visible = item.source_index == scene.selected()
            || matches!(
                hovered,
                Some(SwitcherHitTarget::Item(window) | SwitcherHitTarget::Close(window))
                    if window == item.item.window
            );
        if !visible {
            return;
        }

        let bounds = rect(item.close);
        unsafe {
            if hovered == Some(SwitcherHitTarget::Close(item.item.window)) {
                let highlight = rounded(bounds, scaled(scene, scene.theme().radii.compact));
                self.context
                    .FillRoundedRectangle(&raw const highlight, &self.close_hover);
            }
            let width = as_f32(icon_size.get());
            let destination = centered(bounds, width);
            self.context.DrawBitmap(
                bitmap,
                Some(&raw const destination),
                1.0,
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                None,
                None,
            );
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
                if self.embedded_bitmaps.get(&key).is_none() {
                    let raster = self.assets.rasterize(
                        *asset,
                        RasterSize::square(size),
                        self.icon_tint,
                    )?;
                    let bitmap = upload_bitmap(&self.context, raster)?;
                    let bytes = usize::try_from(size.get())
                        .unwrap_or(usize::MAX)
                        .saturating_mul(usize::try_from(size.get()).unwrap_or(usize::MAX))
                        .saturating_mul(4);
                    self.embedded_bitmaps.insert(key, bitmap, bytes);
                }
            }
            DockIcon::Raster(raster) => {
                let key = raster_key(raster);
                if self.raster_bitmaps.get(&key).is_none() {
                    let bitmap = upload_bgra_pixels(
                        &self.context,
                        raster.width(),
                        raster.height(),
                        raster.pixels(),
                        raster.stride(),
                    )?;
                    let bytes = usize::try_from(raster.width())
                        .unwrap_or(usize::MAX)
                        .saturating_mul(
                            usize::try_from(raster.height()).unwrap_or(usize::MAX),
                        )
                        .saturating_mul(4);
                    self.raster_bitmaps.insert(key, bitmap, bytes);
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
                .peek(&(*asset, size))
                .ok_or(RendererError::BitmapCacheInvariant),
            DockIcon::Raster(raster) => self
                .raster_bitmaps
                .peek(&raster_key(raster))
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
    unsafe { context.CreateSolidColorBrush(color, None) }
}

fn upload_bitmap(
    context: &ID2D1DeviceContext,
    raster: &RasterImage,
) -> Result<ID2D1Bitmap1, RendererError> {
    let size = raster.size();
    Ok(upload_bgra_pixels(
        context,
        size.width(),
        size.height(),
        raster.pixels(),
        raster.stride()?,
    )?)
}

fn rect(value: lotus_ui::geometry::PhysicalRect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: as_f32(value.min_x()),
        top: as_f32(value.min_y()),
        right: as_f32(value.max_x()),
        bottom: as_f32(value.max_y()),
    }
}

fn centered(bounds: D2D_RECT_F, size: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: (bounds.left + bounds.right - size) / 2.0,
        top: (bounds.top + bounds.bottom - size) / 2.0,
        right: (bounds.left + bounds.right + size) / 2.0,
        bottom: (bounds.top + bounds.bottom + size) / 2.0,
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
