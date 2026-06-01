//! PR #20 § 11.3 — hybrid : open-loop QPS with concurrency cap.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::Semaphore;

use super::RateController;

/// Hybrid controller : token bucket gating the request rate, plus a
/// semaphore enforcing a concurrency cap.  Falls back to closed-loop
/// behaviour when the controller saturates (semaphore blocks).
pub struct HybridController {
    target_qps: Mutex<f64>,
    tokens: Mutex<f64>,
    last_refill: Mutex<Instant>,
    semaphore: Arc<Semaphore>,
    capacity: Mutex<f64>,
    dropped: AtomicU64,
    sent: AtomicU64,
}

impl HybridController {
    /// Build with a concurrency cap.  The initial QPS is 0 ; the engine
    /// updates it from the pattern engine.
    #[must_use]
    pub fn new(concurrency_cap: u32) -> Self {
        Self {
            target_qps: Mutex::new(0.0),
            tokens: Mutex::new(0.0),
            last_refill: Mutex::new(Instant::now()),
            semaphore: Arc::new(Semaphore::new(concurrency_cap.max(1) as usize)),
            capacity: Mutex::new(1.0),
            dropped: AtomicU64::new(0),
            sent: AtomicU64::new(0),
        }
    }

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
}

#[async_trait]
impl RateController for HybridController {
    async fn acquire(&self) -> bool {
        // 1. QPS gate (open-loop token bucket).
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
        if !consumed {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        // 2. Concurrency gate (closed-loop semaphore).  This blocks when
        //    saturated — by design, hybrid mode degrades to closed-loop.
        //    Permit is forgotten + re-credited via `release()` after the
        //    request completes (audit-pass fix : was leaking before).
        match self.semaphore.clone().acquire_owned().await {
            Ok(permit) => {
                permit.forget();
                self.sent.fetch_add(1, Ordering::Relaxed);
                true
            }
            Err(_) => false,
        }
    }

    fn release(&self) {
        self.semaphore.add_permits(1);
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
        "hybrid"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hybrid_default_drops_at_zero_qps() {
        let h = HybridController::new(10);
        // QPS = 0, no tokens → acquire fails.
        assert!(!h.acquire().await);
    }

    #[tokio::test]
    async fn hybrid_acquires_after_qps_set() {
        let h = HybridController::new(10);
        h.set_target_qps(100.0);
        // Wait briefly for tokens to refill.
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        let mut ok = 0;
        for _ in 0..3 {
            if h.acquire().await {
                ok += 1;
            }
        }
        assert!(ok >= 1);
    }

    #[test]
    fn name_is_hybrid() {
        let h = HybridController::new(1);
        assert_eq!(h.name(), "hybrid");
    }
}
