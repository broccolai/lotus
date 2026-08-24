use std::fmt::Write as _;
use std::fs;
use std::fs::File;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, BufReader, Cursor, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use atomic_write_file::AtomicWriteFile;
use image::codecs::gif::GifDecoder;
use image::metadata::LoopCount;
use image::{
    AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbaImage,
    imageops,
};
use lotus_ui::icon::{RasterIcon, RasterIconError};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::resource_cache::BoundedResourceCache;
use crate::responsiveness::CacheClass;

const MAX_DIMENSION: u32 = 512;
const CUSTOM_IMAGE_CACHE_BYTES: usize = 4 * 1024 * 1024;
const MAX_MASCOT_FRAMES: usize = 120;
const MAX_MASCOT_DECODED_BYTES: usize = 16 * 1024 * 1024;
const MIN_FRAME_DELAY: Duration = Duration::from_millis(20);
const MAX_FRAME_DELAY: Duration = Duration::from_secs(60);

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
    #[error("the animated mascot exceeds Lotus's supported image limits")]
    AnimatedMascotTooLarge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MascotLoopCount {
    Infinite,
    Finite(u32),
}

#[derive(Clone)]
pub struct MascotFrame {
    pub icon: RasterIcon,
    pub delay: Duration,
}

#[derive(Clone)]
pub struct MascotAnimation {
    pub frames: Vec<MascotFrame>,
    pub loop_count: MascotLoopCount,
}

#[derive(Clone)]
pub struct MascotImage {
    pub icon: RasterIcon,
    pub animation: Option<MascotAnimation>,
}

