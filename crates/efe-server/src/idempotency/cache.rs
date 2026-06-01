//! Bounded LRU-ish cache of `(tenant, request_id) → ExecuteResponse`
//! with per-entry TTL.
//!
//! The cache is **best-effort**: it short-circuits identical replays to
//! stabilize client retry behaviour. It does not provide cryptographic
//! replay protection.
//!
//! Implementation: `DashMap` for concurrent gets ; eviction is FIFO
//! once `capacity` is reached, and TTL is enforced on read.

use std::time::{Duration, Instant};

use dashmap::DashMap;

use crate::proto::v1::ExecuteResponse;

/// Composite key: tenant + request_id.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct CacheKey {
    tenant: String,
    request_id: String,
}

#[derive(Clone, Debug)]
struct Entry {
    response: ExecuteResponse,
    inserted_at: Instant,
}

/// Idempotency cache for `SwarmService::Execute`.
#[derive(Debug)]
pub struct IdempotencyCache {
    entries: DashMap<CacheKey, Entry>,
    capacity: usize,
    ttl: Duration,
    // FIFO queue of keys, used for capacity-driven eviction.
    queue: parking_lot_like_mutex::Mutex<std::collections::VecDeque<CacheKey>>,
}

// Minimal mutex wrapper — we would rather use parking_lot but stick to
// std here to avoid yet another workspace dep.
mod parking_lot_like_mutex {
    use std::sync::{self, PoisonError};
    pub struct Mutex<T>(sync::Mutex<T>);
    impl<T> std::fmt::Debug for Mutex<T>
    where
        T: std::fmt::Debug,
    {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self.0.lock() {
                Ok(g) => f.debug_struct("Mutex").field("inner", &*g).finish(),
                Err(_) => f.write_str("Mutex(<poisoned>)"),
            }
        }
    }
    impl<T> Mutex<T> {
        pub fn new(t: T) -> Self {
            Self(sync::Mutex::new(t))
        }
        pub fn lock(&self) -> sync::MutexGuard<'_, T> {
            self.0.lock().unwrap_or_else(PoisonError::into_inner)
        }
    }
}

