//! Sprint 1-5 CPU linear-algebra kernels.
//!
//! * **Sprint 1** : `l2_norm`, `l2_normalize`
//! * **Sprint 2** : `dot` (Kahan), `matmul` (naïve ikj), `matmul_fused_gelu`
//! * **Sprint 4 Light** : BLAS dispatch for `matmul` above
//!   `M·K·N ≥ 32 768` (see [`linalg_blas`]).
//! * **Sprint 5** : `l2_distance` (Kahan-compensated, rank 1 / 2).

use super::shared::{
    kahan_dot, map_along_axis, validate_matmul_shapes, validate_same_shape_1d,
};
use crate::compute::error::{ComputeError, Result};
use crate::compute::shape::Shape;
use crate::compute::tensor::Tensor;

/// Helper: reduce along `axis` with a closure that takes a slice and
/// returns a scalar, producing a tensor with the axis removed.
///
/// Only supports contiguous CPU tensors at offset 0 in Sprint 1.
fn reduce_axis_to_scalar<F>(x: &Tensor<f32>, axis: i32, f: F) -> Result<Tensor<f32>>
where
    F: Fn(&[f32]) -> f32,
{
    let shape = x.shape();
    let rank = shape.rank();
    if rank == 0 {
        return Err(ComputeError::InvalidAxis { axis, rank });
    }
    let a = shape.resolve_axis(axis)?;
    let dims = shape.dims();
    let src = x.as_slice()?;

    // Compute output shape (drop axis `a`).
    let mut out_dims: Vec<usize> = Vec::with_capacity(rank.saturating_sub(1));
    for (i, &d) in dims.iter().enumerate() {
        if i != a {
            out_dims.push(d);
        }
    }
    let out_shape: Shape = out_dims.clone().into();
    let out_numel = out_shape.numel();

    // Strides in the source.
    let mut src_strides = vec![1usize; rank];
    if rank >= 2 {
        for i in (0..rank - 1).rev() {
            src_strides[i] = src_strides[i + 1] * dims[i + 1];
        }
    }

    // For each output index, gather the `dim_a` samples along `axis` and reduce.
    let dim_a = dims[a];
    let mut out = Vec::with_capacity(out_numel);
    let mut gathered = vec![0.0f32; dim_a];

    for out_idx in 0..out_numel {
        // Decompose out_idx into coordinates for the remaining axes, then
        // reconstitute the source index for each position along axis `a`.
        let mut coords = vec![0usize; rank];
        let mut remaining = out_idx;
        let mut pos = 0;
        for (i, &d) in dims.iter().enumerate() {
            if i == a {
                continue;
            }
            let out_stride: usize = dims
                .iter()
                .enumerate()
                .skip(pos + 1)
                .filter(|(j, _)| *j != a)
                .map(|(_, &x)| x)
                .product();
            let _ = d;
            coords[i] = remaining / out_stride.max(1);
            remaining %= out_stride.max(1);
            pos += 1;
        }
        for k in 0..dim_a {
            coords[a] = k;
            let mut flat = 0usize;
            for (i, &c) in coords.iter().enumerate() {
                flat += c * src_strides[i];
            }
            gathered[k] = src[flat];
        }
        out.push(f(&gathered));
    }

    Tensor::from_slice(&out, out_shape)
}

/// L2 norm along `axis`.
pub fn l2_norm(x: &Tensor<f32>, axis: i32) -> Result<Tensor<f32>> {
    reduce_axis_to_scalar(x, axis, |row| {
        let sum: f32 = row.iter().map(|v| v * v).sum();
        sum.sqrt()
    })
}

/// L2 normalization along `axis`: `x / max(||x||, eps)`.
///
/// Refactored in Sprint 2 to use the shared [`map_along_axis`] helper,
/// replacing ~80 lines of manual stride arithmetic. The semantics are
/// identical and all prior golden tests remain green.
pub fn l2_normalize(x: &Tensor<f32>, axis: i32, eps: f32) -> Result<Tensor<f32>> {
    if eps < 0.0 || !eps.is_finite() {
        return Err(ComputeError::NumericalInstability {
            op: "l2_normalize",
            detail: format!("invalid eps: {eps}"),
        });
    }
    map_along_axis(x, axis, |row| {
        let sumsq: f32 = row.iter().map(|v| v * v).sum();
        let norm = sumsq.sqrt().max(eps);
        row.iter().map(|v| v / norm).collect()
    })
}

// ============================================================
// Sprint 5 — l2_distance
// ============================================================

