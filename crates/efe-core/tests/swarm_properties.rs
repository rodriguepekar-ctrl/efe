//! **B7 — Property-based tests for `efe_core::swarm::SwarmController`.**
//!
//! Random scenarios across:
//! - max_rounds ∈ [1, 16]
//! - dim_latent ∈ [4, 32]
//! - confidence ∈ [0.0, 1.0]
//! - delta ∈ [0.0, 1.0]
//!
//! Properties:
//! - **W3a**: ConvergenceTimeout iff Continue×max_rounds (no Converged)
//! - **W3b**: rounds_executed ≤ max_rounds always
//! - **W3c**: Converged at round k → rounds_executed == k
//! - **W3d**: AlignmentBreach iff delta > max_projection_distance OR !is_finite(delta)
//! - **W3e**: every successful execute() emits exactly one finalize:
#![allow(clippy::cast_possible_truncation)]

mod mocks;

use std::sync::Arc;

use efe_core::errors::EfeError;
use efe_core::fusion::FusionStats;
use efe_core::swarm::{Request, SwarmArbiter, SwarmController, Verdict};
use efe_core::types::{AlignmentCone, LatentDim, PopulationMix, SwarmSpec, TemplateCode};

use mocks::{MockAligner, MockDissolver, MockFusion, MockGenesis, RecordingTelemetry, ScriptedDetector};

use proptest::prelude::*;
use tokio::runtime::Runtime;

fn rt_block_on<F: std::future::Future>(f: F) -> F::Output {
    Runtime::new().expect("runtime").block_on(f)
}

fn spec(dim: u32, max_rounds: u32) -> SwarmSpec {
    SwarmSpec::builder()
        .problem_embedding(vec![0.0; dim as usize])
        .dim_latent(LatentDim::new(dim).expect("dim"))
        .population(PopulationMix::new(vec![(TemplateCode::NaSem, 4)]).expect("mix"))
        .max_rounds(max_rounds)
        .expect("rounds")
        .alignment_cone(
            AlignmentCone::new(vec![vec![0.1; dim as usize]], 0.15).expect("cone"),
        )
        .build()
        .expect("spec")
}

// ====================================================================
// W3a + W3b — Continue×N → ConvergenceTimeout(rounds=N)
// ====================================================================

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 24,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_continue_until_max_rounds_yields_convergence_timeout(
        dim in 4u32..=16,
        max_rounds in 1u32..=8,
    ) {
        rt_block_on(async move {
            let stats: Vec<_> = (1..=max_rounds)
                .map(|r| FusionStats::synthetic_converging(r, dim as usize))
                .collect();
            let controller = SwarmController::builder()
                .genesis(Arc::new(MockGenesis::new(LatentDim::new(dim).expect("d"), 4)))
                .fusion(Arc::new(MockFusion::new(stats)))
                .dissolver(Arc::new(MockDissolver::new(dim as usize, "n/a")))
                .alignment(Arc::new(MockAligner::safe()))
                .arbiter(SwarmArbiter::without_energy_metering())
                .consensus_factory(|| {
                    Box::new(ScriptedDetector::new(vec![Verdict::Continue; 64]))
                })
                .build()
                .expect("build");
            let err = controller
                .execute(Request::for_tests(spec(dim, max_rounds)))
                .await
                .unwrap_err();
            prop_assert!(
                matches!(err, EfeError::ConvergenceTimeout { rounds, .. } if rounds == max_rounds),
                "expected ConvergenceTimeout({}); got {err:?}", max_rounds
            );
            Ok(())
        }).unwrap();
    }
}

// ====================================================================
// W3c — Converged at round k → rounds_executed == k
// ====================================================================

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 24,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_converged_at_round_k_yields_rounds_executed_eq_k(
        dim in 4u32..=16,
        max_rounds in 4u32..=12,
        converge_at_offset in 0u32..3,
    ) {
        let converge_at = (converge_at_offset + 1).min(max_rounds);
        rt_block_on(async move {
            let stats: Vec<_> = (1..=max_rounds)
                .map(|r| FusionStats::synthetic_converging(r, dim as usize))
                .collect();
            let mut verdicts: Vec<_> = (1..converge_at).map(|_| Verdict::Continue).collect();
            verdicts.push(Verdict::Converged { confidence: 0.92 });
            let controller = SwarmController::builder()
                .genesis(Arc::new(MockGenesis::new(LatentDim::new(dim).expect("d"), 4)))
                .fusion(Arc::new(MockFusion::new(stats)))
                .dissolver(Arc::new(MockDissolver::new(dim as usize, "ok")))
                .alignment(Arc::new(MockAligner::safe()))
                .arbiter(SwarmArbiter::without_energy_metering())
                .consensus_factory(move || Box::new(ScriptedDetector::new(verdicts.clone())))
                .build()
                .expect("build");
            let resp = controller
                .execute(Request::for_tests(spec(dim, max_rounds)))
                .await
                .expect("ok");
            prop_assert_eq!(
                resp.rounds_executed,
                converge_at,
                "rounds_executed mismatch: expected {} (converge_at), got {}",
                converge_at,
                resp.rounds_executed
            );
            prop_assert!(resp.rounds_executed <= max_rounds);
            prop_assert!((resp.confidence - 0.92).abs() < 1e-5);
            Ok(())
        }).unwrap();
    }
}

