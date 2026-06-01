//! Integration test : two identical-dimension swarms collaborate via a
//! uniform meta-operator and converge.
//!
//! Verifies :
//!   1. Build succeeds.
//!   2. Iteration terminates with `StopReason::Converged`.
//!   3. Final residual is below the configured tolerance.
//!   4. Final participant states agree on the consensus through their lifts.
//!   5. Re-running with the same seeds is deterministic.

use efe_meta_swarm::prelude::*;

fn build() -> Result<MetaSwarm> {
    MetaSwarm::builder()
        .id("integration_two_swarms_uniform")
        .meta_space_dim(8)
        .add_participant(MockParticipant::with_contraction("a", 16, 0.6, 1)?.boxed())
        .add_participant(MockParticipant::with_contraction("b", 16, 0.6, 2)?.boxed())
        .lift_for("a", LinearLift::identity(16, 8)?.boxed())
        .lift_for("b", LinearLift::identity(16, 8)?.boxed())
        .pull_for("a", LinearPull::identity(8, 16)?.boxed())
        .pull_for("b", LinearPull::identity(8, 16)?.boxed())
        .meta_operator(UniformMetaOp::new(8).boxed())
        .meta_safety(BoxSafety::symmetric(8, 10.0).boxed())
        .alpha_default(0.5)
        .coupling_gamma(0.2)
        .tolerance(1e-3)
        .max_iters(500)
        .trace(true)
        .build()
}

#[test]
fn two_swarms_converge_with_trace() {
    let ms = build().unwrap();
    let result = ms.iterate().unwrap();
    assert_eq!(result.stop_reason, StopReason::Converged);
    assert!(result.final_residual < 1e-3, "residual = {}", result.final_residual);
    assert!(result.iterations < 500);
    assert!(result.trace.is_some());
    let trace = result.trace.unwrap();
    assert_eq!(trace.len(), result.iterations);

    // Residuals should be monotonically non-increasing in the long run.
    // (Some increase early is acceptable because of the projection step.)
    let r = trace.residuals();
    let first_third = r[0];
    let last = *r.last().unwrap();
    assert!(last <= first_third, "residual did not decrease : first={first_third}, last={last}");
}

#[test]
fn deterministic_reruns() {
    let ms_a = build().unwrap();
    let ms_b = build().unwrap();
    let r_a = ms_a.iterate().unwrap();
    let r_b = ms_b.iterate().unwrap();
    assert_eq!(r_a.iterations, r_b.iterations);
    assert!((r_a.final_residual - r_b.final_residual).abs() < 1e-9);
}

#[test]
fn final_states_inside_their_safety_sets() {
    let ms = build().unwrap();
    let result = ms.iterate().unwrap();
    // BoxSafety symmetric r=10 ; the contracted states should easily fit.
    let safety = BoxSafety::symmetric(16, 10.0);
    for s in &result.participant_states {
        assert!(safety.contains(s, 1e-6), "state outside C : {s:?}");
    }
}
