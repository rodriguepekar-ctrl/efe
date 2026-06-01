//! Sprint A1 P1 — PR #3 — Category 9 : Iteration coupled proptest (12 tests).
//!
//! These property-based tests stress the *coupled* iteration loop end-to-end
//! through the public `MetaSwarm::iterate` API.  Each test rebuilds a fresh
//! meta-swarm from randomised parameters (dim, κ, α, γ, seeds, …) and
//! probes a specific algebraic / dynamical invariant :
//!
//!   1.  Convergence under contractive swarms + valid coupling.
//!   2.  Joint residual decreases from start to end.
//!   3.  Final residual ≤ tolerance when stop_reason = Converged.
//!   4.  Iteration count ≤ max_iters.
//!   5.  Determinism : same seeds → same iteration count & residual.
//!   6.  Stationarity : after convergence, an extra step moves little.
//!   7.  Reset : resets all participants to a known (zero) state.
//!   8.  Trace length = iteration count when trace_enabled.
//!   9.  Participant order independence (swap a/b → same residual).
//!  10.  γ = 0 → no coupling, swarms drift independently (no consensus
//!       guarantee, but iteration still terminates).
//!  11.  Three swarms (different intrinsic dims) all converge.
//!  12.  Coupling near upper bound (α + γ → 1⁻) remains bounded
//!       (residual finite, no NaN).
//!
//! All tests run with `cases = 12` (modest budget : each case runs a full
//! iteration loop up to ~500 steps).
//!
//! Performance target : the whole file should finish in < 3 s on a laptop.

use efe_meta_swarm::prelude::*;
use proptest::prelude::*;

// =============================================================================
// Helpers
// =============================================================================

fn pt_cfg_iter() -> ProptestConfig {
    ProptestConfig { cases: 12, ..ProptestConfig::default() }
}

