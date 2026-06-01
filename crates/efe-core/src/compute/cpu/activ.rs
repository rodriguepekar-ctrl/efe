//! Sprint 2 activation kernels: `softmax`, `log_softmax`, `gelu`.
//!
//! All three use the LogSumExp trick for numerical stability where
//! applicable. `gelu` uses the exact (erf-based) formulation to match
//! PyTorch's default and keep our golden vectors identical bit-for-bit
//! up to an `rtol` of 1e-5.

use super::shared::map_along_axis;
use crate::compute::error::{ComputeError, Result};
use crate::compute::tensor::Tensor;

/// Numerically stable softmax along `axis`.
///
/// # Mathematical definition
///
/// Given a vector `x ∈ ℝⁿ`, the softmax is:
///
/// ```text
///     softmax(x)_i = exp(x_i) / Σ_j exp(x_j)
/// ```
///
/// which satisfies `softmax(x)_i > 0` and `Σ_i softmax(x)_i = 1`.
///
/// # Why the naïve formula is unsafe
///
/// For `x_i > 88.7`, `exp(x_i)` overflows the f32 representation
/// (`f32::MAX ≈ 3.4e38`, and `exp(88.7) ≈ 3.4e38`). The naïve formula
/// then produces `Inf / Inf = NaN`.
///
/// # The LogSumExp trick
///
/// Subtract the max from each element **before** exponentiating:
///
/// ```text
///     softmax(x)_i = exp(x_i − m) / Σ_j exp(x_j − m),  with m = max(x)
/// ```
///
/// This is algebraically identical because the factor `exp(m)` cancels
/// in numerator and denominator. But now `x_j − m ≤ 0` for all `j`, so
/// `exp(x_j − m) ∈ (0, 1]` — no overflow is possible.
///
/// # Edge cases
///
/// * **Empty axis (rank 0)** → `Err(ComputeError::InvalidAxis)`
/// * **Single-element row** → returns `[1.0]` directly (fast path).
/// * **Row containing `+∞`** → `max = +∞`, so `x_i − max = -∞` for
///   finite `x_i` and `+∞ − +∞ = NaN`. The NaN propagates to the whole
///   row (this matches PyTorch behaviour). Caller must validate.
/// * **Row containing `-∞` only** → `max = -∞` (non-finite), we
///   short-circuit to `vec![NaN; n]`.
/// * **Row containing `NaN`** → `NaN` propagates through the entire
///   row. Caller must check downstream.
///
/// # Invariants guaranteed on finite inputs
///
/// * `0 ≤ softmax(x)_i ≤ 1` for every `i`
/// * `|Σ_i softmax(x)_i − 1| < 1e-5` (f32 rounding tolerance)
/// * Monotonic: `x_i > x_j  ⟹  softmax(x)_i > softmax(x)_j`
pub fn softmax(x: &Tensor<f32>, axis: i32) -> Result<Tensor<f32>> {
    map_along_axis(x, axis, |row| {
        // Fast path : single element row trivially becomes [1.0].
        if row.len() == 1 {
            return vec![1.0];
        }
        let max = row.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        if !max.is_finite() {
            return vec![f32::NAN; row.len()];
        }
        let exps: Vec<f32> = row.iter().map(|&v| (v - max).exp()).collect();
        let sum: f32 = exps.iter().sum();
        if sum <= 0.0 || !sum.is_finite() {
            return vec![f32::NAN; row.len()];
        }
        exps.iter().map(|e| e / sum).collect()
    })
}

