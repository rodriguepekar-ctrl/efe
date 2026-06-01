//! Sprint D18 — PG-backed shared leaky-bucket rate-limit.
//!
//! Cf. architecture § 7.
//!
//! Pre-D18, rate-limit buckets lived in per-replica `DashMap`.
//! Under N replicas, a tenant routed across replicas saw an
//! effective bucket = N × configured-bucket → DoS bypass
//! (T-D18-3 CRIT × HIGH).
//!
//! Post-D18, bucket state lives in `efe_ratelimit_buckets`
//! (migration 0007) with `FOR UPDATE` row-level pessimistic
//! locking on each check.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

// **Audit fix D18-A3** — bump the D18 metrics from inside each impl.
use efe_telemetry::metrics::d18_instruments::{
    ratelimit_bucket_available, ratelimit_decisions_total, ratelimit_outcome_str,
    RateLimitOutcomeLabel,
};

fn bump_ratelimit_outcome(tenant_id: &str, label: RateLimitOutcomeLabel) {
    ratelimit_decisions_total()
        .with_label_values(&[tenant_id, ratelimit_outcome_str(label)])
        .inc();
}

/// Errors from the rate-limit store.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RateLimitError {
    /// SQL error from sqlx.
    #[error("sql error: {0}")]
    Sql(String),
    /// The tenant is not configured ; the caller decides default
    /// policy (typically allow).
    #[error("tenant {0:?} not configured in efe_ratelimit_buckets")]
    TenantNotConfigured(String),
    /// Invariant violation : data corruption (e.g. capacity ≤ 0).
    #[error("invariant violation: {0}")]
    Invariant(String),
}

/// Outcome of a [`RateLimitStore::check_and_decrement`] call.
#[derive(Debug, Clone, PartialEq)]
pub enum RateLimitDecision {
    /// Token taken — request may proceed.
    Allowed,
    /// Bucket empty — request throttled.
    Throttled {
        /// Tokens currently available (may be < cost).
        available: f64,
        /// Suggested retry-after delay.
        retry_after: Duration,
    },
}

impl RateLimitDecision {
    /// `true` iff the request was allowed.
    #[must_use]
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }
}

/// Shared rate-limit store. Production = [`PgRateLimitStore`] ;
/// tests = [`MockRateLimitStore`].
#[async_trait]
pub trait RateLimitStore: Send + Sync + std::fmt::Debug {
    /// Atomic check-and-decrement for a tenant. `cost` is the
    /// number of tokens to consume on success (typically 1).
    async fn check_and_decrement(
        &self,
        tenant_id: &str,
        cost: f64,
    ) -> Result<RateLimitDecision, RateLimitError>;

    /// Configure (or re-configure) a tenant's bucket. Sets
    /// `available = capacity` initially.
    async fn configure_tenant(
        &self,
        tenant_id: &str,
        capacity: u32,
        refill_per_sec: f64,
    ) -> Result<(), RateLimitError>;

    /// Read current available tokens (for metrics).
    async fn available(&self, tenant_id: &str) -> Result<f64, RateLimitError>;
}

// ====================================================================
// Production : PgRateLimitStore
// ====================================================================

/// Production impl of `RateLimitStore` backed by Postgres with
/// `FOR UPDATE` pessimistic locking.
#[derive(Debug, Clone)]
pub struct PgRateLimitStore {
    /// Symbolic source-of-truth label for tracing.
    pub label: String,
}

impl PgRateLimitStore {
    /// Construct.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
        }
    }
}

#[async_trait]
impl RateLimitStore for PgRateLimitStore {
    async fn check_and_decrement(
        &self,
        _tenant_id: &str,
        _cost: f64,
    ) -> Result<RateLimitDecision, RateLimitError> {
        bump_ratelimit_outcome(_tenant_id, RateLimitOutcomeLabel::Error);
        // **D18 v0.1.19 stub** — see note in `idempotency_pg.rs`.
        // The real impl is the algorithm in architecture § 7.2 :
        //
        //   BEGIN ;
        //   SELECT capacity, refill_per_sec, available, last_refill_at
        //     FROM efe_ratelimit_buckets WHERE tenant_id = $1 FOR UPDATE ;
        //   -- compute new_available via leak rate
        //   -- if new_available < cost : UPDATE + return Throttled
        //   -- else : new_available -= cost ; UPDATE + return Allowed
        //   COMMIT ;
        Err(RateLimitError::Sql(
            "PgRateLimitStore::check_and_decrement not yet wired (D18-bis)".into(),
        ))
    }

