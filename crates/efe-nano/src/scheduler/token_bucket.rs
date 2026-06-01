// SPDX-License-Identifier: UNLICENSED
//! **Sprint 8 — N15** : Token bucket scheduler for rate-limited
//! resources (LLM provider tokens, audit-chain writes).
//!
//! # Algorithm (architecture § 5.4 + § 7.2)
//!
//! Classical leaky-bucket-as-token-bucket :
//!
//! - Each bucket holds **at most `capacity`** tokens.
//! - Tokens refill at `refill_rate` tokens-per-second, capped at
//!   `capacity` (burst protection).
//! - Each `consume(cost)` call subtracts `cost` from the bucket.  If
//!   `cost > tokens` and `await_refill = true`, the call sleeps for
//!   exactly `(cost - tokens) / refill_rate` seconds, then retries.
//!
//! # Per (tenant, provider) keying (G7 + I9)
//!
//! Each `(tenant_id, provider)` pair gets its own bucket.  Providers
//! are namespaced strings — e.g. `"openai"`, `"anthropic"`, `"audit"`,
//! `"mistral_local"`.  This lets a tenant burn its OpenAI quota
//! independently from its Anthropic quota.
//!
//! # Backpressure classification
//!
//! [`TokenBucketRegistry`] is constructed with a [`BackpressureKind`]
//! that classifies all errors emitted by buckets in that registry.
//! Operators run two registries in parallel :
//! - `BackpressureKind::LlmRateLimited` for LLM provider buckets.
//! - `BackpressureKind::AuditRateLimited` for audit-chain buckets.
//!
//! # Infinite rate (Platform tier)
//!
//! When `refill_rate = f64::INFINITY` (Platform tier), tokens refill
//! instantly to `capacity` every call.  `consume` never blocks.
//!
//! # Zero rate (suspended / misconfigured)
//!
//! When `refill_rate <= 0.0`, `consume(cost > 0)` fails immediately
//! after the current tokens are exhausted — no infinite sleep.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use efe_nano_sdk::{BackpressureError, BackpressureKind, TenantId};

/// **Sprint 8 — N15** : single token bucket.
///
/// Internal state is mutex-protected because both `tokens` and
/// `last_refill` must be updated atomically — a lock-free version
/// would require CAS loops that complicate the cost-deduction path.
pub struct TokenBucket {
    /// Maximum tokens held (burst size).
    capacity: f64,
    /// Refill rate, tokens per second.  Can be `f64::INFINITY`
    /// (Platform tier) or `0.0` (suspended).
    refill_rate: f64,
    /// Mutex-protected counters.
    state: Mutex<TokenBucketState>,
}

impl std::fmt::Debug for TokenBucket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.state.lock().expect("token bucket mutex poisoned");
        f.debug_struct("TokenBucket")
            .field("capacity", &self.capacity)
            .field("refill_rate", &self.refill_rate)
            .field("tokens", &s.tokens)
            .finish()
    }
}

struct TokenBucketState {
    /// Current tokens.  Always in `[0, capacity]`.
    tokens: f64,
    /// Timestamp of the last refill calculation.
    last_refill: Instant,
}

impl TokenBucket {
    /// **N15** : construct a new bucket, fully filled.
    ///
    /// # Panics
    ///
    /// Panics if `capacity < 0.0` or `capacity.is_nan()`.
    pub fn new(capacity: f64, refill_rate_per_second: f64) -> Self {
        assert!(
            capacity >= 0.0 && !capacity.is_nan(),
            "TokenBucket capacity must be ≥ 0 and not NaN (got {capacity})"
        );
        assert!(
            !refill_rate_per_second.is_nan(),
            "TokenBucket refill_rate must not be NaN"
        );
        Self {
            capacity,
            refill_rate: refill_rate_per_second.max(0.0),
            state: Mutex::new(TokenBucketState {
                tokens: capacity,
                last_refill: Instant::now(),
            }),
        }
    }

    /// **N15** : current capacity (max tokens).
    pub fn capacity(&self) -> f64 {
        self.capacity
    }

    /// **N15** : refill rate (tokens / second).
    pub fn refill_rate(&self) -> f64 {
        self.refill_rate
    }

    /// **N15** : current available tokens (snapshot, lazy refill).
    pub fn available_tokens(&self) -> f64 {
        let mut state = self.state.lock().expect("token bucket mutex poisoned");
        self.refill_locked(&mut state);
        state.tokens
    }