pub fn load_mascot_image(path: &Path) -> Result<MascotImage, CustomImageError> {
    match image_format(path)? {
        Some(ImageFormat::Gif) => load_animated_mascot(path),
        Some(_) | None => load_custom_image(path).map(|icon| MascotImage {
            icon,
            animation: None,
        }),
    }
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

pub struct CustomImageCache {
    images: BoundedResourceCache<PathBuf, RasterIcon>,
}

impl Default for CustomImageCache {
    fn default() -> Self {
        Self {
            images: BoundedResourceCache::new(
                CacheClass::CustomImages,
                CUSTOM_IMAGE_CACHE_BYTES,
            ),
        }
    }
}

impl CustomImageCache {
    pub fn image(&mut self, path: &Path) -> Result<RasterIcon, CustomImageError> {
        if let Some(image) = self.images.get(path) {
            return Ok(image.clone());
        }

        let image = load_custom_image(path)?;
        let _ = self
            .images
            .insert(path.to_path_buf(), image.clone(), image.pixels().len());
        Ok(image)
    }

    pub fn clear(&mut self) {
        self.images.clear();
    }
}

pub fn import_custom_image(
    source: &Path,
    settings_directory: &Path,
) -> Result<PathBuf, CustomImageError> {
    if matches!(image_format(source)?, Some(ImageFormat::Gif)) {
        return import_animated_mascot(source, settings_directory);
    }

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

fn import_animated_mascot(
    source: &Path,
    settings_directory: &Path,
) -> Result<PathBuf, CustomImageError> {
    let _ = load_animated_mascot(source)?;
    let bytes = fs::read(source).map_err(|error| CustomImageError::Store {
        path: source.to_path_buf(),
        source: error,
    })?;
    let identity = bytes_identity(&bytes);
    let directory = settings_directory.join("assets");
    fs::create_dir_all(&directory).map_err(|source| CustomImageError::Store {
        path: directory.clone(),
        source,
    })?;
    let destination = directory.join(format!("mascot-{identity}.gif"));
    if destination.exists() {
        return Ok(destination);
    }

    let mut output =
        AtomicWriteFile::open(&destination).map_err(|source| CustomImageError::Store {
            path: destination.clone(),
            source,
        })?;
    output
        .write_all(&bytes)
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

pub fn import_application_icon(
    source: &Path,
    settings_directory: &Path,
) -> Result<PathBuf, CustomImageError> {
    let image = square_image(decode(source)?);
    let identity = image_identity(&image);
    let directory = settings_directory.join("assets").join("app-icons");
    fs::create_dir_all(&directory).map_err(|source| CustomImageError::Store {
        path: directory.clone(),
        source,
    })?;
    let destination = directory.join(format!("{identity}.png"));
    if destination.exists() {
        return Ok(destination);
    }

    store_png(&image, &destination)?;
    Ok(destination)
}

fn decode(path: &Path) -> Result<RgbaImage, image::ImageError> {
    Ok(image::open(path)?
        .thumbnail(MAX_DIMENSION, MAX_DIMENSION)
        .to_rgba8())
}

fn image_format(path: &Path) -> Result<Option<ImageFormat>, CustomImageError> {
    let reader = ImageReader::open(path).map_err(image::ImageError::IoError)?;
    reader
        .with_guessed_format()
        .map(|reader| reader.format())
        .map_err(image::ImageError::IoError)
        .map_err(Into::into)
}

fn load_animated_mascot(path: &Path) -> Result<MascotImage, CustomImageError> {
    let file = File::open(path).map_err(image::ImageError::IoError)?;
    let decoder = GifDecoder::new(BufReader::new(file))?;
    let (width, height) = decoder.dimensions();
    let decoded_bytes = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(CustomImageError::AnimatedMascotTooLarge)?;
    if width > MAX_DIMENSION
        || height > MAX_DIMENSION
        || decoded_bytes > MAX_MASCOT_DECODED_BYTES
    {
        return Err(CustomImageError::AnimatedMascotTooLarge);
    }
    let loop_count = match decoder.loop_count() {
        LoopCount::Infinite => MascotLoopCount::Infinite,
        LoopCount::Finite(count) => MascotLoopCount::Finite(count.get()),
    };
    let mut frames = Vec::new();
    for frame in decoder.into_frames() {
        if frames.len() == MAX_MASCOT_FRAMES {
            return Err(CustomImageError::AnimatedMascotTooLarge);
        }
        let total_decoded_bytes = frames
            .len()
            .checked_add(1)
            .and_then(|frame_count| decoded_bytes.checked_mul(frame_count))
            .ok_or(CustomImageError::AnimatedMascotTooLarge)?;
        if total_decoded_bytes > MAX_MASCOT_DECODED_BYTES {
            return Err(CustomImageError::AnimatedMascotTooLarge);
        }
        let frame = frame?;
        let delay = bounded_delay(Duration::from(frame.delay()));
        let image = frame.into_buffer();
        let mut pixels = image.into_raw();
        premultiply_rgba_to_bgra(&mut pixels);
        let identity = format!("{}#{}", path.to_string_lossy(), frames.len());
        let icon = RasterIcon::new(identity, width, height, pixels)?;
        frames.push(MascotFrame { icon, delay });
    }
    let icon = frames
        .first()
        .map(|frame| frame.icon.clone())
        .ok_or(CustomImageError::AnimatedMascotTooLarge)?;
    let animation = (frames.len() > 1).then_some(MascotAnimation { frames, loop_count });
    Ok(MascotImage { icon, animation })
}

fn bounded_delay(delay: Duration) -> Duration {
    delay.clamp(MIN_FRAME_DELAY, MAX_FRAME_DELAY)
}

fn square_image(image: RgbaImage) -> RgbaImage {
    let side = image.width().max(image.height()).clamp(1, MAX_DIMENSION);
    let resized = if image.width() > side || image.height() > side {
        imageops::thumbnail(&image, side, side)
    } else {
        image
    };
    let mut canvas = RgbaImage::new(side, side);
    let left = side.saturating_sub(resized.width()) / 2;
    let top = side.saturating_sub(resized.height()) / 2;
    imageops::overlay(&mut canvas, &resized, i64::from(left), i64::from(top));
    canvas
}

fn image_identity(image: &RgbaImage) -> String {
    let mut digest = Sha256::new();
    digest.update(image.width().to_le_bytes());
    digest.update(image.height().to_le_bytes());
    digest.update(image.as_raw());
    let mut identity = String::with_capacity(64);
    for byte in digest.finalize() {
        write!(identity, "{byte:02x}").expect("writing to a String cannot fail");
    }
    identity
}

fn bytes_identity(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    let mut identity = String::with_capacity(64);
    for byte in digest.finalize() {
        write!(identity, "{byte:02x}").expect("writing to a String cannot fail");
    }
    identity
}

fn store_png(image: &RgbaImage, destination: &Path) -> Result<(), CustomImageError> {
    let mut encoded = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image.clone()).write_to(&mut encoded, ImageFormat::Png)?;
    let mut output =
        AtomicWriteFile::open(destination).map_err(|source| CustomImageError::Store {
            path: destination.to_path_buf(),
            source,
        })?;
    output
        .write_all(encoded.get_ref())
        .map_err(|source| CustomImageError::Store {
            path: destination.to_path_buf(),
            source,
        })?;
    output.commit().map_err(|source| CustomImageError::Store {
        path: destination.to_path_buf(),
        source,
    })?;
    Ok(())
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
