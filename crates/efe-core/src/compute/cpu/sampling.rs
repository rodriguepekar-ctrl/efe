//! Sprint 3 CPU kernels : `multinomial_sample`, `kl_divergence`.
//!
//! All sampling ops take an explicit `seed: u64` for reproducibility.
//! The RNG is [`rand_pcg::Pcg64Mcg`], a permuted congruential generator
//! with good statistical quality (passes BigCrush) and cheap seeding.

use super::shared::reduce_axis_pairwise_to_scalar;
use crate::compute::error::{ComputeError, Result};
use crate::compute::tensor::Tensor;

// =====================================================================
// kl_divergence
// =====================================================================

/// Smoothed Kullback-Leibler divergence along `axis`.
///
/// # Definition
///
/// ```text
///     KL(p ‖ q) = Σ_i  p_i · log( (p_i + ε) / (q_i + ε) )
/// ```
///
/// Capped at 50.0 for downstream stability (M09 drift uses this cap to
/// prevent outlier distributions from saturating the drift score).
///
/// # Why smoothing?
///
/// Without `ε`, any `q_i = 0` with `p_i > 0` yields `+∞`, and any
/// `q_i = 0` with `p_i = 0` yields `0·log(0/0) = NaN`. The additive
/// smoothing `(x + ε)` handles both by keeping all log arguments
/// strictly positive.
///
/// # Preconditions
///
/// * `p.shape() == q.shape()`
/// * `ε > 0` and finite (else `Err(NumericalInstability)`)
/// * Every element of `p` and `q` is non-negative and finite (else
///   `Err(NumericalInstability)`). **Audit 5 hardening**: previous
///   Sprint 3 revisions silently produced NaN outputs when a caller
///   passed a negative or non-finite probability ; we now reject
///   upfront to match the defensive contract shared with
///   [`multinomial_sample`].
///
/// # Invariants
///
/// * `KL(p ‖ p) ≈ 0` to within `1e-5`
/// * `KL(p ‖ q) ≥ 0` for probabilities
/// * Asymmetric : `KL(p ‖ q) ≠ KL(q ‖ p)` in general
/// * Always in `[0, 50]` (capped, now that non-finite inputs are rejected)
pub fn kl_divergence(
    p: &Tensor<f32>,
    q: &Tensor<f32>,
    axis: i32,
    eps: f32,
) -> Result<Tensor<f32>> {
    if eps <= 0.0 || !eps.is_finite() {
        return Err(ComputeError::NumericalInstability {
            op: "kl_divergence",
            detail: format!("eps must be > 0 and finite, got {eps}"),
        });
    }
    // Audit-5 hardening : upfront validation of every element of `p`
    // and `q`. Previously, negative or non-finite probabilities would
    // flow through the closure and produce NaN (since `log(x)` with
    // `x ≤ 0` is NaN, `clamp(NaN, …) = NaN`), silently violating the
    // "Always ≤ 50" contract advertised in the docstring. We now
    // reject explicitly so callers see a structured
    // `NumericalInstability` error instead of propagating garbage.
    let p_slice = p.as_slice()?;
    let q_slice = q.as_slice()?;
    for (idx, &v) in p_slice.iter().enumerate() {
        if v < 0.0 || !v.is_finite() {
            return Err(ComputeError::NumericalInstability {
                op: "kl_divergence",
                detail: format!(
                    "p[{idx}] = {v} is negative or non-finite; \
                     KL requires p_i ≥ 0 and finite"
                ),
            });
        }
    }
    for (idx, &v) in q_slice.iter().enumerate() {
        if v < 0.0 || !v.is_finite() {
            return Err(ComputeError::NumericalInstability {
                op: "kl_divergence",
                detail: format!(
                    "q[{idx}] = {v} is negative or non-finite; \
                     KL requires q_i ≥ 0 and finite"
                ),
            });
        }
    }

    reduce_axis_pairwise_to_scalar(p, q, axis, |p_row, q_row| {
        let mut kl = 0.0f32;
        for (&pi, &qi) in p_row.iter().zip(q_row.iter()) {
            // Inputs are now validated non-negative and finite upfront,
            // so `(pi + eps) / (qi + eps)` is strictly positive and
            // `ln(...)` is finite. The clamp remains a defensive bound
            // for floating-point rounding on pathological distributions.
            let ratio = (pi + eps) / (qi + eps);
            kl += pi * ratio.ln();
        }
        kl.clamp(0.0, 50.0)
    })
}

