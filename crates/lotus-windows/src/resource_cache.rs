use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::Hash;

use crate::responsiveness::METRICS;

pub(crate) struct BoundedResourceCache<K, V> {
    entries: HashMap<K, RasterCacheEntry<V>>,
    byte_budget: usize,
    used_bytes: usize,
    next_access: u64,
}

struct RasterCacheEntry<V> {
    value: V,
    bytes: usize,
    last_access: u64,
}

impl<K, V> BoundedResourceCache<K, V>
where
    K: Clone + Eq + Hash,
{
    pub(crate) fn new(byte_budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            byte_budget,
            used_bytes: 0,
            next_access: 0,
        }
    }

    pub(crate) fn get<Q>(&mut self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        let access = self.next_access();
        let entry = self.entries.get_mut(key)?;
        entry.last_access = access;
        Some(&entry.value)
    }

    pub(crate) fn peek<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.entries.get(key).map(|entry| &entry.value)
    }

    pub(crate) fn contains<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.entries.contains_key(key)
    }

    pub(crate) fn insert(&mut self, key: K, value: V, bytes: usize) -> Option<V> {
        if bytes > self.byte_budget {
            return Some(value);
        }

        let replaced_bytes = if let Some(previous) = self.entries.remove(&key) {
            self.used_bytes = self.used_bytes.saturating_sub(previous.bytes);
            Some(previous.bytes)
        } else {
            None
        };

        while self.used_bytes.saturating_add(bytes) > self.byte_budget {
            self.evict_least_recently_used();
        }

        let access = self.next_access();
        self.entries.insert(
            key,
            RasterCacheEntry {
                value,
                bytes,
                last_access: access,
            },
        );
        self.used_bytes = self.used_bytes.saturating_add(bytes);
        if let Some(previous_bytes) = replaced_bytes {
            METRICS.record_cache_bytes_replaced(previous_bytes, bytes);
        } else {
            METRICS.record_cache_insert(bytes);
        }
        None
    }

    pub(crate) fn clear(&mut self) {
        METRICS.record_cache_remove(self.entries.len(), self.used_bytes);
        self.entries.clear();
        self.used_bytes = 0;
    }

    fn next_access(&mut self) -> u64 {
        let access = self.next_access;
        self.next_access = self.next_access.wrapping_add(1);
        access
    }

    fn evict_least_recently_used(&mut self) {
        let Some(key) = self
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_access)
            .map(|(key, _)| key.clone())
        else {
            return;
        };

        let entry = self
            .entries
            .remove(&key)
            .expect("least recently used raster cache entry must exist");
        self.used_bytes = self.used_bytes.saturating_sub(entry.bytes);
        METRICS.record_cache_eviction(entry.bytes);
    }
}

impl<K, V> Drop for BoundedResourceCache<K, V> {
    fn drop(&mut self) {
        METRICS.record_cache_remove(self.entries.len(), self.used_bytes);
    }
}
