//! Sprint D18 — health-check service for K8s liveness + readiness
//! probes.
//!
//! Cf. architecture § 13.
//!
//! Liveness probe : process up + gRPC responding. Failed → kill.
//! Readiness probe : ready to receive traffic. Failed → retire from
//! LB Service but keep running.
//!
//! The architecture defines `efe.health.v1.Health.Check` as the
//! readiness target. v0.1.19 ships the [`HealthService`] state
//! machine ; the gRPC handler wiring (tonic-generated) lands in
//! D18-bis when the proto include is added.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

/// Service-level health status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    /// Pod is starting ; not yet ready to serve.
    NotServing,
    /// Pod is ready ; LB Service may route traffic to it.
    Serving,
    /// Pod is shutting down ; reject new RPCs.
    ShuttingDown,
}

impl HealthStatus {
    /// Stable string used in metric labels + logs.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotServing => "not_serving",
            Self::Serving => "serving",
            Self::ShuttingDown => "shutting_down",
        }
    }

    /// Encode as u8 for the AtomicU8 backing store.
    fn as_u8(self) -> u8 {
        match self {
            Self::NotServing => 0,
            Self::Serving => 1,
            Self::ShuttingDown => 2,
        }
    }

    /// Decode from u8.
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Serving,
            2 => Self::ShuttingDown,
            _ => Self::NotServing,
        }
    }
}

/// Cheap-to-clone health service state machine. Used by both the
/// liveness probe (always returns the current status) and the
/// readiness probe (returns Serving only when all pre-conditions
/// have been ticked).
#[derive(Debug, Clone)]
pub struct HealthService {
    status: Arc<AtomicU8>,
    /// Pre-conditions that must all be true for readiness :
    pg_pool_ok: Arc<AtomicU8>,
    bundle_loaded: Arc<AtomicU8>,
    jwks_prefetched: Arc<AtomicU8>,
}

impl HealthService {
    /// Construct in `NotServing` state with no pre-conditions met.
    #[must_use]
    pub fn new() -> Self {
        Self {
            status: Arc::new(AtomicU8::new(HealthStatus::NotServing.as_u8())),
            pg_pool_ok: Arc::new(AtomicU8::new(0)),
            bundle_loaded: Arc::new(AtomicU8::new(0)),
            jwks_prefetched: Arc::new(AtomicU8::new(0)),
        }
    }

    /// Mark the PG pool as established. Once all pre-conditions are
    /// ticked, the service auto-transitions to `Serving`.
    pub fn mark_pg_pool_ok(&self) {
        self.pg_pool_ok.store(1, Ordering::SeqCst);
        self.maybe_promote();
    }

    /// Mark the embedder bundle as loaded.
    pub fn mark_bundle_loaded(&self) {
        self.bundle_loaded.store(1, Ordering::SeqCst);
        self.maybe_promote();
    }

    /// Mark the JWKS pre-fetch as done.
    pub fn mark_jwks_prefetched(&self) {
        self.jwks_prefetched.store(1, Ordering::SeqCst);
        self.maybe_promote();
    }

    /// Force `Serving` (bypasses pre-conditions ; for test +
    /// degraded-mode operators that intentionally skip a check).
    pub fn force_serving(&self) {
        self.status
            .store(HealthStatus::Serving.as_u8(), Ordering::SeqCst);
    }

    /// Begin shutdown — readiness probe goes red, liveness still
    /// green so K8s doesn't kill the pod mid-drain.
    pub fn begin_shutdown(&self) {
        self.status
            .store(HealthStatus::ShuttingDown.as_u8(), Ordering::SeqCst);
    }

    /// Current status.
    #[must_use]
    pub fn status(&self) -> HealthStatus {
        HealthStatus::from_u8(self.status.load(Ordering::SeqCst))
    }

    /// Liveness check : `true` iff the process is alive (always
    /// true unless catastrophic).
    #[must_use]
    pub fn is_live(&self) -> bool {
        // For now : if we can read the AtomicU8, we're live.
        // A future enhancement could check internal invariants.
        let _ = self.status.load(Ordering::Relaxed);
        true
    }