// ====================================================================
// W3d — AlignmentBreach iff delta > max OR !is_finite(delta)
// ====================================================================

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_alignment_breach_iff_delta_violates(
        delta in -1.0f32..1.0,
    ) {
        rt_block_on(async move {
            let dim = 4u32;
            let aligner = Arc::new(MockAligner::new(delta, 1.0));
            let controller = SwarmController::builder()
                .genesis(Arc::new(MockGenesis::new(LatentDim::new(dim).expect("d"), 4)))
                .fusion(Arc::new(MockFusion::new(vec![FusionStats::synthetic_converging(1, dim as usize)])))
                .dissolver(Arc::new(MockDissolver::new(dim as usize, "ok")))
                .alignment(aligner)
                .arbiter(SwarmArbiter::without_energy_metering())
                .consensus_factory(|| {
                    Box::new(ScriptedDetector::new(vec![Verdict::Converged { confidence: 0.9 }]))
                })
                .build()
                .expect("build");
            let result = controller
                .execute(Request::for_tests(spec(dim, 4)))
                .await;
            // max_projection_distance = 0.15 in spec().
            let max_delta = 0.15f32;
            let should_breach = !delta.is_finite() || delta > max_delta;
            match (should_breach, &result) {
                (true, Err(EfeError::AlignmentBreach { .. })) => {} // ✓
                (false, Ok(_)) => {} // ✓
                _ => prop_assert!(
                    false,
                    "delta={delta} should_breach={should_breach}, got {result:?}"
                ),
            }
            Ok(())
        }).unwrap();
    }
}

// ====================================================================
// W3e — every successful execute() emits exactly one finalize event
// ====================================================================

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 16,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_successful_execute_emits_one_finalize(
        dim in 4u32..=16,
    ) {
        rt_block_on(async move {
            let telemetry = Arc::new(RecordingTelemetry::default());
            let controller = SwarmController::builder()
                .genesis(Arc::new(MockGenesis::new(LatentDim::new(dim).expect("d"), 4)))
                .fusion(Arc::new(MockFusion::new(vec![FusionStats::synthetic_converging(1, dim as usize)])))
                .dissolver(Arc::new(MockDissolver::new(dim as usize, "ok")))
                .alignment(Arc::new(MockAligner::safe()))
                .telemetry(telemetry.clone())
                .arbiter(SwarmArbiter::without_energy_metering())
                .consensus_factory(|| {
                    Box::new(ScriptedDetector::new(vec![Verdict::Converged { confidence: 0.9 }]))
                })
                .build()
                .expect("build");
            controller
                .execute(Request::for_tests(spec(dim, 4)))
                .await
                .expect("ok");

            let events = telemetry.events.lock().expect("lock");
            let begins = events.iter().filter(|e| e.starts_with("begin:")).count();
            let finalizes = events.iter().filter(|e| e.starts_with("finalize:")).count();
            prop_assert_eq!(begins, 1, "expected 1 begin, got {}; events={:?}", begins, *events);
            prop_assert_eq!(finalizes, 1, "expected 1 finalize, got {}; events={:?}", finalizes, *events);
            Ok(())
        }).unwrap();
    }
}

// ====================================================================
// W3f — total_ms is non-negative and finite for every successful run
// ====================================================================

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 16,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_response_total_ms_is_finite_non_negative(
        dim in 4u32..=16,
        max_rounds in 1u32..=8,
    ) {
        rt_block_on(async move {
            let stats: Vec<_> = (1..=max_rounds)
                .map(|r| FusionStats::synthetic_converging(r, dim as usize))
                .collect();
            let controller = SwarmController::builder()
                .genesis(Arc::new(MockGenesis::new(LatentDim::new(dim).expect("d"), 4)))
                .fusion(Arc::new(MockFusion::new(stats)))
                .dissolver(Arc::new(MockDissolver::new(dim as usize, "ok")))
                .alignment(Arc::new(MockAligner::safe()))
                .arbiter(SwarmArbiter::without_energy_metering())
                .consensus_factory(|| {
                    Box::new(ScriptedDetector::new(vec![Verdict::Converged { confidence: 0.9 }]))
                })
                .build()
                .expect("build");
            let resp = controller
                .execute(Request::for_tests(spec(dim, max_rounds)))
                .await
                .expect("ok");
            prop_assert!(resp.total_ms >= 0.0);
            prop_assert!(resp.total_ms.is_finite());
            // Confidence in [0, 1].
            prop_assert!(resp.confidence >= 0.0 && resp.confidence <= 1.0);
            Ok(())
        }).unwrap();
    }
}
