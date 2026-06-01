//! **Sprint M1 extension D34** — Backend trait for swarm execution.
//!
//! ## Mission
//!
//! Pre-D34 the only way to execute a swarm was through the concrete
//! [`crate::swarm::controller::SwarmController`] struct. This forced
//! every layer that wanted to "run a swarm" (Mode-2, Mode-3, tests,
//! benches) to depend on the concrete controller type — losing the
//! ability to inject test stubs or alternative production backends.
//!
//! Sprint D34 introduces the [`ExecuteBackend`] trait — the abstract
//! contract every swarm-running entity satisfies. `SwarmController`
//! implements it natively ; `efe-server` exposes a re-export so
//! existing handlers don't need to change their imports.
//!
//! ## Why a separate file
//!
//! The trait is small but is a **cross-module dependency** : Mode-2,
//! Mode-3, fusion/hybrid, and test stubs all reference it. Hosting
//! it in its own file avoids cyclic-import risk and gives grep an
//! anchor (`grep "trait ExecuteBackend" crates/efe-core/`).

use async_trait::async_trait;

use crate::errors::Result;
use crate::swarm::request::{Request, Response};

/// **Sprint D34** — Abstract swarm execution backend.
///
/// Implementations :
///   * [`crate::swarm::controller::SwarmController`] — the production
///     CPU MVP backend (Mode-1).
///   * `efe_kernels::CudaBackend` (post-D28) — GPU backend.
///   * `efe_server::services::SwarmControllerBackend` — adapter
///     wrapping the controller (legacy ; will be removed once
///     D34 ships).
///   * Test stubs (`StubBackend`, `EchoBackend`, …).
///
/// ## Trait bounds
///
/// `Send + Sync` so a `Arc<dyn ExecuteBackend>` can move across
/// tokio tasks. The `async fn` returns a `Result<Response>` so
/// errors propagate without `unwrap_or_else` chains at the caller.
#[async_trait]
pub trait ExecuteBackend: Send + Sync {
    /// Execute one inference request end-to-end.
    ///
    /// **Contract** :
    ///   * The implementation MAY mutate internal state (memory pools,
    ///     telemetry counters) but MUST NOT mutate `req` (it's owned
    ///     and dropped after the call).
    ///   * Multiple concurrent calls are allowed ; the implementation
    ///     is responsible for its own thread safety.
    ///   * Errors are typed via [`crate::errors::EfeError`] ; the
    ///     caller is responsible for mapping them to transport-level
    ///     status codes (gRPC / HTTP / Python exceptions).
    async fn execute(&self, req: Request) -> Result<Response>;
}

/// Convenience type alias used by callers that want a shared,
/// dynamically-dispatched handle.
pub type DynExecuteBackend = std::sync::Arc<dyn ExecuteBackend>;

// =====================================================================
// SwarmController implements ExecuteBackend
// =====================================================================
//
// **Audit fix C-A2** — without this impl, `Mode2Controller<SwarmController>`
// could not be constructed in production (the generic bound `B: ExecuteBackend`
// failed to resolve). Pre-fix the only impls of `ExecuteBackend` were the
// test stubs in `mode2/mod.rs`, `mode3/mod.rs`, and this file's own
// `tests` module — the production swarm controller was disconnected from
// the trait surface.
//
// The impl is a thin forwarding shim : it calls the inherent
// `SwarmController::execute(self, req)` method via the explicit
// type-qualified path to avoid the recursive trait-method lookup
// that `self.execute(req)` would trigger inside a trait impl.
//
// **Wiring contract** : `efe_server` and `efe_ffi` can now hold an
// `Arc<SwarmController>` and pass it directly to
// `Mode2Controller::new(...)` / `Mode3Controller::new(...)`. The
// adapter struct `efe_server::services::SwarmControllerBackend` becomes
// redundant ; its removal is `DT-C-AUDIT-CLEANUP-1`.

#[async_trait]
impl ExecuteBackend for crate::swarm::controller::SwarmController {
    async fn execute(&self, req: Request) -> Result<Response> {
        // Explicit type-qualified call to the inherent method to avoid
        // recursing into the trait impl. Both have signature
        // `async fn execute(&self, Request) -> Result<Response>` so a
        // bare `self.execute(req)` would loop forever via trait-method
        // dispatch.
        crate::swarm::controller::SwarmController::execute(self, req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        AlignmentCone, LatentDim, PopulationMix, SwarmSpec, TemplateCode,
    };
    use crate::{DecodedOutput, Request, Response};

    /// Minimal stub backend used by Mode-2 / Mode-3 tests so they
    /// don't have to instantiate a full `SwarmController`.
    struct StubBackend {
        confidence: f32,
    }

    #[async_trait]
    impl ExecuteBackend for StubBackend {
        async fn execute(&self, req: Request) -> Result<Response> {
            let mut resp = Response::for_tests(req.id, DecodedOutput::Text("stub".into()));
            resp.confidence = self.confidence;
            Ok(resp)
        }
    }

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
        Request::synthetic(spec, "test".into(), crate::types::RequestId::new())
    }

    #[tokio::test]
    async fn stub_backend_implements_trait() {
        let backend: DynExecuteBackend = std::sync::Arc::new(StubBackend { confidence: 0.7 });
        let resp = backend.execute(sample_request()).await.unwrap();
        assert!((resp.confidence - 0.7).abs() < 1e-6);
    }

    #[tokio::test]
    async fn dyn_execute_backend_is_send_sync() {
        // Compile-time assertion that DynExecuteBackend satisfies
        // Send + Sync. Required so callers can pass it across
        // tokio tasks without warnings.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<DynExecuteBackend>();
    }
}
