//! Shared helpers for CPU backend implementations.
//!
//! This module factors out the iteration patterns that show up across
//! multiple ops. It is private (`pub(super)`) — external users go through
//! [`crate::compute::ComputeBackend`].

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::compute::error::{ComputeError, Result};
use crate::compute::tensor::Tensor;

// ============================================================
// Axis iteration
// ============================================================

/// Compute C-order (row-major) strides for a shape's dimensions.
///
/// Returns a `Vec<usize>` of the same length as `dims.len()`.
pub(super) fn row_major_strides(dims: &[usize]) -> Vec<usize> {
    let rank = dims.len();
    let mut s = vec![1usize; rank];
    if rank >= 2 {
        for i in (0..rank - 1).rev() {
            s[i] = s[i + 1] * dims[i + 1];
        }
    }
    s
}

/// Map a function element-wise along one axis, keeping the output shape
/// identical to the input shape.
///
/// `f` receives a freshly gathered slice of length `dims[axis]` and must
/// return a `Vec<f32>` of the **same length**. Returning a different
/// length yields `ComputeError::Internal`.
///
/// Used by `softmax`, `log_softmax`, `l2_normalize`, and any other
/// axis-wise op that preserves shape.
pub(super) fn map_along_axis<F>(x: &Tensor<f32>, axis: i32, f: F) -> Result<Tensor<f32>>
where
    F: Fn(&[f32]) -> Vec<f32>,
{
    let shape = x.shape();
    let rank = shape.rank();
    if rank == 0 {
        return Err(ComputeError::InvalidAxis { axis, rank });
    }
    let a = shape.resolve_axis(axis)?;
    let dims = shape.dims();
    let src = x.as_slice()?;

    let src_strides = row_major_strides(dims);
    let dim_a = dims[a];

    // Output buffer (same size as input)
    let numel = shape.numel();
    let mut out = vec![0.0f32; numel];

    // Compute strides over the *reduced* shape (all axes except `a`)
    let reduced_dims: Vec<usize> = dims
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != a)
        .map(|(_, &d)| d)
        .collect();
    let reduced_strides = row_major_strides(&reduced_dims);

    let n_slices: usize = reduced_dims.iter().product::<usize>().max(1);
    let mut coords = vec![0usize; rank];
    let mut gathered = vec![0.0f32; dim_a];

    for slice_idx in 0..n_slices {
        // Decompose slice_idx over non-axis dimensions
        let mut rem = slice_idx;
        let mut pos = 0usize;
        for i in 0..rank {
            if i == a {
                continue;
            }
            let stride = if reduced_strides.is_empty() {
                1
            } else {
                reduced_strides[pos]
            };
            coords[i] = if stride > 0 { rem / stride } else { 0 };
            if stride > 0 {
                rem %= stride;
            }
            pos += 1;
        }

        // Gather along axis
        for k in 0..dim_a {
            coords[a] = k;
            let mut flat = 0usize;
            for (i, &c) in coords.iter().enumerate() {
                flat += c * src_strides[i];
            }
            gathered[k] = src[flat];
        }

        // Apply f
        let transformed = f(&gathered);
        if transformed.len() != dim_a {
            return Err(ComputeError::Internal {
                detail: format!(
                    "map_along_axis: f returned {} elements, expected {}",
                    transformed.len(),
                    dim_a
                ),
            });
        }

        // Scatter back
        for k in 0..dim_a {
            coords[a] = k;
            let mut flat = 0usize;
            for (i, &c) in coords.iter().enumerate() {
                flat += c * src_strides[i];
            }
            out[flat] = transformed[k];
        }
    }

    Tensor::from_slice(&out, shape.clone())
}

// ============================================================
// Shape validation
// ============================================================

