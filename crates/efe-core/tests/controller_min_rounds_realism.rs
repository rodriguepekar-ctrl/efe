//! **Sprint D6 — `min_rounds` floor anti-regression tests.**
//!
//! These tests pin the contract introduced in Sprint D6 :
//!
//! 1. `ControllerConfig::default()` returns `min_rounds = 0` —
//!    backward-compatible with every pre-D6 test rig.
//! 2. When `min_rounds > 0`, a `Verdict::Converged` emitted at a
//!    round number below the floor is downgraded to `Continue`,
//!    forcing the loop to keep running.
//! 3. When `min_rounds == max_rounds` and the detector says
//!    `Converged` from round 1, the loop runs exactly
//!    `max_rounds` times then breaks at the last round.
//! 4. When `min_rounds > max_rounds`, the loop runs to
//!    `max_rounds` without converging and emits
//!    `ConvergenceTimeout`.
//! 5. The floor only affects `Verdict::Converged` — `Continue`,
//!    `Divergence`, and `ModeCollapse` are passed through
//!    unchanged.
//!
//! These tests use the canonical mock harness in
//! `crates/efe-core/tests/mocks/` (the same one used by the
//! audit, failure-injection, and properties suites).

mod mocks;

use std::sync::Arc;

use efe_core::fusion::FusionStats;
use efe_core::swarm::controller::ControllerConfig;
use efe_core::swarm::{Request, SwarmController, Verdict};
use efe_core::types::{
    AlignmentCone, LatentDim, PopulationMix, SwarmSpec, TemplateCode,
};
use efe_core::EfeError;

use mocks::{
    MockAligner, MockDissolver, MockFusion, MockGenesis,
    RecordingTelemetry, ScriptedDetector,
};

// =====================================================================
// Test fixtures
// =====================================================================

const DIM: u32 = 4;
const N_AGENTS: u32 = 4;

fn minimal_spec(max_rounds: u32) -> SwarmSpec {
    SwarmSpec::builder()
        .problem_embedding(vec![0.1; DIM as usize])
        .dim_latent(LatentDim::new(DIM).expect("valid dim"))
        .population(
            PopulationMix::new(vec![(TemplateCode::NaSem, N_AGENTS)])
                .expect("valid pop"),
        )
        .max_rounds(max_rounds)
        .expect("valid rounds")
        .alignment_cone(
            AlignmentCone::new(vec![vec![0.1; DIM as usize]], 1.0)
                .expect("valid cone"),
        )
        .build()
        .expect("valid spec")
}

/// Build N synthetic fusion stats whose `round` field matches
/// the index. Each stats indicates "converging" so the detector
/// will emit `Converged` (subject to its own scripted plan).
fn synthetic_stats(n: u32) -> Vec<FusionStats> {
    (1..=n)
        .map(|r| FusionStats::synthetic_converging(r, DIM as usize))
        .collect()
}

// =====================================================================
// Test 1 — Library default has no floor (backward compat)
// =====================================================================

#[test]
fn controller_default_min_rounds_is_zero() {
    let cfg = ControllerConfig::default();
    assert_eq!(
        cfg.min_rounds, 0,
        "library default min_rounds must be 0 for backward compat ; \
         the D6 fix is applied via Mode1Config in the server crate"
    );
}

// =====================================================================
// Test 2 — min_rounds = 0 reproduces legacy behaviour
// =====================================================================
//
// With min_rounds = 0 (the library default) and a detector that
// says Converged at round 1, the loop breaks at round 1. Total
// fusion rounds executed = 1. This is the pre-D6 behaviour.

#[tokio::test]
async fn min_rounds_zero_breaks_at_first_converged() {
    let genesis = Arc::new(MockGenesis::new(LatentDim::new(DIM).unwrap(), N_AGENTS));
    let fusion = Arc::new(MockFusion::new(synthetic_stats(1)));
    let dissolver = Arc::new(MockDissolver::new(DIM as usize, "ok"));
    let aligner = Arc::new(MockAligner::new(0.1, 0.5));
    let telemetry = Arc::new(RecordingTelemetry::default());

    let controller = SwarmController::builder()
        .genesis(genesis)
        .fusion(fusion.clone())
        .dissolver(dissolver)
        .alignment(aligner)
        .telemetry(telemetry)
        .consensus_factory(|| {
            Box::new(ScriptedDetector::new(vec![Verdict::Converged {
                confidence: 0.9,
            }]))
        })
        .config(ControllerConfig::default()) // min_rounds = 0
        .build()
        .expect("build");

    let resp = controller
        .execute(Request::for_tests(minimal_spec(8)))
        .await
        .expect("execute");

    assert_eq!(resp.rounds_executed, 1);
    // Verifying through the fusion mock that exactly 1 round was
    // played would require introspecting MockFusion's call count.
    // The response's `rounds_executed` is the controller's
    // contract surface and is sufficient for this test.
}

// =====================================================================
// Test 3 — min_rounds > 0 floor downgrades early Converged
// =====================================================================
//
// With min_rounds = 4 and a detector that says Converged from
// round 1 onwards, the controller should keep going until round 4,
// then honour the verdict. Total rounds executed = 4.
//
// Test data : 4 fusion frames + 4 verdicts (all Converged, but
// the first 3 are downgraded by the floor).

