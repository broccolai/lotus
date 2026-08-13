use std::collections::HashMap;
use std::num::NonZeroU32;

use resvg::usvg;
use thiserror::Error;
use tiny_skia::{Pixmap, Transform};

const LOTUS_PIXEL_SVG: &[u8] = include_bytes!("../../assets/ui/lotus-pixel.svg");
const FLUENT_POWER_SVG: &[u8] = include_bytes!("../../assets/fluent/power-24-regular.svg");
const FLUENT_VOLUME_SVG: &[u8] =
    include_bytes!("../../assets/fluent/speaker-2-24-regular.svg");
const FLUENT_SETTINGS_SVG: &[u8] =
    include_bytes!("../../assets/fluent/settings-24-regular.svg");
const FLUENT_TRAY_SVG: &[u8] =
    include_bytes!("../../assets/fluent/chevron-up-24-regular.svg");
const FLUENT_DISMISS_SVG: &[u8] =
    include_bytes!("../../assets/fluent/dismiss-24-regular.svg");
const FLUENT_DESKTOP_SVG: &[u8] =
    include_bytes!("../../assets/fluent/desktop-24-regular.svg");
const FLUENT_LOCK_SVG: &[u8] =
    include_bytes!("../../assets/fluent/lock-closed-24-regular.svg");
const FLUENT_RESTART_SVG: &[u8] =
    include_bytes!("../../assets/fluent/arrow-clockwise-24-regular.svg");
const FLUENT_SEARCH_SVG: &[u8] =
    include_bytes!("../../assets/fluent/search-24-regular.svg");
const MAX_RASTER_DIMENSION: u32 = 4_096;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SvgAsset {
    LotusPixel,
    FluentPower,
    FluentVolume,
    FluentSettings,
    FluentTray,
    FluentDismiss,
    FluentDesktop,
    FluentLock,
    FluentRestart,
    FluentSearch,
}

impl SvgAsset {
    pub const ALL: [Self; 10] = [
        Self::LotusPixel,
        Self::FluentPower,
        Self::FluentVolume,
        Self::FluentSettings,
        Self::FluentTray,
        Self::FluentDismiss,
        Self::FluentDesktop,
        Self::FluentLock,
        Self::FluentRestart,
        Self::FluentSearch,
    ];

    const fn source(self) -> &'static [u8] {
        match self {
            Self::LotusPixel => LOTUS_PIXEL_SVG,
            Self::FluentPower => FLUENT_POWER_SVG,
            Self::FluentVolume => FLUENT_VOLUME_SVG,
            Self::FluentSettings => FLUENT_SETTINGS_SVG,
            Self::FluentTray => FLUENT_TRAY_SVG,
            Self::FluentDismiss => FLUENT_DISMISS_SVG,
            Self::FluentDesktop => FLUENT_DESKTOP_SVG,
            Self::FluentLock => FLUENT_LOCK_SVG,
            Self::FluentRestart => FLUENT_RESTART_SVG,
            Self::FluentSearch => FLUENT_SEARCH_SVG,
        }
    }

    const fn is_interface_symbol(self) -> bool {
        matches!(
            self,
            Self::FluentPower
                | Self::FluentVolume
                | Self::FluentSettings
                | Self::FluentTray
                | Self::FluentDismiss
                | Self::FluentDesktop
                | Self::FluentLock
                | Self::FluentRestart
                | Self::FluentSearch
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RasterSize {
    width: NonZeroU32,
    height: NonZeroU32,
}

impl RasterSize {
    pub const fn new(width: u32, height: u32) -> Option<Self> {
        let Some(width) = NonZeroU32::new(width) else {
            return None;
        };
        let Some(height) = NonZeroU32::new(height) else {
            return None;
        };
        Some(Self { width, height })
    }

    pub const fn square(side: NonZeroU32) -> Self {
        Self {
            width: side,
            height: side,
        }
    }

    pub const fn width(self) -> u32 {
        self.width.get()
    }

    pub const fn height(self) -> u32 {
        self.height.get()
    }
}

pub struct RasterImage {
    size: RasterSize,
    pixels: Vec<u8>,
}

impl RasterImage {
    pub const fn size(&self) -> RasterSize {
        self.size
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn stride(&self) -> Result<u32, AssetError> {
        self.size
            .width()
            .checked_mul(4)
            .ok_or(AssetError::RasterTooLarge)
    }
}

pub struct SvgAssetCache {
    trees: HashMap<SvgAsset, usvg::Tree>,
    rasters: HashMap<(SvgAsset, RasterSize), RasterImage>,
}

impl SvgAssetCache {
    pub fn create() -> Result<Self, AssetError> {
        let mut trees = HashMap::new();
        for asset in SvgAsset::ALL {
            trees.insert(
                asset,
                usvg::Tree::from_data(asset.source(), &usvg::Options::default())?,
            );
        }
        Ok(Self {
            trees,
            rasters: HashMap::new(),
        })
    }

    pub fn rasterize(
        &mut self,
        asset: SvgAsset,
        size: RasterSize,
    ) -> Result<&RasterImage, AssetError> {
        let key = (asset, size);
        if !self.rasters.contains_key(&key) {
            let tree = self.trees.get(&asset).ok_or(AssetError::CacheInvariant)?;
            let mut raster = rasterize_tree(tree, size)?;
            if asset.is_interface_symbol() {
                tint_interface_symbol(&mut raster.pixels);
            }
            self.rasters.insert(key, raster);
        }

        self.rasters.get(&key).ok_or(AssetError::CacheInvariant)
    }
}

#[derive(Debug, Error)]
pub enum AssetError {
    #[error("embedded SVG could not be parsed: {0}")]
    InvalidSvg(#[from] usvg::Error),
    #[error("requested SVG raster is too large")]
    RasterTooLarge,
    #[error("SVG raster cache lost an inserted entry")]
    CacheInvariant,
}

fn rasterize_tree(tree: &usvg::Tree, size: RasterSize) -> Result<RasterImage, AssetError> {
    if size.width() > MAX_RASTER_DIMENSION || size.height() > MAX_RASTER_DIMENSION {
        return Err(AssetError::RasterTooLarge);
    }

    let mut pixmap =
        Pixmap::new(size.width(), size.height()).ok_or(AssetError::RasterTooLarge)?;
    let tree_size = tree.size();
    let transform = Transform::from_scale(
        pixels_to_f32(size.width()) / tree_size.width(),
        pixels_to_f32(size.height()) / tree_size.height(),
    );
    resvg::render(tree, transform, &mut pixmap.as_mut());

    let mut pixels = pixmap.take();
    rgba_to_bgra(&mut pixels);
    Ok(RasterImage { size, pixels })
}

fn rgba_to_bgra(pixels: &mut [u8]) {
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
}

fn tint_interface_symbol(pixels: &mut [u8]) {
    const BLUE: u16 = 255;
    const GREEN: u16 = 250;
    const RED: u16 = 247;
    const MAX: u16 = 255;
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        pixel[0] = u8::try_from(alpha * BLUE / MAX).unwrap_or(u8::MAX);
        pixel[1] = u8::try_from(alpha * GREEN / MAX).unwrap_or(u8::MAX);
        pixel[2] = u8::try_from(alpha * RED / MAX).unwrap_or(u8::MAX);
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "asset raster dimensions are capped well below f32's exact integer range"
)]
const fn pixels_to_f32(value: u32) -> f32 {
    value as f32
}
