//! `TracingTelemetryGuard` — per-swarm state accumulator that emits
//! structured `tracing` events and increments Prometheus instruments.
//!
//! Lifecycle:
//! 1. Constructed by `TracingTelemetryCollector::begin_swarm` (see the
//!    parent module).
//! 2. Records phase completions, round stats, custom events, errors.
//! 3. Terminates with [`TelemetryGuard::finalize`] which commits the
//!    final verdict / latency. If the guard is dropped **before**
//!    `finalize` (typically a panic path), the defensive `Drop` impl
//!    emits a WARN so the incident is visible in logs.

use std::time::{Duration, Instant};

use efe_core::fusion::FusionStats;
use efe_core::{EfeError, SwarmId, TelemetryGuard};

use crate::metrics::instruments;

/// Per-swarm telemetry guard.
pub struct TracingTelemetryGuard {
    swarm_id: SwarmId,
    tenant: String,
    request_id: String,
    started_at: Instant,
    rounds_recorded: u32,
    finalized: bool,
    last_verdict: &'static str,
}

impl std::fmt::Debug for TracingTelemetryGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TracingTelemetryGuard")
            .field("swarm_id", &self.swarm_id.to_string())
            .field("tenant", &self.tenant)
            .field("rounds", &self.rounds_recorded)
            .field("finalized", &self.finalized)
            .finish()
    }
}

impl TracingTelemetryGuard {
    /// Open a new guard for a swarm.
    pub(crate) fn new(id: SwarmId, req: &efe_core::Request) -> Self {
        tracing::info!(
            swarm_id = %id,
            tenant_id = %req.tenant_id,
            request_id = %req.id,
            n_agents = req.spec.population.total(),
            dim_latent = req.spec.dim_latent.get(),
            max_rounds = req.spec.max_rounds.get(),
            "swarm begin"
        );
        Self {
            swarm_id: id,
            tenant: req.tenant_id.clone(),
            request_id: req.id.as_str().to_string(),
            started_at: Instant::now(),
            rounds_recorded: 0,
            finalized: false,
            last_verdict: "pending",
        }
    }

    /// Return a classification suitable for the `verdict` label of the
    /// `efe_swarm_duration_seconds` histogram.
    #[must_use]
    pub fn last_verdict(&self) -> &'static str {
        self.last_verdict
    }
}

impl TelemetryGuard for TracingTelemetryGuard {
    fn record_phase_complete(&mut self, phase: &str, elapsed: Duration) {
        tracing::debug!(
            swarm_id = %self.swarm_id,
            tenant_id = %self.tenant,
            request_id = %self.request_id,
            phase = phase,
            elapsed_ms = elapsed.as_secs_f64() * 1000.0,
            "phase complete"
        );
    }

    fn record_phase_error(&mut self, phase: &str, err: &EfeError) {
        let code = err.code();
        instruments::errors_total()
            .with_label_values(&[code, &self.tenant])
            .inc();
        tracing::error!(
            swarm_id = %self.swarm_id,
            tenant_id = %self.tenant,
            request_id = %self.request_id,
            phase = phase,
            error_code = code,
            error = %err,
            "phase error"
        );
    }

    fn record_round(&mut self, round: u32, stats: &FusionStats) {
        self.rounds_recorded = self.rounds_recorded.saturating_add(1);
        instruments::swarm_rounds_total().inc();
        tracing::debug!(
            swarm_id = %self.swarm_id,
            tenant_id = %self.tenant,
            round = round,
            variance = stats.variance,
            gradient_norm = stats.gradient_norm,
            outlier_ratio = stats.outlier_ratio,
            n_active = stats.n_active,
            has_nan = stats.has_nan,
            has_inf = stats.has_inf,
            "round stats"
        );
    }

    fn record_event(&mut self, round: u32, event: &str) {
        tracing::info!(
            swarm_id = %self.swarm_id,
            tenant_id = %self.tenant,
            round = round,
            event = event,
            "swarm event"
        );
    }

