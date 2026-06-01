//! Sprint A1 P1 — PR #3 — Category 10 : Convergence edge-case proptest (12).
//!
//! Edge-case property tests for the convergence behaviour of the coupled
//! iteration.  Each test exercises a *corner* of the parameter space that
//! a unit test alone could miss :
//!
//!   1.  max_iters = 1 → exactly 1 step executed.
//!   2.  Loose tolerance (1e-1) → converges in very few iterations.
//!   3.  Tight tolerance + low max_iters → MaxItersHit.
//!   4.  Single participant → still converges (trivial meta-aggregation).
//!   5.  Identity Φ with γ > 0 → fixed point reached in O(1) iterations.
//!   6.  α = 0 (no relaxation) → still terminates (pure coupling).
//!   7.  γ very small → terminates with bounded residual.
//!   8.  Re-running iterate() twice : second run uses final state of first.
//!   9.  Residuals always non-negative.
//!  10.  Final meta-consensus has the correct dimension.
//!  11.  Final participant states have the correct dimension.
//!  12.  Empty trace when trace_enabled = false.
//!
//! These tests deliberately combine `proptest` shrinkage with extremal
//! parameter values to surface boundary issues (off-by-one, NaN, dim mismatch).

use efe_meta_swarm::prelude::*;
use proptest::prelude::*;

fn pt_cfg_edge() -> ProptestConfig {
    ProptestConfig { cases: 12, ..ProptestConfig::default() }
}

/// Two-participant builder used by all tests in this file.  Caller picks
/// max_iters / tolerance / trace ; everything else is a standard contractive
/// setup.
#[allow(clippy::too_many_arguments)]
fn make(
    meta_dim: usize,
    p_dim: usize,
    kappa: f32,
    alpha: f64,
    gamma: f64,
    seed_a: u64,
    seed_b: u64,
    max_iters: usize,
    tolerance: f64,
    trace: bool,
) -> Result<MetaSwarm> {
    MetaSwarm::builder()
        .id("pt10")
        .meta_space_dim(meta_dim)
        .add_participant(MockParticipant::with_contraction("a", p_dim, kappa, seed_a)?.boxed())
        .add_participant(MockParticipant::with_contraction("b", p_dim, kappa, seed_b)?.boxed())
        .lift_for("a", LinearLift::identity(p_dim, meta_dim)?.boxed())
        .lift_for("b", LinearLift::identity(p_dim, meta_dim)?.boxed())
        .pull_for("a", LinearPull::identity(meta_dim, p_dim)?.boxed())
        .pull_for("b", LinearPull::identity(meta_dim, p_dim)?.boxed())
        .meta_operator(UniformMetaOp::new(meta_dim).boxed())
        .meta_safety(BoxSafety::symmetric(meta_dim, 10.0).boxed())
        .alpha_default(alpha)
        .coupling_gamma(gamma)
        .tolerance(tolerance)
        .max_iters(max_iters)
        .trace(trace)
        .build()
}

/// Single-participant variant — needed for Test 4.
#[allow(clippy::too_many_arguments)]
fn make_single(
    meta_dim: usize,
    p_dim: usize,
    kappa: f32,
    alpha: f64,
    gamma: f64,
    seed: u64,
    max_iters: usize,
    tolerance: f64,
) -> Result<MetaSwarm> {
    MetaSwarm::builder()
        .id("pt10-single")
        .meta_space_dim(meta_dim)
        .add_participant(MockParticipant::with_contraction("a", p_dim, kappa, seed)?.boxed())
        .lift_for("a", LinearLift::identity(p_dim, meta_dim)?.boxed())
        .pull_for("a", LinearPull::identity(meta_dim, p_dim)?.boxed())
        .meta_operator(UniformMetaOp::new(meta_dim).boxed())
        .meta_safety(BoxSafety::symmetric(meta_dim, 10.0).boxed())
        .alpha_default(alpha)
        .coupling_gamma(gamma)
        .tolerance(tolerance)
        .max_iters(max_iters)
        .build()
}