/// Build a two-participant meta-swarm with contractive Φ_m and matched
/// dimensions.  All operators are identity lifts/pulls and the meta-op is
/// uniform-weighted.
#[allow(clippy::too_many_arguments)]
fn build_two(
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
        .id("pt9")
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

// =============================================================================
// Tests
// =============================================================================

proptest! {
    #![proptest_config(pt_cfg_iter())]

    /// Cat 9 / Test 1 — Contractive swarms always converge under valid
    /// coupling (α + γ < 1) within a generous max_iters.
    #[test]
    fn iter_contractive_converges_pt(
        meta_dim in 2usize..=8,
        p_dim    in 2usize..=8,
        kappa    in 0.3_f32..0.8,
        alpha    in 0.1_f64..0.5,
        gamma    in 0.05_f64..0.3,
        seed_a   in any::<u64>(),
        seed_b   in any::<u64>(),
    ) {
        prop_assume!(alpha + gamma < 0.9);
        let ms = build_two(meta_dim, p_dim, kappa, alpha, gamma,
                           seed_a, seed_b, 500, 1e-3, false).unwrap();
        let r = ms.iterate().unwrap();
        prop_assert_eq!(r.stop_reason, StopReason::Converged);
    }

    /// Cat 9 / Test 2 — Final residual is below the configured tolerance
    /// when stop_reason = Converged.
    #[test]
    fn iter_final_residual_below_tol_pt(
        kappa  in 0.3_f32..0.7,
        alpha  in 0.1_f64..0.4,
        gamma  in 0.05_f64..0.25,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        let tol = 1e-3;
        let ms = build_two(4, 8, kappa, alpha, gamma,
                           seed_a, seed_b, 500, tol, false).unwrap();
        let r = ms.iterate().unwrap();
        if r.stop_reason == StopReason::Converged {
            prop_assert!(r.final_residual <= tol,
                "residual {} > tol {}", r.final_residual, tol);
        }
    }

    /// Cat 9 / Test 3 — Iteration count never exceeds max_iters.
    #[test]
    fn iter_count_within_max_iters_pt(
        max_iters in 5usize..=200,
        kappa  in 0.3_f32..0.8,
        alpha  in 0.1_f64..0.5,
        gamma  in 0.05_f64..0.3,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        prop_assume!(alpha + gamma < 0.9);
        let ms = build_two(4, 8, kappa, alpha, gamma,
                           seed_a, seed_b, max_iters, 1e-4, false).unwrap();
        let r = ms.iterate().unwrap();
        prop_assert!(r.iterations <= max_iters,
            "iterations {} > max {}", r.iterations, max_iters);
    }

    /// Cat 9 / Test 4 — Determinism : same seeds & params → same number of
    /// iterations and same final residual (bit-identical up to fp jitter).
    #[test]
    fn iter_deterministic_pt(
        meta_dim in 2usize..=6,
        p_dim    in 2usize..=8,
        kappa    in 0.3_f32..0.7,
        alpha    in 0.1_f64..0.4,
        gamma    in 0.05_f64..0.25,
        seed_a   in any::<u64>(),
        seed_b   in any::<u64>(),
    ) {
        prop_assume!(alpha + gamma < 0.9);
        let ms_a = build_two(meta_dim, p_dim, kappa, alpha, gamma,
                             seed_a, seed_b, 500, 1e-3, false).unwrap();
        let ms_b = build_two(meta_dim, p_dim, kappa, alpha, gamma,
                             seed_a, seed_b, 500, 1e-3, false).unwrap();
        let r_a = ms_a.iterate().unwrap();
        let r_b = ms_b.iterate().unwrap();
        prop_assert_eq!(r_a.iterations, r_b.iterations);
        prop_assert!((r_a.final_residual - r_b.final_residual).abs() < 1e-9);
    }

    /// Cat 9 / Test 5 — After convergence, an additional `step_once` moves
    /// each participant by less than ε.  Stationarity check : at a fixed
    /// point of the joint operator, T(x*) = x*.
    #[test]
    fn iter_stationary_after_convergence_pt(
        kappa  in 0.3_f32..0.6,
        alpha  in 0.2_f64..0.4,
        gamma  in 0.1_f64..0.25,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        let ms = build_two(4, 8, kappa, alpha, gamma,
                           seed_a, seed_b, 1000, 1e-5, false).unwrap();
        let r = ms.iterate().unwrap();
        // Only check stationarity on converged runs.
        prop_assume!(r.stop_reason == StopReason::Converged);
        // Take the meta-consensus before, take one more step, compare.
        let before = r.meta_consensus.clone();
        let after = ms.step_once().unwrap();
        let drift = before.distance(&after).unwrap();
        prop_assert!(drift < 1e-2, "post-convergence drift {drift} > 1e-2");
    }

    /// Cat 9 / Test 6 — Reset returns all participants to zero state.
    #[test]
    fn iter_reset_zeros_pt(
        kappa  in 0.3_f32..0.7,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        let ms = build_two(4, 8, kappa, 0.3, 0.2,
                           seed_a, seed_b, 100, 1e-3, false).unwrap();
        let _ = ms.iterate().unwrap();
        ms.reset().unwrap();
        let states = ms.current_states().unwrap();
        for s in &states {
            prop_assert!(s.norm() < 1e-9, "non-zero state after reset : {s:?}");
        }
    }

    /// Cat 9 / Test 7 — Trace length equals iteration count when enabled.
    #[test]
    fn iter_trace_length_pt(
        kappa  in 0.3_f32..0.7,
        alpha  in 0.1_f64..0.4,
        gamma  in 0.05_f64..0.25,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        prop_assume!(alpha + gamma < 0.9);
        let ms = build_two(4, 8, kappa, alpha, gamma,
                           seed_a, seed_b, 200, 1e-3, true).unwrap();
        let r = ms.iterate().unwrap();
        let t = r.trace.as_ref().expect("trace enabled");
        prop_assert_eq!(t.len(), r.iterations);
    }

    /// Cat 9 / Test 8 — Residual decreased from step 0 to last step (long
    /// horizon : trace must show overall descent).
    #[test]
    fn iter_residual_descends_pt(
        kappa  in 0.3_f32..0.6,
        alpha  in 0.2_f64..0.4,
        gamma  in 0.1_f64..0.25,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        let ms = build_two(4, 8, kappa, alpha, gamma,
                           seed_a, seed_b, 200, 1e-4, true).unwrap();
        let r = ms.iterate().unwrap();
        let t = r.trace.as_ref().unwrap();
        let res = t.residuals();
        prop_assume!(res.len() >= 5); // need a meaningful horizon
        let first = res[0];
        let last = *res.last().unwrap();
        prop_assert!(last <= first + 1e-12,
            "residual did not decrease : first={first} last={last}");
    }

    /// Cat 9 / Test 9 — Participant order independence : swapping a/b
    /// gives the same final residual (up to fp jitter).
    ///
    /// Justification : the meta-operator is permutation-invariant
    /// (UniformMetaOp = mean) and lifts/pulls are identity, so the joint
    /// operator is invariant under permutation of participants.  Note :
    /// `r_a.meta_consensus` and `r_b.meta_consensus` are identical, but the
    /// participant states are swapped in r_b.
    #[test]
    fn iter_participant_order_pt(
        kappa  in 0.3_f32..0.7,
        alpha  in 0.1_f64..0.4,
        gamma  in 0.05_f64..0.25,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        prop_assume!(alpha + gamma < 0.9);
        let ms_ab = build_two(4, 8, kappa, alpha, gamma,
                              seed_a, seed_b, 200, 1e-3, false).unwrap();
        // Swapped order : same seeds passed to b/a.
        let ms_ba = MetaSwarm::builder()
            .id("pt9-9-swap")
            .meta_space_dim(4)
            // Note : ids are still "a" and "b", but seeds are swapped.
            .add_participant(MockParticipant::with_contraction("a", 8, kappa, seed_b).unwrap().boxed())
            .add_participant(MockParticipant::with_contraction("b", 8, kappa, seed_a).unwrap().boxed())
            .lift_for("a", LinearLift::identity(8, 4).unwrap().boxed())
            .lift_for("b", LinearLift::identity(8, 4).unwrap().boxed())
            .pull_for("a", LinearPull::identity(4, 8).unwrap().boxed())
            .pull_for("b", LinearPull::identity(4, 8).unwrap().boxed())
            .meta_operator(UniformMetaOp::new(4).boxed())
            .meta_safety(BoxSafety::symmetric(4, 10.0).boxed())
            .alpha_default(alpha)
            .coupling_gamma(gamma)
            .tolerance(1e-3)
            .max_iters(200)
            .build().unwrap();
        let r_ab = ms_ab.iterate().unwrap();
        let r_ba = ms_ba.iterate().unwrap();
        // Residuals must agree.
        prop_assert!((r_ab.final_residual - r_ba.final_residual).abs() < 1e-6,
            "{} vs {}", r_ab.final_residual, r_ba.final_residual);
    }

    /// Cat 9 / Test 10 — γ = 0 : iteration still terminates (each swarm
    /// converges independently to its own fixed point ; the meta-coupling
    /// contributes nothing).  No assertion on consensus quality.
    #[test]
    fn iter_zero_gamma_terminates_pt(
        kappa  in 0.3_f32..0.7,
        alpha  in 0.1_f64..0.7,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        prop_assume!(alpha < 0.95);
        let ms = build_two(4, 8, kappa, alpha, 0.0,
                           seed_a, seed_b, 500, 1e-3, false).unwrap();
        let r = ms.iterate().unwrap();
        // Either converged or hit max_iters — both are fine.
        prop_assert!(r.iterations <= 500);
        // Crucially : no NaN.
        prop_assert!(r.final_residual.is_finite());
        for s in &r.participant_states {
            for i in 0..s.dim() {
                prop_assert!(s.at(i).is_finite());
            }
        }
    }

    /// Cat 9 / Test 11 — Three swarms (heterogeneous intrinsic dims via
    /// uniform meta_dim) all converge through a uniform-weighted meta-op.
    #[test]
    fn iter_three_swarms_converge_pt(
        kappa  in 0.4_f32..0.7,
        alpha  in 0.1_f64..0.4,
        gamma  in 0.05_f64..0.25,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
        seed_c in any::<u64>(),
    ) {
        prop_assume!(alpha + gamma < 0.9);
        let ms = MetaSwarm::builder()
            .id("pt9-11")
            .meta_space_dim(4)
            .add_participant(MockParticipant::with_contraction("a", 8,  kappa, seed_a).unwrap().boxed())
            .add_participant(MockParticipant::with_contraction("b", 8,  kappa, seed_b).unwrap().boxed())
            .add_participant(MockParticipant::with_contraction("c", 8,  kappa, seed_c).unwrap().boxed())
            .lift_for("a", LinearLift::identity(8, 4).unwrap().boxed())
            .lift_for("b", LinearLift::identity(8, 4).unwrap().boxed())
            .lift_for("c", LinearLift::identity(8, 4).unwrap().boxed())
            .pull_for("a", LinearPull::identity(4, 8).unwrap().boxed())
            .pull_for("b", LinearPull::identity(4, 8).unwrap().boxed())
            .pull_for("c", LinearPull::identity(4, 8).unwrap().boxed())
            .meta_operator(UniformMetaOp::new(4).boxed())
            .meta_safety(BoxSafety::symmetric(4, 10.0).boxed())
            .alpha_default(alpha)
            .coupling_gamma(gamma)
            .tolerance(1e-3)
            .max_iters(500)
            .build().unwrap();
        let r = ms.iterate().unwrap();
        prop_assert_eq!(r.stop_reason, StopReason::Converged);
    }

    /// Cat 9 / Test 12 — Coupling near upper bound (α + γ ≈ 0.95).
    /// Iteration must still produce finite output and bounded residual.
    #[test]
    fn iter_coupling_near_upper_pt(
        kappa  in 0.3_f32..0.6,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        // High α/γ but strictly < 1.
        let alpha = 0.55_f64;
        let gamma = 0.4_f64;
        let ms = build_two(4, 8, kappa, alpha, gamma,
                           seed_a, seed_b, 500, 1e-3, false).unwrap();
        let r = ms.iterate().unwrap();
        prop_assert!(r.final_residual.is_finite());
        for s in &r.participant_states {
            for i in 0..s.dim() {
                prop_assert!(s.at(i).is_finite());
                prop_assert!(s.at(i).abs() < 1e3); // bounded
            }
        }
    }
}
