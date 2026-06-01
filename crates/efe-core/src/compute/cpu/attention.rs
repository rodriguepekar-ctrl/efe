//! Sprint 3 CPU kernels : `topk`, `scaled_dot_product_attention`.

use super::shared::{topk_row, transpose_2d};
use crate::compute::error::{ComputeError, Result};
use crate::compute::shape::Shape;
use crate::compute::tensor::Tensor;

// =====================================================================
// topk
// =====================================================================

/// Top-k values and their indices along `axis`.
///
/// # Returns
///
/// A tuple `(values, indices)` where both tensors have the same shape
/// as `x` with `dim[axis]` replaced by `k`. Values are sorted in
/// **descending** order along the reduced axis ; ties are broken by
/// **smallest index**.
///
/// # Preconditions
///
/// * `k > 0`
/// * `k ≤ x.shape()[axis]`
///
/// # Complexity
///
/// `O(n log k)` per row thanks to the heap-based implementation (see
/// [`super::shared::topk_row`]). Better than a full sort when `k ≪ n`.
pub fn topk(
    x: &Tensor<f32>,
    k: u32,
    axis: i32,
) -> Result<(Tensor<f32>, Tensor<i32>)> {
    let k = k as usize;
    let shape = x.shape();
    let rank = shape.rank();
    if rank == 0 {
        return Err(ComputeError::InvalidAxis { axis, rank });
    }
    let a = shape.resolve_axis(axis)?;
    let dim_a = shape.dims()[a];
    if k == 0 || k > dim_a {
        return Err(ComputeError::ShapeMismatch {
            op: "topk",
            expected: format!("0 < k ≤ {dim_a}"),
            actual: format!("k = {k}"),
        });
    }

    // Build output shape : replace axis `a` with `k`.
    let out_dims: Vec<usize> = shape
        .dims()
        .iter()
        .enumerate()
        .map(|(i, &d)| if i == a { k } else { d })
        .collect();
    let out_shape: Shape = out_dims.clone().into();

    // Strides
    let src = x.as_slice()?;
    let src_dims = shape.dims();
    let rank_ = src_dims.len();
    let mut src_strides = vec![1usize; rank_];
    if rank_ >= 2 {
        for i in (0..rank_ - 1).rev() {
            src_strides[i] = src_strides[i + 1] * src_dims[i + 1];
        }
    }
    let mut out_strides = vec![1usize; rank_];
    if rank_ >= 2 {
        for i in (0..rank_ - 1).rev() {
            out_strides[i] = out_strides[i + 1] * out_dims[i + 1];
        }
    }

    let mut vals_out = vec![0.0f32; out_shape.numel()];
    let mut idx_out = vec![0i32; out_shape.numel()];

    // Iterate over the outer shape (all axes except `a`).
    let outer_dims: Vec<usize> = src_dims
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != a)
        .map(|(_, &d)| d)
        .collect();
    let outer_numel: usize = outer_dims.iter().product::<usize>().max(1);
    let outer_strides: Vec<usize> = {
        let mut s = vec![1usize; outer_dims.len()];
        if outer_dims.len() >= 2 {
            for i in (0..outer_dims.len() - 1).rev() {
                s[i] = s[i + 1] * outer_dims[i + 1];
            }
        }
        s
    };

    let mut coords = vec![0usize; rank_];
    let mut gathered = vec![0.0f32; dim_a];

    for outer_idx in 0..outer_numel {
        // Decompose outer_idx into non-axis coordinates.
        let mut rem = outer_idx;
        let mut pos = 0usize;
        for i in 0..rank_ {
            if i == a {
                continue;
            }
            let stride = if outer_strides.is_empty() {
                1
            } else {
                outer_strides[pos]
            };
            coords[i] = if stride > 0 { rem / stride } else { 0 };
            if stride > 0 {
                rem %= stride;
            }
            pos += 1;
        }

        // Gather along axis `a`
        for k_i in 0..dim_a {
            coords[a] = k_i;
            let mut flat = 0usize;
            for (i, &c) in coords.iter().enumerate() {
                flat += c * src_strides[i];
            }
            gathered[k_i] = src[flat];
        }

        // Run topk on the gathered row
        let (top_vals, top_idxs) = topk_row(&gathered, k);

        // Scatter results at positions coords[a] = 0..k in out buffers
        for ki in 0..k {
            coords[a] = ki;
            let mut flat = 0usize;
            for (i, &c) in coords.iter().enumerate() {
                flat += c * out_strides[i];
            }
            vals_out[flat] = top_vals[ki];
            idx_out[flat] = top_idxs[ki];
        }
    }

    let values = Tensor::<f32>::from_slice(&vals_out, out_shape.clone())?;
    let indices = Tensor::<i32>::from_slice(&idx_out, out_shape)?;
    Ok((values, indices))
}

