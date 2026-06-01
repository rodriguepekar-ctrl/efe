//! Inflight `Iterate` semaphore.  Enforces `MAX_INFLIGHT` per
//! Phase-2 config (default 10).  Mitigates threat **T7** (Iterate
//! flood `DoS` — cf. `docs/security/PHASE2_THREAT_MODEL.md`).

use std::sync::Arc;

use tokio::sync::Semaphore;

/// Bounded concurrent-Iterate semaphore.
///
/// A permit is acquired when an `Iterate` RPC starts and dropped
/// when the handler returns.  Beyond `max` concurrent permits, the
/// next `try_acquire()` returns `None` and the caller MUST reject
/// with `RESOURCE_EXHAUSTED`.
#[derive(Debug, Clone)]
pub struct InflightSemaphore {
    sem: Arc<Semaphore>,
    max: usize,
}

impl InflightSemaphore {
    /// Build a semaphore with capacity `max` (minimum 1).
    #[must_use]
    pub fn new(max: usize) -> Self {
        let max = max.max(1);
        Self {
            sem: Arc::new(Semaphore::new(max)),
            max,
        }
    }

    /// Try to acquire a permit immediately.  Returns `None` if the
    /// concurrent limit is reached or the semaphore was closed.  The
    /// owned permit type is used because tonic handlers move state
    /// across `.await` points and borrowed permits do not implement
    /// `Send`.
    pub fn try_acquire(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        Arc::clone(&self.sem).try_acquire_owned().ok()
    }

    /// Maximum concurrent permits.
    #[must_use]
    pub fn max(&self) -> usize {
        self.max
    }

    /// Currently available permits (= `max` − inflight).
    #[must_use]
    pub fn available(&self) -> usize {
        self.sem.available_permits()
    }

    /// Currently in-flight Iterate calls.
    #[must_use]
    pub fn inflight(&self) -> usize {
        self.max.saturating_sub(self.sem.available_permits())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn try_acquire_succeeds_up_to_max() {
        let sem = InflightSemaphore::new(3);
        let p1 = sem.try_acquire().expect("permit 1");
        let p2 = sem.try_acquire().expect("permit 2");
        let p3 = sem.try_acquire().expect("permit 3");
        assert_eq!(sem.inflight(), 3);
        assert!(sem.try_acquire().is_none(), "4th must be rejected");
        drop(p1);
        drop(p2);
        drop(p3);
    }

    #[tokio::test]
    async fn permit_release_replenishes() {
        let sem = InflightSemaphore::new(2);
        let p1 = sem.try_acquire().expect("p1");
        let p2 = sem.try_acquire().expect("p2");
        assert!(sem.try_acquire().is_none());
        drop(p1);
        assert_eq!(sem.inflight(), 1);
        let p3 = sem.try_acquire().expect("p3 after release");
        drop(p2);
        drop(p3);
        assert_eq!(sem.inflight(), 0);
    }

    #[test]
    fn min_capacity_is_one() {
        let sem = InflightSemaphore::new(0);
        assert_eq!(sem.max(), 1);
    }

    #[tokio::test]
    async fn inflight_count_is_accurate() {
        let sem = InflightSemaphore::new(5);
        assert_eq!(sem.inflight(), 0);
        let p1 = sem.try_acquire().unwrap();
        assert_eq!(sem.inflight(), 1);
        let p2 = sem.try_acquire().unwrap();
        assert_eq!(sem.inflight(), 2);
        assert_eq!(sem.available(), 3);
        drop(p1);
        drop(p2);
        assert_eq!(sem.inflight(), 0);
    }
}