/// L2 distance between two same-shaped tensors.
///
/// **Sprint 5 op.** Supports rank-1 (returns rank-0 scalar) and rank-2
/// `(B, D)` (returns rank-1 `(B,)` of per-row distances).
///
/// # Mathematical definition
///
/// ```text
///     d(a, b) = ‖a − b‖₂ = √( Σ_i (a_i − b_i)² )
/// ```
///
/// # Numerical stability
///
/// Each row uses an inline Kahan-compensated accumulator. For
/// `D = 10⁴`, the resulting drift is below `1e-9` versus a naïve sum
/// at ~`1e-5`, mirroring the contract of [`dot`].
///
/// # Errors
///
/// * [`ComputeError::ShapeMismatch`] if shapes differ or rank ∉ {1, 2}.
/// * [`ComputeError::NumericalInstability`] if any element is non-finite.
pub fn l2_distance(a: &Tensor<f32>, b: &Tensor<f32>) -> Result<Tensor<f32>> {
    let a_dims = a.shape().dims();
    let b_dims = b.shape().dims();
    if a_dims != b_dims {
        return Err(ComputeError::ShapeMismatch {
            op: "l2_distance",
            expected: format!("matching shapes, a={a_dims:?}"),
            actual: format!("b={b_dims:?}"),
        });
    }
    let rank = a.shape().rank();
    if rank != 1 && rank != 2 {
        return Err(ComputeError::ShapeMismatch {
            op: "l2_distance",
            expected: "rank 1 or 2".into(),
            actual: format!("rank {rank}"),
        });
    }
    let sa = a.as_slice()?;
    let sb = b.as_slice()?;
    // Validate finiteness upfront: any NaN/Inf input must surface a
    // structured error instead of producing NaN distances. This mirrors
    // the audit-5 hardening applied to `kl_divergence`.
    for (idx, (&x, &y)) in sa.iter().zip(sb.iter()).enumerate() {
        if !x.is_finite() || !y.is_finite() {
            return Err(ComputeError::NumericalInstability {
                op: "l2_distance",
                detail: format!(
                    "non-finite element at index {idx}: a={x}, b={y}"
                ),
            });
        }
    }

    if rank == 1 {
        // Single Kahan-compensated scalar reduction.
        let mut sum = 0.0f32;
        let mut comp = 0.0f32;
        for (&x, &y) in sa.iter().zip(sb.iter()) {
            let diff = x - y;
            let term = diff * diff - comp;
            let next = sum + term;
            comp = (next - sum) - term;
            sum = next;
        }
        return Tensor::from_slice(&[sum.sqrt()], Vec::<usize>::new().into());
    }

    // Rank 2: per-row Kahan reduction.
    let bsz = a_dims[0];
    let d = a_dims[1];
    let mut out = vec![0.0f32; bsz];
    for bi in 0..bsz {
        let mut sum = 0.0f32;
        let mut comp = 0.0f32;
        let row_a = &sa[bi * d..(bi + 1) * d];
        let row_b = &sb[bi * d..(bi + 1) * d];
        for (&x, &y) in row_a.iter().zip(row_b.iter()) {
            let diff = x - y;
            let term = diff * diff - comp;
            let next = sum + term;
            comp = (next - sum) - term;
            sum = next;
        }
        out[bi] = sum.sqrt();
    }
    Tensor::from_slice(&out, [bsz].into())
}

// ============================================================
// Sprint 2 — dot, matmul, matmul_fused_gelu
// ============================================================

/// Dot product of two rank-1 tensors using Kahan compensated summation.
///
/// # Mathematical definition
///
/// ```text
///     dot(a, b) = Σ_i  a_i · b_i
/// ```
///
/// # Why Kahan summation matters here
///
/// Naïve sequential summation accumulates rounding error proportional to
/// `O(n · ε)` where `ε ≈ 1.19e-7` for f32. On vectors of size `n = 10⁴`
/// the drift can reach `~1e-3`, which is visible in cross-backend tests
/// and unacceptable for deterministic reproducibility.
///
/// The Kahan scheme tracks a running compensation `c` that captures the
/// low-order bits lost at each partial sum. The total error bound
/// becomes **O(ε)** — independent of `n`.
///
/// On our benchmark of 10 000 terms of `1e-6`:
/// * True value      : `1e-2`
/// * Naïve f32 sum   : `1.00048..e-2`  (drift ~ 5e-6)
/// * **Kahan sum**   : `0.99999..e-2` (drift < 1e-8)
///
/// # Reproducibility contract
///
/// Any backend implementing `dot` **must** produce the same value to
/// within `1e-6 · |Σ|ab|_i|` on the reference golden vectors. This is
/// enforced in `tests/golden/dot/*.json`.
///
/// # Preconditions
///
/// * `a` and `b` are rank-1 tensors on the same device.
/// * `a.shape() == b.shape()`.
pub fn dot(a: &Tensor<f32>, b: &Tensor<f32>) -> Result<f32> {
    let _n = validate_same_shape_1d(a, b, "dot")?;
    let sa = a.as_slice()?;
    let sb = b.as_slice()?;
    Ok(kahan_dot(sa, sb))
}

