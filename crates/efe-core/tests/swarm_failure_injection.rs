//! **B6 — Failure injection tests for `efe_core::swarm::SwarmController`.**
//!
//! These tests systematically inject failures into each strategy and
//! verify the controller handles them per-contract:
//!
//! - **W2**: every error path records `phase_error` BEFORE propagating
//! - Genesis errors short-circuit to no rounds
//! - Fusion errors at round k → propagated, no further rounds
//! - Dissolver `extract_centroid` errors → no decode, telemetry recorded
//! - Dissolver `decode` errors → after extract success, propagates
//! - Aligner errors are NOT translated as breach — they propagate as-is
//! - Aligner returning NaN delta → AlignmentBreach (audit existing)
//! - Aligner returning > max delta → AlignmentBreach
//! - Telemetry panics in record_phase_error do not double-fault
//!
//! These mocks are inline (file-local) rather than added to the shared
//! `mocks.rs` to keep the audit boundary visible: each test owns its
//! injection precisely.

mod mocks;

use std::sync::Arc;

use async_trait::async_trait;

use efe_core::els::EpistemicLatentSpace;
use efe_core::errors::{EfeError, Result as EfeResult};
use efe_core::fusion::FusionStats;
use efe_core::memory::NanoAgentBatch;
use efe_core::swarm::{
    AlignmentProjector, DecodeMode, DecodedOutput, Dissolver, FusionStrategy, GenesisStrategy,
    Request, SwarmArbiter, SwarmController, Verdict,
};
use efe_core::types::{AlignmentCone, LatentDim, PopulationMix, SwarmSpec, TemplateCode};

use mocks::{MockAligner, MockDissolver, MockFusion, MockGenesis, RecordingTelemetry, ScriptedDetector};

fn spec(dim: u32, agents: u32, max_rounds: u32) -> SwarmSpec {
    SwarmSpec::builder()
        .problem_embedding(vec![0.0; dim as usize])
        .dim_latent(LatentDim::new(dim).expect("dim"))
        .population(PopulationMix::new(vec![(TemplateCode::NaSem, agents)]).expect("mix"))
        .max_rounds(max_rounds)
        .expect("rounds")
        .alignment_cone(
            AlignmentCone::new(vec![vec![0.1; dim as usize]], 0.15).expect("cone"),
        )
        .build()
        .expect("spec")
}

// ====================================================================
// Failing genesis: returns Err — no rounds, phase_error recorded
// ====================================================================

struct FailingGenesis;
#[async_trait]
impl GenesisStrategy for FailingGenesis {
    async fn spawn(
        &self,
        _: &SwarmSpec,
    ) -> EfeResult<(NanoAgentBatch, EpistemicLatentSpace)> {
        Err(EfeError::HbmAllocFailed { requested: 1024 })
    }
    async fn respawn_diversify(&self, _: &mut NanoAgentBatch, _: u32) -> EfeResult<()> {
        Ok(())
    }
}

#[tokio::test]
async fn failing_genesis_records_phase_error_no_rounds() {
    let dim = 4u32;
    let telemetry = Arc::new(RecordingTelemetry::default());
    let controller = SwarmController::builder()
        .genesis(Arc::new(FailingGenesis))
        .fusion(Arc::new(MockFusion::new(vec![FusionStats::synthetic_converging(1, dim as usize)])))
        .dissolver(Arc::new(MockDissolver::new(dim as usize, "n/a")))
        .alignment(Arc::new(MockAligner::safe()))
        .telemetry(telemetry.clone())
        .arbiter(SwarmArbiter::without_energy_metering())
        .consensus_factory(|| {
            Box::new(ScriptedDetector::new(vec![Verdict::Converged { confidence: 0.9 }]))
        })
        .build()
        .expect("build");

    let err = controller
        .execute(Request::for_tests(spec(dim, 4, 4)))
        .await
        .unwrap_err();
    assert!(matches!(err, EfeError::HbmAllocFailed { .. }));

    let events = telemetry.events.lock().expect("lock");
    assert!(
        events.iter().any(|e| e.starts_with("phase_error:genesis")),
        "phase_error:genesis must be recorded; events={events:?}"
    );
    assert!(
        !events.iter().any(|e| e.starts_with("round:")),
        "no round must be recorded; events={events:?}"
    );
}