    async fn configure_tenant(
        &self,
        _tenant_id: &str,
        _capacity: u32,
        _refill_per_sec: f64,
    ) -> Result<(), RateLimitError> {
        Err(RateLimitError::Sql(
            "PgRateLimitStore::configure_tenant not yet wired (D18-bis)".into(),
        ))
    }

    async fn available(&self, _tenant_id: &str) -> Result<f64, RateLimitError> {
        Err(RateLimitError::Sql(
            "PgRateLimitStore::available not yet wired (D18-bis)".into(),
        ))
    }
}

// ====================================================================
// Test-only mock with the canonical leaky-bucket algorithm
// ====================================================================

/// In-memory mock impl. Implements the canonical leaky-bucket
/// algorithm (architecture § 7.2) for use in unit + integration
/// tests.
#[derive(Debug, Default)]
pub struct MockRateLimitStore {
    inner: Mutex<MockState>,
}

#[derive(Debug, Default)]
struct MockState {
    buckets: HashMap<String, MockBucket>,
}

#[derive(Debug, Clone)]
struct MockBucket {
    capacity: u32,
    refill_per_sec: f64,
    available: f64,
    last_refill_at: DateTime<Utc>,
}

impl MockRateLimitStore {
    /// Construct an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl RateLimitStore for MockRateLimitStore {
    async fn check_and_decrement(
        &self,
        tenant_id: &str,
        cost: f64,
    ) -> Result<RateLimitDecision, RateLimitError> {
        let mut g = self.inner.lock().map_err(|_| {
            bump_ratelimit_outcome(tenant_id, RateLimitOutcomeLabel::Error);
            RateLimitError::Sql("mock ratelimit mutex poisoned".into())
        })?;
        let bucket = match g.buckets.get_mut(tenant_id) {
            Some(b) => b,
            None => {
                bump_ratelimit_outcome(tenant_id, RateLimitOutcomeLabel::Unconfigured);
                return Err(RateLimitError::TenantNotConfigured(tenant_id.to_string()));
            }
        };

        let now = Utc::now();
        let elapsed_secs = (now - bucket.last_refill_at).num_milliseconds() as f64 / 1000.0;
        let new_available = (bucket.available + (elapsed_secs.max(0.0) * bucket.refill_per_sec))
            .min(bucket.capacity as f64);

        if new_available < cost {
            // Persist refill but reject.
            bucket.available = new_available.max(0.0);
            bucket.last_refill_at = now;
            let retry_secs = if bucket.refill_per_sec > 0.0 {
                ((cost - new_available) / bucket.refill_per_sec).max(0.0)
            } else {
                3600.0 // unrefillable bucket → 1h fallback
            };
            ratelimit_bucket_available()
                .with_label_values(&[tenant_id])
                .set(bucket.available as i64);
            bump_ratelimit_outcome(tenant_id, RateLimitOutcomeLabel::Throttled);
            return Ok(RateLimitDecision::Throttled {
                available: new_available.max(0.0),
                retry_after: Duration::from_secs_f64(retry_secs),
            });
        }

        bucket.available = (new_available - cost).max(0.0);
        bucket.last_refill_at = now;
        ratelimit_bucket_available()
            .with_label_values(&[tenant_id])
            .set(bucket.available as i64);
        bump_ratelimit_outcome(tenant_id, RateLimitOutcomeLabel::Allowed);
        Ok(RateLimitDecision::Allowed)
    }

    async fn configure_tenant(
        &self,
        tenant_id: &str,
        capacity: u32,
        refill_per_sec: f64,
    ) -> Result<(), RateLimitError> {
        if capacity == 0 {
            return Err(RateLimitError::Invariant("capacity must be > 0".into()));
        }
        if refill_per_sec < 0.0 {
            return Err(RateLimitError::Invariant(
                "refill_per_sec must be >= 0".into(),
            ));
        }
        let mut g = self
            .inner
            .lock()
            .map_err(|_| RateLimitError::Sql("mock ratelimit mutex poisoned".into()))?;
        g.buckets.insert(
            tenant_id.to_string(),
            MockBucket {
                capacity,
                refill_per_sec,
                available: capacity as f64,
                last_refill_at: Utc::now(),
            },
        );
        Ok(())
    }

