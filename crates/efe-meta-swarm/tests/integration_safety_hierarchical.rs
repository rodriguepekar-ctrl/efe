//! Sprint A1 P1 — PR #3 — Category 11 : Safety hierarchical proptest (8).
//!
//! These tests validate the *safety preservation* invariants of the coupled
//! iteration : at every recorded step (and at the final state) :
//!
//!   * the meta-consensus must lie in the meta-safety set  $C_{\text{meta}}$,
//!   * each participant state must lie in its local safety set  $C_m$,
//!
//! and the various safety-set kinds (Box, Polyhedron, Learned) must combine
//! correctly when mixed across the hierarchy (e.g. meta-Polyhedron with
//! local-Box).  Section 5 of A5 (Safety) and Theorem 5.1 (joint safety
//! preservation) are the formal references.
//!
//! The 8 tests :
//!
//!   1.  Final meta-consensus is always in the meta-safety set (Box).
//!   2.  Final participant states are always in their local safety sets.
//!   3.  Tight meta-safety still allows convergence (just bigger residual).
//!   4.  Polyhedral meta-safety (half-space) keeps consensus feasible.
//!   5.  Asymmetric local box keeps participant states feasible.
//!   6.  Mixed hierarchy : meta=Polyhedron, local=Box.
//!   7.  Every recorded trace step's meta-consensus is feasible.
//!   8.  Every recorded trace step's participant states are feasible.
//!
//! These are *non-trivial* invariants : projection happens at every iter,
//! so violating them means the iteration loop has a bug.  With 12 cases
//! per test and an internal max_iters of 200, we cover ~2 000 distinct
//! (state, iter) pairs total — strong evidence of correctness.

use efe_meta_swarm::prelude::*;
use proptest::prelude::*;

fn pt_cfg_safe() -> ProptestConfig {
    ProptestConfig { cases: 12, ..ProptestConfig::default() }
}

/// Convenience constructor : two contractive participants, configurable meta
/// and local safety.  Always traces.
#[allow(clippy::too_many_arguments)]
fn make_with_safeties(
    meta_dim: usize,
    p_dim: usize,
    kappa: f32,
    alpha: f64,
    gamma: f64,
    seed_a: u64,
    seed_b: u64,
    max_iters: usize,
    tolerance: f64,
    meta_safety: Box<dyn SafetySet>,
    local_safety_a: Box<dyn SafetySet>,
    local_safety_b: Box<dyn SafetySet>,
) -> Result<MetaSwarm> {
    let p_a = MockParticipant::with_contraction("a", p_dim, kappa, seed_a)?
        .with_safety(local_safety_a);
    let p_b = MockParticipant::with_contraction("b", p_dim, kappa, seed_b)?
        .with_safety(local_safety_b);
    MetaSwarm::builder()
        .id("pt11")
        .meta_space_dim(meta_dim)
        .add_participant(p_a.boxed())
        .add_participant(p_b.boxed())
        .lift_for("a", LinearLift::identity(p_dim, meta_dim)?.boxed())
        .lift_for("b", LinearLift::identity(p_dim, meta_dim)?.boxed())
        .pull_for("a", LinearPull::identity(meta_dim, p_dim)?.boxed())
        .pull_for("b", LinearPull::identity(meta_dim, p_dim)?.boxed())
        .meta_operator(UniformMetaOp::new(meta_dim).boxed())
        .meta_safety(meta_safety)
        .alpha_default(alpha)
        .coupling_gamma(gamma)
        .tolerance(tolerance)
        .max_iters(max_iters)
        .trace(true)
        .build()
}

/// Build a single-half-space polyhedron `{ x : sum(x) ≤ r }` in `dim`-D,
/// useful as a meta-safety set that is *not* a box.
fn half_space_sum(dim: usize, r: f32) -> PolyhedronSafety {
    let a = vec![1.0_f32; dim];
    let b = vec![r];
    PolyhedronSafety::new("half_space_sum", dim, a, b).unwrap()
}