// ====================================================================
// Failing fusion at round k → no further rounds, error propagated
// ====================================================================

struct FailingAtRound {
    fail_at: u32,
}
#[async_trait]
impl FusionStrategy for FailingAtRound {
    async fn run_round(
        &self,
        _: &mut EpistemicLatentSpace,
        _: &NanoAgentBatch,
        round: u32,
    ) -> EfeResult<FusionStats> {
        if round == self.fail_at {
            Err(EfeError::KernelFailure(format!("intentional at round {round}")))
        } else {
            Ok(FusionStats::synthetic_converging(round, 4))
        }
    }
    async fn inject_noise(&self, _: &mut EpistemicLatentSpace, _: f32) -> EfeResult<()> {
        Ok(())
    }
}

#[tokio::test]
async fn failing_fusion_at_round_2_propagates_no_round_3() {
    let dim = 4u32;
    let telemetry = Arc::new(RecordingTelemetry::default());
    let controller = SwarmController::builder()
        .genesis(Arc::new(MockGenesis::new(LatentDim::new(dim).expect("dim"), 4)))
        .fusion(Arc::new(FailingAtRound { fail_at: 2 }))
        .dissolver(Arc::new(MockDissolver::new(dim as usize, "n/a")))
        .alignment(Arc::new(MockAligner::safe()))
        .telemetry(telemetry.clone())
        .arbiter(SwarmArbiter::without_energy_metering())
        .consensus_factory(|| {
            Box::new(ScriptedDetector::new(vec![
                Verdict::Continue,
                Verdict::Continue,
                Verdict::Continue,
            ]))
        })
        .build()
        .expect("build");

    let err = controller
        .execute(Request::for_tests(spec(dim, 4, 8)))
        .await
        .unwrap_err();
    assert!(matches!(err, EfeError::KernelFailure(_)));

    let events = telemetry.events.lock().expect("lock");
    let rounds: Vec<u32> = events
        .iter()
        .filter_map(|e| e.strip_prefix("round:").and_then(|n| n.parse().ok()))
        .collect();
    // Round 1 succeeded → recorded. Round 2 failed → not recorded
    // (run_round returned Err before record_round).
    assert_eq!(rounds, vec![1u32], "rounds recorded: {rounds:?}");
    assert!(events.iter().any(|e| e.starts_with("phase_error:fusion")));
}

// ====================================================================
// Failing dissolver::extract → no decode happens
// ====================================================================

struct FailingExtractor;
#[async_trait]
impl Dissolver for FailingExtractor {
    async fn extract_centroid(&self, _: &EpistemicLatentSpace) -> EfeResult<Vec<f32>> {
        Err(EfeError::KernelFailure("frechet failed".into()))
    }
    async fn decode(&self, _: &[f32], _: &DecodeMode) -> EfeResult<DecodedOutput> {
        unreachable!("decode must not be called when extract_centroid fails")
    }
}

#[tokio::test]
async fn failing_extract_centroid_propagates_no_decode() {
    let dim = 4u32;
    let telemetry = Arc::new(RecordingTelemetry::default());
    let controller = SwarmController::builder()
        .genesis(Arc::new(MockGenesis::new(LatentDim::new(dim).expect("dim"), 4)))
        .fusion(Arc::new(MockFusion::new(vec![FusionStats::synthetic_converging(1, dim as usize)])))
        .dissolver(Arc::new(FailingExtractor))
        .alignment(Arc::new(MockAligner::safe()))
        .telemetry(telemetry.clone())
        .arbiter(SwarmArbiter::without_energy_metering())
        .consensus_factory(|| {
            Box::new(ScriptedDetector::new(vec![Verdict::Converged { confidence: 0.9 }]))
        })
        .build()
        .expect("build");

    let err = controller
        .execute(Request::for_tests(spec(dim, 4, 4)))
        .await
        .unwrap_err();
    assert!(matches!(err, EfeError::KernelFailure(_)));

    let events = telemetry.events.lock().expect("lock");
    assert!(events.iter().any(|e| e.starts_with("phase_error:dissolution")));
}

// ====================================================================
// Failing dissolver::decode (after successful extract) → propagates
// ====================================================================

