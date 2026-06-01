//! gRPC service handlers.
//!
//! Each sub-module implements one tonic service. They all take an
//! [`ExecuteBackend`] (for the swarm service) or an
//! [`Arc<dyn AlignmentRegistry>`] (for the admin service) rather than
//! concrete types, so that integration tests can inject stubs.

pub mod admin;
// Sprint Φ-D-STREAM (D80) — RoundEmitter trait + concrete impls.
// Cf. `docs/EFE_STREAMING_RPC_COMPLETION_ARCHITECTURE.md` § 4.
/// Sprint **D12-bis** — `CatalogService` gRPC handler.
///
/// Bridges the proto layer ([`crate::proto::v1::catalog`]) to the
/// in-memory `CatalogRegistry` trait (D12). Wires D13 observability
/// (8 instruments + 16 spans + 5 typed event builders) at every
/// emit-site.
pub mod catalog;
/// **Phase-2 PR #11** — meta-swarm gRPC client adapter (plumbing only).
/// `MetaSwarmClient` is built at startup when `EFE_META_SWARM_ENABLED=true`
/// and held by the runtime state ; PR #15 will wire the dispatch.
///
/// Architecture : `docs/EFE_A1_PHASE2_PR11_ARCHITECTURE.md`.
pub mod meta_swarm;
pub(crate) mod metrics;
/// Sprint **D34** (M4 § 13) — `Mode2Service` gRPC handler.
pub mod mode2;
/// Sprint **D37** (M4 § 14) — `Mode3Service` gRPC handler.
pub mod mode3;
pub mod round_emitter;
pub mod swarm;
/// **Sprint M4 audit fixes S-A2 / S-A3** — synthetic SwarmSpec
/// construction shared by Mode-2 / Mode-3.
pub(crate) mod synthetic_spec;

pub use catalog::CatalogServiceImpl;
pub use mode2::Mode2ServiceImpl;
pub use mode3::Mode3ServiceImpl;

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Instant;

use efe_core::{Request, Response, Result, SwarmController, TelemetryCollector};

/// Minimal abstraction over the swarm controller. Present because
/// `SwarmController` is a concrete struct in `efe-core`; wrapping it in
/// a trait lets us inject a stub backend in tests without rebuilding
/// the entire strategy pipeline.
///
/// The trait intentionally does **not** require `Debug` — the real
/// [`SwarmController`] does not derive it. Components that need to
/// log the backend implement their own opaque `Debug`.
#[async_trait]
pub trait ExecuteBackend: Send + Sync {
    /// Execute a domain request end-to-end.
    async fn execute(&self, req: Request) -> Result<Response>;
}

/// Adapter around the concrete [`SwarmController`].
///
/// Use this when wiring a production server:
/// `Server::builder().backend(Arc::new(SwarmControllerBackend::new(controller)))`.
pub struct SwarmControllerBackend {
    controller: SwarmController,
}

impl SwarmControllerBackend {
    /// Wrap a pre-built controller.
    #[must_use]
    pub fn new(controller: SwarmController) -> Self {
        Self { controller }
    }
}

#[async_trait]
impl ExecuteBackend for SwarmControllerBackend {
    async fn execute(&self, req: Request) -> Result<Response> {
        self.controller.execute(req).await
    }
}

/// Instrumented wrapper that calls `TelemetryCollector::begin_swarm` +
/// `TelemetryGuard::finalize` around any underlying [`ExecuteBackend`].
///
/// Rationale (audit finding M11-#9): injecting a collector directly
/// into `SwarmControllerBuilder` is the canonical path for
/// production-scale integrations, but not every backend in practice is
/// a full controller — tests use stubs, and the MVP binary uses an
/// `UnimplementedBackend`. This wrapper guarantees that every request
/// reaching an `ExecuteBackend` produces the lifecycle telemetry
/// (`efe_swarm_rounds_total`, `efe_swarm_duration_seconds`,
/// `efe_swarm_confidence`, `efe_errors_total`) regardless of the
/// underlying backend's own telemetry discipline.
pub struct InstrumentedBackend {
    inner: Arc<dyn ExecuteBackend>,
    collector: Arc<dyn TelemetryCollector>,
}

