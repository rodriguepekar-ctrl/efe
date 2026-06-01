//! Shared health-probe state.
//!
//! Backed by atomics so the `Health` RPC handler can read it without
//! taking a Mutex.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use efe_meta_swarm_proto::meta_swarm::health_response::Status as HealthStatus;
use efe_meta_swarm_proto::meta_swarm::HealthResponse;

/// Shared health state.  Cloned freely ; the inner `Arc` makes the
/// clone cheap.
#[derive(Debug, Clone, Default)]
pub struct HealthState {
    inner: Arc<HealthInner>,
}

#[derive(Debug, Default)]
struct HealthInner {
    status: AtomicU32, // HealthStatus int repr
    inflight: AtomicU32,
    reload_gen: AtomicU32,
}

impl HealthState {
    /// New, defaulting to SERVING.
    #[must_use]
    pub fn serving() -> Self {
        let s = Self::default();
        s.inner
            .status
            .store(HealthStatus::Serving as u32, Ordering::Relaxed);
        s
    }

    /// Snapshot the current state into a `HealthResponse`.
    #[must_use]
    pub fn snapshot(&self) -> HealthResponse {
        HealthResponse {
            #[allow(clippy::cast_possible_wrap)]
            status: self.inner.status.load(Ordering::Relaxed) as i32,
            inflight: self.inner.inflight.load(Ordering::Relaxed),
            reload_gen: self.inner.reload_gen.load(Ordering::Relaxed),
        }
    }

    pub fn set_not_serving(&self) {
        self.inner
            .status
            .store(HealthStatus::NotServing as u32, Ordering::Relaxed);
    }

    pub fn set_serving(&self) {
        self.inner
            .status
            .store(HealthStatus::Serving as u32, Ordering::Relaxed);
    }

    pub fn set_reloading(&self) {
        self.inner
            .status
            .store(HealthStatus::Reloading as u32, Ordering::Relaxed);
    }

    /// Increment the in-flight counter.  Returns the new value.
    pub fn inflight_inc(&self) -> u32 {
        self.inner.inflight.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Decrement the in-flight counter.  Returns the new value.
    pub fn inflight_dec(&self) -> u32 {
        // We use saturating subtraction guard since underflow would
        // be a bug elsewhere (more decrements than increments).
        let prev = self.inner.inflight.fetch_sub(1, Ordering::Relaxed);
        prev.saturating_sub(1)
    }

    /// Increment + return the new reload generation counter.
    pub fn bump_reload_gen(&self) -> u32 {
        self.inner.reload_gen.fetch_add(1, Ordering::Relaxed) + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_serving() {
        let s = HealthState::serving();
        let snap = s.snapshot();
        assert_eq!(snap.status, HealthStatus::Serving as i32);
        assert_eq!(snap.inflight, 0);
        assert_eq!(snap.reload_gen, 0);
    }

    #[test]
    fn set_not_serving_flips_status() {
        let s = HealthState::serving();
        s.set_not_serving();
        assert_eq!(s.snapshot().status, HealthStatus::NotServing as i32);
        s.set_serving();
        assert_eq!(s.snapshot().status, HealthStatus::Serving as i32);
        s.set_reloading();
        assert_eq!(s.snapshot().status, HealthStatus::Reloading as i32);
    }

    #[test]
    fn inflight_counter() {
        let s = HealthState::serving();
        assert_eq!(s.inflight_inc(), 1);
        assert_eq!(s.inflight_inc(), 2);
        assert_eq!(s.snapshot().inflight, 2);
        assert_eq!(s.inflight_dec(), 1);
        assert_eq!(s.inflight_dec(), 0);
    }

    #[test]
    fn reload_gen_monotonic() {
        let s = HealthState::serving();
        assert_eq!(s.bump_reload_gen(), 1);
        assert_eq!(s.bump_reload_gen(), 2);
        assert_eq!(s.snapshot().reload_gen, 2);
    }
}
