//! **Sprint M1 extension D37 + D38** — Mode-3 backend in `efe-core`.
//!
//! ## Mission
//!
//! Pre-D37 the Mode-3 logic lived in `efe-server` (1 163 LOC). Same
//! architectural defect as Mode-2 — domain logic in transport.
//! D37 backports `Mode3Controller` ; D38 adds the parallel runner.
//!
//! Cf. `EFE_CORE_EXTENSION_ARCHITECTURE.md` § 11 + § 12.
//!
//! ## Pipeline
//!
//! ```text
//!   1. Clamp k to [1, K_MAX] (SLO C-5 ; K_MAX = 5).
//!   2. Resolve aggregator by id.
//!   3. Spawn k sub-swarms via parallel::run_k_swarms<F>.
//!   4. Each sub-swarm dispatches on `mode_per_call` :
//!        * "mode1" → mode1_backend.execute(synth_request)
//!        * "mode2" → mode2_controller.execute(...)
//!   5. Collect SubVerdictData[].
//!   6. Aggregator::aggregate(...).
//!   7. Build Mode3Verdict.
//! ```

pub mod aggregator;
pub mod parallel;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::errors::{EfeError, Result};
use crate::swarm::backend::ExecuteBackend;
use crate::swarm::mode2::Mode2Controller;
use crate::swarm::verdict::Verdict;
use crate::types::RequestId;

pub use aggregator::{
    AggregationError, AggregationMetadata, AggregationOutput, MeanAggregator, MedianAggregator,
    Mode3Aggregator, SubVerdictData, WeightedAggregator,
};

/// **SLO C-5** hard cap on the parallelism factor.
///
/// Inputs above 5 are clamped server-side (no error — the caller
/// receives `k_clamped = true` in the response). Above k=5 the GPU
/// memory budget exceeds the T4 baseline (16 GiB) per single-tenant
/// concurrent calls.
pub const K_MAX: u32 = 5;

/// Default `k` when the caller passes 0.
pub const DEFAULT_K: u32 = 3;

/// Default aggregator id.
pub const DEFAULT_AGGREGATOR: &str = "mean";

/// Default `mode_per_call`.
pub const DEFAULT_MODE_PER_CALL: &str = "mode1";

/// Effective k after clamping to `[1, K_MAX]`. Returns the
/// `(k_effective, k_clamped)` pair.
#[must_use]
pub fn clamp_k(k: u32) -> (u32, bool) {
    if k == 0 {
        (DEFAULT_K, false)
    } else if k > K_MAX {
        (K_MAX, true)
    } else {
        (k, false)
    }
}

// =====================================================================
// Mode3Controller
// =====================================================================

/// **Sprint D37** — Generic Mode-3 orchestrator.
///
/// Generic over `B: ExecuteBackend + 'static` so callers can inject
/// real backend (production) or stub backend (tests).
///
/// ## Sub-modes
///
///   * `mode_per_call = "mode1"` → calls `mode1_backend.execute()`
///     directly with a synthetic spec built from `(query, seed_i)`.
///   * `mode_per_call = "mode2"` → calls `mode2_controller.execute()`
///     which internally consults the LLM router cache + spawns a
///     swarm.
///
/// `mode2_controller` is `Option` — callers that don't need Mode-2
/// fan-out can pass `None` ; the mode2 path then returns
/// `EfeError::InvalidConfig`.
pub struct Mode3Controller<B: ExecuteBackend + 'static> {
    mode1_backend: Arc<B>,
    mode2_controller: Option<Arc<Mode2Controller<B>>>,
    aggregators: HashMap<String, Arc<dyn Mode3Aggregator>>,
}

impl<B: ExecuteBackend + 'static> std::fmt::Debug for Mode3Controller<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mode3Controller")
            .field("aggregators", &self.aggregators.keys().collect::<Vec<_>>())
            .field("mode2_enabled", &self.mode2_controller.is_some())
            .finish()
    }
}