/// Validate the shape preconditions for matrix multiplication.
///
/// Returns `(M, K, N)` on success. Fails with `ShapeMismatch` if:
/// * either operand is not rank 2
/// * the inner dimensions don't agree (A.cols != B.rows)
pub(super) fn validate_matmul_shapes(
    a: &Tensor<f32>,
    b: &Tensor<f32>,
) -> Result<(usize, usize, usize)> {
    let ash = a.shape();
    let bsh = b.shape();
    if ash.rank() != 2 || bsh.rank() != 2 {
        return Err(ComputeError::ShapeMismatch {
            op: "matmul",
            expected: "both tensors rank 2".into(),
            actual: format!("a.rank={}, b.rank={}", ash.rank(), bsh.rank()),
        });
    }
    let m = ash.dims()[0];
    let k = ash.dims()[1];
    let k2 = bsh.dims()[0];
    let n = bsh.dims()[1];
    if k != k2 {
        return Err(ComputeError::ShapeMismatch {
            op: "matmul",
            expected: format!("(M, {k}) × ({k}, N)"),
            actual: format!("({m}, {k}) × ({k2}, {n})"),
        });
    }
    Ok((m, k, n))
}

/// Transpose a 2-D tensor into a fresh contiguous buffer.
///
/// Not zero-copy: produces a new allocation. A zero-copy view based on
/// stride-swapping will arrive in Sprint 4 once the `Tensor` type exposes
/// view semantics for non-contiguous layouts.
pub(super) fn transpose_2d(x: &Tensor<f32>) -> Result<Tensor<f32>> {
    let dims = x.shape().dims();
    if dims.len() != 2 {
        return Err(ComputeError::ShapeMismatch {
            op: "transpose_2d",
            expected: "rank 2".into(),
            actual: format!("rank {}", dims.len()),
        });
    }
    let (m, n) = (dims[0], dims[1]);
    let src = x.as_slice()?;
    let mut out = vec![0.0f32; m * n];
    for i in 0..m {
        for j in 0..n {
            out[j * m + i] = src[i * n + j];
        }
    }
    Tensor::from_slice(&out, [n, m].into())
}

/// Reduce two tensors of matching shape along `axis` with a binary
/// closure that takes two aligned slices and returns a scalar.
///
/// Used by `kl_divergence`. The two tensors must have identical shapes.
pub(super) fn reduce_axis_pairwise_to_scalar<F>(
    p: &Tensor<f32>,
    q: &Tensor<f32>,
    axis: i32,
    f: F,
) -> Result<Tensor<f32>>
where
    F: Fn(&[f32], &[f32]) -> f32,
{
    if p.shape().dims() != q.shape().dims() {
        return Err(ComputeError::ShapeMismatch {
            op: "reduce_axis_pairwise_to_scalar",
            expected: format!("{}", p.shape()),
            actual: format!("{}", q.shape()),
        });
    }
    let shape = p.shape();
    let rank = shape.rank();
    if rank == 0 {
        return Err(ComputeError::InvalidAxis { axis, rank });
    }
    let a = shape.resolve_axis(axis)?;
    let dims = shape.dims();
    let p_src = p.as_slice()?;
    let q_src = q.as_slice()?;

    let src_strides = row_major_strides(dims);
    let dim_a = dims[a];

    let reduced_dims: Vec<usize> = dims
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != a)
        .map(|(_, &d)| d)
        .collect();
    let out_shape: crate::compute::shape::Shape = reduced_dims.clone().into();
    let out_numel = out_shape.numel();
    let reduced_strides = row_major_strides(&reduced_dims);

    let mut out = Vec::with_capacity(out_numel);
    let mut coords = vec![0usize; rank];
    let mut p_row = vec![0.0f32; dim_a];
    let mut q_row = vec![0.0f32; dim_a];

    for out_idx in 0..out_numel {
        // Decompose out_idx over non-axis dims
        let mut rem = out_idx;
        let mut pos = 0usize;
        for i in 0..rank {
            if i == a {
                continue;
            }
            let stride = if reduced_strides.is_empty() {
                1
            } else {
                reduced_strides[pos]
            };
            coords[i] = if stride > 0 { rem / stride } else { 0 };
            if stride > 0 {
                rem %= stride;
            }
            pos += 1;
        }
        // Gather
        for k in 0..dim_a {
            coords[a] = k;
            let mut flat = 0usize;
            for (i, &c) in coords.iter().enumerate() {
                flat += c * src_strides[i];
            }
            p_row[k] = p_src[flat];
            q_row[k] = q_src[flat];
        }
        out.push(f(&p_row, &q_row));
    }

    Tensor::from_slice(&out, out_shape)
}

/// Totally-ordered `f32` wrapper for `BinaryHeap` usage.
///
/// Uses `f32::total_cmp` (IEEE-754 totalOrder), which means `NaN` is
/// considered greater than `+Inf`. This gives a deterministic ordering
/// even in presence of NaN inputs.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct TotalF32(pub f32);