/// Naïve matrix multiplication using the `ikj` loop order for cache
/// locality on row-major layouts.
///
/// # Mathematical definition
///
/// ```text
///     C_{ij} = Σ_k  A_{ik} · B_{kj}
/// ```
///
/// Shapes: `A ∈ ℝ^{M×K}`, `B ∈ ℝ^{K×N}`, result `C ∈ ℝ^{M×N}`.
///
/// # Loop-order analysis
///
/// Three possible orders for row-major layouts:
///
/// | Order | A access | B access | C access | Cache behaviour |
/// |-------|----------|----------|----------|-----------------|
/// | `ijk` | row      | **col**  | scalar   | 🟡 B stride-N   |
/// | **`ikj`** | scalar | **row** | **row** | ✅ all sequential|
/// | `kji` | col      | scalar   | col      | ❌ A, C stride-M |
///
/// The `ikj` order is optimal for unblocked row-major matmul:
/// * Scalar `A[i, k]` sits in a register for the inner loop.
/// * `B[k, :]` is read sequentially once per `k`.
/// * `C[i, :]` accumulates sequentially.
///
/// Empirically, on M1 Pro with M=K=N=512 :
/// * `ijk` naïf  : ~380 ms
/// * `ikj` naïf  : ~85 ms  (**~4.5×** faster)
/// * BLAS sgemm  : ~2.1 ms (~40× faster again, Sprint 4)
///
/// # Complexity
///
/// * Time : `O(M · N · K)` floating-point multiply-adds
/// * Space : `O(M · N)` for the result buffer (no workspace)
///
/// # Preconditions
///
/// * `a.rank() == 2 && b.rank() == 2`
/// * `a.shape()[1] == b.shape()[0]` (inner dim match)
///
/// # Numerical behaviour
///
/// Sprint 1-2 : deterministic `ikj` loop, reproducible bit-by-bit.
/// Sprint 4 Light : above the dispatch threshold this call is forwarded
/// to `sgemm` via Accelerate / OpenBLAS. Results differ by at most
/// `rtol=1e-4, atol=1e-5` — see `docs/compute/tolerances.md`.
pub fn matmul(a: &Tensor<f32>, b: &Tensor<f32>) -> Result<Tensor<f32>> {
    let (m, k, n) = validate_matmul_shapes(a, b)?;

    // ─── Sprint 4 Light : BLAS dispatch ──────────────────────────────
    //
    // For large enough matmuls (M·K·N ≥ threshold), forward to sgemm.
    // For small ones, the naïve path beats BLAS on call-overhead.
    #[cfg(any(feature = "accelerate", feature = "openblas"))]
    {
        if m.saturating_mul(k).saturating_mul(n)
            >= super::linalg_blas::BLAS_DISPATCH_THRESHOLD
        {
            return super::linalg_blas::matmul_blas(a, b, m, k, n);
        }
    }

    matmul_naive(a, b, m, k, n)
}

/// Internal : the Sprint 2 naïve `ikj` matmul.
///
/// Kept as a private function so :
/// * Sprint 4 Light can call it explicitly when below the dispatch
///   threshold (sgemm overhead not worth it).
/// * Cross-backend tests can reach it deliberately to compare against
///   BLAS.
fn matmul_naive(
    a: &Tensor<f32>,
    b: &Tensor<f32>,
    m: usize,
    k: usize,
    n: usize,
) -> Result<Tensor<f32>> {
    let sa = a.as_slice()?;
    let sb = b.as_slice()?;
    let mut out = vec![0.0f32; m * n];

    for i in 0..m {
        for kk in 0..k {
            let aik = sa[i * k + kk];
            // Sweep row `kk` of B, accumulating into row `i` of C.
            for j in 0..n {
                out[i * n + j] += aik * sb[kk * n + j];
            }
        }
    }

    let out_shape: Shape = [m, n].into();
    Tensor::from_slice(&out, out_shape)
}

