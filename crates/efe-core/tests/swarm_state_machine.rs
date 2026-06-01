//! **B4 — State machine tests for `efe_core::swarm::SwarmState`.**
//!
//! These tests pin down the swarm execution state machine:
//!
//! - **W1**: every transition through the controller is monotonic
//!   (Pending → Genesis → Fusion → Dissolution → {Done | Failed})
//! - All state labels are stable, snake_case, and grep-able
//! - Terminal states correctly identify themselves
//! - Round indexing in `Fusion { round }` is consistent
//!
//! Note: `SwarmState` is currently advisory (recorded into telemetry
//! spans), not a typestate. The audit treats this as documentation
//! and tests the *observable* transitions through telemetry events.

mod mocks;

use std::sync::Arc;

use efe_core::fusion::FusionStats;
use efe_core::swarm::{
    ControllerConfig, Request, SwarmArbiter, SwarmController, SwarmState, Verdict,
};
use efe_core::types::{AlignmentCone, LatentDim, PopulationMix, SwarmSpec, TemplateCode};

use mocks::{
    MockAligner, MockDissolver, MockFusion, MockGenesis, RecordingTelemetry, ScriptedDetector,
};

// --------------- helpers ---------------

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
// W1 — labels are stable snake_case
// ====================================================================

#[test]
fn state_labels_are_grep_stable() {
    let labels = [
        SwarmState::Pending.label(),
        SwarmState::Genesis.label(),
        SwarmState::Fusion { round: 1 }.label(),
        SwarmState::Fusion { round: 999 }.label(),
        SwarmState::Dissolution.label(),
        SwarmState::Done.label(),
        SwarmState::Failed.label(),
    ];
    for l in labels {
        assert!(
            l.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "label `{l}` is not snake_case"
        );
        assert!(!l.is_empty());
    }
    // Fusion variants share the same label regardless of round.
    assert_eq!(
        SwarmState::Fusion { round: 1 }.label(),
        SwarmState::Fusion { round: 999 }.label(),
    );
}

// ====================================================================
// W1.b — terminal states are exactly Done and Failed
// ====================================================================

#[test]
fn state_terminal_is_done_or_failed() {
    assert!(SwarmState::Done.is_terminal());
    assert!(SwarmState::Failed.is_terminal());
    assert!(!SwarmState::Pending.is_terminal());
    assert!(!SwarmState::Genesis.is_terminal());
    assert!(!SwarmState::Fusion { round: 1 }.is_terminal());
    assert!(!SwarmState::Fusion { round: u32::MAX }.is_terminal());
    assert!(!SwarmState::Dissolution.is_terminal());
}

// ====================================================================
// W1.c — happy path emits genesis → round → finalize through telemetry
// ====================================================================

#[tokio::test]
async fn state_happy_path_emits_monotonic_transitions() {
    let dim = 4;
    let genesis = Arc::new(MockGenesis::new(LatentDim::new(dim).expect("dim"), 4));
    let fusion = Arc::new(MockFusion::new(vec![
        FusionStats::synthetic_converging(1, dim as usize),
    ]));
    let dissolver = Arc::new(MockDissolver::new(dim as usize, "ok"));
    let aligner = Arc::new(MockAligner::safe());
    let telemetry = Arc::new(RecordingTelemetry::default());

    let controller = SwarmController::builder()
        .genesis(genesis)
        .fusion(fusion)
        .dissolver(dissolver)
        .alignment(aligner)
        .telemetry(telemetry.clone())
        .arbiter(SwarmArbiter::without_energy_metering())
        .consensus_factory(|| {
            Box::new(ScriptedDetector::new(vec![Verdict::Converged {
                confidence: 0.9,
            }]))
        })
        .config(ControllerConfig::default())
        .build()
        .expect("build");

    let req = Request::for_tests(spec(dim, 4, 8));
    controller.execute(req).await.expect("ok");

    let events = telemetry.events.lock().expect("lock");

    // The events must observe genesis BEFORE round (any round) BEFORE finalize.
    let genesis_pos = events
        .iter()
        .position(|e| e.starts_with("phase_complete:genesis"))
        .expect("genesis emitted");
    let round_pos = events
        .iter()
        .position(|e| e.starts_with("round:"))
        .expect("round emitted");
    let finalize_pos = events
        .iter()
        .position(|e| e.starts_with("finalize:"))
        .expect("finalize emitted");

    assert!(
        genesis_pos < round_pos,
        "monotonic violation: genesis pos {genesis_pos} >= round pos {round_pos}"
    );
    assert!(
        round_pos < finalize_pos,
        "monotonic violation: round pos {round_pos} >= finalize pos {finalize_pos}"
    );
}

