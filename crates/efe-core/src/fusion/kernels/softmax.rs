//! Softmax kernel — row-wise, numerically stabilized (max-shift).
//!
//! Given an `N×N` flat f32 matrix of scores (row-major), produces a
//! matrix where each row is either:
//!
//! - A valid softmax distribution (entries in `[0, 1]`, sum = 1).
//! - Or an all-zero row, when the input row was entirely `-∞`
//!   (meaning there's no valid attention target for that agent).
//!
//! The caller must interpret zero-rows as "identity" — the downstream
//! aggregate kernel does exactly that.
//!
//! ## Sprint 5 — façade over `ComputeBackend::softmax`
//!
//! Pre-Sprint-5, this kernel rolled its own LogSumExp implementation
//! (~50 LOC). After Sprint 5 it delegates the actual softmax to
//! [`crate::compute::ComputeBackend::softmax`] and only retains the
//! domain-specific glue: NaN fail-fast and all-(-∞) row zero-out.
//!
//! # NaN contract
//!
//! If any input score is `NaN`, the kernel returns
//! [`EfeError::NumericalExplosion`] rather than silently propagating.
//! Silent propagation was a MVP bug: a single NaN anywhere in the
//! `N × N` scores made all rows become all-NaN, which clip detects
//! only at round-end — several kernels downstream consumed the garbage.

use crate::compute::{global_backend, Tensor};
use crate::errors::{EfeError, Result};

/// Outcome of one softmax pass.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SoftmaxMetrics {
    /// Number of rows that were entirely `-∞` on input (softmax → 0s).
    pub zero_rows: u32,
}

