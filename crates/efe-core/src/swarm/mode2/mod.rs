//! **Sprint M1 extension D34** — Mode-2 backend in `efe-core`.
//!
//! ## Mission
//!
//! Pre-D34 the Mode-2 logic lived entirely in
//! `crates/efe-server/src/services/mode2/` (1 188 LOC). Domain logic
//! was coupled to the gRPC transport — `efe-ffi` PyO3 bindings could
//! not expose Mode-2 in-process, `efe-bench` could not benchmark
//! Mode-2 without spinning up a tonic server, and tests required a
//! full server boot.
//!
//! Sprint D34 backports the algorithmic core to `efe-core`. The
//! `efe-server::services::mode2` module is reduced to a thin gRPC
//! shim (~280 LOC) that delegates to [`Mode2Controller`].
//!
//! Cf. `EFE_CORE_EXTENSION_ARCHITECTURE.md` § 10.
//!
//! ## What lives here
//!
//!   * [`Mode2Controller<B>`] — generic orchestrator over any
//!     [`crate::swarm::backend::ExecuteBackend`].
//!   * [`router::LlmRouter`] trait + [`router::MockRouter`] impl.
//!   * [`cache::Mode2Cache`] — DashMap-backed anchor cache with TTL.
//!   * [`synthesis::build_synthetic_request`] — proto ↔ domain
//!     bridge.
//!   * [`Mode2Verdict`] / [`RouterMetadata`] — output types.
//!
//! ## What stays in `efe-server`
//!
//!   * `OpenAIRouter` / `AnthropicRouter` — depend on `reqwest`, a
//!     server-only HTTP client crate.
//!   * `Mode2ServiceImpl` — the gRPC handler that wraps
//!     `Mode2Controller` for the `tonic` transport.

pub mod cache;
pub mod router;
pub mod synthesis;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::errors::Result;
use crate::swarm::backend::ExecuteBackend;
use crate::swarm::verdict::Verdict;
use crate::types::RequestId;

pub use cache::Mode2Cache;
pub use router::{
    DynamicAnchor, LlmRouter, LlmRouterError, MockRouter, RouterOutput,
};
pub use synthesis::{build_synthetic_request, verdict_to_proto_u8, SYNTH_DIM};

/// **Default cache TTL** — 5 minutes per M4 § 13.4.
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(5 * 60);

/// **Default top_k** when caller passes 0 (= unset).
pub const DEFAULT_TOP_K: u32 = 5;

// =====================================================================
// Mode2Controller
// =====================================================================

/// **Sprint D34** — Generic Mode-2 orchestrator.
///
/// Generic over `B: ExecuteBackend + 'static` so callers can
/// inject :
///   * A real `Arc<SwarmController>` for production.
///   * A test stub for unit tests.
///   * A future GPU backend for the canary rollout (D32).
///
/// ## Pipeline (one `execute()` call)
///
/// ```text
///   1. Resolve router by id (return Status::NotFound on miss).
///   2. Cache lookup keyed on xxhash64(query, top_k, router_id).
///   3. Generate anchors on miss ; insert into cache.
///   4. Mix the router's call_id into the seed (D34 invariant).
///   5. Build synthetic Request via build_synthetic_request().
///   6. backend.execute(req).await  ← REAL swarm invocation.
///   7. Project to Mode2Verdict.
/// ```
///
/// ## SLO contracts
///
///   * **C-2** Mode-2 p99 latency ≤ 1500 ms (incl. one router call).
///   * **C-4** Cache hit rate ≥ 60 % on synthetic load fixture.
///   * **C-6** Determinism : same `(query, seed)` ⇒ same verdict.
pub struct Mode2Controller<B: ExecuteBackend + 'static> {
    backend: Arc<B>,
    routers: HashMap<String, Arc<dyn LlmRouter>>,
    cache: Mode2Cache,
}

impl<B: ExecuteBackend + 'static> std::fmt::Debug for Mode2Controller<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mode2Controller")
            .field("routers", &self.routers.keys().collect::<Vec<_>>())
            .field("cache", &self.cache)
            .finish()
    }
}