impl InstrumentedBackend {
    /// Wrap an existing backend with a telemetry collector.
    #[must_use]
    pub fn new(inner: Arc<dyn ExecuteBackend>, collector: Arc<dyn TelemetryCollector>) -> Self {
        Self { inner, collector }
    }
}

#[async_trait]
impl ExecuteBackend for InstrumentedBackend {
    async fn execute(&self, req: Request) -> Result<Response> {
        // Pre-generate a swarm id so the telemetry guard can be bound
        // to a correlatable identifier even on the pre-execute log line.
        let swarm_id = req.spec.id;
        let started = Instant::now();
        let mut guard = self.collector.begin_swarm(swarm_id, &req);

        match self.inner.execute(req).await {
            Ok(resp) => {
                // Synthesise a minimal FusionStats from the response so
                // the guard can finalise the duration histogram + the
                // confidence histogram. The real values come from the
                // ConsensusDetector when a real controller runs, but a
                // stub backend may not produce any; we use the response
                // confidence as the source of truth.
                let total_ms = started.elapsed().as_secs_f32() * 1000.0;
                let mut stats = efe_core::fusion::FusionStats::synthetic_with(
                    resp.rounds_executed,
                    0,
                    0.0,
                    0.0,
                );
                stats.confidence = resp.confidence;
                guard.finalize(total_ms, &stats);
                Ok(resp)
            }
            Err(e) => {
                guard.record_phase_error("execute", &e);
                // We still call finalize so the duration histogram gets
                // a sample with the `numerical_issue` / `inconclusive`
                // verdict — essential for error-rate dashboards.
                let total_ms = started.elapsed().as_secs_f32() * 1000.0;
                let stats = efe_core::fusion::FusionStats::synthetic_with(0, 0, 0.0, 0.0);
                guard.finalize(total_ms, &stats);
                Err(e)
            }
        }
    }
}

/// Boxed dynamic handle.
pub type DynExecuteBackend = Arc<dyn ExecuteBackend>;

#[cfg(test)]
mod tests {
    use super::*;
    use efe_core::types::{AlignmentCone, LatentDim, PopulationMix, SwarmSpec, TemplateCode};
    use efe_core::{DecodedOutput, NullTelemetryCollector};

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

    #[derive(Debug, Clone)]
    struct StubBackend {
        fail: bool,
    }

    #[async_trait]
    impl ExecuteBackend for StubBackend {
        async fn execute(&self, req: Request) -> Result<Response> {
            if self.fail {
                Err(efe_core::EfeError::InvalidConfig("stub".into()))
            } else {
                let mut resp = Response::for_tests(req.id, DecodedOutput::Text("ok".into()));
                resp.rounds_executed = 2;
                resp.confidence = 0.9;
                Ok(resp)
            }
        }
    }

    #[tokio::test]
    async fn instrumented_backend_happy_path_delegates() {
        let inner: Arc<dyn ExecuteBackend> = Arc::new(StubBackend { fail: false });
        let collector: Arc<dyn TelemetryCollector> = Arc::new(NullTelemetryCollector);
        let wrapped = InstrumentedBackend::new(inner, collector);
        let resp = wrapped.execute(sample_request()).await.unwrap();
        assert_eq!(resp.rounds_executed, 2);
    }

    #[tokio::test]
    async fn instrumented_backend_forwards_errors() {
        let inner: Arc<dyn ExecuteBackend> = Arc::new(StubBackend { fail: true });
        let collector: Arc<dyn TelemetryCollector> = Arc::new(NullTelemetryCollector);
        let wrapped = InstrumentedBackend::new(inner, collector);
        let err = wrapped.execute(sample_request()).await.unwrap_err();
        assert!(matches!(err, efe_core::EfeError::InvalidConfig(_)));
    }
}