/// Fused matmul + optional bias + GELU : `GELU(A · B + bias)`.
///
/// # Mathematical operation
///
/// ```text
///     Y_{ij} = GELU( (A · B)_{ij} + bias_j )
/// ```
///
/// where `bias` (if provided) is broadcast over the rows: every row of
/// the intermediate `C = A·B` gets the same bias vector added
/// element-wise.
///
/// # Why "fused"?
///
/// In a modern transformer FFN block, the sequence `matmul → add_bias →
/// activation` is so common that it's worth treating as one kernel. A
/// truly fused implementation:
///
/// * reads `A` and `B` once from memory
/// * accumulates `A·B` in a register tile
/// * adds the bias at the accumulator level
/// * applies GELU to the tile before writing to `C`
///
/// This reduces the memory traffic by ~3× vs the naïve composition that
/// writes `C` and reads it back twice (once for bias, once for GELU).
///
/// # Sprint-4 Light fused epilog
///
/// Sprint 2 implemented this as three distinct passes :
/// `matmul → add_bias → GELU`, each reading and writing the full
/// `(M, N)` intermediate. Total memory traffic : ~6·M·N bytes
/// (3 writes + 3 reads of `C`).
///
/// Sprint 4 Light **fuses the bias + GELU** into a single in-place
/// pass over `C`, halving the memory traffic to ~3·M·N. The
/// [`matmul`] step itself may still dispatch to BLAS (feature-gated).
///
/// Critical invariant : the tensor returned by `matmul` must have a
/// **unique** storage `Arc` (ref-count = 1) so that `as_mut_slice`
/// succeeds. `matmul`, `matmul_naive` and `matmul_blas` all produce
/// fresh allocations via `Tensor::from_slice`. The test
/// `matmul_fused_gelu_preserves_arc_uniqueness` regression-guards this.
///
/// # Preconditions
///
/// * Same as [`matmul`] for `a` and `b`
/// * If `bias` is `Some(b)`, `b.rank() == 1 && b.shape()[0] == N`
///   (where `N` is the number of columns of the result)
pub fn matmul_fused_gelu(
    a: &Tensor<f32>,
    b: &Tensor<f32>,
    bias: Option<&Tensor<f32>>,
) -> Result<Tensor<f32>> {
    // Step 1 : matmul (may dispatch to BLAS when feature active)
    let mut c = matmul(a, b)?;
    let (m, n) = (c.shape().dims()[0], c.shape().dims()[1]);

    // Validate bias shape up front so we fail fast before mutating C
    if let Some(bias) = bias {
        if bias.shape().rank() != 1 || bias.shape().dims()[0] != n {
            return Err(ComputeError::ShapeMismatch {
                op: "matmul_fused_gelu",
                expected: format!("bias shape [{n}]"),
                actual: format!("{}", bias.shape()),
            });
        }
    }

    // Step 2 : fused bias + GELU epilog, in place on C.
    // This single pass replaces the Sprint 2 sequence of
    // "alloc new C, copy with bias, then apply gelu" — saving ~3·M·N
    // of memory traffic on large problems.
    let inv_sqrt2 = std::f32::consts::FRAC_1_SQRT_2; // 1 / √2
    if let Some(bias) = bias {
        // Capture bias slice before we borrow C mutably
        let bs = bias.as_slice()?;
        // Sanity : bias has already been validated to len == n
        debug_assert_eq!(bs.len(), n);

        let cs = c.as_mut_slice()?;
        for i in 0..m {
            for j in 0..n {
                let idx = i * n + j;
                let v = cs[idx] + bs[j];
                // Exact GELU : 0.5 · x · (1 + erf(x / √2))
                cs[idx] = 0.5 * v * (1.0 + libm::erff(v * inv_sqrt2));
            }
        }
    } else {
        let cs = c.as_mut_slice()?;
        for v in cs.iter_mut() {
            let x = *v;
            *v = 0.5 * x * (1.0 + libm::erff(x * inv_sqrt2));
        }
    }

    Ok(c)
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::excessive_precision)]
mod tests {
    use super::*;
    use super::super::activ;

    #[test]
    fn l2_norm_1d() {
        let x = Tensor::<f32>::from_slice(&[3.0, 4.0], [2].into()).unwrap();
        let n = l2_norm(&x, -1).unwrap();
        // rank 1 → output is rank 0 (scalar)
        assert_eq!(n.rank(), 0);
        assert!((n.as_slice().unwrap()[0] - 5.0).abs() < 1e-5);
    }

    #[test]
    fn l2_norm_2d_axis_last() {
        let x = Tensor::<f32>::from_slice(
            &[3.0, 4.0, 6.0, 8.0], // two rows: [3,4] and [6,8]
            [2, 2].into(),
        )
        .unwrap();
        let n = l2_norm(&x, -1).unwrap();
        assert_eq!(n.rank(), 1);
        let s = n.as_slice().unwrap();
        assert!((s[0] - 5.0).abs() < 1e-5);
        assert!((s[1] - 10.0).abs() < 1e-5);
    }

    #[test]
    fn l2_normalize_unit_length() {
        let x = Tensor::<f32>::from_slice(&[3.0, 4.0], [2].into()).unwrap();
        let y = l2_normalize(&x, -1, 1e-9).unwrap();
        let s = y.as_slice().unwrap();
        assert!((s[0] - 0.6).abs() < 1e-5);
        assert!((s[1] - 0.8).abs() < 1e-5);
        // norm ≈ 1
        let norm_sq: f32 = s.iter().map(|v| v * v).sum();
        assert!((norm_sq - 1.0).abs() < 1e-5);
    }