impl<B: ExecuteBackend + 'static> Mode2Controller<B> {
    /// Construct with explicit cache TTL.
    ///
    /// Production callers (efe-server) inject a 5-min TTL ; tests
    /// pass smaller values to exercise expiration. The TTL is
    /// per-controller, not per-cache-entry.
    #[must_use]
    pub fn with_ttl(
        backend: Arc<B>,
        routers: HashMap<String, Arc<dyn LlmRouter>>,
        cache_ttl: Duration,
    ) -> Self {
        Self {
            backend,
            routers,
            cache: Mode2Cache::new(cache_ttl),
        }
    }

    /// Construct with the M4 § 13.4 default TTL (5 minutes).
    #[must_use]
    pub fn new(backend: Arc<B>, routers: HashMap<String, Arc<dyn LlmRouter>>) -> Self {
        Self::with_ttl(backend, routers, DEFAULT_CACHE_TTL)
    }

    /// Default routers : `mock` only (no network deps in efe-core).
    /// Production callers (efe-server) inject `openai` + `anthropic`
    /// alongside via a custom `routers` map.
    #[must_use]
    pub fn default_routers(dim: usize) -> HashMap<String, Arc<dyn LlmRouter>> {
        let mut m: HashMap<String, Arc<dyn LlmRouter>> = HashMap::new();
        m.insert(
            "mock".to_string(),
            Arc::new(MockRouter::with_dim(dim)) as Arc<dyn LlmRouter>,
        );
        m
    }

    /// Number of registered routers.
    pub fn n_routers(&self) -> usize {
        self.routers.len()
    }

    /// Cache snapshot (for telemetry).
    pub fn cache(&self) -> &Mode2Cache {
        &self.cache
    }

    /// Resolve a router by id ; map to typed error on miss.
    pub(crate) fn resolve_router(&self, id: &str) -> Result<Arc<dyn LlmRouter>> {
        self.routers.get(id).cloned().ok_or_else(|| {
            crate::errors::EfeError::InvalidConfig(format!("unknown llm_router_id: {id}"))
        })
    }

    /// **Sprint D34** — Execute a Mode-2 query end-to-end.
    ///
    /// Pipeline detailed at the struct level. Returns a
    /// [`Mode2Verdict`] that mirrors `Verdict` shape but adds
    /// router metadata for observability.
    ///
    /// ## Determinism
    ///
    /// `(query_text, top_k_anchors, router_id, seed)` ⇒ same
    /// `(verdict, confidence)`. The `request_id` is regenerated
    /// each call ; the swarm trajectory is reproducible thanks
    /// to the seed mixing rule below.
    ///
    /// ## Seed mixing
    ///
    /// We compute `mixed_seed = seed XOR fnv1a(router.llm_call_id)`
    /// so two distinct routers produce distinct swarm trajectories
    /// on the same query. This is a Mode-3 fan-out invariant
    /// (different sub-swarms with the same query but different
    /// router-output ids must explore different geometries).
    pub async fn execute(
        &self,
        query_text: &str,
        top_k_anchors: u32,
        router_id: &str,
        tenant_id: &str,
        seed: u64,
        bypass_cache: bool,
    ) -> Result<Mode2Verdict> {
        let started = Instant::now();
        let router = self.resolve_router(router_id)?;
        let effective_top_k = if top_k_anchors == 0 {
            DEFAULT_TOP_K
        } else {
            top_k_anchors
        };
        let cache_key = Mode2Cache::key(query_text, effective_top_k, router_id);

        let router_output = if bypass_cache {
            router
                .generate_anchors(query_text, effective_top_k)
                .await
                .map_err(crate::errors::EfeError::from)?
        } else if let Some(hit) = self.cache.get(cache_key) {
            hit
        } else {
            let out = router
                .generate_anchors(query_text, effective_top_k)
                .await
                .map_err(crate::errors::EfeError::from)?;
            self.cache.insert(cache_key, out.clone());
            out
        };

        // Seed-mixing rule (FNV-1a on the router call_id bytes).
        // Stable across versions ; documented in module doc.
        let mixed_seed = seed
            ^ router_output
                .llm_call_id
                .as_bytes()
                .iter()
                .fold(0u64, |acc, b| {
                    acc.wrapping_mul(0x100000001b3).wrapping_add(u64::from(*b))
                });

        let req = build_synthetic_request(query_text, mixed_seed, tenant_id)?;
        let resp = self.backend.execute(req).await?;
        let total_ms = started.elapsed().as_secs_f32() * 1000.0;

        Ok(Mode2Verdict {
            verdict: resp.verdict,
            confidence: resp.confidence,
            request_id: resp.request_id,
            total_ms,
            router: RouterMetadata {
                router_id: router_id.to_string(),
                anchors_generated: router_output.anchors.len() as u32,
                latency_ms: router_output.latency_ms,
                from_cache: router_output.from_cache,
                llm_call_id: router_output.llm_call_id,
            },
        })
    }
}

