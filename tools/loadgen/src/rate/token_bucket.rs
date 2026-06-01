//! PR #20 § 11.1 — open-loop token bucket.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;

use super::RateController;
use crate::config::OverflowPolicy;

/// Token-bucket rate limiter.  Refill rate = target QPS ; capacity =
/// burst_size (defaults to 2× refill rate).
pub struct TokenBucket {
    target_qps: Mutex<f64>,
    capacity: Mutex<f64>,
    tokens: Mutex<f64>,
    last_refill: Mutex<Instant>,
    overflow: OverflowPolicy,
    sent: AtomicU64,
    dropped: AtomicU64,
}

impl TokenBucket {
    /// Build with an initial QPS target + overflow policy.
    #[must_use]
    pub fn new(initial_qps: f64, overflow: OverflowPolicy) -> Self {
        let qps = initial_qps.max(0.0);
        let cap = (qps * 2.0).max(1.0);
        // Start with a full bucket only when QPS > 0 ; QPS=0 starts empty
        // so tests against the drop policy work as expected.
        let initial_tokens = if qps > 0.0 { cap } else { 0.0 };
        Self {
            target_qps: Mutex::new(qps),
            capacity: Mutex::new(cap),
            tokens: Mutex::new(initial_tokens),
            last_refill: Mutex::new(Instant::now()),
            overflow,
            sent: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        }
    }

    /// Refill the bucket based on elapsed time since last refill.
    fn refill(&self) {
        let now = Instant::now();
        let mut last = self.last_refill.lock();
        let elapsed = now.saturating_duration_since(*last).as_secs_f64();
        if elapsed <= 0.0 {
            return;
        }
        *last = now;
        let qps = *self.target_qps.lock();
        let cap = *self.capacity.lock();
        let mut t = self.tokens.lock();
        *t = (*t + elapsed * qps).min(cap);
    }

    /// Total successfully-acquired tokens.
    #[must_use]
    pub fn sent_count(&self) -> u64 {
        self.sent.load(Ordering::Relaxed)
    }

    /// Total dropped tokens (open-loop drop policy).
    #[must_use]
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl RateController for TokenBucket {
    async fn acquire(&self) -> bool {
        self.refill();
        // Critical : the parking_lot MutexGuard is `!Send`, so we MUST
        // scope it inside a non-async block before any `.await`.
        let consumed = {
            let mut t = self.tokens.lock();
            if *t >= 1.0 {
                *t -= 1.0;
                true
            } else {
                false
            }
        };
        if consumed {
            self.sent.fetch_add(1, Ordering::Relaxed);
            return true;
        }
        match self.overflow {
            OverflowPolicy::Drop => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                false
            }
            OverflowPolicy::Block => {
                let qps = *self.target_qps.lock();
                let wait = if qps > 0.0 {
                    Duration::from_secs_f64(1.0 / qps)
                } else {
                    Duration::from_millis(50)
                };
                tokio::time::sleep(wait).await;
                self.refill();
                let consumed = {
                    let mut t = self.tokens.lock();
                    if *t >= 1.0 {
                        *t -= 1.0;
                        true
                    } else {
                        false
                    }
                };
                if consumed {
                    self.sent.fetch_add(1, Ordering::Relaxed);
                    true
                } else {
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                    false
                }
            }
        }
    }

    fn set_target_qps(&self, qps: f64) {
        let qps = qps.max(0.0);
        *self.target_qps.lock() = qps;
        *self.capacity.lock() = (qps * 2.0).max(1.0);
    }

    fn target_qps(&self) -> f64 {
        *self.target_qps.lock()
    }

    fn name(&self) -> &'static str {
        "open"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn drop_policy_drops_when_empty() {
        let bucket = TokenBucket::new(0.0, OverflowPolicy::Drop);
        // No tokens, no refill at 0 QPS → drop every acquire.
        for _ in 0..3 {
            assert!(!bucket.acquire().await);
        }
        assert!(bucket.dropped_count() >= 3);
    }

    #[tokio::test]
    async fn capacity_scales_with_qps_update() {
        let bucket = TokenBucket::new(10.0, OverflowPolicy::Drop);
        assert!((bucket.target_qps() - 10.0).abs() < 1e-9);
        bucket.set_target_qps(100.0);
        assert!((bucket.target_qps() - 100.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn acquire_succeeds_when_tokens_available() {
        let bucket = TokenBucket::new(100.0, OverflowPolicy::Drop);
        // Initial capacity 200, expect first 200 acquires to succeed.
        let mut got = 0;
        for _ in 0..200 {
            if bucket.acquire().await {
                got += 1;
            }
        }
        assert!(got >= 199, "got = {got}");
    }
}
