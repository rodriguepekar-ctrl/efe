//! Learned safety projector — Phase-1 stub.
//!
//! In Phase 1 the "learned" projector is just an alias for any closure
//! that maps `Tensor -> Tensor` and is *advertised* as a non-expansive
//! approximation of an underlying safety set.  We do not load real NN
//! weights here ;  that is deferred to Phase 2's `file-loading` feature.
//!
//! Theorem 5.2 of A5 covers the soft-safety regime: under uniform
//! projection error bounded by `epsilon`, the iterates stay within
//! `epsilon` of the true safety set.

use crate::{MetaSwarmError, Result, Tensor};
use super::SafetySet;

/// Generic learned projector.  Holds a boxed closure that performs the
/// projection ;  the closure is responsible for honouring the
/// non-expansiveness contract.
pub struct LearnedSafety {
    name:    String,
    dim:     usize,
    epsilon: f64,
    project_fn: Box<dyn Fn(&Tensor) -> Result<Tensor> + Send + Sync>,
}

impl std::fmt::Debug for LearnedSafety {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LearnedSafety")
            .field("name", &self.name)
            .field("dim", &self.dim)
            .field("epsilon", &self.epsilon)
            .finish_non_exhaustive()
    }
}

impl LearnedSafety {
    /// Construct from an arbitrary closure.  The caller asserts that the
    /// closure's output is within `epsilon` (in the metric distance) of
    /// the exact projection onto the intended safety set.
    pub fn from_fn<F>(
        name: impl Into<String>,
        dim: usize,
        epsilon: f64,
        f: F,
    ) -> Self
    where F: Fn(&Tensor) -> Result<Tensor> + Send + Sync + 'static
    {
        Self {
            name: name.into(),
            dim,
            epsilon,
            project_fn: Box::new(f),
        }
    }

    /// Soft-safety guarantee parameter $\varepsilon \ge 0$.
    #[must_use]
    pub fn epsilon(&self) -> f64 { self.epsilon }
}

impl SafetySet for LearnedSafety {
    fn dim(&self) -> usize { self.dim }

    fn project(&self, x: &Tensor) -> Result<Tensor> {
        if x.dim() != self.dim {
            return Err(MetaSwarmError::DimensionMismatch { expected: self.dim, actual: x.dim() });
        }
        (self.project_fn)(x)
    }

    fn contains(&self, _x: &Tensor, _tolerance: f64) -> bool {
        // We have no analytic test ; a learned projector is consulted to
        // *map* a point to the set, but knowing whether an arbitrary point
        // is already inside requires the original set, which we don't
        // store.  Phase 1 conservatively returns false.
        false
    }

    fn name(&self) -> &str { &self.name }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_learned_projector_works() {
        let p = LearnedSafety::from_fn(
            "identity_test", 4, 0.0,
            |x| Ok(x.clone()),
        );
        let x = Tensor::pseudo_random(4, 5);
        let y = p.project(&x).unwrap();
        assert!(x.distance(&y).unwrap() < 1e-9);
    }

    #[test]
    fn dim_mismatch_is_reported() {
        let p = LearnedSafety::from_fn(
            "test", 4, 0.0,
            |x| Ok(x.clone()),
        );
        let x = Tensor::zeros(3);
        match p.project(&x) {
            Err(MetaSwarmError::DimensionMismatch { expected: 4, actual: 3 }) => (),
            other => panic!("expected DimensionMismatch, got {other:?}"),
        }
    }

    // =========================================================================
    // ===  Sprint A1 P1 — PR #3 — Category 8 : LearnedSafety proptest (5) ===
    // =========================================================================

    use proptest::prelude::*;

    fn pt_cfg_learned() -> ProptestConfig {
        ProptestConfig { cases: 32, ..ProptestConfig::default() }
    }

    proptest! {
        #![proptest_config(pt_cfg_learned())]

        /// Cat 8 / Test 1 — Identity learned projector preserves input.
        #[test]
        fn learned_identity_preserves_pt(
            dim in 1usize..=32,
            seed in any::<u64>(),
        ) {
            let p = LearnedSafety::from_fn(
                "pt8-1", dim, 0.0,
                |x| Ok(x.clone()),
            );
            let x = Tensor::pseudo_random(dim, seed);
            let y = p.project(&x).unwrap();
            prop_assert!(x.distance(&y).unwrap() < 1e-6);
        }

        /// Cat 8 / Test 2 — Epsilon getter returns the configured value.
        #[test]
        fn learned_epsilon_getter_pt(
            dim in 1usize..=16,
            eps in 0.0_f64..1.0,
        ) {
            let p = LearnedSafety::from_fn(
                "pt8-2", dim, eps,
                |x| Ok(x.clone()),
            );
            prop_assert!((p.epsilon() - eps).abs() < 1e-12);
        }

        /// Cat 8 / Test 3 — Scale-by-half projector preserves Lipschitz ≤ 0.5.
        #[test]
        fn learned_scale_half_non_expansive_pt(
            dim in 1usize..=16,
            seed_a in any::<u64>(),
            seed_b in any::<u64>(),
        ) {
            let p = LearnedSafety::from_fn(
                "pt8-3", dim, 0.0,
                |x| Ok(x.scale(0.5)),
            );
            let a = Tensor::pseudo_random(dim, seed_a);
            let b = Tensor::pseudo_random(dim, seed_b);
            let pa = p.project(&a).unwrap();
            let pb = p.project(&b).unwrap();
            let ratio = pa.distance(&pb).unwrap()
                / a.distance(&b).unwrap().max(1e-9);
            prop_assert!(ratio <= 0.5 + 1e-5);
        }

        /// Cat 8 / Test 4 — `contains` is conservatively false in Phase 1.
        #[test]
        fn learned_contains_returns_false_pt(
            dim in 1usize..=16,
            seed in any::<u64>(),
        ) {
            let p = LearnedSafety::from_fn(
                "pt8-4", dim, 0.0,
                |x| Ok(x.clone()),
            );
            let x = Tensor::pseudo_random(dim, seed);
            prop_assert!(!p.contains(&x, 1e6));
        }

        /// Cat 8 / Test 5 — Dim mismatch rejected.
        #[test]
        fn learned_dim_mismatch_pt(
            dim in 2usize..=16,
            wrong in 1usize..=64,
            seed in any::<u64>(),
        ) {
            prop_assume!(wrong != dim);
            let p = LearnedSafety::from_fn(
                "pt8-5", dim, 0.0,
                |x| Ok(x.clone()),
            );
            let x = Tensor::pseudo_random(wrong, seed);
            let is_dim_mismatch = matches!(
                p.project(&x),
                Err(MetaSwarmError::DimensionMismatch { .. })
            );
            prop_assert!(is_dim_mismatch);
        }
    }
}
