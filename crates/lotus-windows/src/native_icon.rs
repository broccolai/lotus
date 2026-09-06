use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lotus_core::window::TrackedWindowKey;
use lotus_ui::icon::{RasterIcon, RasterIconError};
use thiserror::Error;
use windows::core::Error;

use crate::NativeError;
use crate::resource_cache::BoundedResourceCache;
use crate::responsiveness::{CacheClass, METRICS};

mod raster;
mod source;

const MAX_ICON_SIZE: u32 = 1_024;
const NATIVE_ICON_CACHE_BYTES: usize = 6 * 1024 * 1024;
const MAX_SOURCE_DESCRIPTORS: usize = 256;
const MAX_NEGATIVE_RESULTS: usize = 256;
const DESCRIPTOR_TTL: Duration = Duration::from_secs(300);
const NEGATIVE_TTL: Duration = Duration::from_secs(30);

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

#[derive(Clone)]
enum SourceDescriptor {
    Extract { path: PathBuf, icon_index: i32 },
    Missing,
}

struct TimedEntry<T> {
    value: T,
    expires_at: Instant,
    last_access: u64,
}

pub struct NativeIconCache {
    icons: BoundedResourceCache<CacheKey, RasterIcon>,
    descriptors: HashMap<String, TimedEntry<SourceDescriptor>>,
    negative_results: HashMap<CacheKey, TimedEntry<()>>,
    next_access: u64,
}

impl Default for NativeIconCache {
    fn default() -> Self {
        Self {
            icons: BoundedResourceCache::new(
                CacheClass::NativeIcons,
                NATIVE_ICON_CACHE_BYTES,
            ),
            descriptors: HashMap::new(),
            negative_results: HashMap::new(),
            next_access: 0,
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
        let source_key = source::normalize_path(&source_path)?;
        let descriptor = self.descriptor_for(&source_key, &source_path);
        let SourceDescriptor::Extract { path, icon_index } = descriptor else {
            return Ok(None);
        };
        let key = CacheKey {
            normalized_path: source::normalize_path(&path)?,
            icon_index,
            size,
        };

        if let Some(icon) = self.icons.get(&key) {
            return Ok(Some(icon.clone()));
        }
        if self.negative_result_is_current(&key) {
            return Ok(None);
        }

        let raster_started = Instant::now();
        let image = raster::extract_icon(&path, icon_index, &key);
        METRICS.record_icon_raster(raster_started.elapsed());
        let image = match image {
            Ok(image) => image,
            Err(error) => {
                self.remember_negative(key);
                return Err(error);
            }
        };
        if let Some(icon) = &image {
            let _ = self.icons.insert(key, icon.clone(), icon.pixels().len());
        } else {
            self.remember_negative(key);
        }
        Ok(image)
    }

    fn descriptor_for(&mut self, key: &str, source_path: &Path) -> SourceDescriptor {
        let now = Instant::now();
        let access = self.next_access();
        if let Some(entry) = self.descriptors.get_mut(key)
            && entry.expires_at > now
        {
            entry.last_access = access;
            return entry.value.clone();
        }

        let discovery_started = Instant::now();
        let descriptor = source::icon_extraction_source(source_path)
            .map_or(SourceDescriptor::Missing, |(path, icon_index)| {
                SourceDescriptor::Extract { path, icon_index }
            });
        METRICS.record_icon_source_discovery(discovery_started.elapsed());
        insert_bounded(
            &mut self.descriptors,
            key.to_owned(),
            TimedEntry {
                value: descriptor.clone(),
                expires_at: now + descriptor_ttl(&descriptor),
                last_access: access,
            },
            MAX_SOURCE_DESCRIPTORS,
        );
        descriptor
    }

    fn negative_result_is_current(&mut self, key: &CacheKey) -> bool {
        let now = Instant::now();
        let access = self.next_access();
        self.negative_results.get_mut(key).is_some_and(|entry| {
            if entry.expires_at <= now {
                return false;
            }
            entry.last_access = access;
            true
        })
    }

    fn remember_negative(&mut self, key: CacheKey) {
        let access = self.next_access();
        insert_bounded(
            &mut self.negative_results,
            key,
            TimedEntry {
                value: (),
                expires_at: Instant::now() + NEGATIVE_TTL,
                last_access: access,
            },
            MAX_NEGATIVE_RESULTS,
        );
    }

    fn next_access(&mut self) -> u64 {
        let access = self.next_access;
        self.next_access = self.next_access.wrapping_add(1);
        access
    }
}

fn insert_bounded<K, V>(
    entries: &mut HashMap<K, TimedEntry<V>>,
    key: K,
    entry: TimedEntry<V>,
    maximum: usize,
) where
    K: Clone + Eq + std::hash::Hash,
{
    if !entries.contains_key(&key) && entries.len() == maximum {
        let least_recent = entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_access)
            .map(|(key, _)| key.clone());
        if let Some(least_recent) = least_recent {
            let _discarded = entries.remove(&least_recent);
        }
    }
    entries.insert(key, entry);
}

fn descriptor_ttl(descriptor: &SourceDescriptor) -> Duration {
    match descriptor {
        SourceDescriptor::Extract { .. } => DESCRIPTOR_TTL,
        SourceDescriptor::Missing => NEGATIVE_TTL,
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
        &format!("window:{}@{size}px", window.id.get()),
        size,
    )
    .map(Some)
}

pub(crate) fn is_shell_namespace_path(path: &Path) -> bool {
    source::is_shell_namespace_path(path)
}
