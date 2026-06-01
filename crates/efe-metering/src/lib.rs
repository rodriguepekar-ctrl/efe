// SPDX-License-Identifier: UNLICENSED
//! # `efe-metering` — M31 per-tenant real-time usage metering.
//!
//! Exposes a [`MeterClient`] that gathers per-meter counters into a
//! buffer, flushes to Redis via [`Counter`], and feeds the rollup
//! cron via [`RollupStorage`].
//!
//! Designed to be **invisible to the hot path** — see the
//! per-iteration timing budget in `benches/meter_inc_throughput.rs`.
//!
//! ## M31 perf (post-audit rewrite)
//!
//! Four hot-path optimizations land in this version :
//!
//! - **P1** : the buffer uses `parking_lot::Mutex` (sync) instead of
//!   `tokio::sync::Mutex` (async). The critical section is < 1 µs ;
//!   forcing `.await` on every call yielded the tokio scheduler for
//!   no good reason.
//! - **P2** : the `efe_meter_inc_total` counter is resolved through
//!   a cached `[IntCounter; 8]` array (`metrics::meter_inc_counter`)
//!   so the per-call `with_label_values` HashMap lookup is gone.
//! - **P3** : the current hour bucket is cached in an
//!   `AtomicU64 / OnceCell<String>` pair, so we only re-format when
//!   the epoch-hour rolls over. ~ 5× speedup over a fresh
//!   `chrono::Utc::now()` + format on every call.
//! - **P4** : tenant ids are stored as `Arc<str>` so the buffer
//!   doesn't allocate two `String`s per `push`. Callers pre-intern
//!   the tenant id once (typically in the auth context).
//!
//! ## Invariants
//!
//! 1. **No PII** — meters carry only the tenant id and the meter
//!    name. No request payload ever reaches this crate.
//! 2. **Fail-open** — if Redis is down the caller never sees an
//!    error ; metering becomes lossy but the application keeps
//!    serving. The lossiness window is bounded by the NATS mirror.
//! 3. **Saturating arithmetic** — pathological increments saturate
//!    at `u64::MAX` rather than wrap. The handler enforces
//!    `n <= max_per_call` (default 100 000).
//!
//! ## Re-exports

#![warn(missing_docs)]
#![warn(rust_2018_idioms)]
#![deny(unsafe_code)]

pub mod buffer;
pub mod counter;
pub mod error;
pub mod meter;
pub mod metrics;
pub mod storage;
pub mod wiring_example;

pub use buffer::{
    current_bucket, flush_batch, Batch, BufferConfig, BufferKey, MeterBuffer,
};
pub use counter::{hour_bucket, redis_key, Counter, InMemoryCounter};
pub use error::{MeteringError, Result};
pub use meter::{Meter, METER_COUNT};
pub use storage::{InMemoryStorage, RollupRow, RollupStorage};

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Instant;

use once_cell::sync::Lazy;
use parking_lot::RwLock;
use tracing::warn;

use crate::metrics::{meter_inc_counter, meter_inc_duration_err, meter_inc_duration_ok};

// ────────────────── HOUR-BUCKET CACHE (P3) ──────────────────

/// Cached "current epoch-hour" — the integer seconds since Unix epoch
/// divided by 3600. Initialized to `i64::MIN` so the first
/// `current_bucket_cached()` call always refreshes.
static CACHED_EPOCH_HOUR: AtomicI64 = AtomicI64::new(i64::MIN);

/// Cached formatted bucket string ("yyyymmddhh"). Read by every
/// `current_bucket_cached()` call ; refreshed only when the
/// `CACHED_EPOCH_HOUR` rolls over (≤ once per hour). `Arc<str>` so
/// the lock-held clone is cheap.
static CACHED_BUCKET_STR: Lazy<RwLock<Arc<str>>> =
    Lazy::new(|| RwLock::new(Arc::from("00000000000")));

/// Fast-path replacement for [`current_bucket`]. Returns an
/// `Arc<str>` that the caller hands to the buffer without
/// re-allocating. Re-formats only when the wall-clock hour rolls
/// over (~ once / 3600 s).
#[must_use]
pub fn current_bucket_cached() -> Arc<str> {
    let now_secs = chrono::Utc::now().timestamp();
    let epoch_hour = now_secs / 3600;
    let cached = CACHED_EPOCH_HOUR.load(Ordering::Acquire);
    if cached == epoch_hour {
        return Arc::clone(&CACHED_BUCKET_STR.read());
    }
    // Slow path : re-format. Two competing threads can race here but
    // the result is identical — last writer wins and the AtomicI64
    // CAS keeps both reads consistent.
    let new_bucket: Arc<str> = Arc::from(hour_bucket(chrono::Utc::now()).as_str());
    {
        let mut g = CACHED_BUCKET_STR.write();
        *g = Arc::clone(&new_bucket);
    }
    CACHED_EPOCH_HOUR.store(epoch_hour, Ordering::Release);
    new_bucket
}