impl<B: ExecuteBackend + 'static> Mode3Controller<B> {
    /// Construct.
    #[must_use]
    pub fn new(
        mode1_backend: Arc<B>,
        mode2_controller: Option<Arc<Mode2Controller<B>>>,
        aggregators: HashMap<String, Arc<dyn Mode3Aggregator>>,
    ) -> Self {
        Self {
            mode1_backend,
            mode2_controller,
            aggregators,
        }
    }

    /// Default registry of 3 aggregators (mean / weighted / median).
    #[must_use]
    pub fn default_aggregators() -> HashMap<String, Arc<dyn Mode3Aggregator>> {
        let mut m: HashMap<String, Arc<dyn Mode3Aggregator>> = HashMap::new();
        m.insert("mean".to_string(), Arc::new(MeanAggregator) as _);
        m.insert("weighted".to_string(), Arc::new(WeightedAggregator) as _);
        m.insert("median".to_string(), Arc::new(MedianAggregator) as _);
        m
    }

    /// Resolve an aggregator by id ; map to typed error on miss.
    pub fn resolve_aggregator(&self, id: &str) -> Result<Arc<dyn Mode3Aggregator>> {
        self.aggregators.get(id).cloned().ok_or_else(|| {
            EfeError::InvalidConfig(format!("unknown aggregator: {id}"))
        })
    }

    /// **Sprint D37** — Sequential k-swarm execution.
    ///
    /// Reference implementation that runs the k sub-swarms one after
    /// another. Useful for benchmarking the parallel speedup, and
    /// for callers that explicitly want sequential semantics
    /// (e.g. when the sub-swarms must observe each other's state).
    pub async fn execute_sequential(
        &self,
        query_text: &str,
        k: u32,
        mode_per_call: &str,
        aggregator_id: &str,
        tenant_id: &str,
        seed: u64,
        top_k_anchors: u32,
        router_id: &str,
    ) -> Result<Mode3Verdict> {
        let started = Instant::now();
        let (k_eff, k_clamped) = clamp_k(k);
        let aggregator = self.resolve_aggregator(aggregator_id)?;

        let mut sub_verdicts = Vec::with_capacity(k_eff as usize);
        for i in 0..k_eff {
            let seed_i = seed.wrapping_add(i as u64);
            let sv = self
                .execute_one_sub_swarm(
                    i,
                    seed_i,
                    mode_per_call,
                    query_text,
                    tenant_id,
                    top_k_anchors,
                    router_id,
                )
                .await?;
            sub_verdicts.push(sv);
        }

        finalize_mode3_verdict(
            sub_verdicts,
            aggregator.as_ref(),
            started,
            k_clamped,
            k_eff,
        )
    }

    /// **Sprint D38** — Parallel k-swarm execution via
    /// `tokio::task::JoinSet` (preferred path for production).
    ///
    /// Uses [`parallel::run_k_swarms`] internally. Each sub-swarm
    /// gets its own seed (`seed + i`) and runs concurrently on the
    /// ambient tokio runtime.
    pub async fn execute_parallel(
        self: Arc<Self>,
        query_text: &str,
        k: u32,
        mode_per_call: &str,
        aggregator_id: &str,
        tenant_id: &str,
        seed: u64,
        top_k_anchors: u32,
        router_id: &str,
    ) -> Result<Mode3Verdict> {
        let started = Instant::now();
        let (k_eff, k_clamped) = clamp_k(k);
        let aggregator = self.resolve_aggregator(aggregator_id)?;

        let query = query_text.to_string();
        let tenant = tenant_id.to_string();
        let mode = mode_per_call.to_string();
        let router_id_owned = router_id.to_string();

        let sub_verdicts = parallel::run_k_swarms(k_eff, |i| {
            let me = self.clone();
            let query = query.clone();
            let tenant = tenant.clone();
            let mode = mode.clone();
            let router_id_owned = router_id_owned.clone();
            async move {
                let seed_i = seed.wrapping_add(i as u64);
                me.execute_one_sub_swarm(
                    i,
                    seed_i,
                    &mode,
                    &query,
                    &tenant,
                    top_k_anchors,
                    &router_id_owned,
                )
                .await
            }
        })
        .await?;

        finalize_mode3_verdict(
            sub_verdicts,
            aggregator.as_ref(),
            started,
            k_clamped,
            k_eff,
        )
    }

    /// Execute one sub-swarm. Dispatches on `mode_per_call`.
    pub(crate) async fn execute_one_sub_swarm(
        &self,
        swarm_idx: u32,
        seed: u64,
        mode: &str,
        query_text: &str,
        tenant_id: &str,
        top_k_anchors: u32,
        router_id: &str,
    ) -> Result<SubVerdictData> {
        let started = Instant::now();
        let (verdict, confidence) = match mode {
            "mode1" => {
                let req = crate::swarm::mode2::synthesis::build_synthetic_request(
                    query_text, seed, tenant_id,
                )?;
                let resp = self.mode1_backend.execute(req).await?;
                (resp.verdict, resp.confidence)
            }
            "mode2" => {
                let m2 = self.mode2_controller.as_ref().ok_or_else(|| {
                    EfeError::InvalidConfig(
                        "mode_per_call=mode2 but Mode-2 controller not wired".into(),
                    )
                })?;
                let effective_top_k = if top_k_anchors == 0 { 5 } else { top_k_anchors };
                let effective_router = if router_id.is_empty() { "mock" } else { router_id };
                let v = m2
                    .execute(
                        query_text,
                        effective_top_k,
                        effective_router,
                        tenant_id,
                        seed,
                        false,
                    )
                    .await?;
                (v.verdict, v.confidence)
            }
            other => {
                return Err(EfeError::InvalidConfig(format!(
                    "unknown mode_per_call: {other}"
                )));
            }
        };
        let latency_ms = started.elapsed().as_millis() as u32;
        Ok(SubVerdictData {
            swarm_idx,
            verdict,
            confidence,
            latency_ms,
            mode_used: mode.to_string(),
            seed_used: seed,
        })
    }
}