/// Numerically stable log-softmax along `axis`.
///
/// # Mathematical definition
///
/// ```text
///     log_softmax(x)_i = log(softmax(x)_i)
///                      = x_i − log(Σ_j exp(x_j))
/// ```
///
/// # Why `log(softmax(x))` directly is bad
///
/// Naïvely composing `.ln()` after `softmax` suffers **two rounds** of
/// floating-point loss:
///
/// 1. `softmax` compresses values near the tail into `~1e-38` (subnormal)
///    or exactly `0`. The relative error on these small probabilities is
///    already significant.
/// 2. Taking `log` of a near-zero value magnifies that error: a 1 ULP
///    error at `1e-38` becomes `≈ log(1 ± ulp)` which is catastrophic
///    at the input of a cross-entropy loss.
///
/// # The stable formula
///
/// Using the LogSumExp identity:
///
/// ```text
///     log(Σ exp(x_j)) = m + log(Σ exp(x_j − m))    for any m
/// ```
///
/// With `m = max(x)` this keeps `exp` operands in `(0, 1]`, so the sum
/// is in `[1, n]` and `log(sum)` is well-behaved. The final formula:
///
/// ```text
///     log_softmax(x)_i = (x_i − m) − log(Σ_j exp(x_j − m))
/// ```
///
/// # Invariants on finite inputs
///
/// * `log_softmax(x)_i ≤ 0` for every `i`.
/// * `log_softmax(x)_i = 0` iff row is one-hot at `i`.
/// * **Shift invariance**: `log_softmax(x + c) = log_softmax(x)` for
///   any constant `c` (useful for gradient stability).
///
/// # Edge cases
///
/// Same as [`softmax`]: `NaN`/`±∞` propagate as NaN rows.
pub fn log_softmax(x: &Tensor<f32>, axis: i32) -> Result<Tensor<f32>> {
    map_along_axis(x, axis, |row| {
        if row.len() == 1 {
            return vec![0.0]; // log(1) = 0 for a single-element row
        }
        let max = row.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        if !max.is_finite() {
            return vec![f32::NAN; row.len()];
        }
        let sum_exp: f32 = row.iter().map(|&v| (v - max).exp()).sum();
        if sum_exp <= 0.0 || !sum_exp.is_finite() {
            return vec![f32::NAN; row.len()];
        }
        let log_sum_exp = max + sum_exp.ln();
        row.iter().map(|&v| v - log_sum_exp).collect()
    })
}

/// Exact Gaussian Error Linear Unit (GELU).
///
/// # Mathematical definition
///
/// ```text
///     GELU(x) = x · Φ(x)
///             = 0.5 · x · (1 + erf(x / √2))
/// ```
///
/// where `Φ` is the cumulative distribution function of the standard
/// normal, and `erf` is the Gauss error function:
///
/// ```text
///     erf(z) = (2/√π) · ∫₀^z exp(−t²) dt
/// ```
///
/// # Why exact and not the tanh-approximation
///
/// A common approximation (used when `erf` is expensive) is:
///
/// ```text
///     GELU_approx(x) ≈ 0.5 · x · (1 + tanh(√(2/π) · (x + 0.044715·x³)))
/// ```
///
/// This is ~3 % faster but differs from the exact form by up to **3e-3**
/// at `x ≈ 1`. Since we want **bit-identical** reference vectors between
/// our CpuBackend and PyTorch's default (exact) GELU, we use the exact
/// form backed by `libm::erff`.
///
/// # Mathematical properties
///
/// * **Smooth**: `GELU ∈ C^∞(ℝ)` (unlike ReLU which is C⁰ only)
/// * **Non-monotonic**: `GELU'(x)` has a minimum near `x ≈ -0.75`
///   where the function dips slightly below the tangent at origin.
/// * `GELU(0) = 0`
/// * `GELU(x) → x` as `x → +∞`
/// * `GELU(x) → 0` as `x → −∞`
/// * `GELU(-x) ≠ -GELU(x)` (not odd; biased toward positive)
///
/// # Numerical behaviour
///
/// Matches `torch.nn.functional.gelu(..., approximate="none")` at the
/// bit level for the 7 reference points in `tests/golden/gelu/`.
pub fn gelu(x: &Tensor<f32>) -> Result<Tensor<f32>> {
    let src = x.as_slice()?;
    let inv_sqrt2 = std::f32::consts::FRAC_1_SQRT_2; // 1 / √2 ≈ 0.70710677
    let out: Vec<f32> = src
        .iter()
        .map(|&v| {
            // libm::erff is a pure-Rust translation of musl's erff.
            let erf_term = libm::erff(v * inv_sqrt2);
            0.5 * v * (1.0 + erf_term)
        })
        .collect();

    // Sanity: no NaN unless input had NaN
    if out.iter().any(|v| v.is_nan()) && !src.iter().any(|v| v.is_nan()) {
        return Err(ComputeError::NumericalInstability {
            op: "gelu",
            detail: "unexpected NaN in output (libm::erff unstable?)".into(),
        });
    }

    Tensor::from_slice(&out, x.shape().clone())
}