impl Eq for TotalF32 {}

impl Ord for TotalF32 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl PartialOrd for TotalF32 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Extract top-k (value, index) pairs from a slice, sorted by value
/// descending with ties broken by smallest index.
///
/// Uses a min-heap of size `k` for `O(n log k)` complexity, better than
/// a full sort when `k << n`.
///
/// # Panics
///
/// Debug-only: panics if `k > row.len()`. Callers should validate first.
pub(super) fn topk_row(row: &[f32], k: usize) -> (Vec<f32>, Vec<i32>) {
    debug_assert!(k <= row.len(), "topk_row: k={} > row.len()={}", k, row.len());

    if k == 0 {
        return (Vec::new(), Vec::new());
    }

    // Min-heap of size k : keep the k largest. Tie-break with Reverse(i)
    // on the secondary key so that smaller indices are preferred.
    let mut heap: BinaryHeap<Reverse<(TotalF32, Reverse<usize>)>> =
        BinaryHeap::with_capacity(k);
    for (i, &v) in row.iter().enumerate() {
        let entry = Reverse((TotalF32(v), Reverse(i)));
        if heap.len() < k {
            heap.push(entry);
        } else if let Some(worst) = heap.peek() {
            // heap.peek() gives the smallest value (because of outer Reverse).
            // Pop and replace if the new value is strictly larger,
            // OR equal in value and smaller in index (tie-break).
            let worst_v = (worst.0).0;
            let worst_i = (worst.0).1 .0;
            let new_is_better = match TotalF32(v).cmp(&worst_v) {
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Equal => i < worst_i,
                std::cmp::Ordering::Less => false,
            };
            if new_is_better {
                heap.pop();
                heap.push(entry);
            }
        }
    }

    // Extract and sort: primary by value descending, secondary by index asc.
    let mut pairs: Vec<(f32, i32)> = heap
        .into_iter()
        .map(|Reverse((TotalF32(v), Reverse(i)))| (v, i as i32))
        .collect();
    pairs.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));

    let values: Vec<f32> = pairs.iter().map(|p| p.0).collect();
    let indices: Vec<i32> = pairs.iter().map(|p| p.1).collect();
    (values, indices)
}

/// Validate that two rank-1 tensors have the same length.
pub(super) fn validate_same_shape_1d(
    a: &Tensor<f32>,
    b: &Tensor<f32>,
    op: &'static str,
) -> Result<usize> {
    if a.shape().rank() != 1 || b.shape().rank() != 1 {
        return Err(ComputeError::ShapeMismatch {
            op,
            expected: "both rank 1".into(),
            actual: format!("a.rank={}, b.rank={}", a.shape().rank(), b.shape().rank()),
        });
    }
    if a.shape().dims() != b.shape().dims() {
        return Err(ComputeError::ShapeMismatch {
            op,
            expected: format!("{}", a.shape()),
            actual: format!("{}", b.shape()),
        });
    }
    Ok(a.shape().dims()[0])
}

// ============================================================
// Numerical helpers
// ============================================================

/// Kahan compensated summation of a slice.
///
/// Used by [`weighted_mean`](super::consensus::weighted_mean) to fold the
/// weight denominator and by the shared-test suite to validate accumulator
/// drift on long reductions.
///
/// # Error analysis
///
/// Naive summation accumulates rounding error as **O(n · ε)** where
/// `ε ≈ 1.19e-7` for f32. For `n = 10⁶` this yields ~0.1 of absolute
/// error, which is catastrophic for reproducibility.
///
/// Kahan compensated summation tracks the bits lost in each partial
/// sum and re-injects them on the next iteration, bringing the total
/// error down to **O(ε)** independent of `n`. Empirically, on 10 000
/// additions of 1e-6 we observe error < 1e-9 (vs ~1e-5 for naive).
///
/// # Reference
///
/// Kahan, W. (1965) "Pracniques: further remarks on reducing truncation
/// errors." *Communications of the ACM* 8(1): 40.
#[inline]
pub(super) fn kahan_sum_slice(xs: &[f32]) -> f32 {
    let mut sum = 0.0f32;
    let mut c = 0.0f32; // running compensation for lost low-order bits
    for &x in xs {
        let y = x - c;
        let t = sum + y;
        c = (t - sum) - y;
        sum = t;
    }
    sum
}

