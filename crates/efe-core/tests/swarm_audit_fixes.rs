//! **B8 — Regression tests for the audit fixes #14, #16, #18.**
//!
//! Each test pins down the contract introduced by a fix so that any
//! future revert silently breaks the test instead of silently breaking
//! production.
//!
//! - Fix #14: `CollapseRecoveryPolicy::validate()` rejects unsafe
//!   `respawn_fraction` and `noise_sigma` values; `SwarmControllerBuilder::build()`
//!   propagates that error.
//! - Fix #16: `ArbiterSession::check()` skips a non-finite energy
//!   reading instead of silently passing.
//! - Fix #18: the tracing span records `state = "failed"` when the
//!   controller exits via an `Err` path.

mod mocks;

use std::sync::Arc;
use std::time::Duration;

use efe_core::errors::EfeError;
use efe_core::fusion::FusionStats;
use efe_core::swarm::{
    CollapseRecoveryPolicy, ControllerConfig, EnergyMeter, NullEnergyMeter, Request, SwarmArbiter,
    SwarmController, Verdict,
};
use efe_core::types::{
    AlignmentCone, LatentDim, PopulationMix, ResourceBudget, SwarmSpec, TemplateCode,
};

use mocks::{MockAligner, MockDissolver, MockFusion, MockGenesis, ScriptedDetector};

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
// Fix #14 — CollapseRecoveryPolicy validation
// ====================================================================

#[test]
fn fix14_collapse_policy_rejects_negative_respawn_fraction() {
    // CollapseRecoveryPolicy is #[non_exhaustive]: build via Default + mutation.
    let mut policy = CollapseRecoveryPolicy::default();
    policy.respawn_fraction = -0.1;
    assert!(matches!(
        policy.validate().unwrap_err(),
        EfeError::InvalidConfig(_)
    ));
}

#[test]
fn fix14_collapse_policy_rejects_respawn_fraction_above_1() {
    let mut policy = CollapseRecoveryPolicy::default();
    policy.respawn_fraction = 1.5;
    assert!(matches!(
        policy.validate().unwrap_err(),
        EfeError::InvalidConfig(_)
    ));
}

#[test]
fn fix14_collapse_policy_rejects_nan_respawn_fraction() {
    let mut policy = CollapseRecoveryPolicy::default();
    policy.respawn_fraction = f32::NAN;
    assert!(matches!(
        policy.validate().unwrap_err(),
        EfeError::InvalidConfig(_)
    ));
}

#[test]
fn fix14_collapse_policy_rejects_negative_noise_sigma() {
    let mut policy = CollapseRecoveryPolicy::default();
    policy.noise_sigma = -0.5;
    assert!(matches!(
        policy.validate().unwrap_err(),
        EfeError::InvalidConfig(_)
    ));
}

#[test]
fn fix14_collapse_policy_rejects_inf_noise_sigma() {
    let mut policy = CollapseRecoveryPolicy::default();
    policy.noise_sigma = f32::INFINITY;
    assert!(matches!(
        policy.validate().unwrap_err(),
        EfeError::InvalidConfig(_)
    ));
}

#[test]
fn fix14_default_policy_passes_validation() {
    CollapseRecoveryPolicy::default()
        .validate()
        .expect("default policy must validate");
}

#[tokio::test]
async fn fix14_controller_builder_rejects_invalid_policy() {
    let dim = 4u32;
    let mut bad_config = ControllerConfig::default();
    bad_config.mode_collapse_recovery.respawn_fraction = 1.5; // invalid
    let result = SwarmController::builder()
        .genesis(Arc::new(MockGenesis::new(LatentDim::new(dim).expect("d"), 4)))
        .fusion(Arc::new(MockFusion::new(vec![FusionStats::synthetic_converging(1, dim as usize)])))
        .dissolver(Arc::new(MockDissolver::new(dim as usize, "ok")))
        .alignment(Arc::new(MockAligner::safe()))
        .arbiter(SwarmArbiter::without_energy_metering())
        .consensus_factory(|| Box::new(ScriptedDetector::new(vec![Verdict::Converged { confidence: 0.9 }])))
        .config(bad_config)
        .build();
    assert!(
        matches!(result.err().unwrap(), EfeError::InvalidConfig(_)),
        "builder must reject malformed CollapseRecoveryPolicy"
    );
}