// =====================================================================
// shannon_entropy — Sprint 5
// =====================================================================

/// Shannon entropy along `axis`.
///
/// # Definition
///
/// ```text
///     H(p) = − Σ_i  p_i · ln( max(p_i, ε) )
/// ```
///
/// with `ε = 1e-12` floor to avoid `ln(0)`. Inputs must be non-negative
/// and finite. The op does **not** normalise `p` — the caller passes a
/// distribution. This contract matches the audit-5 hardening of
/// [`kl_divergence`].
///
/// # Invariants
///
/// * `H(uniform K-vector) = ln(K)` exactly (within rtol = 1e-5).
/// * `H(one-hot) ≈ 0` (the small floor `ε` introduces a residual
///   `−1·ln(1) − (K−1)·0·ln(ε) = 0`).
/// * `H(p) ≥ 0` always.
///
/// # Errors
///
/// * [`ComputeError::NumericalInstability`] if any `p_i < 0` or
///   non-finite.
pub fn shannon_entropy(p: &Tensor<f32>, axis: i32) -> Result<Tensor<f32>> {
    // Upfront validation.
    let p_slice = p.as_slice()?;
    for (idx, &v) in p_slice.iter().enumerate() {
        if v < 0.0 || !v.is_finite() {
            return Err(ComputeError::NumericalInstability {
                op: "shannon_entropy",
                detail: format!(
                    "p[{idx}] = {v} is negative or non-finite"
                ),
            });
        }
    }
    // We re-use `reduce_axis_pairwise_to_scalar` by passing `p` for both
    // arguments and ignoring the second slice. Cleaner: implement a
    // dedicated single-tensor reducer. For now we factor through the
    // existing helper to avoid duplicating axis-walk plumbing.
    let eps = 1e-12_f32;
    reduce_axis_pairwise_to_scalar(p, p, axis, |row, _q_unused| {
        let mut h = 0.0f32;
        for &pi in row {
            // Floor avoids ln(0) when p_i is exactly 0; the contribution
            // pi · ln(eps) collapses to 0 because pi == 0 in that case.
            let safe = pi.max(eps);
            h -= pi * safe.ln();
        }
        // Defensive clamp: tiny negative drift on a deterministic input
        // (e.g. a perfectly one-hot `p`) can produce `−0.0` or
        // numerical noise of magnitude < 1e-7. Clamp to 0 to honour
        // `H ≥ 0` invariant exactly.
        h.max(0.0)
    })
}

// =====================================================================
// multinomial_sample — Sprint 3 Étape 8
// =====================================================================

// Implementation moved to Étape 8 below.
pub use _multinomial::multinomial_sample;

#[allow(clippy::items_after_statements)] // inner mod follows `use` by design
mod _multinomial {
    //! Inner module isolating the `rand`/`rand_pcg` usage.

    use crate::compute::error::{ComputeError, Result};
    use crate::compute::tensor::Tensor;
    use rand::{RngCore, SeedableRng};
    use rand_pcg::Pcg64Mcg;