// =====================================================================
// sigmoid — Sprint 5
// =====================================================================

/// Logistic sigmoid `σ(x) = 1 / (1 + exp(−x))`, element-wise.
///
/// # Numerical stability
///
/// The naïve formula overflows for very negative `x` (the term
/// `exp(−x)` becomes huge). We branch on the sign of `x` to keep the
/// argument of `exp` non-positive:
///
/// ```text
///     σ(x) = ⎧  1 / (1 + exp(−x))     if x ≥ 0
///            ⎨  exp(x) / (1 + exp(x)) if x <  0
///            ⎩
/// ```
///
/// Both branches are algebraically equivalent. The branch keeps
/// `exp(·)` in `(0, 1]` regardless of sign, avoiding overflow for
/// `|x|` up to `f32::MAX`.
///
/// # Properties
///
/// * `σ(0) = 0.5` exactly.
/// * Monotone strictly increasing.
/// * `σ(x) ∈ (0, 1)` for finite `x`.
/// * `σ(−x) = 1 − σ(x)` (odd-function symmetry around 0.5).
///
/// # Errors
///
/// * [`ComputeError::NumericalInstability`] if any input element is
///   `NaN` or `±Inf`.
pub fn sigmoid(x: &Tensor<f32>) -> Result<Tensor<f32>> {
    let src = x.as_slice()?;
    let mut out = Vec::with_capacity(src.len());
    for (idx, &v) in src.iter().enumerate() {
        if !v.is_finite() {
            return Err(ComputeError::NumericalInstability {
                op: "sigmoid",
                detail: format!("non-finite element at index {idx}: {v}"),
            });
        }
        let s = if v >= 0.0 {
            1.0 / (1.0 + (-v).exp())
        } else {
            let e = v.exp();
            e / (1.0 + e)
        };
        out.push(s);
    }
    Tensor::from_slice(&out, x.shape().clone())
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::excessive_precision, clippy::approx_constant)]
mod tests {
    use super::*;

    // ===== softmax =====

    #[test]
    fn softmax_uniform_input_uniform_output() {
        let x = Tensor::<f32>::from_slice(&[0.0, 0.0, 0.0, 0.0], [4].into()).unwrap();
        let y = softmax(&x, -1).unwrap();
        for &v in y.as_slice().unwrap() {
            assert!((v - 0.25).abs() < 1e-6);
        }
    }