// =====================================================================
// Output types
// =====================================================================

/// **Sprint D34** — Mode-2 verdict envelope.
///
/// Mirrors [`crate::Verdict`] but adds router metadata used by
/// dashboards (`router_id`, `anchors_generated`, `from_cache`,
/// `latency_ms`).
#[derive(Debug, Clone)]
pub struct Mode2Verdict {
    /// The swarm's terminal verdict (rich enum, not the i32 proto).
    pub verdict: Verdict,
    /// Confidence in [0.0, 1.0] from the consensus detector.
    pub confidence: f32,
    /// Server-assigned correlation id.
    pub request_id: RequestId,
    /// Total wall time including the LLM call (ms).
    pub total_ms: f32,
    /// Router observability payload.
    pub router: RouterMetadata,
}

/// **Sprint D34** — Router observability payload.
#[derive(Debug, Clone)]
pub struct RouterMetadata {
    /// Stable router id (`"mock"`, `"openai"`, …).
    pub router_id: String,
    /// Number of anchors actually returned (may differ from
    /// requested top_k if the LLM emitted fewer).
    pub anchors_generated: u32,
    /// LLM call latency in ms (excludes swarm time).
    pub latency_ms: u32,
    /// True when the result came from the in-process cache.
    pub from_cache: bool,
    /// Provider correlation id (or synthesised by mock router).
    pub llm_call_id: String,
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::backend::DynExecuteBackend;
    use crate::types::{
        AlignmentCone, LatentDim, PopulationMix, SwarmSpec, TemplateCode,
    };
    use crate::DecodedOutput;
    use async_trait::async_trait;

    fn sample_request() -> crate::Request {
        let spec = SwarmSpec::builder()
            .problem_embedding(vec![0.0; 4])
            .dim_latent(LatentDim::new(4).expect("valid"))
            .population(PopulationMix::new(vec![(TemplateCode::NaSem, 4)]).expect("valid"))
            .max_rounds(3)
            .expect("valid")
            .alignment_cone(AlignmentCone::new(vec![vec![0.1; 4]], 0.15).expect("valid"))
            .build()
            .expect("valid");
        crate::Request::synthetic(spec, "t".into(), RequestId::new())
    }

    /// Stub backend that returns a deterministic Converged response.
    /// Used by Mode-2 unit tests so we don't need a full swarm controller.
    struct StubBackend {
        confidence: f32,
    }

    #[async_trait]
    impl ExecuteBackend for StubBackend {
        async fn execute(&self, req: crate::Request) -> Result<crate::Response> {
            let mut resp = crate::Response::for_tests(req.id, DecodedOutput::Text("stub".into()));
            resp.verdict = Verdict::Converged {
                confidence: self.confidence,
            };
            resp.confidence = self.confidence;
            Ok(resp)
        }
    }

    fn build_test_controller(confidence: f32) -> Mode2Controller<StubBackend> {
        let routers = Mode2Controller::<StubBackend>::default_routers(8);
        Mode2Controller::new(Arc::new(StubBackend { confidence }), routers)
    }