// ====================================================================
// W2 — error path records phase_error AT the failing phase, never
//      after, and finalize is NOT emitted on the error path
//      (audit-S5-bis defect #8)
// ====================================================================

#[tokio::test]
async fn state_error_in_genesis_records_no_round_no_finalize() {
    use efe_core::errors::{EfeError, Result};
    use efe_core::swarm::GenesisStrategy;
    struct PanickingGenesis(#[allow(dead_code)] LatentDim);
    #[async_trait::async_trait]
    impl GenesisStrategy for PanickingGenesis {
        async fn spawn(
            &self,
            _: &SwarmSpec,
        ) -> Result<(efe_core::memory::NanoAgentBatch, efe_core::els::EpistemicLatentSpace)>
        {
            Err(EfeError::HbmAllocFailed { requested: 0 })
        }
        async fn respawn_diversify(
            &self,
            _: &mut efe_core::memory::NanoAgentBatch,
            _: u32,
        ) -> Result<()> {
            Ok(())
        }
    }

    let dim = 4;
    let genesis = Arc::new(PanickingGenesis(LatentDim::new(dim).expect("dim")));
    let fusion = Arc::new(MockFusion::new(vec![
        FusionStats::synthetic_converging(1, dim as usize),
    ]));
    let dissolver = Arc::new(MockDissolver::new(dim as usize, "n/a"));
    let aligner = Arc::new(MockAligner::safe());
    let telemetry = Arc::new(RecordingTelemetry::default());

    let controller = SwarmController::builder()
        .genesis(genesis)
        .fusion(fusion)
        .dissolver(dissolver)
        .alignment(aligner)
        .telemetry(telemetry.clone())
        .arbiter(SwarmArbiter::without_energy_metering())
        .consensus_factory(|| {
            Box::new(ScriptedDetector::new(vec![Verdict::Converged {
                confidence: 0.9,
            }]))
        })
        .build()
        .expect("build");

    let req = Request::for_tests(spec(dim, 4, 8));
    let err = controller.execute(req).await.unwrap_err();
    assert!(matches!(err, EfeError::HbmAllocFailed { .. }));

    let events = telemetry.events.lock().expect("lock");
    // Genesis phase_error MUST be recorded, BEFORE any round event.
    let phase_err_pos = events
        .iter()
        .position(|e| e.starts_with("phase_error:genesis"))
        .expect("phase_error:genesis recorded");
    let round_present = events.iter().any(|e| e.starts_with("round:"));
    assert!(
        !round_present,
        "no round should be recorded when genesis fails; events={events:?}"
    );

    // The current contract (defect #8 audit): on error path, `execute()`
    // calls `record_phase_error("swarm", err)` for the top-level error,
    // and DOES NOT call `finalize`. We assert finalize is NOT present.
    let finalize_present = events.iter().any(|e| e.starts_with("finalize:"));
    assert!(
        !finalize_present,
        "finalize must not be called on error path; events={events:?}"
    );
    // Phase error recorded twice: once at genesis (inspect_err), once at
    // execute() level under "swarm". We just verify the genesis one
    // appears, position is < end.
    assert!(phase_err_pos < events.len());
}