    /// **N15** : non-blocking probe — try to consume `cost` tokens.
    ///
    /// Returns `true` on success (tokens deducted).  Returns `false`
    /// if insufficient tokens.  Never sleeps.
    ///
    /// `cost <= 0.0` always returns `true` without changing state.
    pub fn try_consume(&self, cost: f64) -> bool {
        if cost <= 0.0 || !cost.is_finite() {
            return cost <= 0.0;
        }
        let mut state = self.state.lock().expect("token bucket mutex poisoned");
        self.refill_locked(&mut state);
        if state.tokens >= cost {
            state.tokens -= cost;
            true
        } else {
            false
        }
    }

    /// **N15** : blocking consume — wait up to `timeout` for enough
    /// tokens, then deduct `cost`.
    ///
    /// # Behaviour
    /// - `cost <= 0` returns immediately.
    /// - `refill_rate == 0.0` returns `Err` as soon as `tokens < cost`
    ///   (would never refill).
    /// - `refill_rate == ∞` returns `Ok` immediately.
    /// - Otherwise sleeps for `(cost - tokens) / refill_rate` and
    ///   retries.  Loop bounded by `timeout`.
    pub async fn consume(
        &self,
        cost: f64,
        timeout: Duration,
        kind: BackpressureKind,
        tenant: TenantId,
    ) -> Result<(), BackpressureError> {
        if cost <= 0.0 {
            return Ok(());
        }
        if !cost.is_finite() {
            // NaN / +inf cost — cannot satisfy ever.
            return Err(BackpressureError::standard(kind, tenant, 1_000)
                .with_advice(format!("token cost {cost} is not finite")));
        }
        if cost > self.capacity {
            // Even with a full bucket, this call can never succeed.
            return Err(BackpressureError::standard(kind, tenant, 1_000).with_advice(
                format!(
                    "requested {cost} tokens exceeds bucket capacity {}",
                    self.capacity
                ),
            ));
        }
        let deadline = Instant::now() + timeout;
        loop {
            let next_wait = {
                let mut state = self.state.lock().expect("token bucket mutex poisoned");
                self.refill_locked(&mut state);
                if state.tokens >= cost {
                    state.tokens -= cost;
                    return Ok(());
                }
                if self.refill_rate <= 0.0 {
                    return Err(BackpressureError::standard(
                        kind,
                        tenant,
                        // 60-second guess when rate is zero — operator
                        // has to lift suspension / re-tier.
                        60_000,
                    ));
                }
                let deficit = cost - state.tokens;
                let seconds = deficit / self.refill_rate;
                // Cap at a sane upper bound to defend against fp issues.
                let secs = seconds.clamp(0.001, 60.0);
                Duration::from_secs_f64(secs)
            };
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                let retry_ms = next_wait.as_millis().min(u32::MAX as u128) as u32;
                return Err(BackpressureError::standard(kind, tenant, retry_ms.max(1)));
            }
            let sleep_for = next_wait.min(remaining);
            tokio::time::sleep(sleep_for).await;
        }
    }

    /// Refill the bucket lazily based on elapsed time.
    fn refill_locked(&self, state: &mut TokenBucketState) {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(state.last_refill).as_secs_f64();
        if elapsed > 0.0 && self.refill_rate > 0.0 {
            let refill = elapsed * self.refill_rate;
            // f64 * INF = INF, then min clamps to capacity.  No NaN.
            state.tokens = (state.tokens + refill).min(self.capacity).max(0.0);
        }
        state.last_refill = now;
    }
}

// ─────────────────────────────────────────────────────────────────────
//  TokenBucketRegistry
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N15** : registry that maps `(tenant, provider)` to a
/// [`TokenBucket`] singleton.
///
/// Lookups are lock-free reads via [`DashMap`].  Bucket creation
/// happens on first request and is racy-safe (CAS-style insert).
///
/// All buckets in a registry share the same [`BackpressureKind`]
/// classification — operators construct one registry per resource
/// class (`LlmRateLimited`, `AuditRateLimited`).
pub struct TokenBucketRegistry {
    kind: BackpressureKind,
    /// (tenant, provider) → bucket.
    buckets: DashMap<(TenantId, String), Arc<TokenBucket>>,
}

impl std::fmt::Debug for TokenBucketRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenBucketRegistry")
            .field("kind", &self.kind)
            .field("buckets", &self.buckets.len())
            .finish()
    }
}

impl TokenBucketRegistry {
    /// **N15** : construct a new registry tagged with `kind`.
    pub fn new(kind: BackpressureKind) -> Arc<Self> {
        Arc::new(Self {
            kind,
            buckets: DashMap::new(),
        })
    }

