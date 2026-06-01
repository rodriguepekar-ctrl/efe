//! **Sprint M1 extension D34** — Mode-2 anchor cache.
//!
//! ## Mission
//!
//! Mode-2's LLM router calls cost 100s of ms even on the fastest
//! providers (OpenAI gpt-4o-mini ≈ 200 ms p99, Anthropic claude-haiku
//! ≈ 250 ms p99). Re-running the same query is wasteful.
//!
//! This module provides the [`Mode2Cache`] : a `DashMap`-backed,
//! per-entry-TTL cache keyed on
//! `xxhash64(query_text || top_k || router_id)`. The cache is
//! **lazy** — entries past their TTL are simply ignored on hit
//! (no background reaper). A future D38 may introduce a real
//! moka-style LRU + reaper if memory pressure becomes an issue.
//!
//! Default TTL : **5 minutes** (M4 § 13.4 contract). Tunable via
//! [`Mode2Cache::new`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use xxhash_rust::xxh64::Xxh64;

use super::router::RouterOutput;

/// Concurrent map cache for `RouterOutput`.
///
/// Cheap to clone — the inner state is `Arc<DashMap<...>>`. Two
/// clones see the same entries (institutional invariant : cache
/// state is per-`Mode2Controller` instance, not per-thread).
#[derive(Clone)]
pub struct Mode2Cache {
    inner: Arc<DashMap<u64, CacheEntry>>,
    ttl: Duration,
}

#[derive(Clone)]
struct CacheEntry {
    output: RouterOutput,
    expires_at: Instant,
}

impl std::fmt::Debug for Mode2Cache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mode2Cache")
            .field("len", &self.inner.len())
            .field("ttl_s", &self.ttl.as_secs())
            .finish()
    }
}

impl Mode2Cache {
    /// New cache with explicit TTL.
    #[must_use]
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
            ttl,
        }
    }

    /// xxhash64 of `(query, top_k, router_id)`.
    ///
    /// **Stability** : the hash function and field ordering are
    /// part of the public contract — operators rely on cache hits
    /// across server restarts (when the cache is persisted to
    /// shared storage post-D38). Changing the hash key would
    /// invalidate all caches at once.
    #[must_use]
    pub fn key(query: &str, top_k: u32, router_id: &str) -> u64 {
        let mut h = Xxh64::new(0);
        h.update(query.as_bytes());
        h.update(&top_k.to_le_bytes());
        h.update(router_id.as_bytes());
        h.digest()
    }

    /// Lookup. Returns `Some(...)` on a fresh cache hit (sets
    /// `from_cache = true` on the cloned output) ; `None` on miss
    /// or expired entry.
    pub fn get(&self, key: u64) -> Option<RouterOutput> {
        let entry = self.inner.get(&key)?;
        if entry.expires_at <= Instant::now() {
            return None;
        }
        let mut out = entry.output.clone();
        out.from_cache = true;
        Some(out)
    }

    /// Insert (overwrites any existing entry).
    pub fn insert(&self, key: u64, output: RouterOutput) {
        self.inner.insert(
            key,
            CacheEntry {
                output,
                expires_at: Instant::now() + self.ttl,
            },
        );
    }

    /// Number of entries (including expired-but-not-evicted).
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// True if the cache has no entries.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// **Sprint D34** — manually evict expired entries. Optional —
    /// the lazy-eviction path on `get` keeps correctness ; this is
    /// purely a memory-pressure release valve.
    pub fn evict_expired(&self) {
        let now = Instant::now();
        self.inner.retain(|_, entry| entry.expires_at > now);
    }
}

#[cfg(test)]
mod tests {
    use super::super::router::DynamicAnchor;
    use super::*;

    fn sample_output() -> RouterOutput {
        RouterOutput {
            anchors: vec![DynamicAnchor {
                label: "a".into(),
                centroid: vec![0.0; 8],
                confidence: 1.0,
            }],
            llm_call_id: "mock-1234".into(),
            from_cache: false,
            latency_ms: 1,
        }
    }

    #[test]
    fn key_is_stable_per_input_triplet() {
        let k1 = Mode2Cache::key("hello", 5, "mock");
        let k2 = Mode2Cache::key("hello", 5, "mock");
        let k3 = Mode2Cache::key("world", 5, "mock");
        let k4 = Mode2Cache::key("hello", 6, "mock");
        let k5 = Mode2Cache::key("hello", 5, "openai");
        assert_eq!(k1, k2);
        assert_ne!(k1, k3);
        assert_ne!(k1, k4);
        assert_ne!(k1, k5);
    }

    #[test]
    fn cache_hit_flips_from_cache_flag() {
        let cache = Mode2Cache::new(Duration::from_secs(60));
        let key = Mode2Cache::key("q", 5, "mock");
        cache.insert(key, sample_output());
        let hit = cache.get(key).expect("cache hit expected");
        assert!(hit.from_cache);
    }

    #[tokio::test]
    async fn cache_miss_after_ttl() {
        let cache = Mode2Cache::new(Duration::from_millis(0));
        let key = Mode2Cache::key("q", 5, "mock");
        cache.insert(key, sample_output());
        // sleep a tick to ensure expires_at < now.
        tokio::time::sleep(Duration::from_millis(2)).await;
        assert!(cache.get(key).is_none());
    }

    #[test]
    fn cache_get_unknown_returns_none() {
        let cache = Mode2Cache::new(Duration::from_secs(60));
        assert!(cache.get(0xdeadbeef).is_none());
    }

    #[test]
    fn cache_len_and_is_empty() {
        let cache = Mode2Cache::new(Duration::from_secs(60));
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        cache.insert(1, sample_output());
        assert!(!cache.is_empty());
        assert_eq!(cache.len(), 1);
    }

    #[tokio::test]
    async fn evict_expired_removes_old_entries() {
        let cache = Mode2Cache::new(Duration::from_millis(0));
        cache.insert(1, sample_output());
        cache.insert(2, sample_output());
        assert_eq!(cache.len(), 2);
        tokio::time::sleep(Duration::from_millis(2)).await;
        cache.evict_expired();
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn cache_clone_shares_state() {
        let c1 = Mode2Cache::new(Duration::from_secs(60));
        let c2 = c1.clone();
        c1.insert(42, sample_output());
        assert_eq!(c2.len(), 1);
        assert!(c2.get(42).is_some());
    }
}