proptest! {
    #![proptest_config(pt_cfg_edge())]

    /// Cat 10 / Test 1 — max_iters = 1 ⇒ exactly 1 step executed.
    #[test]
    fn edge_max_iters_one_pt(
        kappa  in 0.3_f32..0.7,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        let ms = make(4, 8, kappa, 0.3, 0.2, seed_a, seed_b, 1, 1e-9, false).unwrap();
        let r = ms.iterate().unwrap();
        prop_assert_eq!(r.iterations, 1);
        // Either Converged (if first step crossed tol) or MaxItersHit.
        prop_assert!(r.stop_reason == StopReason::Converged
                  || r.stop_reason == StopReason::MaxItersHit);
    }

    /// Cat 10 / Test 2 — Very loose tolerance ⇒ converges in few iterations.
    #[test]
    fn edge_loose_tol_pt(
        kappa  in 0.3_f32..0.6,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        let ms = make(4, 8, kappa, 0.3, 0.2, seed_a, seed_b, 500, 1e-1, false).unwrap();
        let r = ms.iterate().unwrap();
        prop_assert_eq!(r.stop_reason, StopReason::Converged);
        prop_assert!(r.iterations < 100,
            "loose tol but {} iters", r.iterations);
    }

    /// Cat 10 / Test 3 — Tight tolerance + tiny max_iters ⇒ MaxItersHit.
    #[test]
    fn edge_max_iters_hit_pt(
        kappa  in 0.3_f32..0.7,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        let ms = make(4, 8, kappa, 0.3, 0.2, seed_a, seed_b, 3, 1e-15, false).unwrap();
        let r = ms.iterate().unwrap();
        // With max_iters=3 and tolerance=1e-15 we almost certainly hit
        // MaxItersHit ; allow rare Converged when initial residual already
        // below 1e-15 (extremely unlikely but possible).
        prop_assert!(r.iterations <= 3);
        prop_assert!(r.stop_reason == StopReason::MaxItersHit
                  || r.stop_reason == StopReason::Converged);
    }

    /// Cat 10 / Test 4 — Single participant still converges to its
    /// internal fixed point.
    #[test]
    fn edge_single_participant_converges_pt(
        kappa in 0.3_f32..0.7,
        seed  in any::<u64>(),
    ) {
        let ms = make_single(4, 8, kappa, 0.3, 0.2, seed, 500, 1e-3).unwrap();
        let r = ms.iterate().unwrap();
        prop_assert_eq!(r.stop_reason, StopReason::Converged);
    }

    /// Cat 10 / Test 5 — Identity Φ + γ > 0 ⇒ pulls drive convergence.
    /// Because identity participants do not contract on their own, the
    /// meta-coupling must be the source of fixed-point progress.
    #[test]
    fn edge_identity_phi_with_coupling_pt(
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        let ms = MetaSwarm::builder()
            .id("pt10-5")
            .meta_space_dim(4)
            .add_participant(MockParticipant::with_identity("a", 8, seed_a).boxed())
            .add_participant(MockParticipant::with_identity("b", 8, seed_b).boxed())
            .lift_for("a", LinearLift::identity(8, 4).unwrap().boxed())
            .lift_for("b", LinearLift::identity(8, 4).unwrap().boxed())
            .pull_for("a", LinearPull::identity(4, 8).unwrap().boxed())
            .pull_for("b", LinearPull::identity(4, 8).unwrap().boxed())
            .meta_operator(UniformMetaOp::new(4).boxed())
            .meta_safety(BoxSafety::symmetric(4, 10.0).boxed())
            .alpha_default(0.3)
            .coupling_gamma(0.2)
            .tolerance(1e-3)
            .max_iters(500)
            .build().unwrap();
        let r = ms.iterate().unwrap();
        prop_assert_eq!(r.stop_reason, StopReason::Converged);
        // Final states should agree (consensus).
        let s = &r.participant_states;
        let dist = s[0].distance(&s[1]).unwrap();
        prop_assert!(dist < 0.1, "consensus not reached, dist = {dist}");
    }

    /// Cat 10 / Test 6 — α = 0 (no relaxation) + γ > 0 : iteration still
    /// terminates.  No assertion on residual quality (this is a corner
    /// case where the math drops one term).
    #[test]
    fn edge_zero_alpha_pt(
        kappa  in 0.3_f32..0.7,
        gamma  in 0.05_f64..0.5,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        let ms = make(4, 8, kappa, 0.0, gamma, seed_a, seed_b, 500, 1e-3, false).unwrap();
        let r = ms.iterate().unwrap();
        prop_assert!(r.iterations <= 500);
        prop_assert!(r.final_residual.is_finite());
    }

    /// Cat 10 / Test 7 — Tiny γ ⇒ slow but bounded convergence.
    #[test]
    fn edge_tiny_gamma_pt(
        kappa  in 0.3_f32..0.7,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        let ms = make(4, 8, kappa, 0.3, 1e-4, seed_a, seed_b, 500, 1e-2, false).unwrap();
        let r = ms.iterate().unwrap();
        prop_assert!(r.iterations <= 500);
        prop_assert!(r.final_residual.is_finite());
    }

    /// Cat 10 / Test 8 — Re-running iterate() : the second run starts from
    /// the converged state, so it should terminate in very few iterations.
    #[test]
    fn edge_rerun_after_convergence_pt(
        kappa  in 0.3_f32..0.6,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        let ms = make(4, 8, kappa, 0.3, 0.2, seed_a, seed_b, 500, 1e-3, false).unwrap();
        let r1 = ms.iterate().unwrap();
        prop_assume!(r1.stop_reason == StopReason::Converged);
        let r2 = ms.iterate().unwrap();
        // Second run starts from a near-fixed point.
        prop_assert!(r2.iterations <= r1.iterations);
        prop_assert!(r2.final_residual <= 1e-3 + 1e-9);
    }

    /// Cat 10 / Test 9 — Residuals are always non-negative (sanity).
    #[test]
    fn edge_residuals_nonneg_pt(
        kappa  in 0.3_f32..0.7,
        alpha  in 0.1_f64..0.5,
        gamma  in 0.05_f64..0.3,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        prop_assume!(alpha + gamma < 0.9);
        let ms = make(4, 8, kappa, alpha, gamma, seed_a, seed_b, 200, 1e-3, true).unwrap();
        let r = ms.iterate().unwrap();
        prop_assert!(r.final_residual >= 0.0);
        for s in r.trace.as_ref().unwrap().residuals() {
            prop_assert!(s >= 0.0, "negative residual : {s}");
        }
    }

    /// Cat 10 / Test 10 — Final meta-consensus has the right dimension.
    #[test]
    fn edge_meta_consensus_dim_pt(
        meta_dim in 2usize..=16,
        kappa    in 0.3_f32..0.7,
        seed_a   in any::<u64>(),
        seed_b   in any::<u64>(),
    ) {
        let ms = make(meta_dim, 8, kappa, 0.3, 0.2, seed_a, seed_b,
                      100, 1e-3, false).unwrap();
        let r = ms.iterate().unwrap();
        prop_assert_eq!(r.meta_consensus.dim(), meta_dim);
    }

    /// Cat 10 / Test 11 — Final participant states all have the right
    /// dimension.
    #[test]
    fn edge_participant_states_dim_pt(
        p_dim   in 2usize..=16,
        kappa   in 0.3_f32..0.7,
        seed_a  in any::<u64>(),
        seed_b  in any::<u64>(),
    ) {
        let ms = make(4, p_dim, kappa, 0.3, 0.2, seed_a, seed_b,
                      100, 1e-3, false).unwrap();
        let r = ms.iterate().unwrap();
        prop_assert_eq!(r.participant_states.len(), 2);
        for s in &r.participant_states {
            prop_assert_eq!(s.dim(), p_dim);
        }
    }

    /// Cat 10 / Test 12 — trace_enabled = false ⇒ no trace recorded.
    #[test]
    fn edge_no_trace_when_disabled_pt(
        kappa  in 0.3_f32..0.7,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        let ms = make(4, 8, kappa, 0.3, 0.2, seed_a, seed_b, 100, 1e-3, false).unwrap();
        let r = ms.iterate().unwrap();
        prop_assert!(r.trace.is_none());
    }
}
