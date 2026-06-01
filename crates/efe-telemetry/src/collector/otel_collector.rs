//! `TracingTelemetryCollector` — factory that produces a per-swarm
//! [`crate::collector::TracingTelemetryGuard`].
//!
//! The name `Tracing*` (rather than `Otel*`) reflects the MVP reality:
//! spans are emitted via the [`tracing`] framework. Any
//! `tracing-opentelemetry` layer installed at startup will forward them
//! to an OTLP endpoint without code changes. Adopting `tracing-opentelemetry`
//! is documented in `M11_ARCHITECTURE.md` §6 and planned for Sprint 10.

use efe_core::{Request, SwarmId, TelemetryCollector, TelemetryGuard};

use crate::collector::otel_guard::TracingTelemetryGuard;

/// Stateless factory. Cheap to clone and share via `Arc`.
#[derive(Clone, Debug, Default)]
pub struct TracingTelemetryCollector;

impl TracingTelemetryCollector {
    /// Construct a new collector.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl TelemetryCollector for TracingTelemetryCollector {
    fn begin_swarm(&self, id: SwarmId, req: &Request) -> Box<dyn TelemetryGuard> {
        Box::new(TracingTelemetryGuard::new(id, req))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_core::types::{AlignmentCone, LatentDim, PopulationMix, SwarmSpec, TemplateCode};

    fn sample_request() -> Request {
        let spec = SwarmSpec::builder()
            .problem_embedding(vec![0.0; 4])
            .dim_latent(LatentDim::new(4).expect("valid"))
            .population(PopulationMix::new(vec![(TemplateCode::NaSem, 4)]).expect("valid"))
            .max_rounds(3)
            .expect("valid")
            .alignment_cone(AlignmentCone::new(vec![vec![0.1; 4]], 0.15).expect("valid"))
            .build()
            .expect("valid spec");
        Request::for_tests(spec)
    }

    #[test]
    fn begin_swarm_returns_guard() {
        let collector = TracingTelemetryCollector::new();
        let req = sample_request();
        let _guard = collector.begin_swarm(SwarmId::new(), &req);
        // Guard is dropped at end of scope — must not panic.
    }

    #[test]
    fn collector_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<TracingTelemetryCollector>();
    }
}
