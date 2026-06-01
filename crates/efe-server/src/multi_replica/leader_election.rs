//! Sprint D18 — leader election trait + impls.
//!
//! Cf. architecture § 8.
//!
//! Pre-D18, background tasks (HMAC self-verify periodic, idempotency
//! expiry sweep, etc.) were spawned per-replica → multiple writers
//! → race conditions (T-D18-1, T-D18-10).
//!
//! Post-D18, a single replica is elected leader via K8s
//! `coordination.k8s.io/v1 Lease` ; only the leader runs background
//! tasks. Lease renewal every `renew_deadline / 2` seconds keeps
//! the leadership alive ; failure to renew → drop leadership →
//! another replica acquires.
//!
//! # Trait abstraction
//!
//! Production = [`K8sLeaseLock`]. Tests = [`MockLeaderLock`].

use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;

// **Audit fix D18-A3** — bump leader-election Prometheus metrics from
// inside each impl. Both stub + mock funnel through these helpers so a
// metric pin test can run against MockLeaderLock alone.
use efe_telemetry::metrics::d18_instruments::{
    leader_acquire_attempts_total, leader_is_leader, leader_outcome_str, LeaderOutcomeLabel,
};

fn bump_leader_outcome(label: LeaderOutcomeLabel) {
    leader_acquire_attempts_total()
        .with_label_values(&[leader_outcome_str(label)])
        .inc();
}

/// Errors from the leader election layer.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LeaderLockError {
    /// K8s API error (typically when called against a real cluster).
    #[error("k8s api error: {0}")]
    K8sApi(String),
    /// Lease conflict — another holder won the race.
    #[error("lease held by another replica: {holder:?}")]
    HeldByOther { holder: String },
    /// Lease lost during renewal.
    #[error("lease lost during renewal")]
    LeaseLost,
    /// Configuration error.
    #[error("configuration error: {0}")]
    Config(String),
}

/// Returned by [`LeaderLock::try_acquire`] when the call succeeds.
#[derive(Debug, Clone)]
pub struct LeaseClaim {
    /// Holder identity (typically `HOSTNAME` of the pod).
    pub holder: String,
    /// Symbolic lease name (e.g. `efe-leader`).
    pub name: String,
    /// Lease duration. Must be renewed before this elapses.
    pub lease_duration: Duration,
}

/// Leader-lock trait. Two impls : [`K8sLeaseLock`] for production +
/// [`MockLeaderLock`] for tests.
#[async_trait]
pub trait LeaderLock: Send + Sync + std::fmt::Debug {
    /// Try to acquire (or renew) the lease. Returns Ok with a
    /// [`LeaseClaim`] iff this replica now holds the lease.
    async fn try_acquire(&self) -> Result<LeaseClaim, LeaderLockError>;

    /// Renew the lease. Should be called every
    /// `renew_deadline / 2` seconds to keep leadership.
    async fn renew(&self) -> Result<(), LeaderLockError>;

    /// Release the lease (best-effort ; called on graceful shutdown).
    async fn release(&self) -> Result<(), LeaderLockError>;

    /// `true` iff this replica currently believes it holds the lease.
    fn is_held(&self) -> bool;
}

// ====================================================================
// Production : K8sLeaseLock
// ====================================================================

/// K8s `coordination.k8s.io/v1 Lease`-backed leader lock.
///
/// **D18 v0.1.19 stub** — the full impl uses the `kube` crate
/// (kube-rs) to talk to the K8s apiserver. To keep the BOM minimal
/// in v0.1.19 (and avoid pulling kube-rs into every test build),
/// the actual K8s client wiring is deferred to D18-bis ; this
/// struct preserves the trait shape so consumer code is stable.
#[derive(Debug)]
pub struct K8sLeaseLock {
    /// Lease name (e.g. `"efe-leader"`).
    pub name: String,
    /// K8s namespace.
    pub namespace: String,
    /// Holder identity (typically `HOSTNAME`).
    pub holder_id: String,
    /// Lease duration. Default 15 s.
    pub lease_duration: Duration,
    /// Renew deadline. Default 10 s.
    pub renew_deadline: Duration,
    held: Mutex<bool>,
}