// =====================================================================
// scaled_dot_product_attention — Étape 9
// =====================================================================

/// Scaled dot-product attention.
///
/// ```text
///     Attention(Q, K, V) = softmax( Q · Kᵀ / √d_k + mask ) · V
/// ```
///
/// # Shape contract
///
/// * `Q : (B, H, T_q, d_k)`
/// * `K : (B, H, T_k, d_k)`
/// * `V : (B, H, T_k, d_v)`
/// * `mask : Option<(T_q, T_k)>` — additive mask, broadcasted to all
///   batch-heads (Sprint 3 simplification ; per-head masks arrive later)
///
/// # Sprint 3 limitations
///
/// * Rank-4 only (we don't support 2D/3D pre-reshaped attention yet).
/// * Naïve loop over `(B, H)` — each batch-head calls the 2D matmul.
///   Sprint 4 Light dispatches each per-head matmul to BLAS when the
///   per-head problem exceeds the dispatch threshold; a true `bmm`
///   batched-matmul is deferred to Sprint 6+.
///
/// # Complexity
///
/// `O(B · H · T² · d_k)`.
#[allow(clippy::too_many_lines)] // validation + 2-level loop exceed default threshold
pub fn scaled_dot_product_attention(
    q: &Tensor<f32>,
    k: &Tensor<f32>,
    v: &Tensor<f32>,
    mask: Option<&Tensor<f32>>,
) -> Result<Tensor<f32>> {
    // Validate ranks
    let q_dims = q.shape().dims();
    let k_dims = k.shape().dims();
    let v_dims = v.shape().dims();
    if q_dims.len() != 4 || k_dims.len() != 4 || v_dims.len() != 4 {
        return Err(ComputeError::ShapeMismatch {
            op: "scaled_dot_product_attention",
            expected: "rank 4 (B, H, T, d)".into(),
            actual: format!(
                "q={}, k={}, v={}",
                q_dims.len(),
                k_dims.len(),
                v_dims.len()
            ),
        });
    }
    let (b, h, t_q, d_k) = (q_dims[0], q_dims[1], q_dims[2], q_dims[3]);
    let (bk, hk, t_k, d_kk) = (k_dims[0], k_dims[1], k_dims[2], k_dims[3]);
    let (bv, hv, t_v, d_v) = (v_dims[0], v_dims[1], v_dims[2], v_dims[3]);
    if b != bk || b != bv || h != hk || h != hv {
        return Err(ComputeError::ShapeMismatch {
            op: "scaled_dot_product_attention",
            expected: "matching batch and heads across Q, K, V".into(),
            actual: format!("q=({b},{h}), k=({bk},{hk}), v=({bv},{hv})"),
        });
    }
    if d_k != d_kk {
        return Err(ComputeError::ShapeMismatch {
            op: "scaled_dot_product_attention",
            expected: format!("d_k match: Q.d_k={d_k}, K.d_k={d_kk}"),
            actual: "mismatch".into(),
        });
    }
    if t_k != t_v {
        return Err(ComputeError::ShapeMismatch {
            op: "scaled_dot_product_attention",
            expected: format!("K.T == V.T (got {t_k} vs {t_v})"),
            actual: "mismatch".into(),
        });
    }
    // Bug-fix audit : `d_k = 0` would cause `1/sqrt(0) = +Inf` in the
    // scale factor, which propagates NaN into every score. Reject at the
    // boundary rather than producing garbage output.
    if d_k == 0 {
        return Err(ComputeError::ShapeMismatch {
            op: "scaled_dot_product_attention",
            expected: "d_k > 0".into(),
            actual: "d_k = 0".into(),
        });
    }

    let mask_data = if let Some(m) = mask {
        let m_dims = m.shape().dims();
        if m_dims.len() != 2 || m_dims[0] != t_q || m_dims[1] != t_k {
            return Err(ComputeError::ShapeMismatch {
                op: "scaled_dot_product_attention",
                expected: format!("mask shape [{t_q}, {t_k}]"),
                actual: format!("{}", m.shape()),
            });
        }
        Some(m.as_slice()?)
    } else {
        None
    };

    let q_src = q.as_slice()?;
    let k_src = k.as_slice()?;
    let v_src = v.as_slice()?;
    let scale = 1.0f32 / (d_k as f32).sqrt();

    let mut out_buf = vec![0.0f32; b * h * t_q * d_v];

    // Outer loop over (batch, head)
    for bi in 0..b {
        for hi in 0..h {
            // Extract 2D tiles
            let q_offset = bi * h * t_q * d_k + hi * t_q * d_k;
            let k_offset = bi * h * t_k * d_k + hi * t_k * d_k;
            let v_offset = bi * h * t_v * d_v + hi * t_v * d_v;

            let q_bh = Tensor::<f32>::from_slice(
                &q_src[q_offset..q_offset + t_q * d_k],
                [t_q, d_k].into(),
            )?;
            let k_bh = Tensor::<f32>::from_slice(
                &k_src[k_offset..k_offset + t_k * d_k],
                [t_k, d_k].into(),
            )?;
            let v_bh = Tensor::<f32>::from_slice(
                &v_src[v_offset..v_offset + t_v * d_v],
                [t_v, d_v].into(),
            )?;

            // K transposed : (d_k, T_k)
            let k_t = transpose_2d(&k_bh)?;
            // Q · Kᵀ : (T_q, T_k)
            let scores = super::linalg::matmul(&q_bh, &k_t)?;
            // Scale and optionally apply mask
            let mut scored_buf: Vec<f32> = scores
                .as_slice()?
                .iter()
                .map(|v| v * scale)
                .collect();
            if let Some(m) = mask_data {
                for (i, v) in scored_buf.iter_mut().enumerate() {
                    *v += m[i];
                }
            }
            let scores_tensor =
                Tensor::<f32>::from_slice(&scored_buf, [t_q, t_k].into())?;
            // softmax along last axis
            let weights = super::activ::softmax(&scores_tensor, -1)?;
            // weights · V : (T_q, d_v)
            let out_bh = super::linalg::matmul(&weights, &v_bh)?;

            // Scatter into out_buf
            let out_offset = bi * h * t_q * d_v + hi * t_q * d_v;
            let out_slice = out_bh.as_slice()?;
            out_buf[out_offset..out_offset + t_q * d_v].copy_from_slice(out_slice);
        }
    }

    Tensor::from_slice(&out_buf, [b, h, t_q, d_v].into())
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    // ===== topk =====

    #[test]
    fn topk_1d_basic() {
        let x = Tensor::<f32>::from_slice(
            &[0.1, 0.5, 0.3, 0.9, 0.7],
            [5].into(),
        )
        .unwrap();
        let (vals, idx) = topk(&x, 3, -1).unwrap();
        assert_eq!(vals.shape().dims(), &[3]);
        assert_eq!(idx.shape().dims(), &[3]);
        assert_eq!(vals.as_slice().unwrap(), &[0.9, 0.7, 0.5]);
        assert_eq!(idx.as_slice().unwrap(), &[3, 4, 1]);
    }

    #[test]
    fn topk_2d_axis_last() {
        let x = Tensor::<f32>::from_slice(
            &[
                0.1, 0.9, 0.3, // row 0 : top-2 = (0.9@1, 0.3@2)
                0.5, 0.5, 0.5, // row 1 : all equal, ties by index
            ],
            [2, 3].into(),
        )
        .unwrap();
        let (vals, idx) = topk(&x, 2, -1).unwrap();
        assert_eq!(vals.shape().dims(), &[2, 2]);
        let v = vals.as_slice().unwrap();
        let i = idx.as_slice().unwrap();
        // Row 0
        assert!((v[0] - 0.9).abs() < 1e-6);
        assert!((v[1] - 0.3).abs() < 1e-6);
        assert_eq!(i[0], 1);
        assert_eq!(i[1], 2);
        // Row 1 : ties → indices 0, 1
        assert_eq!(i[2], 0);
        assert_eq!(i[3], 1);
    }

    #[test]
    fn topk_k_equals_n_returns_all_sorted_desc() {
        let x = Tensor::<f32>::from_slice(&[3.0, 1.0, 2.0], [3].into()).unwrap();
        let (vals, _idx) = topk(&x, 3, -1).unwrap();
        let v = vals.as_slice().unwrap();
        assert_eq!(v[0], 3.0);
        assert_eq!(v[1], 2.0);
        assert_eq!(v[2], 1.0);
    }

    #[test]
    fn topk_rejects_k_zero() {
        let x = Tensor::<f32>::from_slice(&[1.0, 2.0], [2].into()).unwrap();
        assert!(topk(&x, 0, -1).is_err());
    }

    #[test]
    fn topk_rejects_k_too_large() {
        let x = Tensor::<f32>::from_slice(&[1.0, 2.0], [2].into()).unwrap();
        assert!(topk(&x, 5, -1).is_err());
    }

    #[test]
    fn topk_single_element() {
        let x = Tensor::<f32>::from_slice(&[42.0], [1].into()).unwrap();
        let (vals, idx) = topk(&x, 1, -1).unwrap();
        assert_eq!(vals.as_slice().unwrap(), &[42.0]);
        assert_eq!(idx.as_slice().unwrap(), &[0]);
    }

    #[test]
    fn topk_handles_negative_values() {
        let x = Tensor::<f32>::from_slice(
            &[-3.0, -1.0, -5.0, -2.0],
            [4].into(),
        )
        .unwrap();
        let (vals, idx) = topk(&x, 2, -1).unwrap();
        assert_eq!(vals.as_slice().unwrap(), &[-1.0, -2.0]);
        assert_eq!(idx.as_slice().unwrap(), &[1, 3]);
    }

    // ===== scaled_dot_product_attention =====

    #[test]
    fn sdpa_rejects_non_rank4() {
        let q = Tensor::<f32>::zeros([2, 3].into()).unwrap();
        let k = Tensor::<f32>::zeros([2, 3].into()).unwrap();
        let v = Tensor::<f32>::zeros([2, 3].into()).unwrap();
        assert!(scaled_dot_product_attention(&q, &k, &v, None).is_err());
    }

    #[test]
    fn sdpa_shape_preservation() {
        let q = Tensor::<f32>::zeros([1, 1, 4, 8].into()).unwrap();
        let k = Tensor::<f32>::zeros([1, 1, 4, 8].into()).unwrap();
        let v = Tensor::<f32>::zeros([1, 1, 4, 8].into()).unwrap();
        let out = scaled_dot_product_attention(&q, &k, &v, None).unwrap();
        assert_eq!(out.shape().dims(), &[1, 1, 4, 8]);
    }

    #[test]
    fn sdpa_identity_uniform_output() {
        // Q = K = V = zeros → attention weights uniform → output = 0
        let q = Tensor::<f32>::zeros([1, 1, 3, 2].into()).unwrap();
        let out = scaled_dot_product_attention(&q, &q, &q, None).unwrap();
        for &v in out.as_slice().unwrap() {
            assert!(v.abs() < 1e-6);
        }
    }

    #[test]
    fn sdpa_rejects_mismatched_shapes() {
        let q = Tensor::<f32>::zeros([1, 1, 4, 8].into()).unwrap();
        let k = Tensor::<f32>::zeros([1, 2, 4, 8].into()).unwrap(); // wrong head count
        let v = Tensor::<f32>::zeros([1, 1, 4, 8].into()).unwrap();
        assert!(scaled_dot_product_attention(&q, &k, &v, None).is_err());
    }

    #[test]
    fn sdpa_rejects_zero_d_k() {
        // Bug-fix guard : d_k = 0 would cause division-by-zero scaling.
        let q = Tensor::<f32>::zeros([1, 1, 2, 0].into()).unwrap();
        let k = Tensor::<f32>::zeros([1, 1, 2, 0].into()).unwrap();
        let v = Tensor::<f32>::zeros([1, 1, 2, 4].into()).unwrap();
        assert!(scaled_dot_product_attention(&q, &k, &v, None).is_err());
    }

    #[test]
    fn sdpa_applies_mask() {
        // With all Q, K, V equal, no mask → uniform attention, output = V mean.
        // With upper-triangular -Inf mask → lower-triangular attention.
        let t = 2;
        let d = 2;
        // Q = K = V = [[1,1],[1,1]]
        let data = vec![1.0f32; t * d];
        let q = Tensor::<f32>::from_slice(&data, [1, 1, t, d].into()).unwrap();
        let mask_data = vec![0.0_f32, f32::NEG_INFINITY, 0.0, 0.0];
        let mask = Tensor::<f32>::from_slice(&mask_data, [t, t].into()).unwrap();
        let out = scaled_dot_product_attention(&q, &q, &q, Some(&mask)).unwrap();
        // Output is a valid tensor (weights normalize to 1 even with mask)
        assert_eq!(out.shape().dims(), &[1, 1, t, d]);
        for &v in out.as_slice().unwrap() {
            assert!(v.is_finite());
        }
    }
}
