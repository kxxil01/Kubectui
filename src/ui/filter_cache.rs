use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, LazyLock, Mutex},
};

use crate::app::AppView;

const MAX_CACHE_ENTRIES_PER_VIEW: usize = 96;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FilterCacheKey {
    query: String,
    snapshot_version: u64,
    data_fingerprint: u64,
    variant: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FilterCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub entries: usize,
}

#[derive(Debug, Default)]
struct FilterCache {
    map: HashMap<FilterCacheKey, Arc<Vec<usize>>>,
    order: VecDeque<FilterCacheKey>,
    hits: u64,
    misses: u64,
}

impl FilterCache {
    fn get(&mut self, key: &FilterCacheKey) -> Option<Arc<Vec<usize>>> {
        let value = self.map.get(key).cloned();
        if value.is_some() {
            self.hits = self.hits.saturating_add(1);
            self.touch(key);
        } else {
            self.misses = self.misses.saturating_add(1);
        }
        value
    }

    fn insert(&mut self, key: FilterCacheKey, value: Arc<Vec<usize>>) {
        if self.map.contains_key(&key) {
            self.map.insert(key.clone(), value);
            self.touch(&key);
            return;
        }
        self.map.insert(key.clone(), value);
        self.order.push_back(key);
        self.evict_if_needed();
    }

    fn touch(&mut self, key: &FilterCacheKey) {
        if self.order.back().is_some_and(|k| k == key) {
            return;
        }
        if let Some(pos) = self.order.iter().position(|item| item == key) {
            self.order.remove(pos);
            self.order.push_back(key.clone());
        }
    }

    fn evict_if_needed(&mut self) {
        while self.order.len() > MAX_CACHE_ENTRIES_PER_VIEW {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
    }

    fn stats(&self) -> FilterCacheStats {
        FilterCacheStats {
            hits: self.hits,
            misses: self.misses,
            entries: self.map.len(),
        }
    }
}

static FILTER_CACHE_SHARDS: LazyLock<Vec<Mutex<FilterCache>>> = LazyLock::new(|| {
    (0..AppView::tabs().len())
        .map(|_| Mutex::new(FilterCache::default()))
        .collect()
});

pub(crate) fn cached_filter_indices<F>(
    view: AppView,
    query: &str,
    snapshot_version: u64,
    data_fingerprint: u64,
    build: F,
) -> Arc<Vec<usize>>
where
    F: FnOnce(&str) -> Vec<usize>,
{
    cached_filter_indices_with_variant(view, query, snapshot_version, data_fingerprint, 0, build)
}

pub(crate) fn cached_filter_indices_with_variant<F>(
    view: AppView,
    query: &str,
    snapshot_version: u64,
    data_fingerprint: u64,
    variant: u64,
    build: F,
) -> Arc<Vec<usize>>
where
    F: FnOnce(&str) -> Vec<usize>,
{
    let query = query.trim();
    let key = FilterCacheKey {
        query: query.to_string(),
        snapshot_version,
        data_fingerprint,
        variant,
    };
    let shard = &FILTER_CACHE_SHARDS[view.index()];

    if let Ok(mut cache) = shard.lock()
        && let Some(hit) = cache.get(&key)
    {
        return hit;
    }

    let built = Arc::new(build(query));
    if let Ok(mut cache) = shard.lock() {
        cache.insert(key, built.clone());
    }
    built
}

pub(crate) fn data_fingerprint<T>(items: &[T], generation: u64) -> u64 {
    let len = items.len() as u64;
    generation.wrapping_mul(0x517cc1b727220a95) ^ len.rotate_left(13)
}

pub(crate) fn filter_cache_stats() -> FilterCacheStats {
    FILTER_CACHE_SHARDS
        .iter()
        .filter_map(|shard| shard.lock().ok().map(|cache| cache.stats()))
        .fold(FilterCacheStats::default(), |mut acc, stat| {
            acc.hits = acc.hits.saturating_add(stat.hits);
            acc.misses = acc.misses.saturating_add(stat.misses);
            acc.entries = acc.entries.saturating_add(stat.entries);
            acc
        })
}
