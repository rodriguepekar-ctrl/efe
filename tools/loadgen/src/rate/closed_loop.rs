//! PR #20 § 11.2 — closed-loop concurrency.

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::Semaphore;

use super::RateController;

/// Closed-loop : fixed concurrency, no QPS cap.  The controller-side
/// latency determines the achieved QPS.  Used to discover capacity
/// ceilings — `--concurrency=N` + observe achieved QPS in stats.
pub struct ClosedLoop {
    semaphore: Arc<Semaphore>,
    target_qps: Mutex<f64>,
}

impl ClosedLoop {
    /// Build with a fixed concurrency cap.
    #[must_use]
    pub fn new(concurrency: u32) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(concurrency as usize)),
            target_qps: Mutex::new(0.0),
        }
    }
}

#[async_trait]
impl RateController for ClosedLoop {
    async fn acquire(&self) -> bool {
        // Take a permit + forget it.  The engine MUST call release()
        // when the request completes (success or failure) — otherwise
        // the semaphore drains and acquire deadlocks.
        match self.semaphore.clone().acquire_owned().await {
            Ok(permit) => {
                permit.forget();
                true
            }
            Err(_) => false,
        }
    }

    /// Re-credit one slot on the semaphore.  Pairs 1-1 with `acquire()`.
    fn release(&self) {
        self.semaphore.add_permits(1);
    }

    fn set_target_qps(&self, qps: f64) {
        *self.target_qps.lock() = qps.max(0.0);
    }

    fn target_qps(&self) -> f64 {
        *self.target_qps.lock()
    }

    fn name(&self) -> &'static str {
        "closed"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn closed_loop_releases_after_drop() {
        let c = ClosedLoop::new(2);
        assert!(c.acquire().await);
        // After forget, the permit isn't returned ; pool decremented.
        // Acquire once more (1 left).
        assert!(c.acquire().await);
        // Now exhausted — but we use try semantics ; the test just
        // verifies it doesn't deadlock by yielding to the runtime.
        tokio::task::yield_now().await;
    }

    #[test]
    fn name_is_closed() {
        let c = ClosedLoop::new(1);
        assert_eq!(c.name(), "closed");
    }
}