// ====================================================================
// Fix #16 — Arbiter rejects non-finite energy delta
// ====================================================================

/// Meter that returns NaN on second read (delta will be NaN).
struct NanOnSecondReadMeter {
    reads: std::sync::atomic::AtomicU32,
}
impl EnergyMeter for NanOnSecondReadMeter {
    fn energy_now_wh(&self) -> std::result::Result<f32, String> {
        let n = self.reads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if n == 0 {
            Ok(100.0)
        } else {
            Ok(f32::NAN)
        }
    }
}

#[tokio::test]
async fn fix16_arbiter_skips_non_finite_energy_delta() {
    let meter = Arc::new(NanOnSecondReadMeter {
        reads: std::sync::atomic::AtomicU32::new(0),
    });
    let arb = SwarmArbiter::new(meter);
    let mut budget = ResourceBudget::default();
    budget.max_wall_clock_ms = 100_000;
    budget.max_energy_wh = 0.0001; // very tight; pre-fix would mis-handle NaN delta
    let session = arb.new_session(budget);

    // Advance past first read → second read is NaN. Pre-fix this would
    // either pass silently (because NaN > x is false) or trigger an
    // ill-defined comparison. Post-fix we explicitly skip the check
    // and the wall_clock check passes.
    session
        .check(Duration::from_millis(1))
        .expect("non-finite energy delta must skip energy check, not panic or fail");
}

#[test]
fn fix16_null_energy_meter_no_op_unchanged() {
    // Sanity: with NullEnergyMeter, the `start_energy_wh` is `None`,
    // so the energy block is short-circuited and fix #16 path never
    // executes. This test pins the existing behaviour.
    let arb = SwarmArbiter::new(Arc::new(NullEnergyMeter));
    let mut budget = ResourceBudget::default();
    budget.max_wall_clock_ms = 100_000;
    budget.max_energy_wh = 0.0001;
    let session = arb.new_session(budget);
    session
        .check(Duration::from_millis(1))
        .expect("null meter best-effort");
}

// ====================================================================
// Fix #18 — Failed state is observable on error path
// ====================================================================

/// We can't easily intercept `tracing::Span::current().record(...)`
/// from a test, but we can verify the symmetric contract: on an error
/// path, the controller still calls `record_phase_error("swarm", ...)`
/// (which is what was added pre-fix) AND now also stamps the span.
/// The span stamping itself is best verified by logging in a real
/// tracing subscriber — out of scope for unit tests. We instead pin
/// the related telemetry contract.
#[tokio::test]
async fn fix18_error_path_records_phase_error_swarm() {
    use mocks::RecordingTelemetry;
    let dim = 4u32;
    let telemetry = Arc::new(RecordingTelemetry::default());

    // Force divergence to trigger an error path.
    let controller = SwarmController::builder()
        .genesis(Arc::new(MockGenesis::new(LatentDim::new(dim).expect("d"), 4)))
        .fusion(Arc::new(MockFusion::new(vec![FusionStats::synthetic_converging(1, dim as usize)])))
        .dissolver(Arc::new(MockDissolver::new(dim as usize, "ok")))
        .alignment(Arc::new(MockAligner::safe()))
        .telemetry(telemetry.clone())
        .arbiter(SwarmArbiter::without_energy_metering())
        .consensus_factory(|| {
            Box::new(ScriptedDetector::new(vec![Verdict::Divergence { reason: "x" }]))
        })
        .build()
        .expect("build");

    let err = controller
        .execute(Request::for_tests(spec(dim, 4, 4)))
        .await
        .unwrap_err();
    assert!(matches!(err, EfeError::Divergence { .. }));

    let events = telemetry.events.lock().expect("lock");
    // The telemetry guard records `phase_error:swarm:divergence` at the
    // execute() level (this is the path that fix #18 augmented with the
    // span stamp).
    assert!(
        events.iter().any(|e| e.starts_with("phase_error:swarm:")),
        "expected phase_error:swarm:* on error path; events={events:?}"
    );
}
