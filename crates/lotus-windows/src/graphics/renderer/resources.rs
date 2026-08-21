use std::num::NonZeroU32;

use windows::Win32::Graphics::Direct2D::ID2D1Bitmap1;

use super::super::assets::{RasterImage, RasterSize, SvgAsset};
use super::super::resources::{raster_key, upload_bgra_pixels};
use super::super::scene::{DockIcon, RasterIcon};
use super::{Direct2DRenderer, RendererError};

impl Direct2DRenderer {
    pub(super) fn ensure_icon(
        &mut self,
        icon: &DockIcon,
        size: NonZeroU32,
    ) -> Result<(), RendererError> {
        match icon {
            DockIcon::Embedded(asset) => self.ensure_embedded_bitmap(*asset, size),
            DockIcon::Raster(raster) => self.ensure_raster_bitmap(raster),
        }
    }

    pub(super) fn ensure_embedded_bitmap(
        &mut self,
        asset: SvgAsset,
        size: NonZeroU32,
    ) -> Result<(), RendererError> {
        let key = (asset, size);
        if self.embedded_bitmaps.get(&key).is_some() {
            return Ok(());
        }

        let raster =
            self.assets
                .rasterize(asset, RasterSize::square(size), self.icon_tint)?;
        let bitmap = upload_raster_image(&self.context, raster)?;
        let bytes = usize::try_from(size.get())
            .unwrap_or(usize::MAX)
            .saturating_mul(usize::try_from(size.get()).unwrap_or(usize::MAX))
            .saturating_mul(4);
        self.embedded_bitmaps.insert(key, bitmap, bytes);
        Ok(())
    }

    pub(super) fn ensure_raster_bitmap(
        &mut self,
        raster: &RasterIcon,
    ) -> Result<(), RendererError> {
        let key = raster_key(raster);
        if self.raster_bitmaps.get(&key).is_some() {
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
        self.raster_bitmaps.insert(key, bitmap, bytes);
        Ok(())
    }

    pub(super) fn bitmap(
        &self,
        icon: &DockIcon,
        embedded_size: NonZeroU32,
    ) -> Result<&ID2D1Bitmap1, RendererError> {
        match icon {
            DockIcon::Embedded(asset) => self
                .embedded_bitmaps
                .peek(&(*asset, embedded_size))
                .ok_or(RendererError::BitmapCacheInvariant),
            DockIcon::Raster(raster) => self
                .raster_bitmaps
                .peek(&raster_key(raster))
                .ok_or(RendererError::BitmapCacheInvariant),
        }
    }
}

fn upload_raster_image(
    context: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext,
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
