use std::path::Path;

use lotus_core::window::TrackedWindowKey;
use lotus_ui::icon::{RasterIcon, RasterIconError};
use thiserror::Error;
use windows::core::Error;

use crate::NativeError;
use crate::resource_cache::BoundedResourceCache;
use crate::responsiveness::CacheClass;

mod raster;
mod source;

const MAX_ICON_SIZE: u32 = 1_024;
const NATIVE_ICON_CACHE_BYTES: usize = 6 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum NativeIconError {
    #[error("native icon paths must be nonempty and contain no null characters")]
    InvalidPath,
    #[error("native icon size must be between 1 and {MAX_ICON_SIZE} physical pixels")]
    InvalidSize,
    #[error("native icon raster dimensions exceed addressable memory")]
    RasterTooLarge,
    #[error(transparent)]
    InvalidRaster(#[from] RasterIconError),
    #[error(transparent)]
    Native(#[from] NativeError),
}

impl From<Error> for NativeIconError {
    fn from(error: Error) -> Self {
        Self::Native(error.into())
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CacheKey {
    normalized_path: String,
    icon_index: i32,
    size: u32,
}

pub struct NativeIconCache {
    icons: BoundedResourceCache<CacheKey, RasterIcon>,
}

impl Default for NativeIconCache {
    fn default() -> Self {
        Self {
            icons: BoundedResourceCache::new(
                CacheClass::NativeIcons,
                NATIVE_ICON_CACHE_BYTES,
            ),
        }
    }
}

impl NativeIconCache {
    pub fn icon(
        &mut self,
        path: &Path,
        size: u32,
    ) -> Result<Option<RasterIcon>, NativeIconError> {
        source::validate_size(size)?;
        let source_path = source::sanitized_path(path)?;
        let normalized_path = source::normalize_path(&source_path)?;
        let extraction = source::icon_extraction_source(&source_path);
        let icon_index = extraction.as_ref().map_or(0, |(_, index)| *index);
        let key = CacheKey {
            normalized_path,
            icon_index,
            size,
        };

        if let Some(icon) = self.icons.get(&key) {
            return Ok(Some(icon.clone()));
        }

        let image = match extraction {
            Some((extraction_path, icon_index)) => {
                raster::extract_icon(&extraction_path, icon_index, &key)?
            }
            None => None,
        };
        if let Some(icon) = &image {
            let _ = self.icons.insert(key, icon.clone(), icon.pixels().len());
        }
        Ok(image)
    }
}

pub fn window_icon(
    window: TrackedWindowKey,
    size: u32,
) -> Result<Option<RasterIcon>, NativeIconError> {
    source::validate_size(size)?;
    let Some(icon) = raster::copy_window_icon(window) else {
        return Ok(None);
    };
    raster::rasterize_icon(
        icon.get(),
        format!("window:{}@{size}px", window.id.get()),
        size,
    )
    .map(Some)
}