    /// Draw `n` independent samples per row from a multinomial.
    ///
    /// # Shape contract
    ///
    /// * `probs` can be rank-1 `(K,)` or rank-2 `(B, K)`.
    /// * Output is always `(B, n)` int32 ; rank-1 input is treated as
    ///   a single batch of size 1.
    ///
    /// # Determinism
    ///
    /// Identical `seed` → identical output, bit-for-bit, across
    /// platforms supporting [`Pcg64Mcg`].
    pub fn multinomial_sample(
        probs: &Tensor<f32>,
        n: u32,
        seed: u64,
    ) -> Result<Tensor<i32>> {
        let (b, k) = match probs.shape().rank() {
            1 => (1usize, probs.shape().dims()[0]),
            2 => (probs.shape().dims()[0], probs.shape().dims()[1]),
            r => {
                return Err(ComputeError::ShapeMismatch {
                    op: "multinomial_sample",
                    expected: "rank 1 or 2".into(),
                    actual: format!("rank {r}"),
                })
            }
        };
        if k == 0 {
            return Err(ComputeError::ShapeMismatch {
                op: "multinomial_sample",
                expected: "K > 0".into(),
                actual: "K = 0".into(),
            });
        }
        let data = probs.as_slice()?;
        let n_usize = n as usize;
        let mut out = vec![0i32; b * n_usize];
        let mut rng = Pcg64Mcg::seed_from_u64(seed);

        for bi in 0..b {
            let row = &data[bi * k..(bi + 1) * k];
            // Validate : finite, non-negative, sum > 0
            let mut row_sum = 0.0f32;
            for &p in row {
                if p < 0.0 || !p.is_finite() {
                    return Err(ComputeError::NumericalInstability {
                        op: "multinomial_sample",
                        detail: format!("row {bi} contains non-finite or negative probability"),
                    });
                }
                row_sum += p;
            }
            if row_sum <= 0.0 {
                return Err(ComputeError::NumericalInstability {
                    op: "multinomial_sample",
                    detail: format!("row {bi}: sum of probabilities is {row_sum}, must be > 0"),
                });
            }

            // Build normalized cumulative distribution function.
            let mut cdf = Vec::with_capacity(k);
            let mut running = 0.0f32;
            for &p in row {
                running += p / row_sum;
                cdf.push(running);
            }
            // Snap last entry to 1.0 so binary search can never miss due
            // to accumulated f32 drift.
            if let Some(last) = cdf.last_mut() {
                *last = 1.0;
            }

            for ki in 0..n_usize {
                // Generate uniform u ∈ [0, 1) via next_u32 / 2^32.
                // The explicit f64 path avoids a rare edge where the
                // ratio rounds up to exactly 1.0 in f32.
                let u = (f64::from(rng.next_u32()) / f64::from(u32::MAX) + 1e-12) as f32;
                let u = u.min(1.0 - f32::EPSILON);
                // `partition_point` returns the first index i with cdf[i] ≥ u.
                let idx = cdf.partition_point(|&v| v < u);
                out[bi * n_usize + ki] = idx.min(k - 1) as i32;
            }
        }

        Tensor::from_slice(&out, [b, n_usize].into())
    }

    #[cfg(test)]
    #[allow(clippy::float_cmp)]
    mod tests {
        use super::*;

        #[test]
        fn multinomial_uniform_produces_valid_indices() {
            let probs = Tensor::<f32>::from_slice(&[0.25, 0.25, 0.25, 0.25], [4].into()).unwrap();
            let s = multinomial_sample(&probs, 1000, 42).unwrap();
            assert_eq!(s.shape().dims(), &[1, 1000]);
            let samples = s.as_slice().unwrap();
            assert!(samples.iter().all(|&i| (0..4).contains(&i)));
        }

        #[test]
        fn multinomial_deterministic_on_seed() {
            let probs = Tensor::<f32>::from_slice(&[0.1, 0.2, 0.3, 0.4], [4].into()).unwrap();
            let s1 = multinomial_sample(&probs, 100, 7).unwrap();
            let s2 = multinomial_sample(&probs, 100, 7).unwrap();
            assert_eq!(s1.as_slice().unwrap(), s2.as_slice().unwrap());
        }