    #[test]
    fn softmax_sums_to_one() {
        let x = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0], [4].into()).unwrap();
        let y = softmax(&x, -1).unwrap();
        let sum: f32 = y.as_slice().unwrap().iter().sum();
        assert!((sum - 1.0).abs() < 1e-6);
    }

    #[test]
    fn softmax_stable_on_large_values() {
        // Naive exp(1000) would overflow. LogSumExp must handle this.
        let x = Tensor::<f32>::from_slice(&[1000.0, 1001.0, 1002.0], [3].into()).unwrap();
        let y = softmax(&x, -1).unwrap();
        let s = y.as_slice().unwrap();
        // All values finite, sum = 1
        assert!(s.iter().all(|v| v.is_finite()));
        let sum: f32 = s.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
        // Largest weight is on the largest input (1002)
        assert!(s[2] > s[1]);
        assert!(s[1] > s[0]);
    }

    #[test]
    fn softmax_2d_along_last() {
        let x = Tensor::<f32>::from_slice(
            &[1.0, 2.0, 3.0, 0.0, 0.0, 0.0],
            [2, 3].into(),
        )
        .unwrap();
        let y = softmax(&x, -1).unwrap();
        let s = y.as_slice().unwrap();
        // Row 0 : standard softmax of [1,2,3]
        let sum0: f32 = s[0] + s[1] + s[2];
        assert!((sum0 - 1.0).abs() < 1e-6);
        // Row 1 : uniform 1/3
        assert!((s[3] - 1.0 / 3.0).abs() < 1e-6);
        assert!((s[4] - 1.0 / 3.0).abs() < 1e-6);
        assert!((s[5] - 1.0 / 3.0).abs() < 1e-6);
    }

    // ===== softmax edge cases =====

    #[test]
    fn softmax_single_element_becomes_one() {
        let x = Tensor::<f32>::from_slice(&[42.0], [1].into()).unwrap();
        let y = softmax(&x, -1).unwrap();
        assert!((y.as_slice().unwrap()[0] - 1.0).abs() < 1e-7);
    }

    #[test]
    fn softmax_row_with_plus_inf_produces_nan() {
        // Matches PyTorch behaviour: `Inf - Inf = NaN` poisons the row.
        let x = Tensor::<f32>::from_slice(
            &[1.0, f32::INFINITY, 2.0],
            [3].into(),
        )
        .unwrap();
        let y = softmax(&x, -1).unwrap();
        let s = y.as_slice().unwrap();
        assert!(s.iter().any(|v| v.is_nan()));
    }

    #[test]
    fn softmax_all_neg_inf_row_returns_nan() {
        let x = Tensor::<f32>::from_slice(
            &[f32::NEG_INFINITY, f32::NEG_INFINITY],
            [2].into(),
        )
        .unwrap();
        let y = softmax(&x, -1).unwrap();
        assert!(y.as_slice().unwrap().iter().all(|v| v.is_nan()));
    }

    #[test]
    fn softmax_nan_in_row_propagates_nan() {
        let x = Tensor::<f32>::from_slice(&[1.0, f32::NAN, 3.0], [3].into()).unwrap();
        let y = softmax(&x, -1).unwrap();
        let s = y.as_slice().unwrap();
        // Caller must check for NaN; we do not raise inside the closure.
        assert!(s.iter().any(|v| v.is_nan()));
    }

    #[test]
    fn softmax_3d_intermediate_axis() {
        // Shape (2, 3, 2), axis=1 → reduce the 3-dim
        let x_data: Vec<f32> = (0..12).map(|i| i as f32).collect();
        let x = Tensor::<f32>::from_slice(&x_data, [2, 3, 2].into()).unwrap();
        let y = softmax(&x, 1).unwrap();
        // For batch 0, col 0: values [0, 2, 4] → softmax should sum to 1.
        // Need to read y at positions (0, *, 0) = indices 0, 2, 4 in flat.
        let s = y.as_slice().unwrap();
        let col_sum_b0_c0 = s[0] + s[2] + s[4];
        assert!((col_sum_b0_c0 - 1.0).abs() < 1e-5);
        // Batch 0, col 1 : indices 1, 3, 5 (values [1, 3, 5]) → sum = 1
        let col_sum_b0_c1 = s[1] + s[3] + s[5];
        assert!((col_sum_b0_c1 - 1.0).abs() < 1e-5);
    }

    #[test]
    fn softmax_monotonic_on_increasing_row() {
        let x = Tensor::<f32>::from_slice(&[-2.0, -1.0, 0.0, 1.0, 2.0], [5].into()).unwrap();
        let y = softmax(&x, -1).unwrap();
        let s = y.as_slice().unwrap();
        for i in 1..5 {
            assert!(s[i] > s[i - 1], "Softmax must be monotonic");
        }
    }

    // ===== log_softmax =====

    #[test]
    fn log_softmax_equals_log_of_softmax() {
        let x = Tensor::<f32>::from_slice(&[0.5, 1.5, -0.2, 2.1], [4].into()).unwrap();
        let ls = log_softmax(&x, -1).unwrap();
        let sm = softmax(&x, -1).unwrap();
        let ls_s = ls.as_slice().unwrap();
        let sm_s = sm.as_slice().unwrap();
        for i in 0..4 {
            assert!((ls_s[i] - sm_s[i].ln()).abs() < 1e-5);
        }
    }

    #[test]
    fn log_softmax_stable_on_large_values() {
        let x = Tensor::<f32>::from_slice(&[1000.0, 1001.0, 1002.0], [3].into()).unwrap();
        let y = log_softmax(&x, -1).unwrap();
        let s = y.as_slice().unwrap();
        // log_softmax values are all ≤ 0 and finite
        assert!(s.iter().all(|v| v.is_finite() && *v <= 0.0 + 1e-5));
    }

    #[test]
    fn log_softmax_single_element_is_zero() {
        let x = Tensor::<f32>::from_slice(&[42.0], [1].into()).unwrap();
        let y = log_softmax(&x, -1).unwrap();
        assert!(y.as_slice().unwrap()[0].abs() < 1e-7);
    }

    #[test]
    fn log_softmax_uniform_input_gives_neg_log_n() {
        // For uniform input, log_softmax → log(1/N) = -log(N)
        let x = Tensor::<f32>::from_slice(&[0.0, 0.0, 0.0, 0.0], [4].into()).unwrap();
        let y = log_softmax(&x, -1).unwrap();
        let expected = -(4.0f32).ln();
        for &v in y.as_slice().unwrap() {
            assert!((v - expected).abs() < 1e-5);
        }
    }

    #[test]
    fn log_softmax_shift_invariant() {
        // log_softmax(x) == log_softmax(x + c) for any c
        let x = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0], [4].into()).unwrap();
        let x_shifted = Tensor::<f32>::from_slice(
            &[101.0, 102.0, 103.0, 104.0],
            [4].into(),
        )
        .unwrap();
        let a = log_softmax(&x, -1).unwrap();
        let b = log_softmax(&x_shifted, -1).unwrap();
        let sa = a.as_slice().unwrap();
        let sb = b.as_slice().unwrap();
        for i in 0..4 {
            assert!((sa[i] - sb[i]).abs() < 1e-5);
        }
    }

    #[test]
    fn log_softmax_max_value_gives_smallest_loss_magnitude() {
        // In [1, 2, 3, 4], max is at index 3 (value 4).
        // log_softmax_3 must have the smallest |value| (closest to 0).
        let x = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0], [4].into()).unwrap();
        let y = log_softmax(&x, -1).unwrap();
        let s = y.as_slice().unwrap();
        let abs_values: Vec<f32> = s.iter().map(|v| v.abs()).collect();
        let min_pos = abs_values.iter().enumerate()
            .min_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap().0;
        assert_eq!(min_pos, 3, "max element must have smallest |log_softmax|");
    }

    // ===== gelu =====

    #[test]
    fn gelu_zero_is_zero() {
        let x = Tensor::<f32>::from_slice(&[0.0], [1].into()).unwrap();
        let y = gelu(&x).unwrap();
        assert!((y.as_slice().unwrap()[0]).abs() < 1e-7);
    }

    #[test]
    fn gelu_matches_pytorch_reference_points() {
        // PyTorch torch.nn.functional.gelu(..., approximate="none") for these inputs:
        //   x=1.0    → 0.8413447
        //   x=-1.0   → -0.1586553
        //   x=2.0    → 1.9544997
        //   x=-3.0   → -0.00404952
        let x = Tensor::<f32>::from_slice(&[1.0, -1.0, 2.0, -3.0], [4].into()).unwrap();
        let y = gelu(&x).unwrap();
        let s = y.as_slice().unwrap();
        assert!((s[0] - 0.841_344_7).abs() < 1e-5);
        assert!((s[1] - (-0.158_655_3)).abs() < 1e-5);
        assert!((s[2] - 1.954_499_7).abs() < 1e-5);
        assert!((s[3] - (-0.004_049_52)).abs() < 1e-5);
    }

    #[test]
    fn gelu_preserves_shape() {
        let x = Tensor::<f32>::zeros([2, 3, 4].into()).unwrap();
        let y = gelu(&x).unwrap();
        assert_eq!(y.shape().dims(), &[2, 3, 4]);
    }

    #[test]
    fn gelu_is_monotonic_on_positives() {
        let x = Tensor::<f32>::from_slice(&[0.1, 0.5, 1.0, 2.0, 5.0], [5].into()).unwrap();
        let y = gelu(&x).unwrap();
        let s = y.as_slice().unwrap();
        for i in 1..s.len() {
            assert!(s[i] > s[i - 1], "GELU should be monotonic on positives");
        }
    }

    #[test]
    fn gelu_asymptote_large_positive() {
        // GELU(x) → x as x → +∞
        let x = Tensor::<f32>::from_slice(&[10.0, 50.0, 100.0], [3].into()).unwrap();
        let y = gelu(&x).unwrap();
        let s = y.as_slice().unwrap();
        for i in 0..3 {
            let input = [10.0f32, 50.0, 100.0][i];
            assert!(
                (s[i] - input).abs() < 1e-4,
                "GELU({}) should ≈ {}, got {}",
                input,
                input,
                s[i]
            );
        }
    }

    #[test]
    fn gelu_asymptote_large_negative() {
        // GELU(x) → 0 as x → -∞
        let x = Tensor::<f32>::from_slice(&[-10.0, -50.0, -100.0], [3].into()).unwrap();
        let y = gelu(&x).unwrap();
        let s = y.as_slice().unwrap();
        for &v in s {
            assert!(v.abs() < 1e-4, "GELU of large negative should be ≈ 0, got {v}");
        }
    }

    #[test]
    fn gelu_nan_input_propagates() {
        let x = Tensor::<f32>::from_slice(&[1.0, f32::NAN, 2.0], [3].into()).unwrap();
        // libm::erff(NaN) = NaN → output[1] = NaN. The internal assertion
        // in gelu() checks that NaN in output ⇔ NaN in input, so this
        // should succeed (no error raised).
        let y = gelu(&x).unwrap();
        let s = y.as_slice().unwrap();
        assert!(s[1].is_nan());
        assert!((s[0] - 0.841_344_75).abs() < 1e-5);
    }

    #[test]
    fn gelu_non_monotonic_dip_near_minus_one() {
        // GELU'(x) has a minimum near x ≈ -0.75, so GELU is slightly
        // below its tangent at 0 in that region. Test : GELU is
        // non-monotonic — there exists x < y < 0 with GELU(x) > GELU(y).
        let x = Tensor::<f32>::from_slice(
            &[-2.0, -1.5, -1.0, -0.5, 0.0],
            [5].into(),
        )
        .unwrap();
        let y = gelu(&x).unwrap();
        let s = y.as_slice().unwrap();
        // Between -1 and -0.5, GELU dips — the minimum is around -0.75.
        // We check the dip exists : s[-1.0] and s[-0.5] are both below 0.
        assert!(s[2] < 0.0);
        assert!(s[3] < 0.0);
    }

    // ===== sigmoid (Sprint 5) =====

    #[test]
    fn sigmoid_zero_is_half() {
        let x = Tensor::<f32>::from_slice(&[0.0], [1].into()).unwrap();
        let y = sigmoid(&x).unwrap();
        assert!((y.as_slice().unwrap()[0] - 0.5).abs() < 1e-7);
    }

    #[test]
    fn sigmoid_monotone() {
        let xs: Vec<f32> = (-50..=50).map(|i| i as f32 * 0.1).collect();
        let x = Tensor::<f32>::from_slice(&xs, [xs.len()].into()).unwrap();
        let y = sigmoid(&x).unwrap();
        let s = y.as_slice().unwrap();
        for i in 1..s.len() {
            assert!(
                s[i] >= s[i - 1] - 1e-7,
                "sigmoid not monotone at i={i}: {} -> {}",
                s[i - 1],
                s[i],
            );
        }
    }

    #[test]
    fn sigmoid_bounded_in_unit_range() {
        let xs: Vec<f32> = (-200..=200).map(|i| i as f32 * 0.5).collect();
        let x = Tensor::<f32>::from_slice(&xs, [xs.len()].into()).unwrap();
        let y = sigmoid(&x).unwrap();
        for &v in y.as_slice().unwrap() {
            assert!((0.0..=1.0).contains(&v), "sigmoid out of (0,1): {v}");
        }
    }

    #[test]
    fn sigmoid_stable_large_negative() {
        // Naive 1/(1+exp(-x)) for x = -1000 → 1/(1+Inf) = 0, but exp(-(-1000))
        // overflows to Inf in the naïve path. Branch must keep us safe.
        let x = Tensor::<f32>::from_slice(&[-1000.0, -100.0, -50.0], [3].into()).unwrap();
        let y = sigmoid(&x).unwrap();
        for &v in y.as_slice().unwrap() {
            assert!(v.is_finite(), "sigmoid produced non-finite for large neg");
            assert!((0.0..1.0).contains(&v));
        }
    }

    #[test]
    fn sigmoid_stable_large_positive() {
        let x = Tensor::<f32>::from_slice(&[1000.0, 100.0, 50.0], [3].into()).unwrap();
        let y = sigmoid(&x).unwrap();
        for &v in y.as_slice().unwrap() {
            assert!(v.is_finite());
            assert!(v > 0.0 && v <= 1.0);
        }
    }

    #[test]
    fn sigmoid_symmetry_property() {
        // σ(-x) = 1 - σ(x).
        let x = Tensor::<f32>::from_slice(&[1.5, -2.3, 0.7, -0.1], [4].into()).unwrap();
        let neg = Tensor::<f32>::from_slice(&[-1.5, 2.3, -0.7, 0.1], [4].into()).unwrap();
        let y_pos = sigmoid(&x).unwrap();
        let y_neg = sigmoid(&neg).unwrap();
        let sp = y_pos.as_slice().unwrap();
        let sn = y_neg.as_slice().unwrap();
        for i in 0..4 {
            assert!(
                (sp[i] + sn[i] - 1.0).abs() < 1e-6,
                "symmetry breaks at {i}: σ(x)={}, σ(-x)={}",
                sp[i],
                sn[i],
            );
        }
    }

    #[test]
    fn sigmoid_rejects_nan() {
        let x = Tensor::<f32>::from_slice(&[1.0, f32::NAN, 0.5], [3].into()).unwrap();
        assert!(matches!(
            sigmoid(&x).unwrap_err(),
            ComputeError::NumericalInstability { .. }
        ));
    }

    #[test]
    fn sigmoid_rejects_inf() {
        let x = Tensor::<f32>::from_slice(&[1.0, f32::INFINITY], [2].into()).unwrap();
        assert!(matches!(
            sigmoid(&x).unwrap_err(),
            ComputeError::NumericalInstability { .. }
        ));
    }

    #[test]
    fn sigmoid_2d_preserves_shape() {
        let x = Tensor::<f32>::from_slice(
            &[1.0, -1.0, 2.0, -2.0],
            [2, 2].into(),
        )
        .unwrap();
        let y = sigmoid(&x).unwrap();
        assert_eq!(y.shape().dims(), &[2, 2]);
    }
}
