// SPDX-License-Identifier: UNLICENSED
//! M31 — Redis-backed per-(tenant, meter, hour) counter abstraction.
//!
//! The `Counter` trait is intentionally narrow so it can be backed by
//! Redis in production and by an in-process map in tests / dev. The
//! production impl (`RedisCounter`) lives in `efe-cache` adapter
//! glue ; this crate keeps the abstraction.
//!
//! ## Key format
//!
//! ```text
//!   m31:counter:{tenant_id}:{yyyymmddhh}
//! ```
//!
//! Each Redis key is a hash whose fields are meter names and whose
//! values are u64 counters. Hour-bucketing keeps the keyspace
//! bounded (~ 24 hashes/tenant active at any one time, with TTL
//! 48 h auto-eviction).

use async_trait::async_trait;
use chrono::{DateTime, Datelike, Timelike, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::error::Result;
use crate::meter::Meter;

/// Format a `yyyymmddhh` bucket from a UTC timestamp.
#[must_use]
pub fn hour_bucket(ts: DateTime<Utc>) -> String {
    format!(
        "{:04}{:02}{:02}{:02}",
        ts.year(),
        ts.month(),
        ts.day(),
        ts.hour()
    )
}

/// Format the canonical Redis key for a (tenant, hour-bucket) pair.
#[must_use]
pub fn redis_key(tenant_id: &str, bucket: &str) -> String {
    format!("m31:counter:{tenant_id}:{bucket}")
}

/// Trait abstracting the counter backend.
#[async_trait]
pub trait Counter: Send + Sync {
    /// Increment `tenant`'s `meter` counter for `bucket` by `n`.
    /// Returns the new value (post-increment).
    async fn incr(&self, tenant: &str, bucket: &str, meter: Meter, n: u64) -> Result<u64>;

    /// Read every meter counter for `tenant` + `bucket`. Returns
    /// only meters with non-zero values.
    async fn read_bucket(&self, tenant: &str, bucket: &str) -> Result<HashMap<Meter, u64>>;

    /// Delete the bucket. Used by the rollup cron after persisting.
    async fn drop_bucket(&self, tenant: &str, bucket: &str) -> Result<()>;

    /// Enumerate all (tenant, bucket) pairs that have at least one
    /// recorded increment. Used by the rollup cron.
    ///
    /// **N.B.** — implementations should use SCAN (not KEYS) to avoid
    /// blocking the Redis event loop.
    async fn scan_buckets(&self) -> Result<Vec<(String, String)>>;
}

// ─────────────────────────── IN-MEMORY IMPL ───────────────────────────

/// Type alias for the in-memory `(tenant, bucket)` → meter-map
/// container. Pulled out to silence the clippy "very complex type"
/// warning and to ground the schema reader-side.
pub type InMemoryBuckets = HashMap<(String, String), HashMap<Meter, u64>>;

/// Test / dev counter that lives in-process. **NEVER use in
/// production** — it loses state on restart and is not shared across
/// pods.
#[derive(Default, Clone)]
pub struct InMemoryCounter {
    inner: Arc<Mutex<InMemoryBuckets>>,
}

impl InMemoryCounter {
    /// Construct a fresh empty counter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot the entire state (for test assertions).
    pub async fn snapshot(&self) -> InMemoryBuckets {
        self.inner.lock().await.clone()
    }
}

#[async_trait]
impl Counter for InMemoryCounter {
    async fn incr(&self, tenant: &str, bucket: &str, meter: Meter, n: u64) -> Result<u64> {
        let mut g = self.inner.lock().await;
        let entry = g
            .entry((tenant.to_string(), bucket.to_string()))
            .or_insert_with(HashMap::new);
        let v = entry.entry(meter).or_insert(0);
        *v = v.saturating_add(n);
        Ok(*v)
    }

    async fn read_bucket(&self, tenant: &str, bucket: &str) -> Result<HashMap<Meter, u64>> {
        let g = self.inner.lock().await;
        Ok(g.get(&(tenant.to_string(), bucket.to_string()))
            .cloned()
            .unwrap_or_default())
    }

    async fn drop_bucket(&self, tenant: &str, bucket: &str) -> Result<()> {
        let mut g = self.inner.lock().await;
        g.remove(&(tenant.to_string(), bucket.to_string()));
        Ok(())
    }

    async fn scan_buckets(&self) -> Result<Vec<(String, String)>> {
        let g = self.inner.lock().await;
        Ok(g.keys().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(hour: u32) -> DateTime<Utc> {
        // 2026-05-11 12:34:56 + hour offset
        chrono::NaiveDate::from_ymd_opt(2026, 5, 11)
            .unwrap()
            .and_hms_opt(hour, 0, 0)
            .unwrap()
            .and_utc()
    }

    #[test]
    fn hour_bucket_format() {
        assert_eq!(hour_bucket(ts(0)), "2026051100");
        assert_eq!(hour_bucket(ts(23)), "2026051123");
    }

    #[test]
    fn redis_key_format() {
        assert_eq!(
            redis_key("acme", "2026051112"),
            "m31:counter:acme:2026051112"
        );
    }

    #[tokio::test]
    async fn inmemory_increments_and_reads() {
        let c = InMemoryCounter::new();
        c.incr("t", "b", Meter::ClassifyRequests, 3).await.unwrap();
        c.incr("t", "b", Meter::ClassifyRequests, 4).await.unwrap();
        c.incr("t", "b", Meter::WebhookDeliveries, 1).await.unwrap();
        let map = c.read_bucket("t", "b").await.unwrap();
        assert_eq!(map[&Meter::ClassifyRequests], 7);
        assert_eq!(map[&Meter::WebhookDeliveries], 1);
    }

    #[tokio::test]
    async fn inmemory_drop_clears_only_target_bucket() {
        let c = InMemoryCounter::new();
        c.incr("t", "b1", Meter::ClassifyRequests, 1).await.unwrap();
        c.incr("t", "b2", Meter::ClassifyRequests, 2).await.unwrap();
        c.drop_bucket("t", "b1").await.unwrap();
        assert!(c.read_bucket("t", "b1").await.unwrap().is_empty());
        assert_eq!(
            c.read_bucket("t", "b2").await.unwrap()[&Meter::ClassifyRequests],
            2
        );
    }

    #[tokio::test]
    async fn inmemory_scan_returns_all() {
        let c = InMemoryCounter::new();
        c.incr("t1", "b", Meter::ClassifyRequests, 1).await.unwrap();
        c.incr("t2", "b", Meter::AgentRunSteps, 1).await.unwrap();
        let mut s = c.scan_buckets().await.unwrap();
        s.sort();
        assert_eq!(
            s,
            vec![("t1".into(), "b".into()), ("t2".into(), "b".into())]
        );
    }

    #[tokio::test]
    async fn inmemory_saturating_add() {
        let c = InMemoryCounter::new();
        let n = c.incr("t", "b", Meter::ClassifyRequests, u64::MAX - 5).await.unwrap();
        assert_eq!(n, u64::MAX - 5);
        let n = c.incr("t", "b", Meter::ClassifyRequests, 100).await.unwrap();
        assert_eq!(n, u64::MAX, "saturated at u64::MAX, not wrapped");
    }
}
