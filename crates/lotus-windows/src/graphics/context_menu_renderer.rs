use std::collections::HashMap;
use std::ffi::c_void;
use std::num::NonZeroU32;

use lotus_ui::geometry::PhysicalRect;
use lotus_ui::theme::Theme;
use thiserror::Error;
use windows::Win32::Foundation::D2DERR_RECREATE_TARGET;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_OPTIONS_TARGET,
    D2D1_BITMAP_PROPERTIES1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
    D2D1_ROUNDED_RECT, D2D1CreateFactory, ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext,
    ID2D1Factory1, ID2D1Image, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::{IDXGISurface, IDXGISwapChain1};
use windows::core::Error as WindowsError;

use super::context_menu_scene::{ContextMenuAction, ContextMenuScene};
use super::device::GraphicsDevice;
use super::surface::SurfaceSize;
use super::theme;

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
    assets: SvgAssetCache,
    icons: HashMap<(SvgAsset, NonZeroU32), ID2D1Bitmap1>,
}

impl ContextMenuRenderer {
    pub(super) fn create(
        graphics: &GraphicsDevice,
        swap_chain: &IDXGISwapChain1,
    ) -> Result<Self, ContextMenuRendererError> {
        let dxgi = graphics.dxgi_device()?;
        // SAFETY: A supported typed factory is requested without retained options.
        let factory: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        // SAFETY: The live DXGI device is compatible with the Direct2D factory.
        let device = unsafe { factory.CreateDevice(&dxgi)? };
        // SAFETY: The live device returns an owned drawing context.
        let context =
            unsafe { device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)? };
        let theme = Theme::default();
        let mut renderer = Self {
            _factory: factory,
            _device: device,
            context: context.clone(),
            target: None,
            panel: brush(&context, &theme::d2d(theme.chrome_overlay))?,
            highlight: brush(&context, &theme::d2d(theme.control_hover))?,
            assets: SvgAssetCache::create()?,
            icons: HashMap::new(),
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
        // SAFETY: Buffer zero exists on the initialized composition swap chain.
        let surface: IDXGISurface = unsafe { chain.GetBuffer(0)? };
        let properties = target_properties();
        // SAFETY: Surface and properties remain live through bitmap creation.
        let target = unsafe {
            self.context
                .CreateBitmapFromDxgiSurface(&surface, Some(&raw const properties))?
        };
        // SAFETY: The bitmap belongs to this context and has TARGET enabled.
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
        let theme = scene.theme();
        theme::set(&self.panel, theme.chrome_overlay);
        theme::set(&self.highlight, theme.control_hover);
        let icon_size =
            NonZeroU32::new((20 * scene.dpi()).div_ceil(96)).unwrap_or(NonZeroU32::MIN);
        for (action, _) in scene.items() {
            self.ensure_icon(action_asset(action), icon_size)?;
        }
        let icons = scene
            .items()
            .into_iter()
            .map(|(action, _)| self.icon(action_asset(action), icon_size).cloned())
            .collect::<Result<Vec<_>, _>>()?;
        let panel = D2D_RECT_F {
            left: 0.5,
            top: 0.5,
            right: as_f32(size.width()) - 0.5,
            bottom: as_f32(size.height()) - 0.5,
        };
        let panel = rounded(panel, scale(scene, theme.radii.panel));
        let transparent = TRANSPARENT;

        // SAFETY: Target, brushes, format, text and local geometry remain live through EndDraw.
        let result = unsafe {
            self.context.BeginDraw();
            self.context.Clear(Some(&raw const transparent));
            self.context
                .FillRoundedRectangle(&raw const panel, &self.panel);
            for ((action, bounds), icon) in scene.items().into_iter().zip(&icons) {
                let bounds = rect(bounds);
                if scene.highlighted(action) {
                    let highlight = rounded(bounds, scale(scene, theme.radii.control));
                    self.context
                        .FillRoundedRectangle(&raw const highlight, &self.highlight);
                }
                let icon_extent = as_f32(icon_size.get());
                let icon_bounds = D2D_RECT_F {
                    left: (bounds.left + bounds.right - icon_extent) * 0.5,
                    top: (bounds.top + bounds.bottom - icon_extent) * 0.5,
                    right: (bounds.left + bounds.right + icon_extent) * 0.5,
                    bottom: (bounds.top + bounds.bottom + icon_extent) * 0.5,
                };
                self.context.DrawBitmap(
                    icon,
                    Some(&raw const icon_bounds),
                    1.0,
                    D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                    None,
                    None,
                );
            }
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

    fn ensure_icon(
        &mut self,
        asset: SvgAsset,
        size: NonZeroU32,
    ) -> Result<(), ContextMenuRendererError> {
        let key = (asset, size);
        if self.icons.contains_key(&key) {
            return Ok(());
        }
        let raster = self.assets.rasterize(asset, RasterSize::square(size))?;
        let properties = source_properties();
        // SAFETY: The raster owns tightly packed premultiplied BGRA8 pixels for this size.
        let bitmap = unsafe {
            self.context.CreateBitmap(
                D2D_SIZE_U {
                    width: raster.size().width(),
                    height: raster.size().height(),
                },
                Some(raster.pixels().as_ptr().cast::<c_void>()),
                raster.stride()?,
                &raw const properties,
            )?
        };
        self.icons.insert(key, bitmap);
        Ok(())
    }

    fn icon(
        &self,
        asset: SvgAsset,
        size: NonZeroU32,
    ) -> Result<&ID2D1Bitmap1, ContextMenuRendererError> {
        self.icons
            .get(&(asset, size))
            .ok_or(ContextMenuRendererError::BitmapCacheInvariant)
    }
}

#[derive(Debug, Error)]
pub(super) enum ContextMenuRendererError {
    #[error(transparent)]
    Asset(#[from] AssetError),
    #[error("uploaded context-menu icon disappeared from the graphics cache")]
    BitmapCacheInvariant,
    #[error(transparent)]
    Windows(#[from] WindowsError),
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

const fn action_asset(action: ContextMenuAction) -> SvgAsset {
    match action {
        ContextMenuAction::RequestShutdown => SvgAsset::FluentPower,
        ContextMenuAction::OpenVolumeMixer => SvgAsset::FluentVolume,
        ContextMenuAction::OpenSettings => SvgAsset::FluentSettings,
        ContextMenuAction::OpenTrayOverflow => SvgAsset::FluentTray,
        ContextMenuAction::QuitLotus => SvgAsset::FluentDismiss,
    }
}

fn scale(scene: &ContextMenuScene, dips: f32) -> f32 {
    as_f32(scene.dpi()) * dips / TARGET_DPI
}

#[allow(
    clippy::cast_precision_loss,
    reason = "menu dimensions remain below f32 exact range"
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
use super::assets::{AssetError, RasterSize, SvgAsset, SvgAssetCache};