impl K8sLeaseLock {
    /// Construct.
    pub fn new(
        name: impl Into<String>,
        namespace: impl Into<String>,
        holder_id: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            namespace: namespace.into(),
            holder_id: holder_id.into(),
            lease_duration: Duration::from_secs(15),
            renew_deadline: Duration::from_secs(10),
            held: Mutex::new(false),
        }
    }

    /// Override the lease duration (must be > renew_deadline).
    pub fn with_lease_duration(mut self, d: Duration) -> Self {
        self.lease_duration = d;
        self
    }

    /// Override the renew deadline.
    pub fn with_renew_deadline(mut self, d: Duration) -> Self {
        self.renew_deadline = d;
        self
    }
}

#[async_trait]
impl LeaderLock for K8sLeaseLock {
    async fn try_acquire(&self) -> Result<LeaseClaim, LeaderLockError> {
        // **D18 v0.1.19 stub**. Real impl :
        //
        //   * GET /apis/coordination.k8s.io/v1/namespaces/<ns>/leases/<name>
        //     - if 404 : POST to create with our holder_identity.
        //     - if found and still-valid (renewTime + leaseDurationSeconds
        //       > now) : return HeldByOther.
        //     - else : PUT to update with our holder_identity + renewTime=now.
        //
        // Until the kube-rs wiring ships, callers using this impl
        // get a Config error so the deployment fail-fast at boot
        // rather than running with broken leader election.
        bump_leader_outcome(LeaderOutcomeLabel::Error);
        Err(LeaderLockError::Config(
            "K8sLeaseLock::try_acquire not yet wired (D18-bis ; \
             use MockLeaderLock for tests)"
                .into(),
        ))
    }

    async fn renew(&self) -> Result<(), LeaderLockError> {
        Err(LeaderLockError::Config(
            "K8sLeaseLock::renew not yet wired (D18-bis)".into(),
        ))
    }

    async fn release(&self) -> Result<(), LeaderLockError> {
        // Best-effort release — even when stubbed, we update the
        // local held flag so subsequent calls don't keep claiming.
        if let Ok(mut g) = self.held.lock() {
            *g = false;
        }
        leader_is_leader().set(0);
        Ok(())
    }

    fn is_held(&self) -> bool {
        self.held.lock().map(|g| *g).unwrap_or(false)
    }
}

// ====================================================================
// Test-only mock
// ====================================================================

/// In-memory mock leader lock. Useful for unit + integration tests.
///
/// Construct with [`MockLeaderLock::available`] for "lease is
/// free" semantics, or [`MockLeaderLock::held_by_other`] for
/// "another replica owns it" semantics.
#[derive(Debug)]
pub struct MockLeaderLock {
    state: Mutex<MockState>,
}

#[derive(Debug)]
struct MockState {
    held_by_us: bool,
    holder_other: Option<String>,
    name: String,
    holder_id: String,
    lease_duration: Duration,
    /// If true, simulate K8s API returning an error on the next call.
    fail_next: bool,
}

impl MockLeaderLock {
    /// Mock that grants the lease on first try.
    pub fn available(holder_id: impl Into<String>) -> Self {
        Self {
            state: Mutex::new(MockState {
                held_by_us: false,
                holder_other: None,
                name: "efe-leader".into(),
                holder_id: holder_id.into(),
                lease_duration: Duration::from_secs(15),
                fail_next: false,
            }),
        }
    }

    /// Mock that refuses the lease (other holder).
    pub fn held_by_other(other: impl Into<String>) -> Self {
        let other_s = other.into();
        Self {
            state: Mutex::new(MockState {
                held_by_us: false,
                holder_other: Some(other_s),
                name: "efe-leader".into(),
                holder_id: "self".into(),
                lease_duration: Duration::from_secs(15),
                fail_next: false,
            }),
        }
    }

    /// Make the next call fail (simulates K8s API outage).
    pub fn fail_next_call(&self) {
        if let Ok(mut g) = self.state.lock() {
            g.fail_next = true;
        }
    }

    /// Force-release for tests that simulate handoff.
    pub fn force_release_to_other(&self, other: impl Into<String>) {
        if let Ok(mut g) = self.state.lock() {
            g.held_by_us = false;
            g.holder_other = Some(other.into());
        }
    }
}

