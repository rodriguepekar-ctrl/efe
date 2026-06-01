// SPDX-License-Identifier: UNLICENSED
//! **Sprint 16 — N24 + ADR-5 + ADR-6 + ADR-7** :
//! `PublisherIdentityCache` — per-runner LRU + TTL cache.
//!
//! Properties (architecture invariants) :
//! - I9 : `get` returns a CLONE — no `Arc`-sharing across requests.
//! - I14 : explicit `evict` API ; cache invalidates on revoke /
//!   suspend via `WiredComplianceRunner::evict_publisher`.
//! - ADR-6 : LRU cap at 1024 entries.
//! - ADR-7 : owned by the runner, never by the registry trait.
//! - F11 : DashMap shard-locks make concurrent insert/get safe.
//!
//! The cache does NOT hold a lock across `.await` (the underlying
//! DashMap shard locks are sync-only) — N20 audit lesson.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use efe_nano_sdk::{PublisherId, PublisherIdentity};

/// **N24 + ADR-6** : default LRU capacity.
pub const DEFAULT_CACHE_CAPACITY: usize = 1024;

/// **N24 + ADR-5** : default entry TTL in seconds.
pub const DEFAULT_CACHE_TTL_SECS: u64 = 60;

/// **N24** : an LRU + TTL cache mapping `PublisherId` →
/// `PublisherIdentity`.
pub struct PublisherIdentityCache {
    map: DashMap<PublisherId, CacheEntry>,
    capacity: usize,
    ttl: Duration,
    hit_count: AtomicU64,
    miss_count: AtomicU64,
    expired_count: AtomicU64,
    evicted_count: AtomicU64,
}

#[derive(Clone)]
struct CacheEntry {
    identity: PublisherIdentity,
    inserted_at: Instant,
}

impl std::fmt::Debug for PublisherIdentityCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PublisherIdentityCache")
            .field("capacity", &self.capacity)
            .field("ttl", &self.ttl)
            .field("len", &self.map.len())
            .field("hit_count", &self.hit_count.load(Ordering::Relaxed))
            .field("miss_count", &self.miss_count.load(Ordering::Relaxed))
            .field("expired_count", &self.expired_count.load(Ordering::Relaxed))
            .field("evicted_count", &self.evicted_count.load(Ordering::Relaxed))
            .finish()
    }
}

impl Default for PublisherIdentityCache {
    fn default() -> Self {
        Self::new(DEFAULT_CACHE_CAPACITY, Duration::from_secs(DEFAULT_CACHE_TTL_SECS))
    }
}