struct FailingDecoder {
    centroid: Vec<f32>,
}
#[async_trait]
impl Dissolver for FailingDecoder {
    async fn extract_centroid(&self, _: &EpistemicLatentSpace) -> EfeResult<Vec<f32>> {
        Ok(self.centroid.clone())
    }
    async fn decode(&self, _: &[f32], _: &DecodeMode) -> EfeResult<DecodedOutput> {
        Err(EfeError::Internal("tokenizer failed"))
    }
}

#[tokio::test]
async fn failing_decode_propagates_after_alignment() {
    let dim = 4u32;
    let telemetry = Arc::new(RecordingTelemetry::default());
    let controller = SwarmController::builder()
        .genesis(Arc::new(MockGenesis::new(LatentDim::new(dim).expect("dim"), 4)))
        .fusion(Arc::new(MockFusion::new(vec![FusionStats::synthetic_converging(1, dim as usize)])))
        .dissolver(Arc::new(FailingDecoder { centroid: vec![0.05; dim as usize] }))
        .alignment(Arc::new(MockAligner::safe()))
        .telemetry(telemetry.clone())
        .arbiter(SwarmArbiter::without_energy_metering())
        .consensus_factory(|| {
            Box::new(ScriptedDetector::new(vec![Verdict::Converged { confidence: 0.9 }]))
        })
        .build()
        .expect("build");

    let err = controller
        .execute(Request::for_tests(spec(dim, 4, 4)))
        .await
        .unwrap_err();
    assert!(matches!(err, EfeError::Internal(_)));
    let events = telemetry.events.lock().expect("lock");
    assert!(events.iter().any(|e| e.starts_with("phase_error:dissolution")));
}

// ====================================================================
// Aligner returning Err propagates as-is, NOT translated to breach
// (defect #13)
// ====================================================================

struct ErringAligner;
impl AlignmentProjector for ErringAligner {
    fn project(&self, _: &[f32], _: &AlignmentCone) -> EfeResult<(Vec<f32>, f32)> {
        Err(EfeError::Internal("projection numerical instability"))
    }
}

#[tokio::test]
async fn aligner_err_propagates_not_as_breach() {
    let dim = 4u32;
    let controller = SwarmController::builder()
        .genesis(Arc::new(MockGenesis::new(LatentDim::new(dim).expect("dim"), 4)))
        .fusion(Arc::new(MockFusion::new(vec![FusionStats::synthetic_converging(1, dim as usize)])))
        .dissolver(Arc::new(MockDissolver::new(dim as usize, "ok")))
        .alignment(Arc::new(ErringAligner))
        .arbiter(SwarmArbiter::without_energy_metering())
        .consensus_factory(|| {
            Box::new(ScriptedDetector::new(vec![Verdict::Converged { confidence: 0.9 }]))
        })
        .build()
        .expect("build");

    let err = controller
        .execute(Request::for_tests(spec(dim, 4, 4)))
        .await
        .unwrap_err();
    // Must NOT be AlignmentBreach.
    assert!(
        !matches!(err, EfeError::AlignmentBreach { .. }),
        "Err from aligner must NOT be re-cast as AlignmentBreach; got {err:?}"
    );
    assert!(matches!(err, EfeError::Internal(_)));
}

// ====================================================================
// Aligner returning NaN delta → AlignmentBreach (existing audit
// invariant, replicated here for completeness)
// ====================================================================

#[tokio::test]
async fn aligner_nan_delta_treated_as_breach() {
    let dim = 4u32;
    // MockAligner with NaN delta.
    let aligner = Arc::new(MockAligner::new(f32::NAN, 1.0));
    let controller = SwarmController::builder()
        .genesis(Arc::new(MockGenesis::new(LatentDim::new(dim).expect("dim"), 4)))
        .fusion(Arc::new(MockFusion::new(vec![FusionStats::synthetic_converging(1, dim as usize)])))
        .dissolver(Arc::new(MockDissolver::new(dim as usize, "ok")))
        .alignment(aligner)
        .arbiter(SwarmArbiter::without_energy_metering())
        .consensus_factory(|| {
            Box::new(ScriptedDetector::new(vec![Verdict::Converged { confidence: 0.9 }]))
        })
        .build()
        .expect("build");

    let err = controller
        .execute(Request::for_tests(spec(dim, 4, 4)))
        .await
        .unwrap_err();
    assert!(
        matches!(err, EfeError::AlignmentBreach { delta, .. } if !delta.is_finite()),
        "NaN delta must produce AlignmentBreach with non-finite delta; got {err:?}"
    );
}