#[async_trait]
impl LeaderLock for MockLeaderLock {
    async fn try_acquire(&self) -> Result<LeaseClaim, LeaderLockError> {
        let mut g = self.state.lock().map_err(|_| {
            bump_leader_outcome(LeaderOutcomeLabel::Error);
            LeaderLockError::Config("mock leader mutex poisoned".into())
        })?;
        if g.fail_next {
            g.fail_next = false;
            bump_leader_outcome(LeaderOutcomeLabel::Error);
            return Err(LeaderLockError::K8sApi("simulated api outage".into()));
        }
        if let Some(other) = &g.holder_other {
            bump_leader_outcome(LeaderOutcomeLabel::HeldByOther);
            return Err(LeaderLockError::HeldByOther {
                holder: other.clone(),
            });
        }
        g.held_by_us = true;
        leader_is_leader().set(1);
        bump_leader_outcome(LeaderOutcomeLabel::Acquired);
        Ok(LeaseClaim {
            holder: g.holder_id.clone(),
            name: g.name.clone(),
            lease_duration: g.lease_duration,
        })
    }

    async fn renew(&self) -> Result<(), LeaderLockError> {
        let mut g = self
            .state
            .lock()
            .map_err(|_| LeaderLockError::Config("mock leader mutex poisoned".into()))?;
        if g.fail_next {
            g.fail_next = false;
            leader_is_leader().set(0);
            return Err(LeaderLockError::LeaseLost);
        }
        if !g.held_by_us {
            leader_is_leader().set(0);
            return Err(LeaderLockError::LeaseLost);
        }
        Ok(())
    }

    async fn release(&self) -> Result<(), LeaderLockError> {
        let mut g = self
            .state
            .lock()
            .map_err(|_| LeaderLockError::Config("mock leader mutex poisoned".into()))?;
        g.held_by_us = false;
        leader_is_leader().set(0);
        Ok(())
    }

    fn is_held(&self) -> bool {
        self.state.lock().map(|g| g.held_by_us).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn available_lock_grants_on_first_try() {
        let lock = MockLeaderLock::available("pod-0");
        let claim = lock.try_acquire().await.unwrap();
        assert_eq!(claim.holder, "pod-0");
        assert!(lock.is_held());
    }

    #[tokio::test]
    async fn held_by_other_returns_error() {
        let lock = MockLeaderLock::held_by_other("pod-1");
        let r = lock.try_acquire().await;
        assert!(matches!(r, Err(LeaderLockError::HeldByOther { .. })));
        assert!(!lock.is_held());
    }

    #[tokio::test]
    async fn renew_works_when_held() {
        let lock = MockLeaderLock::available("pod-0");
        lock.try_acquire().await.unwrap();
        lock.renew().await.unwrap();
    }

    #[tokio::test]
    async fn renew_errors_when_not_held() {
        let lock = MockLeaderLock::held_by_other("other");
        let r = lock.renew().await;
        assert!(matches!(r, Err(LeaderLockError::LeaseLost)));
    }

    #[tokio::test]
    async fn release_makes_held_false() {
        let lock = MockLeaderLock::available("pod-0");
        lock.try_acquire().await.unwrap();
        assert!(lock.is_held());
        lock.release().await.unwrap();
        assert!(!lock.is_held());
    }

    #[tokio::test]
    async fn fail_next_returns_api_error() {
        let lock = MockLeaderLock::available("pod-0");
        lock.fail_next_call();
        let r = lock.try_acquire().await;
        assert!(matches!(r, Err(LeaderLockError::K8sApi(_))));
    }

    #[tokio::test]
    async fn force_release_to_other_handoff() {
        let lock = MockLeaderLock::available("pod-0");
        lock.try_acquire().await.unwrap();
        assert!(lock.is_held());
        lock.force_release_to_other("pod-1");
        assert!(!lock.is_held());
        let r = lock.try_acquire().await;
        assert!(matches!(r, Err(LeaderLockError::HeldByOther { .. })));
    }

    #[tokio::test]
    async fn k8s_lease_lock_constructor_works() {
        let lock = K8sLeaseLock::new("efe-leader", "efe", "pod-0")
            .with_lease_duration(Duration::from_secs(20))
            .with_renew_deadline(Duration::from_secs(15));
        assert_eq!(lock.name, "efe-leader");
        assert_eq!(lock.namespace, "efe");
        assert_eq!(lock.lease_duration, Duration::from_secs(20));
        // Stubbed methods return Config errors.
        let r = lock.try_acquire().await;
        assert!(matches!(r, Err(LeaderLockError::Config(_))));
    }

    #[tokio::test]
    async fn k8s_lease_lock_release_is_noop_safe() {
        let lock = K8sLeaseLock::new("efe-leader", "efe", "pod-0");
        // Release does NOT return Config error (best-effort).
        lock.release().await.unwrap();
        assert!(!lock.is_held());
    }
}