impl IdempotencyCache {
    /// Construct a cache with the given capacity and TTL.
    #[must_use]
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            entries: DashMap::with_capacity(capacity.max(1)),
            capacity: capacity.max(1),
            ttl,
            queue: parking_lot_like_mutex::Mutex::new(std::collections::VecDeque::with_capacity(
                capacity.max(1),
            )),
        }
    }

    /// Look up a cached response.
    ///
    /// Returns `None` on miss or expired entry. Expired entries are
    /// evicted opportunistically.
    #[must_use]
    pub fn get(&self, tenant: &str, request_id: &str) -> Option<ExecuteResponse> {
        if request_id.is_empty() {
            return None;
        }
        let key = CacheKey {
            tenant: tenant.to_string(),
            request_id: request_id.to_string(),
        };
        let now = Instant::now();
        if let Some(e) = self.entries.get(&key) {
            if now.saturating_duration_since(e.inserted_at) <= self.ttl {
                return Some(e.response.clone());
            }
            drop(e);
            self.entries.remove(&key);
        }
        None
    }

    /// Store a response. No-op if `request_id` is empty.
    ///
    /// If `(tenant, request_id)` already maps to an entry, the
    /// underlying response is refreshed **in place** and the FIFO queue
    /// is left untouched. This avoids a subtle capacity leak where a
    /// duplicate key would be queued twice: later, the first pop would
    /// evict the still-live entry even though the map was under
    /// capacity.
    pub fn insert(&self, tenant: &str, request_id: &str, resp: ExecuteResponse) {
        if request_id.is_empty() {
            return;
        }
        let key = CacheKey {
            tenant: tenant.to_string(),
            request_id: request_id.to_string(),
        };
        let entry = Entry {
            response: resp,
            inserted_at: Instant::now(),
        };
        // `DashMap::insert` returns the previous value (if any).
        // We only enqueue when the key is *new* to the map.
        let was_new = self.entries.insert(key.clone(), entry).is_none();
        if was_new {
            let mut queue = self.queue.lock();
            queue.push_back(key);
            // Evict while we exceed capacity. Defend against stale
            // queue entries (keys that were already removed from the
            // map) by looping.
            while queue.len() > self.capacity {
                match queue.pop_front() {
                    Some(victim) => {
                        self.entries.remove(&victim);
                    }
                    None => break,
                }
            }
        }
    }

    /// Current number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    fn resp(id: &str) -> ExecuteResponse {
        ExecuteResponse {
            request_id: id.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn get_miss_when_empty() {
        let c = IdempotencyCache::new(10, Duration::from_secs(1));
        assert!(c.get("t", "r1").is_none());
    }

    #[test]
    fn insert_then_get() {
        let c = IdempotencyCache::new(10, Duration::from_secs(1));
        c.insert("t", "r1", resp("r1"));
        let got = c.get("t", "r1").unwrap();
        assert_eq!(got.request_id, "r1");
    }

    #[test]
    fn ttl_expires() {
        let c = IdempotencyCache::new(10, Duration::from_millis(10));
        c.insert("t", "r1", resp("r1"));
        sleep(Duration::from_millis(25));
        assert!(c.get("t", "r1").is_none());
    }

    #[test]
    fn tenant_isolation() {
        let c = IdempotencyCache::new(10, Duration::from_secs(1));
        c.insert("a", "r1", resp("a-r1"));
        c.insert("b", "r1", resp("b-r1"));
        assert_eq!(c.get("a", "r1").unwrap().request_id, "a-r1");
        assert_eq!(c.get("b", "r1").unwrap().request_id, "b-r1");
    }

    #[test]
    fn empty_request_id_is_noop() {
        let c = IdempotencyCache::new(10, Duration::from_secs(1));
        c.insert("t", "", resp(""));
        assert!(c.get("t", "").is_none());
    }

    #[test]
    fn capacity_eviction_fifo() {
        let c = IdempotencyCache::new(2, Duration::from_secs(10));
        c.insert("t", "r1", resp("r1"));
        c.insert("t", "r2", resp("r2"));
        c.insert("t", "r3", resp("r3"));
        // r1 evicted.
        assert!(c.get("t", "r1").is_none());
        assert!(c.get("t", "r2").is_some());
        assert!(c.get("t", "r3").is_some());
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn zero_capacity_clamped_to_one() {
        let c = IdempotencyCache::new(0, Duration::from_secs(10));
        c.insert("t", "r1", resp("r1"));
        // At least stored, then eventually evicted.
        assert!(c.len() <= 1);
    }

    // Regression test for the audit finding: inserting the same key
    // twice must not double-count it in the FIFO queue, otherwise the
    // next distinct insert would evict a *live* entry even though the
    // map was under capacity.
    #[test]
    fn duplicate_insert_does_not_shrink_effective_capacity() {
        let c = IdempotencyCache::new(2, Duration::from_secs(10));
        c.insert("t", "r1", resp("r1"));
        c.insert("t", "r1", resp("r1-v2")); // dup: should NOT enqueue twice.
        c.insert("t", "r2", resp("r2"));
        // Both r1 and r2 must still be present after this sequence.
        assert_eq!(c.get("t", "r1").map(|r| r.request_id), Some("r1-v2".into()));
        assert_eq!(c.get("t", "r2").map(|r| r.request_id), Some("r2".into()));
        assert_eq!(c.len(), 2);

        // A third *distinct* key pushes capacity over — r1 must be
        // evicted (FIFO) but r2 and r3 remain.
        c.insert("t", "r3", resp("r3"));
        assert!(c.get("t", "r1").is_none());
        assert!(c.get("t", "r2").is_some());
        assert!(c.get("t", "r3").is_some());
        assert_eq!(c.len(), 2);
    }
}