/// Helper that wraps the post-collection aggregation + Mode3Verdict
/// construction. Shared by sequential + parallel paths.
fn finalize_mode3_verdict(
    sub_verdicts: Vec<SubVerdictData>,
    aggregator: &dyn Mode3Aggregator,
    started: Instant,
    k_clamped: bool,
    k_effective: u32,
) -> Result<Mode3Verdict> {
    let agg_out = aggregator
        .aggregate(&sub_verdicts)
        .map_err(EfeError::from)?;
    Ok(Mode3Verdict {
        final_verdict: agg_out.final_verdict,
        sub_verdicts,
        aggregation: AggregationMetadata::from(&agg_out),
        request_id: RequestId::new(),
        total_ms: started.elapsed().as_secs_f32() * 1000.0,
        k_clamped,
        k_effective,
    })
}

// =====================================================================
// Output
// =====================================================================

/// **Sprint D37** — Mode-3 verdict envelope.
#[derive(Debug, Clone)]
pub struct Mode3Verdict {
    /// Aggregated final verdict (the answer Mode-3 returns).
    pub final_verdict: Verdict,
    /// Per-sub-swarm verdicts in arrival order.
    pub sub_verdicts: Vec<SubVerdictData>,
    /// Aggregation metadata (strategy, confidence sum, dispersion).
    pub aggregation: AggregationMetadata,
    /// Server-assigned correlation id.
    pub request_id: RequestId,
    /// Total wall time (ms).
    pub total_ms: f32,
    /// True when input k was > K_MAX ; the actual k used = K_MAX.
    pub k_clamped: bool,
    /// Effective k after clamping.
    pub k_effective: u32,
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DecodedOutput;
    use async_trait::async_trait;

    /// Stub backend used by Mode-3 unit tests. Returns a deterministic
    /// confidence scaled by the swarm idx (decoded from request_id).
    struct StubBackend;
    #[async_trait]
    impl ExecuteBackend for StubBackend {
        async fn execute(&self, req: crate::Request) -> Result<crate::Response> {
            let mut resp = crate::Response::for_tests(req.id, DecodedOutput::Text("stub".into()));
            resp.verdict = Verdict::Converged { confidence: 0.7 };
            resp.confidence = 0.7;
            Ok(resp)
        }
    }

    fn build_test_controller() -> Arc<Mode3Controller<StubBackend>> {
        Arc::new(Mode3Controller::new(
            Arc::new(StubBackend),
            None,
            Mode3Controller::<StubBackend>::default_aggregators(),
        ))
    }

    // ─────────────────────────────────────────────────────────────
    // clamp_k
    // ─────────────────────────────────────────────────────────────

    #[test]
    fn clamp_k_zero_defaults_to_3() {
        assert_eq!(clamp_k(0), (3, false));
    }

    #[test]
    fn clamp_k_above_max_clamps_and_flags() {
        assert_eq!(clamp_k(10), (K_MAX, true));
    }

    #[test]
    fn clamp_k_within_range_passes_through() {
        for k in 1..=K_MAX {
            assert_eq!(clamp_k(k), (k, false));
        }
    }

