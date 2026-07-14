// SPDX-License-Identifier: UNLICENSED
//! **Couche 3 · M1.c — the H1 gate, exercised end-to-end (mock participants).**
//!
//! These tests need no `efe-core`/`efe-server`: they drive the public `MetaSwarm` API
//! with `MockParticipant`s (which return a `Stub` certificate) to prove the *gate*
//! behaviour M1.c revives:
//!
//! * **strict mode rejects an unverified `Stub`** — the H1 pre-check fails-closed;
//! * **lenient mode still accepts `Stub`** — Phase-1 backward-compat;
//! * **`require_converged` fails-closed** — a non-converged run never yields a consensus.
//!
//! The *real* empirical canary over an efe-core `Φ` (and the strict fail-closed-on-
//! non-convergence path with a participant that actually passes H1) is exercised in
//! `tests/integration_local_participant.rs`.

use efe_meta_swarm::MetaSwarm;
use efe_meta_swarm::error::MetaSwarmError;
use efe_meta_swarm::iteration::StopReason;
use efe_meta_swarm::operators::{Lift, LinearLift, LinearPull, MetaOperator, Pull, UniformMetaOp};
use efe_meta_swarm::participants::MockParticipant;
use efe_meta_swarm::safety::{BoxSafety, SafetySet};

const M_DIM: usize = 4;
const P_DIM: usize = 8;

/// Two-participant meta-swarm over `MockParticipant`s with a given operator kappa,
/// strictness, and iteration budget. Mirrors the `builder.rs` test harness.
fn build(kappa: f32, strict: bool, max_iters: usize, tol: f64) -> MetaSwarm {
    MetaSwarm::builder()
        .id("m1c-gate")
        .meta_space_dim(M_DIM)
        .add_participant(
            MockParticipant::with_contraction("a", P_DIM, kappa, 1)
                .expect("valid kappa")
                .boxed(),
        )
        .add_participant(
            MockParticipant::with_contraction("b", P_DIM, kappa, 2)
                .expect("valid kappa")
                .boxed(),
        )
        .lift_for("a", LinearLift::identity(P_DIM, M_DIM).unwrap().boxed())
        .lift_for("b", LinearLift::identity(P_DIM, M_DIM).unwrap().boxed())
        .pull_for("a", LinearPull::identity(M_DIM, P_DIM).unwrap().boxed())
        .pull_for("b", LinearPull::identity(M_DIM, P_DIM).unwrap().boxed())
        .meta_operator(UniformMetaOp::new(M_DIM).boxed())
        .meta_safety(BoxSafety::symmetric(M_DIM, 5.0).boxed())
        .alpha_default(0.5)
        .coupling_gamma(0.2)
        .tolerance(tol)
        .max_iters(max_iters)
        .strict(strict)
        .build()
        .expect("build meta-swarm")
}

/// STRICT mode: a `MockParticipant` returns a `Stub` (unverified) certificate, so the
/// H1 pre-check must fail-closed — no iteration happens, no consensus is produced.
#[test]
fn strict_mode_rejects_unverified_stub() {
    let ms = build(0.5, /* strict */ true, 500, 1e-4);
    match ms.iterate() {
        Err(MetaSwarmError::HypothesisCheckFailed { hypothesis: "H1" }) => {}
        other => panic!("strict mode must reject a Stub-only participant with H1 failure, got {other:?}"),
    }
}

/// LENIENT mode (Phase-1 default): the same `Stub` participants are accepted, so the
/// iteration runs. (Backward-compat: existing mock demos keep working.)
#[test]
fn lenient_mode_accepts_stub_and_runs() {
    let ms = build(0.5, /* strict */ false, 500, 1e-4);
    let r = ms.iterate().expect("lenient mode must accept Stub participants and iterate");
    // A 0.5-contraction under the coupled iteration converges.
    assert_eq!(r.stop_reason, StopReason::Converged, "0.5-contraction should converge");
    assert!(r.converged());
}

/// `require_converged` is the fail-closed accessor: a run that hit `max_iters` without
/// converging exposes its (non-converged) fields but MUST refuse to hand back a
/// consensus. Here `max_iters = 1` with an unreachable tolerance guarantees non-conv.
#[test]
fn require_converged_fails_closed_on_non_convergence() {
    let ms = build(0.5, /* strict */ false, 1, 1e-12);
    let r = ms.iterate().expect("lenient run returns a result even when it does not converge");
    assert_eq!(r.stop_reason, StopReason::MaxItersHit, "1 iter + tiny tol ⇒ MaxItersHit");
    assert!(!r.converged());
    match r.require_converged() {
        Err(MetaSwarmError::ConvergenceFailed { iters: 1, .. }) => {}
        other => panic!("require_converged must fail-closed on non-convergence, got {other:?}"),
    }
}

/// A sanity check that `bounding_box` (the canary's sampling domain) reflects a box
/// safety set exactly — the seam the real `LocalParticipant` canary relies on.
#[test]
fn box_safety_exposes_its_bounding_box() {
    let c = BoxSafety::new(vec![0.02; 4], vec![0.4; 4]).unwrap();
    let (lo, hi) = c.bounding_box().expect("BoxSafety has a bounding box");
    assert_eq!(lo, vec![0.02; 4]);
    assert_eq!(hi, vec![0.4; 4]);
}
