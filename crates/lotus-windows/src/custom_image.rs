use std::fmt::Write as _;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, Cursor, Write};
use std::path::{Path, PathBuf};

use atomic_write_file::AtomicWriteFile;
use image::{DynamicImage, ImageFormat, RgbaImage};
use lotus_ui::icon::{RasterIcon, RasterIconError};
use sha2::{Digest, Sha256};
use thiserror::Error;

const MAX_DIMENSION: u32 = 512;

#[derive(Debug, Error)]
pub enum CustomImageError {
    #[error("could not process the selected image: {0}")]
    Image(#[from] image::ImageError),
    #[error("could not store the selected image at `{path}`: {source}")]
    Store {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Raster(#[from] RasterIconError),
}

pub fn load_custom_image(path: &Path) -> Result<RasterIcon, CustomImageError> {
    let image = decode(path)?;
    let (width, height) = image.dimensions();
    let mut pixels = image.into_raw();
    premultiply_rgba_to_bgra(&mut pixels);
    let mut hasher = DefaultHasher::new();
    pixels.hash(&mut hasher);
    let identity = format!("{}#{:016x}", path.to_string_lossy(), hasher.finish());
    RasterIcon::new(identity, width, height, pixels).map_err(Into::into)
}

pub fn import_custom_image(
    source: &Path,
    settings_directory: &Path,
) -> Result<PathBuf, CustomImageError> {
    let image = decode(source)?;
    let mut digest = Sha256::new();
    digest.update(image.width().to_le_bytes());
    digest.update(image.height().to_le_bytes());
    digest.update(image.as_raw());
    let mut identity = String::with_capacity(64);
    for byte in digest.finalize() {
        write!(identity, "{byte:02x}").expect("writing to a String cannot fail");
    }

    let directory = settings_directory.join("assets");
    fs::create_dir_all(&directory).map_err(|source| CustomImageError::Store {
        path: directory.clone(),
        source,
    })?;
    let destination = directory.join(format!("mascot-{identity}.png"));
    if destination.exists() {
        return Ok(destination);
    }

    let mut encoded = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image).write_to(&mut encoded, ImageFormat::Png)?;
    let mut output =
        AtomicWriteFile::open(&destination).map_err(|source| CustomImageError::Store {
            path: destination.clone(),
            source,
        })?;
    output
        .write_all(encoded.get_ref())
        .map_err(|source| CustomImageError::Store {
            path: destination.clone(),
            source,
        })?;
    output.commit().map_err(|source| CustomImageError::Store {
        path: destination.clone(),
        source,
    })?;
    Ok(destination)
}

fn decode(path: &Path) -> Result<RgbaImage, image::ImageError> {
    Ok(image::open(path)?
        .thumbnail(MAX_DIMENSION, MAX_DIMENSION)
        .to_rgba8())
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
