use std::hash::{DefaultHasher, Hash, Hasher};

use image::imageops::FilterType;
use lotus_ui::icon::{RasterIcon, RasterIconError};
use thiserror::Error;

const ARTWORK_SAMPLE_SIZE: u32 = 160;

pub fn decode_artwork(
    source_id: &str,
    encoded: &[u8],
) -> Result<RasterIcon, MediaArtworkError> {
    let image = image::load_from_memory(encoded)?
        .resize_to_fill(
            ARTWORK_SAMPLE_SIZE,
            ARTWORK_SAMPLE_SIZE,
            FilterType::Lanczos3,
        )
        .to_rgba8();
    let mut pixels = image.into_raw();
    premultiply_rgba_to_bgra(&mut pixels);
    let mut hasher = DefaultHasher::new();
    source_id.hash(&mut hasher);
    encoded.hash(&mut hasher);
    let identity = format!("media:{:016x}", hasher.finish());
    RasterIcon::new(identity, ARTWORK_SAMPLE_SIZE, ARTWORK_SAMPLE_SIZE, pixels)
        .map_err(Into::into)
}

#[derive(Debug, Error)]
pub enum MediaArtworkError {
    #[error("the media artwork could not be decoded: {0}")]
    Image(#[from] image::ImageError),
    #[error(transparent)]
    Raster(#[from] RasterIconError),
}

fn premultiply_rgba_to_bgra(pixels: &mut [u8]) {
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        let red = u16::from(pixel[0]);
        let green = u16::from(pixel[1]);
        let blue = u16::from(pixel[2]);
        pixel[0] = u8::try_from(blue.saturating_mul(alpha) / 255).unwrap_or(u8::MAX);
        pixel[1] = u8::try_from(green.saturating_mul(alpha) / 255).unwrap_or(u8::MAX);
        pixel[2] = u8::try_from(red.saturating_mul(alpha) / 255).unwrap_or(u8::MAX);
    }
}
