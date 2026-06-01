//! Sprint D18 — 3-phase graceful shutdown.
//!
//! Cf. architecture § 11.
//!
//! K8s sends SIGTERM ; the process has `terminationGracePeriodSeconds`
//! (60 s default) before SIGKILL. We sequence the shutdown in 3
//! strict phases :
//!
//!   1. **Phase 1 — drain RPC handlers** (≤ 10 s) :
//!      * tonic GOAWAY frame envoyé (clients re-route)
//!      * any RPC en cours peut compléter
//!   2. **Phase 2 — flush AuditEmitter** (≤ 10 s) :
//!      * D14-bis HMAC chain pending writes drained via
//!        `audit_emitter.drain_pending`
//!   3. **Phase 3 — close pg_pool + flush OTel** (≤ 30 s) :
//!      * pg_pool.close()
//!      * OtelGuard::force_flush
//!
//! **Critical invariant** : audit chain MUST be flushed before
//! pg_pool close. T-D18-11 mitigation.

use std::time::{Duration, Instant};

// **Audit fix D18-A3** — observe per-phase shutdown duration via the
// canonical D18 histogram. Always observed at the trait sealing point
// (`run_phase`), so any consumer of `ShutdownSequence` automatically
// participates in the metric.
use efe_telemetry::metrics::d18_instruments::shutdown_phase_seconds;

/// Symbolic phase identifier (used in metrics + logs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownPhase {
    /// Phase 1 : drain RPC handlers.
    DrainRpc,
    /// Phase 2 : flush AuditEmitter.
    AuditFlush,
    /// Phase 3 : close pg_pool + flush OTel.
    PgClose,
}

impl ShutdownPhase {
    /// Stable string used in metric labels.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DrainRpc => "drain_rpc",
            Self::AuditFlush => "audit_flush",
            Self::PgClose => "pg_close",
        }
    }

    /// Iterator over all phases in canonical order.
    pub fn all() -> impl Iterator<Item = Self> {
        [Self::DrainRpc, Self::AuditFlush, Self::PgClose].into_iter()
    }
}

/// Per-phase timeout configuration.
#[derive(Debug, Clone, Copy)]
pub struct ShutdownTimeouts {
    /// Phase 1 timeout.
    pub drain_rpc: Duration,
    /// Phase 2 timeout.
    pub audit_flush: Duration,
    /// Phase 3 timeout.
    pub pg_close: Duration,
}

impl ShutdownTimeouts {
    /// Default timeouts (matching architecture § 11.1).
    #[must_use]
    pub const fn default_d18() -> Self {
        Self {
            drain_rpc: Duration::from_secs(10),
            audit_flush: Duration::from_secs(10),
            pg_close: Duration::from_secs(30),
        }
    }

    /// Total wall-clock budget if all phases consume their max.
    #[must_use]
    pub fn total(&self) -> Duration {
        self.drain_rpc + self.audit_flush + self.pg_close
    }

    /// Validate : each phase > 0, total ≤ K8s
    /// terminationGracePeriodSeconds (typically 60s).
    pub fn validate(&self, k8s_grace_period: Duration) -> Result<(), String> {
        if self.drain_rpc.is_zero() {
            return Err("drain_rpc must be > 0".into());
        }
        if self.audit_flush.is_zero() {
            return Err("audit_flush must be > 0".into());
        }
        if self.pg_close.is_zero() {
            return Err("pg_close must be > 0".into());
        }
        if self.total() > k8s_grace_period {
            return Err(format!(
                "total shutdown budget ({} s) exceeds K8s gracePeriod ({} s)",
                self.total().as_secs(),
                k8s_grace_period.as_secs()
            ));
        }
        Ok(())
    }
}

impl Default for ShutdownTimeouts {
    fn default() -> Self {
        Self::default_d18()
    }
}

/// Outcome of one phase.
#[derive(Debug, Clone, Copy)]
pub struct PhaseOutcome {
    /// The phase that ran.
    pub phase: ShutdownPhase,
    /// Wall-clock duration of the phase.
    pub elapsed: Duration,
    /// `true` iff the phase completed within its timeout.
    pub within_budget: bool,
}

/// Aggregate outcome of the full shutdown sequence.
#[derive(Debug, Clone)]
pub struct ShutdownOutcome {
    /// Per-phase results.
    pub phases: Vec<PhaseOutcome>,
    /// Total wall-clock duration.
    pub total: Duration,
    /// `true` iff every phase completed within its timeout.
    pub all_clean: bool,
}

/// 3-phase shutdown driver.
///
/// Each `run_*` method takes a future representing the work for
/// that phase ; the driver wraps it in `tokio::time::timeout`,
/// records the outcome, and returns. If a phase times out, its
/// `within_budget = false` is recorded, but subsequent phases
/// still run (we want to flush as much state as possible before
/// SIGKILL).
pub struct ShutdownSequence {
    timeouts: ShutdownTimeouts,
    outcomes: Vec<PhaseOutcome>,
}

impl ShutdownSequence {
    /// Construct.
    #[must_use]
    pub fn new(timeouts: ShutdownTimeouts) -> Self {
        Self {
            timeouts,
            outcomes: Vec::with_capacity(3),
        }
    }

    /// Run phase 1 (drain RPC) with its timeout.
    pub async fn run_drain_rpc<F>(&mut self, work: F)
    where
        F: std::future::Future<Output = ()>,
    {
        self.run_phase(ShutdownPhase::DrainRpc, self.timeouts.drain_rpc, work)
            .await;
    }