/// Kahan compensated dot product.
///
/// Equivalent to `kahan_sum_slice(&(a ⊙ b))` but avoids materialising
/// the intermediate element-wise product buffer.
#[inline]
pub(super) fn kahan_dot(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let mut sum = 0.0f32;
    let mut c = 0.0f32;
    for i in 0..a.len() {
        let y = a[i] * b[i] - c;
        let t = sum + y;
        c = (t - sum) - y;
        sum = t;
    }
    sum
}

/// Validate that `axis` resolves to a valid dimension index for `shape`.
///
/// Small helper that centralises the error construction and supports
/// negative axes (Python-style). Convenience wrapper around
/// [`Shape::resolve_axis`], kept here for symmetry with other helpers.
#[allow(dead_code)]
#[inline]
pub(super) fn validate_axis(
    shape: &crate::compute::shape::Shape,
    axis: i32,
) -> Result<usize> {
    shape.resolve_axis(axis)
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    clippy::excessive_precision,
    clippy::approx_constant,
    clippy::redundant_closure_for_method_calls
)]
mod tests {
    use super::*;

    #[test]
    fn row_major_strides_basic() {
        assert_eq!(row_major_strides(&[]), Vec::<usize>::new());
        assert_eq!(row_major_strides(&[5]), vec![1]);
        assert_eq!(row_major_strides(&[2, 3]), vec![3, 1]);
        assert_eq!(row_major_strides(&[2, 3, 4]), vec![12, 4, 1]);
    }

    #[test]
    fn kahan_dot_correct_on_simple_vectors() {
        let a = [1.0, 2.0, 3.0];
        let b = [4.0, 5.0, 6.0];
        // 1*4 + 2*5 + 3*6 = 4 + 10 + 18 = 32
        assert!((kahan_dot(&a, &b) - 32.0).abs() < 1e-6);
    }

    #[test]
    fn kahan_dot_robust_on_large_sum() {
        // 1000 terms of 1e-7 summed with Kahan should give ~1e-4 precisely
        let n = 1000;
        let a = vec![1.0f32; n];
        let b = vec![1e-7f32; n];
        let expected = 1e-4f32;
        let got = kahan_dot(&a, &b);
        assert!((got - expected).abs() < 1e-8, "got {got}, expected {expected}");
    }