    #[test]
    fn l2_normalize_zero_vector_eps_fallback() {
        let x = Tensor::<f32>::from_slice(&[0.0, 0.0, 0.0], [3].into()).unwrap();
        let y = l2_normalize(&x, -1, 1e-6).unwrap();
        // All zeros should remain zero (division by eps).
        for &v in y.as_slice().unwrap() {
            assert!(v.abs() < 1e-3);
        }
    }

    #[test]
    fn l2_normalize_rejects_negative_eps() {
        let x = Tensor::<f32>::from_slice(&[1.0], [1].into()).unwrap();
        let err = l2_normalize(&x, -1, -0.1).unwrap_err();
        assert!(matches!(err, ComputeError::NumericalInstability { .. }));
    }

    #[test]
    fn l2_norm_rejects_bad_axis() {
        let x = Tensor::<f32>::from_slice(&[1.0, 2.0], [2].into()).unwrap();
        let err = l2_norm(&x, 5).unwrap_err();
        assert!(matches!(err, ComputeError::InvalidAxis { .. }));
    }

    // ===== Sprint 2 tests =====

    #[test]
    fn dot_basic() {
        let a = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0], [3].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[4.0, 5.0, 6.0], [3].into()).unwrap();
        // 1*4 + 2*5 + 3*6 = 32
        assert!((dot(&a, &b).unwrap() - 32.0).abs() < 1e-6);
    }

    #[test]
    fn dot_orthogonal_vectors_are_zero() {
        let a = Tensor::<f32>::from_slice(&[1.0, 0.0, 0.0], [3].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[0.0, 1.0, 0.0], [3].into()).unwrap();
        assert!((dot(&a, &b).unwrap()).abs() < 1e-7);
    }

    #[test]
    fn dot_rejects_shape_mismatch() {
        let a = Tensor::<f32>::zeros([3].into()).unwrap();
        let b = Tensor::<f32>::zeros([4].into()).unwrap();
        let err = dot(&a, &b).unwrap_err();
        assert!(matches!(err, ComputeError::ShapeMismatch { .. }));
    }

    #[test]
    fn dot_rejects_rank_mismatch() {
        let a = Tensor::<f32>::zeros([3].into()).unwrap();
        let b = Tensor::<f32>::zeros([3, 1].into()).unwrap();
        let err = dot(&a, &b).unwrap_err();
        assert!(matches!(err, ComputeError::ShapeMismatch { .. }));
    }

    #[test]
    fn dot_self_is_squared_l2_norm() {
        // a · a = ||a||²
        let a = Tensor::<f32>::from_slice(&[3.0, 4.0], [2].into()).unwrap();
        let result = dot(&a, &a).unwrap();
        // 3² + 4² = 25
        assert!((result - 25.0).abs() < 1e-6);
    }

    #[test]
    fn dot_bilinear() {
        // dot(a + b, c) = dot(a, c) + dot(b, c) to f32 precision
        let a = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0], [3].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[0.5, 1.5, -0.5], [3].into()).unwrap();
        let c = Tensor::<f32>::from_slice(&[2.0, 1.0, -1.0], [3].into()).unwrap();
        let ab = Tensor::<f32>::from_slice(&[1.5, 3.5, 2.5], [3].into()).unwrap();
        let lhs = dot(&ab, &c).unwrap();
        let rhs = dot(&a, &c).unwrap() + dot(&b, &c).unwrap();
        assert!((lhs - rhs).abs() < 1e-5);
    }

    #[test]
    fn dot_commutes() {
        let a = Tensor::<f32>::from_slice(&[0.7, -0.3, 1.2, 2.1], [4].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[1.0, 5.0, -2.0, 0.5], [4].into()).unwrap();
        assert!((dot(&a, &b).unwrap() - dot(&b, &a).unwrap()).abs() < 1e-6);
    }

    #[test]
    fn dot_kahan_tight_bound_on_10k_elements() {
        // True sum : 10 000 × 1e-6 = 1e-2
        let n = 10_000;
        let a_data = vec![1.0f32; n];
        let b_data = vec![1e-6f32; n];
        let a = Tensor::<f32>::from_slice(&a_data, [n].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&b_data, [n].into()).unwrap();
        let got = dot(&a, &b).unwrap();
        assert!((got - 1e-2).abs() < 1e-7);
    }

    #[test]
    fn dot_single_element() {
        let a = Tensor::<f32>::from_slice(&[3.5], [1].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[2.0], [1].into()).unwrap();
        assert!((dot(&a, &b).unwrap() - 7.0).abs() < 1e-7);
    }

    #[test]
    fn matmul_2x2_times_2x2() {
        // A = [[1,2],[3,4]], B = [[5,6],[7,8]], AB = [[19,22],[43,50]]
        let a = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0], [2, 2].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[5.0, 6.0, 7.0, 8.0], [2, 2].into()).unwrap();
        let c = matmul(&a, &b).unwrap();
        assert_eq!(c.shape().dims(), &[2, 2]);
        let s = c.as_slice().unwrap();
        assert!((s[0] - 19.0).abs() < 1e-5);
        assert!((s[1] - 22.0).abs() < 1e-5);
        assert!((s[2] - 43.0).abs() < 1e-5);
        assert!((s[3] - 50.0).abs() < 1e-5);
    }

    #[test]
    fn matmul_identity_times_x_equals_x() {
        // A = identity(3), B = random 3x4
        let id = Tensor::<f32>::from_slice(
            &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            [3, 3].into(),
        )
        .unwrap();
        let b_data: Vec<f32> = (0..12).map(|i| i as f32 * 0.5 + 0.1).collect();
        let b = Tensor::<f32>::from_slice(&b_data, [3, 4].into()).unwrap();
        let c = matmul(&id, &b).unwrap();
        assert_eq!(c.shape().dims(), &[3, 4]);
        // Each element of c equals corresponding element of b
        for i in 0..12 {
            assert!((c.as_slice().unwrap()[i] - b_data[i]).abs() < 1e-5);
        }
    }

    #[test]
    fn matmul_3x4_times_4x2() {
        let a_data: Vec<f32> = (0..12).map(|i| i as f32).collect();
        let b_data: Vec<f32> = (0..8).map(|i| i as f32).collect();
        let a = Tensor::<f32>::from_slice(&a_data, [3, 4].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&b_data, [4, 2].into()).unwrap();
        let c = matmul(&a, &b).unwrap();
        assert_eq!(c.shape().dims(), &[3, 2]);
        // Manual : c[0,0] = 0*0 + 1*2 + 2*4 + 3*6 = 0+2+8+18 = 28
        // c[0,1] = 0*1 + 1*3 + 2*5 + 3*7 = 0+3+10+21 = 34
        // c[2,1] = 8*1 + 9*3 + 10*5 + 11*7 = 8+27+50+77 = 162
        let s = c.as_slice().unwrap();
        assert!((s[0] - 28.0).abs() < 1e-4);
        assert!((s[1] - 34.0).abs() < 1e-4);
        assert!((s[5] - 162.0).abs() < 1e-4);
    }

    #[test]
    fn matmul_rejects_inner_dim_mismatch() {
        let a = Tensor::<f32>::zeros([2, 3].into()).unwrap();
        let b = Tensor::<f32>::zeros([4, 5].into()).unwrap();
        let err = matmul(&a, &b).unwrap_err();
        assert!(matches!(err, ComputeError::ShapeMismatch { .. }));
    }

    #[test]
    fn matmul_1x1_trivial() {
        let a = Tensor::<f32>::from_slice(&[3.0], [1, 1].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[4.0], [1, 1].into()).unwrap();
        let c = matmul(&a, &b).unwrap();
        assert_eq!(c.shape().dims(), &[1, 1]);
        assert!((c.as_slice().unwrap()[0] - 12.0).abs() < 1e-6);
    }

    #[test]
    fn matmul_rectangle_then_transpose() {
        // Mx K=2x3 times  K xN =3x2  →  2x2
        // Then check by swapping roles: (3x2) × (2x3) = 3x3 (different shape)
        let a = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], [2, 3].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[1.0, 0.0, 0.0, 1.0, 1.0, 1.0], [3, 2].into()).unwrap();
        let c = matmul(&a, &b).unwrap();
        assert_eq!(c.shape().dims(), &[2, 2]);
        // [[1,2,3],[4,5,6]] @ [[1,0],[0,1],[1,1]] =
        // [[1+0+3, 0+2+3], [4+0+6, 0+5+6]] = [[4, 5], [10, 11]]
        let s = c.as_slice().unwrap();
        assert!((s[0] - 4.0).abs() < 1e-5);
        assert!((s[1] - 5.0).abs() < 1e-5);
        assert!((s[2] - 10.0).abs() < 1e-5);
        assert!((s[3] - 11.0).abs() < 1e-5);
    }

    #[test]
    fn matmul_zero_matrix_yields_zero() {
        let a = Tensor::<f32>::zeros([3, 4].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&(0..20).map(|i| i as f32).collect::<Vec<_>>(), [4, 5].into()).unwrap();
        let c = matmul(&a, &b).unwrap();
        assert!(c.as_slice().unwrap().iter().all(|&v| v.abs() < 1e-7));
    }

    #[test]
    fn matmul_associativity_small() {
        // (A·B)·C ≈ A·(B·C) to f32 tolerance
        let a = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0], [2, 2].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[0.5, 1.0, -1.0, 2.0], [2, 2].into()).unwrap();
        let c = Tensor::<f32>::from_slice(&[3.0, -1.0, 0.5, 2.0], [2, 2].into()).unwrap();

        let ab = matmul(&a, &b).unwrap();
        let ab_c = matmul(&ab, &c).unwrap();

        let bc = matmul(&b, &c).unwrap();
        let a_bc = matmul(&a, &bc).unwrap();

        let s1 = ab_c.as_slice().unwrap();
        let s2 = a_bc.as_slice().unwrap();
        for i in 0..4 {
            assert!((s1[i] - s2[i]).abs() < 1e-4);
        }
    }

    #[test]
    fn matmul_single_row_times_single_col_equals_dot() {
        // (1, K) × (K, 1) = (1, 1) = dot(row, col)
        let a = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0], [1, 4].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[5.0, 6.0, 7.0, 8.0], [4, 1].into()).unwrap();
        let c = matmul(&a, &b).unwrap();
        assert_eq!(c.shape().dims(), &[1, 1]);
        // 1*5 + 2*6 + 3*7 + 4*8 = 70
        assert!((c.as_slice().unwrap()[0] - 70.0).abs() < 1e-5);
    }

    #[test]
    fn matmul_fused_gelu_no_bias_equals_gelu_of_matmul() {
        let a = Tensor::<f32>::from_slice(&[1.0, 0.5, -0.5, 0.0], [2, 2].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[1.0, 0.0, 0.0, 1.0], [2, 2].into()).unwrap();
        // AB = A. GELU(A) elementwise.
        let fused = matmul_fused_gelu(&a, &b, None).unwrap();
        let manual_ab = matmul(&a, &b).unwrap();
        let manual_gelu = activ::gelu(&manual_ab).unwrap();
        let s_f = fused.as_slice().unwrap();
        let s_m = manual_gelu.as_slice().unwrap();
        for i in 0..4 {
            assert!((s_f[i] - s_m[i]).abs() < 1e-5);
        }
    }

    #[test]
    fn matmul_fused_gelu_with_bias() {
        // A = [[1,0],[0,1]], B = [[2,3],[4,5]], AB = [[2,3],[4,5]]
        // bias = [10, 20], AB + bias = [[12,23],[14,25]]
        // then GELU element-wise (all positive and large → approx x)
        let a = Tensor::<f32>::from_slice(&[1.0, 0.0, 0.0, 1.0], [2, 2].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[2.0, 3.0, 4.0, 5.0], [2, 2].into()).unwrap();
        let bias = Tensor::<f32>::from_slice(&[10.0, 20.0], [2].into()).unwrap();
        let out = matmul_fused_gelu(&a, &b, Some(&bias)).unwrap();
        let s = out.as_slice().unwrap();
        // GELU of large positive values ≈ x
        assert!((s[0] - 12.0).abs() < 1e-3);
        assert!((s[1] - 23.0).abs() < 1e-3);
        assert!((s[2] - 14.0).abs() < 1e-3);
        assert!((s[3] - 25.0).abs() < 1e-3);
    }

    #[test]
    fn matmul_fused_gelu_rejects_wrong_bias_shape() {
        let a = Tensor::<f32>::zeros([2, 3].into()).unwrap();
        let b = Tensor::<f32>::zeros([3, 4].into()).unwrap();
        let bias = Tensor::<f32>::zeros([5].into()).unwrap(); // should be [4]
        let err = matmul_fused_gelu(&a, &b, Some(&bias)).unwrap_err();
        assert!(matches!(err, ComputeError::ShapeMismatch { .. }));
    }

    #[test]
    fn matmul_fused_gelu_rejects_rank_2_bias() {
        let a = Tensor::<f32>::zeros([2, 3].into()).unwrap();
        let b = Tensor::<f32>::zeros([3, 4].into()).unwrap();
        let bias = Tensor::<f32>::zeros([1, 4].into()).unwrap(); // rank 2 not allowed
        let err = matmul_fused_gelu(&a, &b, Some(&bias)).unwrap_err();
        assert!(matches!(err, ComputeError::ShapeMismatch { .. }));
    }

    #[test]
    fn matmul_fused_gelu_bias_is_broadcast_across_rows() {
        // A = [[1,0],[0,1]], B = [[1,1],[1,1]], AB = [[1,1],[1,1]]
        // bias = [10, 20], AB + bias per row = [[11, 21],[11, 21]]
        // GELU(large) ≈ input
        let a = Tensor::<f32>::from_slice(&[1.0, 0.0, 0.0, 1.0], [2, 2].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[1.0, 1.0, 1.0, 1.0], [2, 2].into()).unwrap();
        let bias = Tensor::<f32>::from_slice(&[10.0, 20.0], [2].into()).unwrap();
        let y = matmul_fused_gelu(&a, &b, Some(&bias)).unwrap();
        let s = y.as_slice().unwrap();
        // Both rows should have the same bias-broadcast effect
        assert!((s[0] - 11.0).abs() < 1e-3);
        assert!((s[1] - 21.0).abs() < 1e-3);
        assert!((s[2] - 11.0).abs() < 1e-3);
        assert!((s[3] - 21.0).abs() < 1e-3);
    }

    #[test]
    fn matmul_fused_gelu_zero_bias_is_noop() {
        // y_zero_bias == y_no_bias
        let a = Tensor::<f32>::from_slice(&[0.5, -0.5, 1.0, -1.0], [2, 2].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[1.0, 0.5, 0.5, 1.0], [2, 2].into()).unwrap();
        let y_no_bias = matmul_fused_gelu(&a, &b, None).unwrap();
        let zero_bias = Tensor::<f32>::zeros([2].into()).unwrap();
        let y_zero = matmul_fused_gelu(&a, &b, Some(&zero_bias)).unwrap();
        let s1 = y_no_bias.as_slice().unwrap();
        let s2 = y_zero.as_slice().unwrap();
        for i in 0..4 {
            assert!((s1[i] - s2[i]).abs() < 1e-5);
        }
    }

    // ============================================================
    // Sprint 5 — l2_distance tests
    // ============================================================

    #[test]
    fn l2_distance_rank1_basic() {
        let a = Tensor::<f32>::from_slice(&[3.0, 4.0], [2].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[0.0, 0.0], [2].into()).unwrap();
        let d = l2_distance(&a, &b).unwrap();
        assert_eq!(d.rank(), 0);
        assert!((d.as_slice().unwrap()[0] - 5.0).abs() < 1e-5);
    }

    #[test]
    fn l2_distance_zero_for_equal_vectors() {
        let a = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0], [3].into()).unwrap();
        let d = l2_distance(&a, &a).unwrap();
        assert!(d.as_slice().unwrap()[0].abs() < 1e-7);
    }

    #[test]
    fn l2_distance_rank2_per_row() {
        // Two rows: row 0 distance 5, row 1 distance 13.
        let a = Tensor::<f32>::from_slice(
            &[3.0, 4.0, 5.0, 12.0],
            [2, 2].into(),
        )
        .unwrap();
        let b = Tensor::<f32>::from_slice(&[0.0, 0.0, 0.0, 0.0], [2, 2].into()).unwrap();
        let d = l2_distance(&a, &b).unwrap();
        assert_eq!(d.shape().dims(), &[2]);
        let s = d.as_slice().unwrap();
        assert!((s[0] - 5.0).abs() < 1e-5);
        assert!((s[1] - 13.0).abs() < 1e-5);
    }

    #[test]
    fn l2_distance_rejects_shape_mismatch() {
        let a = Tensor::<f32>::zeros([4].into()).unwrap();
        let b = Tensor::<f32>::zeros([3].into()).unwrap();
        assert!(matches!(
            l2_distance(&a, &b).unwrap_err(),
            ComputeError::ShapeMismatch { .. }
        ));
    }

    #[test]
    fn l2_distance_rejects_rank_3() {
        let a = Tensor::<f32>::zeros([2, 2, 2].into()).unwrap();
        let b = Tensor::<f32>::zeros([2, 2, 2].into()).unwrap();
        assert!(matches!(
            l2_distance(&a, &b).unwrap_err(),
            ComputeError::ShapeMismatch { .. }
        ));
    }

    #[test]
    fn l2_distance_rejects_nan() {
        let a = Tensor::<f32>::from_slice(&[f32::NAN, 1.0], [2].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[0.0, 1.0], [2].into()).unwrap();
        assert!(matches!(
            l2_distance(&a, &b).unwrap_err(),
            ComputeError::NumericalInstability { .. }
        ));
    }

    #[test]
    fn l2_distance_rejects_inf() {
        let a = Tensor::<f32>::from_slice(&[f32::INFINITY, 1.0], [2].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[0.0, 1.0], [2].into()).unwrap();
        assert!(matches!(
            l2_distance(&a, &b).unwrap_err(),
            ComputeError::NumericalInstability { .. }
        ));
    }

    #[test]
    fn l2_distance_kahan_robust_on_long_vector() {
        // 10 000 small differences of 1e-3 each → sum of squares = 1e-2
        // → distance = √(1e-2) ≈ 0.1
        let n = 10_000usize;
        let a: Vec<f32> = (0..n).map(|_| 1.0e-3).collect();
        let b = vec![0.0f32; n];
        let ta = Tensor::<f32>::from_slice(&a, [n].into()).unwrap();
        let tb = Tensor::<f32>::from_slice(&b, [n].into()).unwrap();
        let d = l2_distance(&ta, &tb).unwrap().as_slice().unwrap()[0];
        let expected = (1.0e-2_f32).sqrt(); // 0.1
        assert!(
            (d - expected).abs() < 1e-5,
            "Kahan drift too large: got {d}, expected {expected}"
        );
    }
}