    async fn available(&self, tenant_id: &str) -> Result<f64, RateLimitError> {
        let g = self
            .inner
            .lock()
            .map_err(|_| RateLimitError::Sql("mock ratelimit mutex poisoned".into()))?;
        g.buckets
            .get(tenant_id)
            .map(|b| b.available)
            .ok_or_else(|| RateLimitError::TenantNotConfigured(tenant_id.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unconfigured_tenant_errors() {
        let store = MockRateLimitStore::new();
        let r = store.check_and_decrement("missing", 1.0).await;
        assert!(matches!(r, Err(RateLimitError::TenantNotConfigured(_))));
    }

    #[tokio::test]
    async fn configured_tenant_allows_within_capacity() {
        let store = MockRateLimitStore::new();
        store.configure_tenant("t", 10, 1.0).await.unwrap();
        let d = store.check_and_decrement("t", 1.0).await.unwrap();
        assert_eq!(d, RateLimitDecision::Allowed);
    }

    #[tokio::test]
    async fn capacity_exceeded_throttles() {
        let store = MockRateLimitStore::new();
        // capacity 2, refill 0 → exhaustion after 2 calls.
        store.configure_tenant("t", 2, 0.0).await.unwrap();
        assert!(store
            .check_and_decrement("t", 1.0)
            .await
            .unwrap()
            .is_allowed());
        assert!(store
            .check_and_decrement("t", 1.0)
            .await
            .unwrap()
            .is_allowed());
        let d = store.check_and_decrement("t", 1.0).await.unwrap();
        assert!(matches!(d, RateLimitDecision::Throttled { .. }));
    }

    #[tokio::test]
    async fn refill_recovers_tokens_over_time() {
        let store = MockRateLimitStore::new();
        store.configure_tenant("t", 5, 100.0).await.unwrap(); // 100 t/s
                                                              // Drain to 0.
        for _ in 0..5 {
            assert!(store
                .check_and_decrement("t", 1.0)
                .await
                .unwrap()
                .is_allowed());
        }
        let d = store.check_and_decrement("t", 1.0).await.unwrap();
        assert!(matches!(d, RateLimitDecision::Throttled { .. }));
        // Wait 100ms → ~10 tokens refilled at 100 t/s.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let d2 = store.check_and_decrement("t", 1.0).await.unwrap();
        assert!(d2.is_allowed());
    }

    #[tokio::test]
    async fn configure_rejects_zero_capacity() {
        let store = MockRateLimitStore::new();
        let r = store.configure_tenant("t", 0, 1.0).await;
        assert!(matches!(r, Err(RateLimitError::Invariant(_))));
    }

    #[tokio::test]
    async fn configure_rejects_negative_refill() {
        let store = MockRateLimitStore::new();
        let r = store.configure_tenant("t", 10, -1.0).await;
        assert!(matches!(r, Err(RateLimitError::Invariant(_))));
    }

    #[tokio::test]
    async fn available_reads_current_tokens() {
        let store = MockRateLimitStore::new();
        store.configure_tenant("t", 5, 0.0).await.unwrap();
        assert_eq!(store.available("t").await.unwrap(), 5.0);
        store.check_and_decrement("t", 2.0).await.unwrap();
        let avail = store.available("t").await.unwrap();
        assert!((avail - 3.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn pg_store_stubbed_until_d18_bis() {
        let s = PgRateLimitStore::new("primary");
        let r = s.check_and_decrement("t", 1.0).await;
        assert!(matches!(r, Err(RateLimitError::Sql(_))));
    }

    #[test]
    fn rate_limit_decision_is_allowed_helper() {
        assert!(RateLimitDecision::Allowed.is_allowed());
        assert!(!RateLimitDecision::Throttled {
            available: 0.0,
            retry_after: Duration::from_secs(1),
        }
        .is_allowed());
    }
}