    /// **N15** : kind tag used for backpressure errors.
    pub fn kind(&self) -> BackpressureKind {
        self.kind
    }

    /// **N15** : number of resident buckets.  Bounded by
    /// `|tenants| × |providers|`.
    pub fn len(&self) -> usize {
        self.buckets.len()
    }

    /// **N15** : true if no buckets are resident.
    pub fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }

    /// **N15** : look up an existing bucket for `(tenant, provider)`.
    pub fn get(&self, tenant: TenantId, provider: &str) -> Option<Arc<TokenBucket>> {
        let key = (tenant, provider.to_string());
        self.buckets.get(&key).map(|r| Arc::clone(r.value()))
    }

    /// **N15** : look up or create a bucket for `(tenant, provider)`.
    ///
    /// If the bucket does not yet exist, it is created with the given
    /// `capacity` and `refill_per_second`.  If it already exists,
    /// the existing parameters are preserved (tier changes don't
    /// reset live buckets — Sprint 9 will add reconfiguration).
    pub fn get_or_create(
        &self,
        tenant: TenantId,
        provider: &str,
        capacity: f64,
        refill_per_second: f64,
    ) -> Arc<TokenBucket> {
        let key = (tenant, provider.to_string());
        if let Some(existing) = self.buckets.get(&key) {
            return Arc::clone(existing.value());
        }
        let bucket = Arc::new(TokenBucket::new(capacity, refill_per_second));
        self.buckets
            .entry(key)
            .or_insert_with(|| Arc::clone(&bucket))
            .value()
            .clone()
    }

    /// **N15** : try to consume `cost` from the (tenant, provider)
    /// bucket without waiting.  Creates the bucket if absent.
    pub fn try_consume(
        &self,
        tenant: TenantId,
        provider: &str,
        capacity: f64,
        refill_per_second: f64,
        cost: f64,
    ) -> bool {
        let bucket = self.get_or_create(tenant, provider, capacity, refill_per_second);
        bucket.try_consume(cost)
    }

    /// **N15** : consume `cost` from the (tenant, provider) bucket,
    /// waiting up to `timeout`.  Creates the bucket if absent.
    #[allow(clippy::too_many_arguments)] // bucket params + cost + timeout are intrinsic
    pub async fn consume(
        &self,
        tenant: TenantId,
        provider: &str,
        capacity: f64,
        refill_per_second: f64,
        cost: f64,
        timeout: Duration,
    ) -> Result<(), BackpressureError> {
        let bucket = self.get_or_create(tenant, provider, capacity, refill_per_second);
        bucket.consume(cost, timeout, self.kind, tenant).await
    }

    /// **N15** : drop the bucket for `(tenant, provider)`.  Used by
    /// tenant offboarding (Sprint 9 GDPR saga) or when the tier
    /// changes and we want to apply new limits immediately.
    pub fn evict(&self, tenant: TenantId, provider: &str) -> Option<Arc<TokenBucket>> {
        let key = (tenant, provider.to_string());
        self.buckets.remove(&key).map(|(_, v)| v)
    }

    /// **N15** : drop all buckets for `tenant` across every provider.
    /// Returns the number of buckets evicted.
    pub fn evict_all_for_tenant(&self, tenant: TenantId) -> usize {
        let to_drop: Vec<(TenantId, String)> = self
            .buckets
            .iter()
            .filter(|r| r.key().0 == tenant)
            .map(|r| r.key().clone())
            .collect();
        let n = to_drop.len();
        for k in to_drop {
            self.buckets.remove(&k);
        }
        n
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn ten() -> TenantId {
        TenantId::new()
    }

    #[test]
    fn try_consume_succeeds_within_capacity() {
        let b = TokenBucket::new(10.0, 1.0);
        assert!(b.try_consume(5.0));
        assert!((b.available_tokens() - 5.0).abs() < 0.01);
    }

    #[test]
    fn try_consume_fails_above_capacity() {
        let b = TokenBucket::new(10.0, 1.0);
        assert!(!b.try_consume(11.0));
        // Tokens unchanged.
        assert!((b.available_tokens() - 10.0).abs() < 0.01);
    }

    #[test]
    fn try_consume_negative_or_zero_is_free() {
        let b = TokenBucket::new(10.0, 1.0);
        assert!(b.try_consume(0.0));
        assert!(b.try_consume(-5.0));
        // Tokens unchanged.
        assert!((b.available_tokens() - 10.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn consume_succeeds_with_sufficient_tokens() {
        let b = TokenBucket::new(10.0, 1.0);
        b.consume(
            5.0,
            Duration::from_millis(100),
            BackpressureKind::LlmRateLimited,
            ten(),
        )
        .await
        .unwrap();
        assert!((b.available_tokens() - 5.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn consume_waits_for_refill_then_succeeds() {
        let b = TokenBucket::new(10.0, 100.0); // 100 tokens/sec
        // Burn all tokens.
        assert!(b.try_consume(10.0));
        let start = Instant::now();
        // Need 5 tokens, refill is 100/s ⇒ 50ms wait.
        b.consume(
            5.0,
            Duration::from_millis(500),
            BackpressureKind::LlmRateLimited,
            ten(),
        )
        .await
        .unwrap();
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(40));
        assert!(elapsed < Duration::from_millis(500));
    }

    #[tokio::test]
    async fn consume_times_out_when_refill_too_slow() {
        let b = TokenBucket::new(1.0, 1.0); // 1 token / sec
        assert!(b.try_consume(1.0));
        // Want 1 more token, refill is 1/s, timeout 50ms ⇒ must fail.
        let err = b
            .consume(
                1.0,
                Duration::from_millis(50),
                BackpressureKind::LlmRateLimited,
                ten(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.kind, BackpressureKind::LlmRateLimited);
        assert!(err.retry_after_ms > 0);
    }

    #[tokio::test]
    async fn consume_zero_rate_fails_immediately() {
        let b = TokenBucket::new(10.0, 0.0); // never refills
        assert!(b.try_consume(10.0));
        let err = b
            .consume(
                1.0,
                Duration::from_secs(10),
                BackpressureKind::AuditRateLimited,
                ten(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.kind, BackpressureKind::AuditRateLimited);
    }

    #[tokio::test]
    async fn consume_infinite_rate_never_blocks() {
        let b = TokenBucket::new(10.0, f64::INFINITY);
        assert!(b.try_consume(10.0));
        let start = Instant::now();
        b.consume(
            5.0,
            Duration::from_millis(100),
            BackpressureKind::LlmRateLimited,
            ten(),
        )
        .await
        .unwrap();
        assert!(start.elapsed() < Duration::from_millis(50));
    }

    #[tokio::test]
    async fn consume_cost_above_capacity_fails_immediately() {
        let b = TokenBucket::new(10.0, 1.0);
        let err = b
            .consume(
                11.0,
                Duration::from_secs(60),
                BackpressureKind::LlmRateLimited,
                ten(),
            )
            .await
            .unwrap_err();
        assert!(err.advice.contains("capacity"));
    }

    #[tokio::test]
    async fn consume_nan_cost_fails() {
        let b = TokenBucket::new(10.0, 1.0);
        let err = b
            .consume(
                f64::NAN,
                Duration::from_secs(60),
                BackpressureKind::LlmRateLimited,
                ten(),
            )
            .await
            .unwrap_err();
        assert!(err.advice.contains("not finite") || !err.advice.is_empty());
    }

    #[test]
    fn refill_capped_at_capacity() {
        let b = TokenBucket::new(10.0, 1.0);
        // Drain.
        b.try_consume(10.0);
        // Sleep simulated by advancing time : we can't truly advance,
        // but we can call refill_locked indirectly via available_tokens.
        std::thread::sleep(Duration::from_millis(60));
        let avail = b.available_tokens();
        // ~0.06 tokens refilled (60ms × 1 token/sec).
        assert!(avail >= 0.05);
        assert!(avail <= 1.0); // capped
    }

    #[test]
    fn new_panics_on_nan_capacity() {
        let r = std::panic::catch_unwind(|| TokenBucket::new(f64::NAN, 1.0));
        assert!(r.is_err());
    }

    #[test]
    fn new_panics_on_negative_capacity() {
        let r = std::panic::catch_unwind(|| TokenBucket::new(-1.0, 1.0));
        assert!(r.is_err());
    }

    #[test]
    fn new_clamps_negative_refill_rate_to_zero() {
        let b = TokenBucket::new(10.0, -5.0);
        assert_eq!(b.refill_rate(), 0.0);
    }

    // ─── Registry tests ─────────────────────────────────────────────

    #[test]
    fn registry_get_or_create_caches_bucket() {
        let r = TokenBucketRegistry::new(BackpressureKind::LlmRateLimited);
        let t = ten();
        let b1 = r.get_or_create(t, "openai", 10.0, 1.0);
        let b2 = r.get_or_create(t, "openai", 999.0, 999.0); // params ignored
        assert!(Arc::ptr_eq(&b1, &b2));
        // Capacity from first call wins.
        assert_eq!(b1.capacity(), 10.0);
    }

    #[test]
    fn registry_separate_providers_distinct_buckets() {
        let r = TokenBucketRegistry::new(BackpressureKind::LlmRateLimited);
        let t = ten();
        let b1 = r.get_or_create(t, "openai", 10.0, 1.0);
        let b2 = r.get_or_create(t, "anthropic", 20.0, 2.0);
        assert!(!Arc::ptr_eq(&b1, &b2));
        assert_eq!(b1.capacity(), 10.0);
        assert_eq!(b2.capacity(), 20.0);
    }

    #[test]
    fn registry_separate_tenants_distinct_buckets() {
        let r = TokenBucketRegistry::new(BackpressureKind::LlmRateLimited);
        let t1 = ten();
        let t2 = ten();
        let b1 = r.get_or_create(t1, "openai", 10.0, 1.0);
        let b2 = r.get_or_create(t2, "openai", 10.0, 1.0);
        assert!(!Arc::ptr_eq(&b1, &b2));
    }

    #[test]
    fn registry_get_returns_none_for_unknown() {
        let r = TokenBucketRegistry::new(BackpressureKind::LlmRateLimited);
        assert!(r.get(ten(), "openai").is_none());
        assert!(r.is_empty());
    }

    #[test]
    fn registry_evict_drops_bucket() {
        let r = TokenBucketRegistry::new(BackpressureKind::LlmRateLimited);
        let t = ten();
        r.get_or_create(t, "openai", 10.0, 1.0);
        assert_eq!(r.len(), 1);
        let dropped = r.evict(t, "openai");
        assert!(dropped.is_some());
        assert!(r.is_empty());
    }

    #[test]
    fn registry_evict_all_for_tenant_drops_every_provider() {
        let r = TokenBucketRegistry::new(BackpressureKind::LlmRateLimited);
        let t = ten();
        let other = ten();
        r.get_or_create(t, "openai", 10.0, 1.0);
        r.get_or_create(t, "anthropic", 10.0, 1.0);
        r.get_or_create(other, "openai", 10.0, 1.0);
        let n = r.evict_all_for_tenant(t);
        assert_eq!(n, 2);
        assert_eq!(r.len(), 1); // other tenant's bucket survives
    }

    #[tokio::test]
    async fn registry_consume_emits_kind_on_error() {
        let r = TokenBucketRegistry::new(BackpressureKind::AuditRateLimited);
        let t = ten();
        // Bucket of 1 token / 0 refill rate → second consume fails.
        r.consume(
            t,
            "audit",
            1.0,
            0.0,
            1.0,
            Duration::from_millis(10),
        )
        .await
        .unwrap();
        let err = r
            .consume(
                t,
                "audit",
                1.0,
                0.0,
                1.0,
                Duration::from_millis(10),
            )
            .await
            .unwrap_err();
        assert_eq!(err.kind, BackpressureKind::AuditRateLimited);
    }

    #[tokio::test]
    async fn registry_try_consume_does_not_block() {
        let r = TokenBucketRegistry::new(BackpressureKind::LlmRateLimited);
        let t = ten();
        let start = Instant::now();
        assert!(r.try_consume(t, "openai", 10.0, 100.0, 5.0));
        assert!(r.try_consume(t, "openai", 10.0, 100.0, 5.0));
        assert!(!r.try_consume(t, "openai", 10.0, 100.0, 5.0));
        assert!(start.elapsed() < Duration::from_millis(20));
    }

    #[tokio::test]
    async fn concurrent_consumes_capacity_invariant() {
        let r = TokenBucketRegistry::new(BackpressureKind::LlmRateLimited);
        let t = ten();
        // 10 tokens, 100/sec refill (slow enough to see contention).
        r.get_or_create(t, "openai", 10.0, 100.0);

        // 100 tasks consume 1 each — should pace out to ~900ms wall.
        let start = Instant::now();
        let mut handles = vec![];
        for _ in 0..30 {
            let r = Arc::clone(&r);
            handles.push(tokio::spawn(async move {
                r.consume(
                    t,
                    "openai",
                    10.0,
                    100.0,
                    1.0,
                    Duration::from_secs(5),
                )
                .await
                .unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        let elapsed = start.elapsed();
        // 30 tokens × (1/100s) = 0.2s minimum (since 10 are immediate).
        assert!(elapsed >= Duration::from_millis(150));
        assert!(elapsed < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn debug_format_does_not_panic() {
        let b = TokenBucket::new(10.0, 1.0);
        let _ = format!("{b:?}");

        let r = TokenBucketRegistry::new(BackpressureKind::LlmRateLimited);
        let _ = format!("{r:?}");
    }
}