    /// Readiness check : `true` iff status is `Serving`.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.status() == HealthStatus::Serving
    }

    /// Check if all pre-conditions are met. Useful for debug output.
    #[must_use]
    pub fn preconditions_met(&self) -> bool {
        self.pg_pool_ok.load(Ordering::SeqCst) == 1
            && self.bundle_loaded.load(Ordering::SeqCst) == 1
            && self.jwks_prefetched.load(Ordering::SeqCst) == 1
    }

    fn maybe_promote(&self) {
        if self.preconditions_met() && self.status() == HealthStatus::NotServing {
            self.status
                .store(HealthStatus::Serving.as_u8(), Ordering::SeqCst);
        }
    }

    /// Snapshot of pre-condition state (for diagnostics).
    #[must_use]
    pub fn snapshot(&self) -> HealthSnapshot {
        HealthSnapshot {
            status: self.status(),
            pg_pool_ok: self.pg_pool_ok.load(Ordering::SeqCst) == 1,
            bundle_loaded: self.bundle_loaded.load(Ordering::SeqCst) == 1,
            jwks_prefetched: self.jwks_prefetched.load(Ordering::SeqCst) == 1,
        }
    }
}

impl Default for HealthService {
    fn default() -> Self {
        Self::new()
    }
}

/// Diagnostic snapshot of the health state machine.
#[derive(Debug, Clone, Copy)]
pub struct HealthSnapshot {
    /// Current status.
    pub status: HealthStatus,
    /// PG pool established.
    pub pg_pool_ok: bool,
    /// Embedder bundle loaded.
    pub bundle_loaded: bool,
    /// OIDC JWKS pre-fetched.
    pub jwks_prefetched: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_not_serving() {
        let h = HealthService::new();
        assert_eq!(h.status(), HealthStatus::NotServing);
        assert!(!h.is_ready());
        assert!(h.is_live());
    }

    #[test]
    fn promotes_to_serving_after_all_preconditions() {
        let h = HealthService::new();
        h.mark_pg_pool_ok();
        assert!(!h.is_ready()); // bundle + jwks still pending
        h.mark_bundle_loaded();
        assert!(!h.is_ready()); // jwks still pending
        h.mark_jwks_prefetched();
        assert!(h.is_ready());
        assert_eq!(h.status(), HealthStatus::Serving);
    }

    #[test]
    fn force_serving_bypasses_preconditions() {
        let h = HealthService::new();
        h.force_serving();
        assert!(h.is_ready());
    }

    #[test]
    fn begin_shutdown_makes_not_ready() {
        let h = HealthService::new();
        h.force_serving();
        assert!(h.is_ready());
        h.begin_shutdown();
        assert!(!h.is_ready());
        assert_eq!(h.status(), HealthStatus::ShuttingDown);
    }

    #[test]
    fn is_live_always_true() {
        let h = HealthService::new();
        assert!(h.is_live());
        h.begin_shutdown();
        assert!(h.is_live()); // still live during shutdown
    }

    #[test]
    fn snapshot_reflects_state() {
        let h = HealthService::new();
        h.mark_pg_pool_ok();
        let snap = h.snapshot();
        assert!(snap.pg_pool_ok);
        assert!(!snap.bundle_loaded);
        assert!(!snap.jwks_prefetched);
        assert_eq!(snap.status, HealthStatus::NotServing);
    }

    #[test]
    fn status_as_str_canonical() {
        assert_eq!(HealthStatus::NotServing.as_str(), "not_serving");
        assert_eq!(HealthStatus::Serving.as_str(), "serving");
        assert_eq!(HealthStatus::ShuttingDown.as_str(), "shutting_down");
    }

    #[test]
    fn clone_shares_state() {
        let h1 = HealthService::new();
        let h2 = h1.clone();
        h1.force_serving();
        assert!(h2.is_ready()); // shared Arc
    }

    #[test]
    fn idempotent_marks_dont_demote() {
        let h = HealthService::new();
        h.mark_pg_pool_ok();
        h.mark_bundle_loaded();
        h.mark_jwks_prefetched();
        assert!(h.is_ready());
        // Re-marking after Serving doesn't demote.
        h.mark_pg_pool_ok();
        assert!(h.is_ready());
    }

    #[test]
    fn preconditions_met_helper() {
        let h = HealthService::new();
        assert!(!h.preconditions_met());
        h.mark_pg_pool_ok();
        h.mark_bundle_loaded();
        h.mark_jwks_prefetched();
        assert!(h.preconditions_met());
    }
}