    #[test]
    fn map_along_axis_identity_returns_input() {
        let x = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0], [2, 2].into()).unwrap();
        let y = map_along_axis(&x, -1, |row| row.to_vec()).unwrap();
        assert_eq!(y.as_slice().unwrap(), x.as_slice().unwrap());
    }

    #[test]
    fn map_along_axis_doubles_each_row() {
        let x = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0], [2, 2].into()).unwrap();
        let y = map_along_axis(&x, -1, |row| row.iter().map(|v| v * 2.0).collect()).unwrap();
        assert_eq!(y.as_slice().unwrap(), &[2.0, 4.0, 6.0, 8.0]);
    }

    #[test]
    fn map_along_axis_axis_zero() {
        // x: (2, 3) = [[1,2,3],[4,5,6]]
        // axis=0: rows along axis-0 are columns: [1,4], [2,5], [3,6]
        // doubling each → [[2,8],[4,10],[6,12]] ← but flattened back to (2,3):
        //   [[2,4,6],[8,10,12]]
        let x = Tensor::<f32>::from_slice(
            &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            [2, 3].into(),
        )
        .unwrap();
        let y = map_along_axis(&x, 0, |col| col.iter().map(|v| v * 2.0).collect()).unwrap();
        assert_eq!(y.as_slice().unwrap(), &[2.0, 4.0, 6.0, 8.0, 10.0, 12.0]);
    }

    #[test]
    fn map_along_axis_length_mismatch_errors() {
        let x = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0], [3].into()).unwrap();
        let err = map_along_axis(&x, -1, |_row| vec![0.0, 0.0]).unwrap_err();
        assert!(matches!(err, ComputeError::Internal { .. }));
    }

    #[test]
    fn validate_matmul_shapes_ok() {
        let a: Tensor<f32> = Tensor::zeros([2, 3].into()).unwrap();
        let b: Tensor<f32> = Tensor::zeros([3, 4].into()).unwrap();
        assert_eq!(validate_matmul_shapes(&a, &b).unwrap(), (2, 3, 4));
    }

    #[test]
    fn validate_matmul_shapes_inner_mismatch() {
        let a: Tensor<f32> = Tensor::zeros([2, 3].into()).unwrap();
        let b: Tensor<f32> = Tensor::zeros([4, 5].into()).unwrap();
        assert!(matches!(
            validate_matmul_shapes(&a, &b),
            Err(ComputeError::ShapeMismatch { .. })
        ));
    }

    #[test]
    fn validate_matmul_shapes_wrong_rank() {
        let a: Tensor<f32> = Tensor::zeros([3].into()).unwrap();
        let b: Tensor<f32> = Tensor::zeros([3, 4].into()).unwrap();
        assert!(validate_matmul_shapes(&a, &b).is_err());
    }

    #[test]
    fn validate_same_shape_1d_accepts() {
        let a: Tensor<f32> = Tensor::zeros([5].into()).unwrap();
        let b: Tensor<f32> = Tensor::zeros([5].into()).unwrap();
        assert_eq!(validate_same_shape_1d(&a, &b, "test").unwrap(), 5);
    }

    #[test]
    fn validate_same_shape_1d_rejects_mismatch() {
        let a: Tensor<f32> = Tensor::zeros([5].into()).unwrap();
        let b: Tensor<f32> = Tensor::zeros([6].into()).unwrap();
        assert!(validate_same_shape_1d(&a, &b, "test").is_err());
    }

    // ===== New Sprint 2 helpers =====

    #[test]
    fn kahan_sum_slice_empty() {
        assert_eq!(kahan_sum_slice(&[]), 0.0);
    }

    #[test]
    fn kahan_sum_slice_single() {
        assert_eq!(kahan_sum_slice(&[3.14]), 3.14);
    }

    #[test]
    fn kahan_sum_slice_matches_naive_for_small_n() {
        let xs: Vec<f32> = (1..=10).map(|i| i as f32).collect();
        let naive: f32 = xs.iter().sum();
        let kahan = kahan_sum_slice(&xs);
        // 1+2+...+10 = 55
        assert!((kahan - 55.0).abs() < 1e-6);
        assert!((kahan - naive).abs() < 1e-6);
    }

    #[test]
    fn kahan_sum_slice_beats_naive_on_long_sum() {
        // 10⁶ copies of 1e-7 should sum to 0.1.
        // Naive f32 can drift by ~1e-4; Kahan should be < 1e-8.
        let n = 1_000_000;
        let xs = vec![1e-7f32; n];
        let kahan = kahan_sum_slice(&xs);
        let naive: f32 = xs.iter().sum();
        let true_val = 0.1f32;
        // Kahan stays within 1e-7 of truth
        assert!(
            (kahan - true_val).abs() < 1e-7,
            "Kahan drift: {}",
            (kahan - true_val).abs()
        );
        // And is at least as accurate as naive
        assert!((kahan - true_val).abs() <= (naive - true_val).abs() + 1e-9);
    }

    #[test]
    fn validate_axis_resolves_negative() {
        let s: crate::compute::shape::Shape = [2, 3, 4].into();
        assert_eq!(validate_axis(&s, -1).unwrap(), 2);
        assert_eq!(validate_axis(&s, -3).unwrap(), 0);
    }

    #[test]
    fn validate_axis_rejects_out_of_range() {
        let s: crate::compute::shape::Shape = [3].into();
        assert!(validate_axis(&s, 5).is_err());
        assert!(validate_axis(&s, -2).is_err());
    }

    // ===== Sprint 3 helpers =====

    #[test]
    fn total_f32_orders_with_total_cmp() {
        // Standard order
        assert!(TotalF32(1.0) < TotalF32(2.0));
        assert!(TotalF32(-1.0) < TotalF32(0.0));
        // NaN is strictly greater than +Inf with total_cmp
        assert!(TotalF32(f32::NAN) > TotalF32(f32::INFINITY));
        // -0.0 < +0.0 with total_cmp
        assert!(TotalF32(-0.0) < TotalF32(0.0));
    }

    #[test]
    fn transpose_2d_basic() {
        // [[1,2,3],[4,5,6]] → [[1,4],[2,5],[3,6]]
        let x = Tensor::<f32>::from_slice(
            &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            [2, 3].into(),
        )
        .unwrap();
        let y = transpose_2d(&x).unwrap();
        assert_eq!(y.shape().dims(), &[3, 2]);
        assert_eq!(y.as_slice().unwrap(), &[1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
    }

    #[test]
    fn transpose_2d_square() {
        let x = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0], [2, 2].into()).unwrap();
        let y = transpose_2d(&x).unwrap();
        assert_eq!(y.as_slice().unwrap(), &[1.0, 3.0, 2.0, 4.0]);
    }

    #[test]
    fn transpose_2d_rejects_rank3() {
        let x = Tensor::<f32>::zeros([2, 3, 4].into()).unwrap();
        assert!(transpose_2d(&x).is_err());
    }

    #[test]
    fn topk_row_basic() {
        // [0.1, 0.5, 0.3, 0.9, 0.7], k=3 → values [0.9, 0.7, 0.5], indices [3, 4, 1]
        let row = [0.1, 0.5, 0.3, 0.9, 0.7];
        let (vals, idx) = topk_row(&row, 3);
        assert_eq!(vals, vec![0.9, 0.7, 0.5]);
        assert_eq!(idx, vec![3, 4, 1]);
    }

    #[test]
    fn topk_row_ties_break_by_smallest_index() {
        // All equal, k=2 → first two indices (0, 1)
        let row = [5.0, 5.0, 5.0, 5.0];
        let (vals, idx) = topk_row(&row, 2);
        assert_eq!(vals, vec![5.0, 5.0]);
        assert_eq!(idx, vec![0, 1]);
    }

    #[test]
    fn topk_row_k_equals_n_returns_all_sorted_desc() {
        let row = [3.0, 1.0, 2.0];
        let (vals, idx) = topk_row(&row, 3);
        assert_eq!(vals, vec![3.0, 2.0, 1.0]);
        assert_eq!(idx, vec![0, 2, 1]);
    }

    #[test]
    fn topk_row_k_zero_empty() {
        let row = [3.0, 1.0, 2.0];
        let (vals, idx) = topk_row(&row, 0);
        assert!(vals.is_empty());
        assert!(idx.is_empty());
    }

    #[test]
    fn reduce_axis_pairwise_basic() {
        // p=[1,2,3], q=[1,1,1], axis=-1, f=|p_i · q_i| sum = 1+2+3 = 6
        let p = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0], [3].into()).unwrap();
        let q = Tensor::<f32>::from_slice(&[1.0, 1.0, 1.0], [3].into()).unwrap();
        let r = reduce_axis_pairwise_to_scalar(&p, &q, -1, |pr, qr| {
            pr.iter().zip(qr.iter()).map(|(a, b)| a * b).sum()
        })
        .unwrap();
        assert_eq!(r.rank(), 0);
        assert!((r.as_slice().unwrap()[0] - 6.0).abs() < 1e-6);
    }

    #[test]
    fn reduce_axis_pairwise_rejects_shape_mismatch() {
        let p = Tensor::<f32>::zeros([3].into()).unwrap();
        let q = Tensor::<f32>::zeros([4].into()).unwrap();
        let r = reduce_axis_pairwise_to_scalar(&p, &q, -1, |_, _| 0.0);
        assert!(r.is_err());
    }

    #[test]
    fn reduce_axis_pairwise_2d_axis_last() {
        // p = [[1,2],[3,4]], q = [[1,1],[2,2]], axis=-1, dot per row
        // Row 0: 1·1 + 2·1 = 3
        // Row 1: 3·2 + 4·2 = 14
        let p = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0], [2, 2].into()).unwrap();
        let q = Tensor::<f32>::from_slice(&[1.0, 1.0, 2.0, 2.0], [2, 2].into()).unwrap();
        let r = reduce_axis_pairwise_to_scalar(&p, &q, -1, |pr, qr| {
            pr.iter().zip(qr.iter()).map(|(a, b)| a * b).sum()
        })
        .unwrap();
        assert_eq!(r.shape().dims(), &[2]);
        let s = r.as_slice().unwrap();
        assert!((s[0] - 3.0).abs() < 1e-6);
        assert!((s[1] - 14.0).abs() < 1e-6);
    }
}