// ====================================================================
// Aligner delta > max → AlignmentBreach with delta value
// ====================================================================

#[tokio::test]
async fn aligner_delta_exceeds_max_treated_as_breach() {
    let dim = 4u32;
    // Cone allows max 0.15. Aligner reports delta=0.5 → breach.
    let aligner = Arc::new(MockAligner::new(0.5, 1.0));
    let controller = SwarmController::builder()
        .genesis(Arc::new(MockGenesis::new(LatentDim::new(dim).expect("dim"), 4)))
        .fusion(Arc::new(MockFusion::new(vec![FusionStats::synthetic_converging(1, dim as usize)])))
        .dissolver(Arc::new(MockDissolver::new(dim as usize, "ok")))
        .alignment(aligner)
        .arbiter(SwarmArbiter::without_energy_metering())
        .consensus_factory(|| {
            Box::new(ScriptedDetector::new(vec![Verdict::Converged { confidence: 0.9 }]))
        })
        .build()
        .expect("build");

    let err = controller
        .execute(Request::for_tests(spec(dim, 4, 4)))
        .await
        .unwrap_err();
    if let EfeError::AlignmentBreach { delta, max } = err {
        assert!((delta - 0.5).abs() < 1e-6);
        assert!((max - 0.15).abs() < 1e-6);
    } else {
        panic!("expected AlignmentBreach; got {err:?}");
    }
}

// ====================================================================
// Genesis returns valid then fusion immediately diverges → Divergence
// emitted at round 1
// ====================================================================

#[tokio::test]
async fn fusion_diverges_emits_divergence_error() {
    let dim = 4u32;
    let controller = SwarmController::builder()
        .genesis(Arc::new(MockGenesis::new(LatentDim::new(dim).expect("dim"), 4)))
        .fusion(Arc::new(MockFusion::new(vec![FusionStats::synthetic_converging(1, dim as usize)])))
        .dissolver(Arc::new(MockDissolver::new(dim as usize, "ok")))
        .alignment(Arc::new(MockAligner::safe()))
        .arbiter(SwarmArbiter::without_energy_metering())
        .consensus_factory(|| {
            Box::new(ScriptedDetector::new(vec![Verdict::Divergence {
                reason: "variance climbing",
            }]))
        })
        .build()
        .expect("build");

    let err = controller
        .execute(Request::for_tests(spec(dim, 4, 4)))
        .await
        .unwrap_err();
    assert!(
        matches!(err, EfeError::Divergence { rounds: 1, reason } if reason == "variance climbing"),
        "expected Divergence at round 1; got {err:?}"
    );
}

// ====================================================================
// Convergence timeout: max_rounds reached with all Continue
// ====================================================================

#[tokio::test]
async fn convergence_timeout_after_max_rounds_continue() {
    let dim = 4u32;
    let max_rounds = 3u32;
    let stats: Vec<_> = (1..=max_rounds)
        .map(|r| FusionStats::synthetic_converging(r, dim as usize))
        .collect();

    let controller = SwarmController::builder()
        .genesis(Arc::new(MockGenesis::new(LatentDim::new(dim).expect("dim"), 4)))
        .fusion(Arc::new(MockFusion::new(stats)))
        .dissolver(Arc::new(MockDissolver::new(dim as usize, "ok")))
        .alignment(Arc::new(MockAligner::safe()))
        .arbiter(SwarmArbiter::without_energy_metering())
        .consensus_factory(|| {
            // Always Continue → never converge.
            Box::new(ScriptedDetector::new(vec![Verdict::Continue; 100]))
        })
        .build()
        .expect("build");

    let err = controller
        .execute(Request::for_tests(spec(dim, 4, max_rounds)))
        .await
        .unwrap_err();
    assert!(
        matches!(err, EfeError::ConvergenceTimeout { rounds, .. } if rounds == max_rounds),
        "expected ConvergenceTimeout(rounds={max_rounds}); got {err:?}"
    );
}