    #[tokio::test]
    async fn execute_calls_backend_with_synthesised_spec() {
        let ctrl = build_test_controller(0.85);
        let v = ctrl
            .execute("hello", 5, "mock", "tenant-x", 42, false)
            .await
            .unwrap();
        assert_eq!(v.confidence, 0.85);
        assert_eq!(v.router.router_id, "mock");
        assert_eq!(v.router.anchors_generated, 5);
        // First call ⇒ from_cache should be false.
        assert!(!v.router.from_cache);
    }

    #[tokio::test]
    async fn second_execute_hits_cache() {
        let ctrl = build_test_controller(0.85);
        let _ = ctrl
            .execute("same", 5, "mock", "t", 42, false)
            .await
            .unwrap();
        let second = ctrl
            .execute("same", 5, "mock", "t", 42, false)
            .await
            .unwrap();
        assert!(second.router.from_cache, "second call should hit cache");
    }

    #[tokio::test]
    async fn bypass_cache_skips_cache() {
        let ctrl = build_test_controller(0.85);
        let _ = ctrl
            .execute("same", 5, "mock", "t", 42, false)
            .await
            .unwrap();
        let second = ctrl
            .execute("same", 5, "mock", "t", 42, /* bypass = */ true)
            .await
            .unwrap();
        assert!(!second.router.from_cache);
    }

    #[tokio::test]
    async fn unknown_router_returns_invalid_config() {
        let ctrl = build_test_controller(0.85);
        let err = ctrl
            .execute("q", 5, "nonexistent", "t", 42, false)
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("nonexistent"));
    }

    #[tokio::test]
    async fn determinism_same_inputs_same_request_id_propagates() {
        // Same query+seed ⇒ same backend Request (mod request_id) ⇒
        // same Verdict via stub backend.
        let ctrl = build_test_controller(0.85);
        let v1 = ctrl.execute("q", 3, "mock", "t", 1, false).await.unwrap();
        let v2 = ctrl.execute("q", 3, "mock", "t", 1, true).await.unwrap();
        // Both produce same confidence (stub) — sanity check.
        assert_eq!(v1.confidence, v2.confidence);
        assert_eq!(v1.router.llm_call_id, v2.router.llm_call_id);
    }

    #[tokio::test]
    async fn top_k_zero_defaults_to_5() {
        let ctrl = build_test_controller(0.85);
        let v = ctrl.execute("q", 0, "mock", "t", 0, false).await.unwrap();
        assert_eq!(v.router.anchors_generated, DEFAULT_TOP_K);
    }

    #[tokio::test]
    async fn dyn_backend_arc_upcast_works() {
        // Compile-time check : `Arc<StubBackend>` upcasts to
        // `Arc<dyn ExecuteBackend>` (i.e. `DynExecuteBackend`).
        // This proves the trait object surface is correct without
        // attempting to construct `Mode2Controller<dyn ...>` (which
        // would violate the implicit `Sized` bound on the generic
        // parameter — trait objects must be wrapped in a concrete
        // adapter type by callers that need dynamic dispatch).
        let _ = sample_request();
        let backend: DynExecuteBackend = Arc::new(StubBackend { confidence: 0.5 });
        // Smoke check the trait method dispatch works.
        let req = sample_request();
        let resp = backend.execute(req).await.unwrap();
        assert_eq!(resp.confidence, 0.5);
    }

    #[test]
    fn n_routers_reflects_map_size() {
        let ctrl = build_test_controller(0.5);
        assert_eq!(ctrl.n_routers(), 1); // mock only by default
    }

    #[test]
    fn default_routers_contains_mock() {
        let m = Mode2Controller::<StubBackend>::default_routers(8);
        assert!(m.contains_key("mock"));
    }

    #[test]
    fn default_cache_ttl_is_5_minutes() {
        assert_eq!(DEFAULT_CACHE_TTL, Duration::from_secs(300));
    }
}