/// In-place row-wise softmax on an `N × N` matrix.
///
/// # Preconditions
/// - `scores.len() == n * n`
///
/// # Algorithm
/// For each row:
/// 1. Scan for NaN — fail-fast if found (programmer/kernel bug).
/// 2. `μ = max(scores)`.
/// 3. If `μ` is `-∞` (all entries `-∞`) → zero the row, bump counter.
/// 4. Else: `e_ij = exp(s_ij - μ)`, `Z = Σ e_ij`, `α_ij = e_ij / Z`.
///
/// # Numerical guarantees
/// - On success, no row contains NaN.
/// - Max-shift prevents overflow even for `s_ij = 1e30`.
/// - Sum-to-one holds to ≤ 1 ulp for reasonable N (< 10⁵).
/// - `exp(-∞) = 0` so masked entries contribute zero weight.
///
/// # Errors
/// - [`EfeError::NumericalExplosion { round: 0 }`] if any score is NaN.
///   The caller re-stamps the round index.
pub(crate) fn softmax_in_place_rows(scores: &mut [f32], n: usize) -> Result<SoftmaxMetrics> {
    debug_assert_eq!(
        scores.len(),
        n.checked_mul(n).expect("softmax N² overflow"),
        "softmax buffer must be N*N"
    );
    if n == 0 {
        return Ok(SoftmaxMetrics::default());
    }

    // Pass A — NaN fail-fast and detection of all-(-∞) rows. Both are
    // fast linear scans and let us bypass the backend round-trip when
    // the input is structurally bad. The `zero_rows` mask is applied
    // *after* the backend call.
    let mut zero_rows = 0u32;
    let mut zero_mask = vec![false; n];
    for i in 0..n {
        let row = &scores[i * n..(i + 1) * n];
        let mut all_neg_inf = true;
        for &v in row {
            if v.is_nan() {
                return Err(EfeError::NumericalExplosion { round: 0 });
            }
            if v != f32::NEG_INFINITY {
                all_neg_inf = false;
            }
        }
        if all_neg_inf {
            zero_mask[i] = true;
            zero_rows += 1;
        }
    }

    // Pass B — replace all-(-∞) rows by zeros so the backend sees a
    // benign input (it would otherwise return NaN for those rows).
    // We retain the original placement so we can scatter back below.
    if zero_rows > 0 {
        for (i, masked) in zero_mask.iter().enumerate() {
            if *masked {
                for v in &mut scores[i * n..(i + 1) * n] {
                    *v = 0.0;
                }
            }
        }
    }

    // Pass C — call the canonical backend softmax on the full (n, n)
    // matrix. The backend uses LogSumExp internally, so the
    // numerical behaviour matches the previous artisanal kernel.
    let backend = global_backend();
    let t = Tensor::<f32>::from_slice(scores, [n, n].into())
        .map_err(|_| EfeError::NumericalExplosion { round: 0 })?;
    let out = backend
        .softmax(&t, -1)
        .map_err(|_| EfeError::NumericalExplosion { round: 0 })?;
    let out_slice = out
        .as_slice()
        .map_err(|_| EfeError::NumericalExplosion { round: 0 })?;
    scores.copy_from_slice(out_slice);

    // Pass D — re-zero the rows that were entirely -∞. The backend
    // would have produced uniform `1/N` for them after our Pass B
    // substitution, but the contract advertises 0-rows.
    for (i, masked) in zero_mask.iter().enumerate() {
        if *masked {
            for v in &mut scores[i * n..(i + 1) * n] {
                *v = 0.0;
            }
        }
    }

    Ok(SoftmaxMetrics { zero_rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn softmax_row_sums_to_one() {
        let n = 4;
        let mut scores = vec![0.0f32; n * n];
        // Uniform row → uniform distribution → sum = 1.
        for (i, s) in scores.iter_mut().enumerate() {
            *s = ((i % n) as f32) * 0.1;
        }
        let m = softmax_in_place_rows(&mut scores, n).unwrap();
        assert_eq!(m.zero_rows, 0);
        for i in 0..n {
            let s: f32 = scores[i * n..(i + 1) * n].iter().sum();
            assert!((s - 1.0).abs() < 1e-5, "row {i} sums to {s}");
        }
    }

    #[test]
    fn softmax_handles_extreme_inputs() {
        // Without max-shift, exp(1e30) would be +inf.
        let n = 3;
        let mut scores = vec![1e30f32; n * n];
        let m = softmax_in_place_rows(&mut scores, n).unwrap();
        assert_eq!(m.zero_rows, 0);
        for i in 0..n {
            let s: f32 = scores[i * n..(i + 1) * n].iter().sum();
            assert!((s - 1.0).abs() < 1e-5);
            // Uniform input → uniform output = 1/N.
            for &v in &scores[i * n..(i + 1) * n] {
                assert!((v - 1.0 / n as f32).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn softmax_zero_row_when_all_neg_inf() {
        let n = 3;
        let mut scores = vec![f32::NEG_INFINITY; n * n];
        let m = softmax_in_place_rows(&mut scores, n).unwrap();
        assert_eq!(m.zero_rows, n as u32);
        for &v in &scores {
            assert!(v.abs() < f32::EPSILON);
        }
    }

    #[test]
    fn softmax_partial_neg_inf_still_sums_to_one() {
        let n = 3;
        let mut scores = vec![0.0f32; n * n];
        scores[0] = f32::NEG_INFINITY;
        scores[1] = 1.0;
        scores[2] = 2.0;
        for i in 1..n {
            for j in 0..n {
                scores[i * n + j] = (j as f32) * 0.1;
            }
        }
        let m = softmax_in_place_rows(&mut scores, n).unwrap();
        assert_eq!(m.zero_rows, 0);
        let row0 = &scores[0..n];
        let s: f32 = row0.iter().sum();
        assert!((s - 1.0).abs() < 1e-5);
        assert!(row0[0].abs() < f32::EPSILON);
        assert!(row0[1] > 0.0 && row0[2] > 0.0);
        assert!(row0[2] > row0[1]);
    }

    #[test]
    fn softmax_monotone_in_input() {
        let n = 4;
        let mut flat = vec![0.0f32; n * n];
        let mut peaky = vec![0.0f32; n * n];
        for j in 0..n {
            flat[j] = (j as f32) * 0.1;
            peaky[j] = j as f32;
        }
        for i in 1..n {
            for j in 0..n {
                flat[i * n + j] = 0.0;
                peaky[i * n + j] = 0.0;
            }
        }
        softmax_in_place_rows(&mut flat, n).unwrap();
        softmax_in_place_rows(&mut peaky, n).unwrap();
        let entropy1: f32 = flat[0..n]
            .iter()
            .map(|&p| if p > 0.0 { -p * p.ln() } else { 0.0 })
            .sum();
        let entropy2: f32 = peaky[0..n]
            .iter()
            .map(|&p| if p > 0.0 { -p * p.ln() } else { 0.0 })
            .sum();
        assert!(
            entropy1 > entropy2,
            "flat entropy {entropy1} should exceed peaky {entropy2}"
        );
    }

    #[test]
    fn softmax_large_n_still_sane() {
        let n = 32;
        let mut scores = vec![0.0f32; n * n];
        for (i, s) in scores.iter_mut().enumerate() {
            *s = ((i as f32) * 0.01).sin();
        }
        let m = softmax_in_place_rows(&mut scores, n).unwrap();
        assert_eq!(m.zero_rows, 0);
        for i in 0..n {
            let s: f32 = scores[i * n..(i + 1) * n].iter().sum();
            assert!((s - 1.0).abs() < 1e-3, "row {i} sums to {s}");
            for &v in &scores[i * n..(i + 1) * n] {
                assert!((0.0..=1.0).contains(&v));
            }
        }
    }

    #[test]
    fn softmax_rejects_nan_input() {
        let n = 3;
        let mut scores = vec![0.0f32; n * n];
        scores[0] = f32::NAN;
        let err = softmax_in_place_rows(&mut scores, n).unwrap_err();
        assert!(matches!(err, EfeError::NumericalExplosion { .. }));
    }

    #[test]
    fn softmax_rejects_nan_in_later_row() {
        // NaN in row 2 should still be caught even after row 0, 1 are fine.
        let n = 3;
        let mut scores = vec![0.0f32; n * n];
        scores[2 * n + 1] = f32::NAN;
        let err = softmax_in_place_rows(&mut scores, n).unwrap_err();
        assert!(matches!(err, EfeError::NumericalExplosion { .. }));
    }
}