    /// Run phase 2 (audit flush) with its timeout.
    pub async fn run_audit_flush<F>(&mut self, work: F)
    where
        F: std::future::Future<Output = ()>,
    {
        self.run_phase(ShutdownPhase::AuditFlush, self.timeouts.audit_flush, work)
            .await;
    }

    /// Run phase 3 (pg close) with its timeout.
    pub async fn run_pg_close<F>(&mut self, work: F)
    where
        F: std::future::Future<Output = ()>,
    {
        self.run_phase(ShutdownPhase::PgClose, self.timeouts.pg_close, work)
            .await;
    }

    async fn run_phase<F>(&mut self, phase: ShutdownPhase, timeout: Duration, work: F)
    where
        F: std::future::Future<Output = ()>,
    {
        let started = Instant::now();
        let timed = tokio::time::timeout(timeout, work).await;
        let elapsed = started.elapsed();
        let within_budget = timed.is_ok();
        if !within_budget {
            tracing::warn!(
                target: "efe.shutdown",
                phase = phase.as_str(),
                elapsed_ms = elapsed.as_millis() as u64,
                budget_ms = timeout.as_millis() as u64,
                "shutdown phase exceeded timeout"
            );
        }
        // Observe the per-phase histogram. **Audit fix D18-A3** —
        // pre-fix, the architecture declared `efe_shutdown_phase_seconds`
        // but no code observed it. The Grafana phase-duration panel
        // showed "no data".
        shutdown_phase_seconds()
            .with_label_values(&[phase.as_str()])
            .observe(elapsed.as_secs_f64());
        self.outcomes.push(PhaseOutcome {
            phase,
            elapsed,
            within_budget,
        });
    }

    /// Finalise : produce the [`ShutdownOutcome`].
    #[must_use]
    pub fn finish(self) -> ShutdownOutcome {
        let total = self.outcomes.iter().map(|p| p.elapsed).sum();
        let all_clean = self.outcomes.iter().all(|p| p.within_budget);
        ShutdownOutcome {
            phases: self.outcomes,
            total,
            all_clean,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_timeouts_match_architecture() {
        let t = ShutdownTimeouts::default_d18();
        assert_eq!(t.drain_rpc, Duration::from_secs(10));
        assert_eq!(t.audit_flush, Duration::from_secs(10));
        assert_eq!(t.pg_close, Duration::from_secs(30));
        assert_eq!(t.total(), Duration::from_secs(50));
    }

    #[test]
    fn validate_rejects_zero_phase() {
        let mut t = ShutdownTimeouts::default_d18();
        t.drain_rpc = Duration::from_secs(0);
        let r = t.validate(Duration::from_secs(60));
        assert!(r.is_err());
    }

    #[test]
    fn validate_rejects_total_over_grace() {
        let t = ShutdownTimeouts {
            drain_rpc: Duration::from_secs(30),
            audit_flush: Duration::from_secs(30),
            pg_close: Duration::from_secs(30),
        };
        let r = t.validate(Duration::from_secs(60));
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("exceeds"));
    }

    #[test]
    fn validate_accepts_default_under_60s_grace() {
        let t = ShutdownTimeouts::default_d18();
        t.validate(Duration::from_secs(60)).unwrap();
    }

    #[test]
    fn phases_iter_yields_three_in_order() {
        let all: Vec<_> = ShutdownPhase::all().collect();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0], ShutdownPhase::DrainRpc);
        assert_eq!(all[1], ShutdownPhase::AuditFlush);
        assert_eq!(all[2], ShutdownPhase::PgClose);
    }

    #[tokio::test]
    async fn happy_path_all_phases_clean() {
        let mut seq = ShutdownSequence::new(ShutdownTimeouts {
            drain_rpc: Duration::from_millis(100),
            audit_flush: Duration::from_millis(100),
            pg_close: Duration::from_millis(100),
        });
        seq.run_drain_rpc(async { /* fast no-op */ }).await;
        seq.run_audit_flush(async { /* fast no-op */ }).await;
        seq.run_pg_close(async { /* fast no-op */ }).await;
        let out = seq.finish();
        assert!(out.all_clean);
        assert_eq!(out.phases.len(), 3);
    }

    #[tokio::test]
    async fn timeout_marks_phase_not_within_budget() {
        let mut seq = ShutdownSequence::new(ShutdownTimeouts {
            drain_rpc: Duration::from_millis(50),
            audit_flush: Duration::from_millis(100),
            pg_close: Duration::from_millis(100),
        });
        // Phase 1 takes 200ms but budget is 50ms → timeout.
        seq.run_drain_rpc(tokio::time::sleep(Duration::from_millis(200)))
            .await;
        seq.run_audit_flush(async {}).await;
        seq.run_pg_close(async {}).await;
        let out = seq.finish();
        assert!(!out.all_clean);
        assert!(!out.phases[0].within_budget);
        assert!(out.phases[1].within_budget);
    }

    #[tokio::test]
    async fn phase_outcome_records_elapsed() {
        let mut seq = ShutdownSequence::new(ShutdownTimeouts::default_d18());
        seq.run_drain_rpc(tokio::time::sleep(Duration::from_millis(50)))
            .await;
        let out = seq.finish();
        assert!(out.phases[0].elapsed >= Duration::from_millis(45));
    }

    #[test]
    fn shutdown_phase_as_str_canonical() {
        assert_eq!(ShutdownPhase::DrainRpc.as_str(), "drain_rpc");
        assert_eq!(ShutdownPhase::AuditFlush.as_str(), "audit_flush");
        assert_eq!(ShutdownPhase::PgClose.as_str(), "pg_close");
    }
}