    fn finalize(&mut self, total_ms: f32, stats: &FusionStats) {
        self.finalized = true;
        // Derive a coarse verdict label for the duration histogram.
        let verdict = if stats.has_numerical_issues() {
            "numerical_issue"
        } else if stats.confidence >= 0.5 {
            "converged"
        } else {
            "inconclusive"
        };
        self.last_verdict = verdict;

        let seconds = (total_ms as f64) / 1000.0;
        instruments::swarm_duration_seconds()
            .with_label_values(&[verdict, &self.tenant])
            .observe(seconds);

        if stats.confidence.is_finite() && stats.confidence >= 0.0 {
            instruments::swarm_confidence()
                .with_label_values(&[&self.tenant])
                .observe(stats.confidence as f64);
        }

        // Sprint D6 audit fix : if `rounds_recorded` was never
        // incremented (typical case when this guard wraps a backend
        // that runs the controller with `NullTelemetryCollector`
        // inside — `InstrumentedBackend` is the outer wrapper and
        // never receives `record_round` events), fall back to the
        // round count carried by the final stats. Without this fix
        // the operator-facing log always read `rounds=0` even on
        // healthy multi-round runs, which masked the post-D6
        // accuracy investigation.
        let logged_rounds = if self.rounds_recorded == 0 {
            stats.round
        } else {
            self.rounds_recorded
        };

        tracing::info!(
            swarm_id = %self.swarm_id,
            tenant_id = %self.tenant,
            request_id = %self.request_id,
            total_ms = total_ms,
            rounds = logged_rounds,
            verdict = verdict,
            confidence = stats.confidence,
            "swarm finalize"
        );
    }
}

impl Drop for TracingTelemetryGuard {
    fn drop(&mut self) {
        if !self.finalized {
            let elapsed_ms = self.started_at.elapsed().as_secs_f64() * 1000.0;
            tracing::warn!(
                swarm_id = %self.swarm_id,
                tenant_id = %self.tenant,
                request_id = %self.request_id,
                rounds = self.rounds_recorded,
                elapsed_ms = elapsed_ms,
                "telemetry guard dropped without finalize — likely panic or early return"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::instruments;
    use efe_core::types::{AlignmentCone, LatentDim, PopulationMix, SwarmSpec, TemplateCode};
    use efe_core::{Request, SwarmId};

    fn sample_request() -> Request {
        let spec = SwarmSpec::builder()
            .problem_embedding(vec![0.0; 4])
            .dim_latent(LatentDim::new(4).expect("valid"))
            .population(PopulationMix::new(vec![(TemplateCode::NaSem, 4)]).expect("valid"))
            .max_rounds(3)
            .expect("valid")
            .alignment_cone(AlignmentCone::new(vec![vec![0.1; 4]], 0.15).expect("valid"))
            .build()
            .expect("valid");
        Request::for_tests(spec)
    }

    fn sample_stats() -> FusionStats {
        FusionStats::synthetic_with(2, 4, 0.01, 0.05)
    }

    #[test]
    fn guard_records_round_and_finalize() {
        instruments::register_all();
        let before_rounds = instruments::swarm_rounds_total().get();

        let req = sample_request();
        let id = SwarmId::new();
        let mut guard = TracingTelemetryGuard::new(id, &req);
        guard.record_round(1, &sample_stats());
        guard.record_round(2, &sample_stats());
        guard.finalize(42.0, &sample_stats().with_confidence(0.9));

        let after_rounds = instruments::swarm_rounds_total().get();
        assert_eq!(after_rounds, before_rounds + 2);
        assert_eq!(guard.last_verdict(), "converged");
    }

    #[test]
    fn guard_records_error_increments_errors_total() {
        instruments::register_all();
        let counter = instruments::errors_total();
        let before = counter.with_label_values(&["invalid_config", "test"]).get();

        let req = sample_request();
        let mut guard = TracingTelemetryGuard::new(SwarmId::new(), &req);
        let err = EfeError::InvalidConfig("unit test".into());
        guard.record_phase_error("genesis", &err);
        guard.finalize(0.0, &sample_stats());

        let after = counter.with_label_values(&["invalid_config", "test"]).get();
        assert_eq!(after, before + 1);
    }

    #[test]
    fn finalize_with_numerical_issue_labels_correctly() {
        instruments::register_all();
        let req = sample_request();
        let mut guard = TracingTelemetryGuard::new(SwarmId::new(), &req);
        let mut stats = sample_stats();
        stats.has_nan = true;
        guard.finalize(1.0, &stats);
        assert_eq!(guard.last_verdict(), "numerical_issue");
    }

    #[test]
    fn finalize_with_low_confidence_is_inconclusive() {
        instruments::register_all();
        let req = sample_request();
        let mut guard = TracingTelemetryGuard::new(SwarmId::new(), &req);
        let stats = sample_stats().with_confidence(0.1);
        guard.finalize(1.0, &stats);
        assert_eq!(guard.last_verdict(), "inconclusive");
    }
}