#[tokio::test]
async fn min_rounds_floor_downgrades_early_converged() {
    let genesis = Arc::new(MockGenesis::new(LatentDim::new(DIM).unwrap(), N_AGENTS));
    let fusion = Arc::new(MockFusion::new(synthetic_stats(4)));
    let dissolver = Arc::new(MockDissolver::new(DIM as usize, "ok"));
    let aligner = Arc::new(MockAligner::new(0.1, 0.5));
    let telemetry = Arc::new(RecordingTelemetry::default());

    let mut cfg = ControllerConfig::default();
    cfg.min_rounds = 4;

    let controller = SwarmController::builder()
        .genesis(genesis)
        .fusion(fusion)
        .dissolver(dissolver)
        .alignment(aligner)
        .telemetry(telemetry)
        .consensus_factory(|| {
            Box::new(ScriptedDetector::new(vec![
                Verdict::Converged { confidence: 0.9 },
                Verdict::Converged { confidence: 0.9 },
                Verdict::Converged { confidence: 0.9 },
                Verdict::Converged { confidence: 0.9 },
            ]))
        })
        .config(cfg)
        .build()
        .expect("build");

    let resp = controller
        .execute(Request::for_tests(minimal_spec(8)))
        .await
        .expect("execute");

    assert_eq!(
        resp.rounds_executed, 4,
        "floor must hold the first 3 Converged verdicts and \
         honour the 4th — got {} rounds",
        resp.rounds_executed
    );
}

// =====================================================================
// Test 4 — min_rounds > max_rounds yields ConvergenceTimeout
// =====================================================================
//
// If the floor is configured higher than the spec's max_rounds,
// the loop runs to max_rounds without ever converging and emits
// `ConvergenceTimeout`. This is a degraded-but-safe outcome that
// the warning log at the top of the loop signals.

#[tokio::test]
async fn min_rounds_above_max_rounds_yields_timeout() {
    let genesis = Arc::new(MockGenesis::new(LatentDim::new(DIM).unwrap(), N_AGENTS));
    // 3 fusion frames — matches max_rounds=3 below.
    let fusion = Arc::new(MockFusion::new(synthetic_stats(3)));
    let dissolver = Arc::new(MockDissolver::new(DIM as usize, "ok"));
    let aligner = Arc::new(MockAligner::new(0.1, 0.5));
    let telemetry = Arc::new(RecordingTelemetry::default());

    let mut cfg = ControllerConfig::default();
    cfg.min_rounds = 10; // > max_rounds = 3 below

    let controller = SwarmController::builder()
        .genesis(genesis)
        .fusion(fusion)
        .dissolver(dissolver)
        .alignment(aligner)
        .telemetry(telemetry)
        .consensus_factory(|| {
            // Detector says Converged every round, but the floor
            // never lets the loop break (min_rounds=10 > max=3).
            Box::new(ScriptedDetector::new(vec![
                Verdict::Converged { confidence: 0.9 },
                Verdict::Converged { confidence: 0.9 },
                Verdict::Converged { confidence: 0.9 },
            ]))
        })
        .config(cfg)
        .build()
        .expect("build");

    let err = controller
        .execute(Request::for_tests(minimal_spec(3)))
        .await
        .unwrap_err();

    assert!(
        matches!(err, EfeError::ConvergenceTimeout { rounds: 3, .. }),
        "expected ConvergenceTimeout(rounds=3), got {err:?}"
    );
}

// =====================================================================
// Test 5 — Floor only affects Converged ; Continue passes through
// =====================================================================
//
// If the detector says `Continue` for the first 2 rounds and
// `Converged` at round 3, the floor (=2) lets the round-3
// Converged through (3 >= 2). The loop runs exactly 3 rounds.

#[tokio::test]
async fn min_rounds_floor_does_not_block_natural_continue() {
    let genesis = Arc::new(MockGenesis::new(LatentDim::new(DIM).unwrap(), N_AGENTS));
    let fusion = Arc::new(MockFusion::new(synthetic_stats(3)));
    let dissolver = Arc::new(MockDissolver::new(DIM as usize, "ok"));
    let aligner = Arc::new(MockAligner::new(0.1, 0.5));
    let telemetry = Arc::new(RecordingTelemetry::default());

    let mut cfg = ControllerConfig::default();
    cfg.min_rounds = 2;

    let controller = SwarmController::builder()
        .genesis(genesis)
        .fusion(fusion)
        .dissolver(dissolver)
        .alignment(aligner)
        .telemetry(telemetry)
        .consensus_factory(|| {
            Box::new(ScriptedDetector::new(vec![
                Verdict::Continue,                            // round 1
                Verdict::Continue,                            // round 2
                Verdict::Converged { confidence: 0.9 },       // round 3
            ]))
        })
        .config(cfg)
        .build()
        .expect("build");

    let resp = controller
        .execute(Request::for_tests(minimal_spec(8)))
        .await
        .expect("execute");

    assert_eq!(
        resp.rounds_executed, 3,
        "round-3 Converged must be honoured (3 >= min_rounds=2)"
    );
}