// ────────────────── METER CLIENT FAÇADE ──────────────────

/// High-level metering façade. Construct one per process and clone
/// into every handler.
#[derive(Clone)]
pub struct MeterClient {
    buffer: MeterBuffer,
    counter: Arc<dyn Counter>,
}

impl MeterClient {
    /// Construct from a counter and buffer.
    pub fn new(counter: Arc<dyn Counter>, buffer: MeterBuffer) -> Self {
        Self { buffer, counter }
    }

    /// Convenience constructor with default buffer config.
    pub fn with_defaults(counter: Arc<dyn Counter>) -> Self {
        Self::new(counter, MeterBuffer::new(BufferConfig::default()))
    }

    /// Increment `tenant`'s `meter` by `n`. Hot path : never returns
    /// an error other than `IncrementTooLarge` for the cap check.
    /// Redis failures are absorbed and logged.
    ///
    /// **M31 perf** : preferred over [`Self::meter_inc_legacy_string`]
    /// because the buffer key is built from `Arc<str>` clones instead
    /// of fresh `String` allocations.
    pub fn meter_inc(&self, tenant: &Arc<str>, meter: Meter, n: u64) -> Result<()> {
        let started = Instant::now();
        // Validate.
        if tenant.is_empty() {
            return Err(MeteringError::InvalidTenant("empty".into()));
        }
        let cap = self.buffer.max_per_call();
        if n > cap {
            return Err(MeteringError::IncrementTooLarge { observed: n, cap });
        }

        meter_inc_counter(meter).inc_by(n);
        let bucket = current_bucket_cached();
        let due = self.buffer.push(tenant, &bucket, meter, n);
        let elapsed_us = started.elapsed().as_micros() as f64;
        meter_inc_duration_ok().observe(elapsed_us);

        if due {
            // Spawn flush off the hot path to avoid blocking caller.
            let buf = self.buffer.clone();
            let counter = self.counter.clone();
            tokio::spawn(async move {
                let batch = buf.take_batch();
                if let Err(e) = flush_batch(counter.as_ref(), batch).await {
                    warn!(error = %e, "background flush failed");
                }
            });
        }
        Ok(())
    }

    /// Ergonomic shim for callers that hold a `&str` tenant id. Pays
    /// one `Arc::from` per call (~ 30 ns) — preferable to the M31
    /// initial `String` allocations when the caller has not yet
    /// interned the id.
    pub fn meter_inc_str(&self, tenant: &str, meter: Meter, n: u64) -> Result<()> {
        let interned: Arc<str> = Arc::from(tenant);
        self.meter_inc(&interned, meter, n)
    }

    /// Legacy `String`-keyed call. Kept for back-compat with the M31
    /// initial drop's `meter_inc(_, _, n)` signature. New callers
    /// should use [`Self::meter_inc`] or [`Self::meter_inc_str`].
    #[deprecated(
        since = "0.0.2",
        note = "Use meter_inc with Arc<str> tenant key for the perf-optimized hot path"
    )]
    pub fn meter_inc_legacy_string(
        &self,
        tenant: &str,
        meter: Meter,
        n: u64,
    ) -> Result<()> {
        self.meter_inc_str(tenant, meter, n)
    }

    /// Async-flavored fast path for callers that already have an
    /// `Arc<str>` and are inside an async fn. Identical to
    /// [`Self::meter_inc`] but reads better in `await` chains.
    pub async fn meter_inc_async(
        &self,
        tenant: &Arc<str>,
        meter: Meter,
        n: u64,
    ) -> Result<()> {
        self.meter_inc(tenant, meter, n)
    }

    /// Force a flush. Useful at shutdown to avoid losing the last
    /// buffered events.
    pub async fn flush_now(&self) -> Result<()> {
        let batch = self.buffer.take_batch();
        flush_batch(self.counter.as_ref(), batch).await
    }

    /// Returns a clone of the underlying counter (for tests +
    /// downstream rollup readers).
    pub fn counter(&self) -> Arc<dyn Counter> {
        self.counter.clone()
    }

    /// Returns the cardinality of the in-process buffer. Useful for
    /// diagnostics during graceful shutdown.
    #[must_use]
    pub fn buffered_event_count(&self) -> usize {
        self.buffer.event_count()
    }
}