    #[test]
    fn k_max_is_5() {
        assert_eq!(K_MAX, 5);
    }

    // ─────────────────────────────────────────────────────────────
    // default_aggregators
    // ─────────────────────────────────────────────────────────────

    #[test]
    fn default_aggregators_has_three_entries() {
        let m = Mode3Controller::<StubBackend>::default_aggregators();
        assert!(m.contains_key("mean"));
        assert!(m.contains_key("weighted"));
        assert!(m.contains_key("median"));
        assert_eq!(m.len(), 3);
    }

    // ─────────────────────────────────────────────────────────────
    // execute_sequential
    // ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn execute_sequential_happy_path() {
        let ctrl = build_test_controller();
        let v = ctrl
            .execute_sequential("hello", 3, "mode1", "mean", "tenant", 42, 5, "mock")
            .await
            .unwrap();
        assert_eq!(v.k_effective, 3);
        assert!(!v.k_clamped);
        assert_eq!(v.sub_verdicts.len(), 3);
        // All sub-verdicts should be Converged with conf 0.7 from stub.
        for sv in &v.sub_verdicts {
            assert!((sv.confidence - 0.7).abs() < 1e-6);
            assert_eq!(sv.mode_used, "mode1");
        }
    }

    #[tokio::test]
    async fn execute_sequential_clamps_above_max() {
        let ctrl = build_test_controller();
        let v = ctrl
            .execute_sequential("q", 100, "mode1", "mean", "t", 0, 5, "mock")
            .await
            .unwrap();
        assert_eq!(v.k_effective, K_MAX);
        assert!(v.k_clamped);
    }

    #[tokio::test]
    async fn execute_sequential_unknown_aggregator_fails() {
        let ctrl = build_test_controller();
        let err = ctrl
            .execute_sequential("q", 3, "mode1", "nonexistent", "t", 0, 5, "mock")
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("nonexistent"));
    }

    #[tokio::test]
    async fn execute_sequential_unknown_mode_fails() {
        let ctrl = build_test_controller();
        let err = ctrl
            .execute_sequential("q", 3, "mode4", "mean", "t", 0, 5, "mock")
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("mode4"));
    }

    #[tokio::test]
    async fn execute_sequential_mode2_without_controller_fails() {
        let ctrl = build_test_controller();
        let err = ctrl
            .execute_sequential("q", 3, "mode2", "mean", "t", 0, 5, "mock")
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("mode2"));
        assert!(format!("{err}").contains("not wired"));
    }

    // ─────────────────────────────────────────────────────────────
    // execute_parallel
    // ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn execute_parallel_happy_path() {
        let ctrl = build_test_controller();
        let v = ctrl
            .execute_parallel("hello", 3, "mode1", "mean", "tenant", 42, 5, "mock")
            .await
            .unwrap();
        assert_eq!(v.k_effective, 3);
        assert!(!v.k_clamped);
        assert_eq!(v.sub_verdicts.len(), 3);
    }

    #[tokio::test]
    async fn execute_parallel_clamps_above_max() {
        let ctrl = build_test_controller();
        let v = ctrl
            .execute_parallel("q", 100, "mode1", "mean", "t", 0, 5, "mock")
            .await
            .unwrap();
        assert_eq!(v.k_effective, K_MAX);
        assert!(v.k_clamped);
    }

    #[tokio::test]
    async fn execute_one_sub_swarm_mode1_calls_backend() {
        let ctrl = build_test_controller();
        let sv = ctrl
            .execute_one_sub_swarm(0, 42, "mode1", "test", "tenant", 5, "mock")
            .await
            .unwrap();
        assert_eq!(sv.swarm_idx, 0);
        assert_eq!(sv.seed_used, 42);
        assert_eq!(sv.mode_used, "mode1");
        assert!((sv.confidence - 0.7).abs() < 1e-6);
    }

    // ─────────────────────────────────────────────────────────────
    // Mode3Verdict
    // ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn mode3_verdict_carries_all_metadata() {
        let ctrl = build_test_controller();
        let v = ctrl
            .execute_sequential("q", 3, "mode1", "mean", "tenant", 0, 5, "mock")
            .await
            .unwrap();
        assert_eq!(v.aggregation.strategy, "mean");
        assert_eq!(v.aggregation.effective_n, 1); // all converged
        assert!(v.total_ms > 0.0);
    }
}
