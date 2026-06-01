//! Observability hooks — forward-compat with Phase 3 OpenTelemetry.
//!
//! Phase 1 ships a `NoopTracer` that ignores all events.  The
//! `MetaSwarm` calls into the tracer at well-defined points so a future
//! impl can correlate spans, expose Prometheus counters, etc.

use crate::Tensor;

/// Trait that the meta-swarm calls during iteration.  Implementations
/// must be cheap : every method is on a hot path.
pub trait MetaSwarmTracer: Send + Sync {
    /// Called once before the first step.
    fn iteration_started(&self, meta_swarm_id: &str, expected_max_iters: usize);

    /// Called after each step.
    fn step_completed(&self, step: usize, residual: f64);

    /// Called when a safety violation is detected (Phase-1 lenient mode).
    fn safety_violation(&self, level: &str, distance: f64);

    /// Called when the iteration loop terminates, regardless of reason.
    fn iteration_ended(
        &self,
        meta_swarm_id: &str,
        iterations: usize,
        residual: f64,
        elapsed_micros: u128,
    );

    /// Called when a final consensus has been produced (post-projection).
    fn consensus_produced(&self, dim: usize);

    /// Forward-compat : called when a tensor is sampled for safety / trace.
    fn tensor_sampled(&self, tag: &str, _tensor: &Tensor) {
        let _ = tag; // default impl is a no-op
    }
}

/// Default tracer — does nothing.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopTracer;

impl MetaSwarmTracer for NoopTracer {
    fn iteration_started(&self, _meta_swarm_id: &str, _expected_max_iters: usize) {}
    fn step_completed(&self, _step: usize, _residual: f64) {}
    fn safety_violation(&self, _level: &str, _distance: f64) {}
    fn iteration_ended(
        &self,
        _meta_swarm_id: &str,
        _iterations: usize,
        _residual: f64,
        _elapsed_micros: u128,
    ) {}
    fn consensus_produced(&self, _dim: usize) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_tracer_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<NoopTracer>();
    }

    #[test]
    fn noop_tracer_calls_are_inert() {
        let t = NoopTracer;
        t.iteration_started("test", 100);
        t.step_completed(0, 1.0);
        t.safety_violation("meta", 0.1);
        t.iteration_ended("test", 10, 0.001, 1234);
        t.consensus_produced(64);
    }
}