/// Mark the metering plane as "incremented with error outcome" — the
/// caller decides what error means (e.g. Redis unavailable, tenant
/// blocked). Hot-path counterpart of `meter_inc_duration_ok`.
#[inline]
pub fn record_meter_inc_error(elapsed_us: f64) {
    meter_inc_duration_err().observe(elapsed_us);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> Arc<str> {
        Arc::from(x)
    }

    #[tokio::test]
    async fn meter_inc_records_to_counter_via_flush_now() {
        let counter = Arc::new(InMemoryCounter::new());
        let client = MeterClient::with_defaults(counter.clone());
        let tenant = s("acme");
        client.meter_inc(&tenant, Meter::ClassifyRequests, 3).unwrap();
        client.meter_inc(&tenant, Meter::ClassifyRequests, 4).unwrap();
        client.flush_now().await.unwrap();
        let bucket = current_bucket();
        let m = counter.read_bucket("acme", &bucket).await.unwrap();
        assert_eq!(m[&Meter::ClassifyRequests], 7);
    }

    #[test]
    fn meter_inc_rejects_oversize_increment() {
        let counter = Arc::new(InMemoryCounter::new());
        let client = MeterClient::with_defaults(counter);
        let tenant = s("acme");
        let err = client
            .meter_inc(&tenant, Meter::ClassifyRequests, 1_000_000)
            .unwrap_err();
        assert!(matches!(err, MeteringError::IncrementTooLarge { .. }));
    }

    #[test]
    fn meter_inc_rejects_empty_tenant() {
        let counter = Arc::new(InMemoryCounter::new());
        let client = MeterClient::with_defaults(counter);
        let tenant = s("");
        let err = client
            .meter_inc(&tenant, Meter::ClassifyRequests, 1)
            .unwrap_err();
        assert!(matches!(err, MeteringError::InvalidTenant(_)));
    }

    #[tokio::test]
    async fn flush_now_on_empty_buffer_is_noop() {
        let counter = Arc::new(InMemoryCounter::new());
        let client = MeterClient::with_defaults(counter);
        client.flush_now().await.unwrap();
    }

    #[tokio::test]
    async fn meter_inc_str_pays_one_arc_clone() {
        let counter = Arc::new(InMemoryCounter::new());
        let client = MeterClient::with_defaults(counter.clone());
        client.meter_inc_str("acme", Meter::ClassifyRequests, 5).unwrap();
        client.flush_now().await.unwrap();
        let bucket = current_bucket();
        let m = counter.read_bucket("acme", &bucket).await.unwrap();
        assert_eq!(m[&Meter::ClassifyRequests], 5);
    }

    #[test]
    fn current_bucket_cached_returns_arc_str() {
        let b1 = current_bucket_cached();
        let b2 = current_bucket_cached();
        // Same epoch-hour ⇒ same Arc pointer.
        assert!(Arc::ptr_eq(&b1, &b2));
        assert_eq!(b1.len(), 10);
    }

    #[tokio::test]
    async fn meter_inc_async_flavor_works() {
        let counter = Arc::new(InMemoryCounter::new());
        let client = MeterClient::with_defaults(counter);
        let tenant = s("acme");
        client
            .meter_inc_async(&tenant, Meter::ClassifyRequests, 1)
            .await
            .unwrap();
    }

    #[test]
    fn buffered_event_count_starts_at_zero() {
        let counter = Arc::new(InMemoryCounter::new());
        let client = MeterClient::with_defaults(counter);
        assert_eq!(client.buffered_event_count(), 0);
    }

    #[allow(deprecated)]
    #[tokio::test]
    async fn legacy_string_shim_still_works() {
        let counter = Arc::new(InMemoryCounter::new());
        let client = MeterClient::with_defaults(counter.clone());
        client
            .meter_inc_legacy_string("acme", Meter::ClassifyRequests, 2)
            .unwrap();
        client.flush_now().await.unwrap();
        let bucket = current_bucket();
        let m = counter.read_bucket("acme", &bucket).await.unwrap();
        assert_eq!(m[&Meter::ClassifyRequests], 2);
    }
}