impl PublisherIdentityCache {
    /// **N24** : construct with explicit capacity + TTL.
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            map: DashMap::new(),
            capacity: capacity.max(1),
            ttl,
            hit_count: AtomicU64::new(0),
            miss_count: AtomicU64::new(0),
            expired_count: AtomicU64::new(0),
            evicted_count: AtomicU64::new(0),
        }
    }

    /// **N24 + I9** : look up an identity, cloning the result so
    /// the caller cannot accidentally mutate the stored value.
    ///
    /// Returns `None` on miss OR on expired entry (which is then
    /// evicted lazily on this call).
    pub fn get(&self, id: PublisherId) -> Option<PublisherIdentity> {
        let now = Instant::now();
        if let Some(entry) = self.map.get(&id) {
            let cached_at = entry.value().inserted_at;
            if now.saturating_duration_since(cached_at) < self.ttl {
                let identity = entry.value().identity.clone();
                drop(entry); // release the shard guard before counter ops
                self.hit_count.fetch_add(1, Ordering::Relaxed);
                return Some(identity);
            }
            // Expired — drop guard then remove.
            drop(entry);
            self.map.remove(&id);
            self.expired_count.fetch_add(1, Ordering::Relaxed);
        }
        self.miss_count.fetch_add(1, Ordering::Relaxed);
        None
    }

    /// **N24** : insert an identity into the cache.  Evicts the
    /// oldest entry if the cap is reached.
    pub fn put(&self, id: PublisherId, identity: PublisherIdentity) {
        if self.map.len() >= self.capacity && !self.map.contains_key(&id) {
            self.evict_oldest();
        }
        self.map.insert(
            id,
            CacheEntry {
                identity,
                inserted_at: Instant::now(),
            },
        );
    }

    /// **N24 + G12 + I14** : explicit eviction.  Returns `true` if
    /// the key was present.
    pub fn evict(&self, id: PublisherId) -> bool {
        if self.map.remove(&id).is_some() {
            self.evicted_count.fetch_add(1, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    /// **N24** : current number of cached entries.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// **N24** : true if the cache holds no entries.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// **N24** : capacity passed at construction time.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// **N24** : TTL passed at construction time.
    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    /// **N24** : cumulative hit count.
    pub fn hit_count(&self) -> u64 {
        self.hit_count.load(Ordering::Relaxed)
    }

    /// **N24** : cumulative miss count.
    pub fn miss_count(&self) -> u64 {
        self.miss_count.load(Ordering::Relaxed)
    }

    /// **N24** : cumulative count of expired entries that were
    /// lazily evicted on read.
    pub fn expired_count(&self) -> u64 {
        self.expired_count.load(Ordering::Relaxed)
    }

    /// **N24** : cumulative count of explicit + capacity-driven
    /// evictions.
    pub fn evicted_count(&self) -> u64 {
        self.evicted_count.load(Ordering::Relaxed)
    }

    /// **N24** : hit ratio in `[0.0, 1.0]` ; `0.0` when no
    /// observations.
    pub fn hit_ratio(&self) -> f64 {
        let h = self.hit_count();
        let m = self.miss_count();
        let total = h + m;
        if total == 0 {
            0.0
        } else {
            (h as f64) / (total as f64)
        }
    }

    /// **N24** : evict every entry.  Used by the admin endpoint
    /// when an operator wants a hard reset.
    pub fn clear(&self) {
        let count = self.map.len() as u64;
        self.map.clear();
        self.evicted_count.fetch_add(count, Ordering::Relaxed);
    }

    fn evict_oldest(&self) {
        // Find the oldest insertion ; remove it.  Sprint 16 ships
        // an O(N) scan ; Sprint 17 may switch to a hashlink LRU
        // if profiling shows the scan dominates.
        let oldest = self
            .map
            .iter()
            .min_by_key(|e| e.value().inserted_at)
            .map(|e| *e.key());
        if let Some(k) = oldest {
            self.map.remove(&k);
            self.evicted_count.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{KycTier, PublisherKey, PublisherStatus};
    use efe_nano_sdk::SignatureAlgorithm;
    use std::time::SystemTime;

    fn fake_identity(id: PublisherId) -> PublisherIdentity {
        PublisherIdentity {
            publisher_id: id,
            display_name: "Acme".into(),
            kyc_tier: KycTier::Tier1,
            status: PublisherStatus::Active,
            jurisdiction: vec![],
            registered_at: SystemTime::now(),
            last_sanctions_check_at: Some(SystemTime::now()),
            active_key: PublisherKey {
                key_id: efe_nano_sdk::KeyId::new(),
                algorithm: SignatureAlgorithm::Ed25519,
                public_key_pem: "x".into(),
                valid_from: SystemTime::now(),
                valid_until: None,
            },
            historical_keys: vec![],
            kyc_dossier_reference: None,
            revocation: None,
            forks_of: None,
        }
    }

    #[test]
    fn fresh_cache_is_empty() {
        let c = PublisherIdentityCache::default();
        assert_eq!(c.len(), 0);
        assert!(c.is_empty());
        assert_eq!(c.hit_count(), 0);
        assert_eq!(c.miss_count(), 0);
    }

    #[test]
    fn default_uses_documented_constants() {
        let c = PublisherIdentityCache::default();
        assert_eq!(c.capacity(), DEFAULT_CACHE_CAPACITY);
        assert_eq!(c.ttl(), Duration::from_secs(DEFAULT_CACHE_TTL_SECS));
    }

    #[test]
    fn capacity_zero_clamped_to_one() {
        let c = PublisherIdentityCache::new(0, Duration::from_secs(60));
        assert_eq!(c.capacity(), 1);
    }

    #[test]
    fn put_then_get_hits() {
        let c = PublisherIdentityCache::default();
        let id = PublisherId::new();
        c.put(id, fake_identity(id));
        let got = c.get(id);
        assert!(got.is_some());
        assert_eq!(c.hit_count(), 1);
        assert_eq!(c.miss_count(), 0);
    }

    #[test]
    fn get_missing_returns_none_and_ticks_miss() {
        let c = PublisherIdentityCache::default();
        let id = PublisherId::new();
        assert!(c.get(id).is_none());
        assert_eq!(c.miss_count(), 1);
    }

    #[test]
    fn get_returns_clone() {
        let c = PublisherIdentityCache::default();
        let id = PublisherId::new();
        let mut original = fake_identity(id);
        original.display_name = "Original".into();
        c.put(id, original);
        let mut got = c.get(id).unwrap();
        got.display_name = "Mutated".into();
        // Re-fetch ; must still be "Original" (we returned a clone).
        let again = c.get(id).unwrap();
        assert_eq!(again.display_name, "Original");
    }

    #[tokio::test]
    async fn expired_entry_returns_none_and_evicts() {
        let c = PublisherIdentityCache::new(10, Duration::from_millis(5));
        let id = PublisherId::new();
        c.put(id, fake_identity(id));
        tokio::time::sleep(Duration::from_millis(15)).await;
        assert!(c.get(id).is_none());
        assert_eq!(c.expired_count(), 1);
        assert!(c.is_empty());
    }

    #[test]
    fn evict_returns_true_on_hit_false_on_miss() {
        let c = PublisherIdentityCache::default();
        let id = PublisherId::new();
        c.put(id, fake_identity(id));
        assert!(c.evict(id));
        assert!(!c.evict(id));
        assert_eq!(c.evicted_count(), 1);
    }

    #[test]
    fn cap_breach_evicts_oldest() {
        let c = PublisherIdentityCache::new(3, Duration::from_secs(60));
        let ids: Vec<PublisherId> = (0..3).map(|_| PublisherId::new()).collect();
        for id in &ids {
            c.put(*id, fake_identity(*id));
            // Small spacing so insertion times differ enough.
            std::thread::sleep(Duration::from_millis(2));
        }
        // Cache at cap ; inserting a 4th evicts the oldest (ids[0]).
        let new_id = PublisherId::new();
        c.put(new_id, fake_identity(new_id));
        assert_eq!(c.len(), 3);
        // ids[0] no longer present.
        assert!(c.get(ids[0]).is_none(), "oldest should have been evicted");
        // new_id is present.
        assert!(c.get(new_id).is_some());
        assert!(c.evicted_count() >= 1);
    }

    #[test]
    fn put_existing_id_updates_in_place_no_eviction() {
        let c = PublisherIdentityCache::new(2, Duration::from_secs(60));
        let id = PublisherId::new();
        c.put(id, fake_identity(id));
        let evicted_before = c.evicted_count();
        c.put(id, fake_identity(id));
        assert_eq!(c.evicted_count(), evicted_before);
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn hit_ratio_zero_when_no_observations() {
        let c = PublisherIdentityCache::default();
        assert_eq!(c.hit_ratio(), 0.0);
    }

    #[test]
    fn hit_ratio_after_mixed_ops() {
        let c = PublisherIdentityCache::default();
        let id = PublisherId::new();
        c.put(id, fake_identity(id));
        c.get(id); // hit
        c.get(id); // hit
        c.get(PublisherId::new()); // miss
        let r = c.hit_ratio();
        assert!((r - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn clear_drops_all_entries() {
        let c = PublisherIdentityCache::default();
        for _ in 0..5 {
            let id = PublisherId::new();
            c.put(id, fake_identity(id));
        }
        assert_eq!(c.len(), 5);
        c.clear();
        assert_eq!(c.len(), 0);
        assert!(c.evicted_count() >= 5);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_put_get_no_torn_state() {
        let c = std::sync::Arc::new(PublisherIdentityCache::default());
        let mut handles = Vec::new();
        for _ in 0..50 {
            let c = c.clone();
            handles.push(tokio::spawn(async move {
                let id = PublisherId::new();
                c.put(id, fake_identity(id));
                c.get(id)
            }));
        }
        let mut hits = 0;
        for h in handles {
            if h.await.unwrap().is_some() {
                hits += 1;
            }
        }
        // All 50 puts then gets must hit.
        assert_eq!(hits, 50);
    }

    #[test]
    fn debug_does_not_panic() {
        let c = PublisherIdentityCache::default();
        let _ = format!("{c:?}");
    }
}