        #[test]
        fn multinomial_different_seed_different_output() {
            let probs = Tensor::<f32>::from_slice(&[0.1, 0.2, 0.3, 0.4], [4].into()).unwrap();
            let s1 = multinomial_sample(&probs, 100, 1).unwrap();
            let s2 = multinomial_sample(&probs, 100, 2).unwrap();
            assert_ne!(s1.as_slice().unwrap(), s2.as_slice().unwrap());
        }

        #[test]
        fn multinomial_batch_2d() {
            let probs = Tensor::<f32>::from_slice(
                &[1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
                [2, 3].into(),
            )
            .unwrap();
            let s = multinomial_sample(&probs, 10, 0).unwrap();
            let samples = s.as_slice().unwrap();
            assert_eq!(s.shape().dims(), &[2, 10]);
            // Row 0 (all mass at 0) → all 0
            assert!(samples[..10].iter().all(|&i| i == 0));
            // Row 1 (all mass at 2) → all 2
            assert!(samples[10..].iter().all(|&i| i == 2));
        }

        #[test]
        fn multinomial_rejects_zero_row() {
            let probs = Tensor::<f32>::from_slice(&[0.0, 0.0, 0.0], [3].into()).unwrap();
            let err = multinomial_sample(&probs, 5, 0).unwrap_err();
            assert!(matches!(err, ComputeError::NumericalInstability { .. }));
        }

        #[test]
        fn multinomial_rejects_negative_probability() {
            let probs = Tensor::<f32>::from_slice(&[0.5, -0.5, 0.5], [3].into()).unwrap();
            let err = multinomial_sample(&probs, 5, 0).unwrap_err();
            assert!(matches!(err, ComputeError::NumericalInstability { .. }));
        }

        #[test]
        fn multinomial_normalizes_unnormalized_input() {
            // Probs that don't sum to 1 but are positive should still work.
            let probs = Tensor::<f32>::from_slice(&[2.0, 4.0, 8.0, 2.0], [4].into()).unwrap();
            let s = multinomial_sample(&probs, 100, 3).unwrap();
            assert_eq!(s.shape().dims(), &[1, 100]);
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    // ===== kl_divergence =====

    #[test]
    fn kl_self_is_zero() {
        let p = Tensor::<f32>::from_slice(&[0.2, 0.3, 0.5], [3].into()).unwrap();
        let kl = kl_divergence(&p, &p, -1, 1e-9).unwrap();
        assert!(kl.as_slice().unwrap()[0].abs() < 1e-5);
    }

    #[test]
    fn kl_is_non_negative() {
        let p = Tensor::<f32>::from_slice(&[0.1, 0.9], [2].into()).unwrap();
        let q = Tensor::<f32>::from_slice(&[0.9, 0.1], [2].into()).unwrap();
        let kl = kl_divergence(&p, &q, -1, 1e-9).unwrap();
        let v = kl.as_slice().unwrap()[0];
        assert!(v >= 0.0);
    }

    #[test]
    fn kl_is_asymmetric() {
        let p = Tensor::<f32>::from_slice(&[0.1, 0.9], [2].into()).unwrap();
        let q = Tensor::<f32>::from_slice(&[0.5, 0.5], [2].into()).unwrap();
        let kl_pq = kl_divergence(&p, &q, -1, 1e-9).unwrap().as_slice().unwrap()[0];
        let kl_qp = kl_divergence(&q, &p, -1, 1e-9).unwrap().as_slice().unwrap()[0];
        // They differ in general
        assert!((kl_pq - kl_qp).abs() > 1e-3);
    }

    #[test]
    fn kl_capped_at_50() {
        // p = [1, 0], q = [ε, 1-ε] : KL is very large without cap
        let p = Tensor::<f32>::from_slice(&[1.0, 0.0], [2].into()).unwrap();
        let q = Tensor::<f32>::from_slice(&[1e-30, 1.0 - 1e-30], [2].into()).unwrap();
        let kl = kl_divergence(&p, &q, -1, 1e-30).unwrap();
        let v = kl.as_slice().unwrap()[0];
        assert!(v <= 50.0);
    }

    #[test]
    fn kl_rejects_non_positive_eps() {
        let p = Tensor::<f32>::from_slice(&[0.5, 0.5], [2].into()).unwrap();
        let q = Tensor::<f32>::from_slice(&[0.5, 0.5], [2].into()).unwrap();
        assert!(kl_divergence(&p, &q, -1, 0.0).is_err());
        assert!(kl_divergence(&p, &q, -1, -1.0).is_err());
    }

    #[test]
    fn kl_rejects_shape_mismatch() {
        let p = Tensor::<f32>::from_slice(&[0.5, 0.5], [2].into()).unwrap();
        let q = Tensor::<f32>::from_slice(&[0.3, 0.3, 0.4], [3].into()).unwrap();
        assert!(kl_divergence(&p, &q, -1, 1e-9).is_err());
    }

    #[test]
    fn kl_batched_2d() {
        // Two rows, both self-KL = 0
        let p = Tensor::<f32>::from_slice(
            &[0.5, 0.5, 0.3, 0.7],
            [2, 2].into(),
        )
        .unwrap();
        let kl = kl_divergence(&p, &p, -1, 1e-9).unwrap();
        assert_eq!(kl.shape().dims(), &[2]);
        let s = kl.as_slice().unwrap();
        assert!(s[0].abs() < 1e-5);
        assert!(s[1].abs() < 1e-5);
    }

    // === Audit 5 — upfront validation of negative/non-finite inputs ===

    #[test]
    fn kl_rejects_negative_p() {
        let p = Tensor::<f32>::from_slice(&[0.5, -0.1, 0.6], [3].into()).unwrap();
        let q = Tensor::<f32>::from_slice(&[0.3, 0.3, 0.4], [3].into()).unwrap();
        let err = kl_divergence(&p, &q, -1, 1e-9).unwrap_err();
        assert!(matches!(err, ComputeError::NumericalInstability { .. }));
    }

    #[test]
    fn kl_rejects_negative_q() {
        let p = Tensor::<f32>::from_slice(&[0.3, 0.3, 0.4], [3].into()).unwrap();
        let q = Tensor::<f32>::from_slice(&[0.5, -0.1, 0.6], [3].into()).unwrap();
        let err = kl_divergence(&p, &q, -1, 1e-9).unwrap_err();
        assert!(matches!(err, ComputeError::NumericalInstability { .. }));
    }

    #[test]
    fn kl_rejects_nan_in_p() {
        let p = Tensor::<f32>::from_slice(&[0.5, f32::NAN, 0.5], [3].into()).unwrap();
        let q = Tensor::<f32>::from_slice(&[0.3, 0.3, 0.4], [3].into()).unwrap();
        let err = kl_divergence(&p, &q, -1, 1e-9).unwrap_err();
        assert!(matches!(err, ComputeError::NumericalInstability { .. }));
    }

    #[test]
    fn kl_rejects_inf_in_q() {
        let p = Tensor::<f32>::from_slice(&[0.3, 0.3, 0.4], [3].into()).unwrap();
        let q = Tensor::<f32>::from_slice(
            &[0.5, f32::INFINITY, 0.5],
            [3].into(),
        )
        .unwrap();
        let err = kl_divergence(&p, &q, -1, 1e-9).unwrap_err();
        assert!(matches!(err, ComputeError::NumericalInstability { .. }));
    }

    #[test]
    fn kl_always_finite_and_bounded_after_hardening() {
        // Pathological : p very concentrated, q uniform. With validation,
        // the output must still be finite AND within [0, 50].
        let p = Tensor::<f32>::from_slice(&[0.999, 0.001], [2].into()).unwrap();
        let q = Tensor::<f32>::from_slice(&[0.001, 0.999], [2].into()).unwrap();
        let kl = kl_divergence(&p, &q, -1, 1e-9).unwrap();
        let v = kl.as_slice().unwrap()[0];
        assert!(v.is_finite(), "KL must be finite, got {v}");
        assert!((0.0..=50.0).contains(&v), "KL must be in [0, 50], got {v}");
    }

    // ===== shannon_entropy (Sprint 5) =====

    #[test]
    fn shannon_entropy_uniform_k4_equals_ln4() {
        let p = Tensor::<f32>::from_slice(&[0.25, 0.25, 0.25, 0.25], [4].into()).unwrap();
        let h = shannon_entropy(&p, -1).unwrap().as_slice().unwrap()[0];
        let expected = 4.0_f32.ln();
        assert!(
            (h - expected).abs() < 1e-5,
            "H(uniform-4) expected {expected}, got {h}"
        );
    }

    #[test]
    fn shannon_entropy_one_hot_is_zero() {
        let p = Tensor::<f32>::from_slice(&[1.0, 0.0, 0.0], [3].into()).unwrap();
        let h = shannon_entropy(&p, -1).unwrap().as_slice().unwrap()[0];
        assert!(h.abs() < 1e-5, "H(one-hot) expected 0, got {h}");
    }

    #[test]
    fn shannon_entropy_non_negative_property() {
        let p = Tensor::<f32>::from_slice(&[0.1, 0.2, 0.3, 0.4], [4].into()).unwrap();
        let h = shannon_entropy(&p, -1).unwrap().as_slice().unwrap()[0];
        assert!(h >= 0.0, "H must be ≥ 0, got {h}");
    }

    #[test]
    fn shannon_entropy_rejects_negative() {
        let p = Tensor::<f32>::from_slice(&[0.5, -0.1, 0.6], [3].into()).unwrap();
        let err = shannon_entropy(&p, -1).unwrap_err();
        assert!(matches!(err, ComputeError::NumericalInstability { .. }));
    }

    #[test]
    fn shannon_entropy_rejects_nan() {
        let p = Tensor::<f32>::from_slice(&[0.5, f32::NAN, 0.5], [3].into()).unwrap();
        let err = shannon_entropy(&p, -1).unwrap_err();
        assert!(matches!(err, ComputeError::NumericalInstability { .. }));
    }

    #[test]
    fn shannon_entropy_batched_2d() {
        // Two rows: row 0 uniform-3 → ln(3), row 1 one-hot → 0.
        let p = Tensor::<f32>::from_slice(
            &[1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0, 1.0, 0.0, 0.0],
            [2, 3].into(),
        )
        .unwrap();
        let h = shannon_entropy(&p, -1).unwrap();
        assert_eq!(h.shape().dims(), &[2]);
        let s = h.as_slice().unwrap();
        let ln3 = 3.0_f32.ln();
        assert!((s[0] - ln3).abs() < 1e-5);
        assert!(s[1].abs() < 1e-5);
    }

    #[test]
    fn shannon_entropy_self_consistency_with_kl() {
        // Identity: KL(p || uniform) = ln(K) − H(p) for valid p.
        // With p = [0.5, 0.25, 0.25] and K=3:
        //   H(p) = -0.5 ln 0.5 − 2·0.25·ln 0.25 = 0.5 ln 2 + 0.5 ln 4
        //        = ln 2 + 0.5 ln 2 ≈ 1.0397
        //   ln 3 ≈ 1.0986 → KL ≈ 0.0589.
        let p = Tensor::<f32>::from_slice(&[0.5, 0.25, 0.25], [3].into()).unwrap();
        let uniform = Tensor::<f32>::from_slice(
            &[1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0],
            [3].into(),
        )
        .unwrap();
        let h = shannon_entropy(&p, -1).unwrap().as_slice().unwrap()[0];
        let kl = kl_divergence(&p, &uniform, -1, 1e-9).unwrap()
            .as_slice().unwrap()[0];
        let ln_k = 3.0_f32.ln();
        // tolerance loose because KL uses smoothing (eps = 1e-9).
        assert!(
            (h + kl - ln_k).abs() < 5e-3,
            "H+KL identity: H={h}, KL={kl}, ln(K)={ln_k}, sum={}",
            h + kl,
        );
    }
}