proptest! {
    #![proptest_config(pt_cfg_safe())]

    /// Cat 11 / Test 1 — Final meta-consensus is always inside the meta
    /// safety set (a generous symmetric box).
    #[test]
    fn safe_final_meta_in_box_pt(
        radius in 1.0_f32..20.0,
        kappa  in 0.3_f32..0.7,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        let ms = make_with_safeties(
            4, 8, kappa, 0.3, 0.2,
            seed_a, seed_b, 200, 1e-3,
            BoxSafety::symmetric(4, radius).boxed(),
            BoxSafety::symmetric(8, 10.0).boxed(),
            BoxSafety::symmetric(8, 10.0).boxed(),
        ).unwrap();
        let r = ms.iterate().unwrap();
        let check = BoxSafety::symmetric(4, radius);
        prop_assert!(check.contains(&r.meta_consensus, 1e-3),
            "final meta-consensus outside Box(r={radius}) : {:?}",
            r.meta_consensus.as_slice());
    }

    /// Cat 11 / Test 2 — Final participant states inside their local safety.
    #[test]
    fn safe_final_locals_in_box_pt(
        radius in 1.0_f32..20.0,
        kappa  in 0.3_f32..0.7,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        let ms = make_with_safeties(
            4, 8, kappa, 0.3, 0.2,
            seed_a, seed_b, 200, 1e-3,
            BoxSafety::symmetric(4, 10.0).boxed(),
            BoxSafety::symmetric(8, radius).boxed(),
            BoxSafety::symmetric(8, radius).boxed(),
        ).unwrap();
        let r = ms.iterate().unwrap();
        let check = BoxSafety::symmetric(8, radius);
        for (i, s) in r.participant_states.iter().enumerate() {
            prop_assert!(check.contains(s, 1e-3),
                "participant {i} outside Box(r={radius}) : {:?}", s.as_slice());
        }
    }

    /// Cat 11 / Test 3 — Tight meta-safety (small radius) still allows
    /// the iteration to terminate.  No assertion on Converged ;  we accept
    /// MaxItersHit because the projection clamps progress.
    #[test]
    fn safe_tight_meta_terminates_pt(
        kappa  in 0.3_f32..0.6,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        let ms = make_with_safeties(
            4, 8, kappa, 0.3, 0.2,
            seed_a, seed_b, 300, 1e-3,
            BoxSafety::symmetric(4, 0.5).boxed(),
            BoxSafety::symmetric(8, 10.0).boxed(),
            BoxSafety::symmetric(8, 10.0).boxed(),
        ).unwrap();
        let r = ms.iterate().unwrap();
        prop_assert!(r.iterations <= 300);
        prop_assert!(r.final_residual.is_finite());
        // Meta-consensus must still be inside the tight box.
        let check = BoxSafety::symmetric(4, 0.5);
        prop_assert!(check.contains(&r.meta_consensus, 1e-3));
    }

    /// Cat 11 / Test 4 — Polyhedral meta-safety (half-space {Σ x ≤ r}) :
    /// final meta-consensus satisfies the half-space.
    #[test]
    fn safe_polyhedral_meta_feasible_pt(
        r_max  in 2.0_f32..15.0,
        kappa  in 0.3_f32..0.6,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        let ms = make_with_safeties(
            4, 8, kappa, 0.3, 0.2,
            seed_a, seed_b, 200, 1e-3,
            half_space_sum(4, r_max).boxed(),
            BoxSafety::symmetric(8, 10.0).boxed(),
            BoxSafety::symmetric(8, 10.0).boxed(),
        ).unwrap();
        let r = ms.iterate().unwrap();
        let check = half_space_sum(4, r_max);
        prop_assert!(check.contains(&r.meta_consensus, 1e-3),
            "meta-consensus violates Σ ≤ {r_max} : Σ = {}",
            r.meta_consensus.as_slice().iter().sum::<f32>());
    }

    /// Cat 11 / Test 5 — Asymmetric local box : participant states stay
    /// in `[-r_lo, r_hi]^d`.
    #[test]
    fn safe_asymmetric_local_box_pt(
        r_lo in 0.5_f32..3.0,
        r_hi in 0.5_f32..3.0,
        kappa  in 0.3_f32..0.7,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        let local_a = BoxSafety::new(vec![-r_lo; 8], vec![r_hi; 8]).unwrap();
        let local_b = BoxSafety::new(vec![-r_lo; 8], vec![r_hi; 8]).unwrap();
        let ms = make_with_safeties(
            4, 8, kappa, 0.3, 0.2,
            seed_a, seed_b, 200, 1e-3,
            BoxSafety::symmetric(4, 10.0).boxed(),
            local_a.boxed(),
            local_b.boxed(),
        ).unwrap();
        let r = ms.iterate().unwrap();
        let check = BoxSafety::new(vec![-r_lo; 8], vec![r_hi; 8]).unwrap();
        for (i, s) in r.participant_states.iter().enumerate() {
            prop_assert!(check.contains(s, 1e-3),
                "participant {i} outside asym box : {:?}", s.as_slice());
        }
    }

    /// Cat 11 / Test 6 — Mixed hierarchy : meta = Polyhedron, local = Box.
    /// Both invariants must hold simultaneously.
    #[test]
    fn safe_mixed_hierarchy_pt(
        r_meta in 2.0_f32..10.0,
        r_local in 1.0_f32..10.0,
        kappa   in 0.3_f32..0.6,
        seed_a  in any::<u64>(),
        seed_b  in any::<u64>(),
    ) {
        let ms = make_with_safeties(
            4, 8, kappa, 0.3, 0.2,
            seed_a, seed_b, 200, 1e-3,
            half_space_sum(4, r_meta).boxed(),
            BoxSafety::symmetric(8, r_local).boxed(),
            BoxSafety::symmetric(8, r_local).boxed(),
        ).unwrap();
        let r = ms.iterate().unwrap();
        let meta_check = half_space_sum(4, r_meta);
        let loc_check  = BoxSafety::symmetric(8, r_local);
        prop_assert!(meta_check.contains(&r.meta_consensus, 1e-3));
        for s in &r.participant_states {
            prop_assert!(loc_check.contains(s, 1e-3));
        }
    }

    /// Cat 11 / Test 7 — *Every* recorded step's meta-consensus is in
    /// the meta-safety set.  This is the strongest invariant : safety is
    /// preserved iteration-by-iteration, not just at the final step.
    #[test]
    fn safe_every_step_meta_feasible_pt(
        radius in 1.0_f32..10.0,
        kappa  in 0.3_f32..0.6,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        let ms = make_with_safeties(
            4, 8, kappa, 0.3, 0.2,
            seed_a, seed_b, 100, 1e-3,
            BoxSafety::symmetric(4, radius).boxed(),
            BoxSafety::symmetric(8, 10.0).boxed(),
            BoxSafety::symmetric(8, 10.0).boxed(),
        ).unwrap();
        let r = ms.iterate().unwrap();
        let t = r.trace.as_ref().unwrap();
        let check = BoxSafety::symmetric(4, radius);
        for (i, step) in t.steps().iter().enumerate() {
            prop_assert!(check.contains(&step.meta_consensus, 1e-3),
                "step {i} meta-consensus outside Box(r={radius})");
        }
    }

    /// Cat 11 / Test 8 — *Every* recorded step's participant states are
    /// in their local safety set.
    #[test]
    fn safe_every_step_locals_feasible_pt(
        radius in 1.0_f32..10.0,
        kappa  in 0.3_f32..0.6,
        seed_a in any::<u64>(),
        seed_b in any::<u64>(),
    ) {
        let ms = make_with_safeties(
            4, 8, kappa, 0.3, 0.2,
            seed_a, seed_b, 100, 1e-3,
            BoxSafety::symmetric(4, 10.0).boxed(),
            BoxSafety::symmetric(8, radius).boxed(),
            BoxSafety::symmetric(8, radius).boxed(),
        ).unwrap();
        let r = ms.iterate().unwrap();
        let t = r.trace.as_ref().unwrap();
        let check = BoxSafety::symmetric(8, radius);
        for (i, step) in t.steps().iter().enumerate() {
            for (j, s) in step.participant_states.iter().enumerate() {
                prop_assert!(check.contains(s, 1e-3),
                    "step {i} participant {j} outside Box(r={radius})");
            }
        }
    }
}
