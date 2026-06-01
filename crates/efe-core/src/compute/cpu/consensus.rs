//! Sprint 3 CPU kernels — the **three core EFE operations**, plus the
//! **two Sprint 5 cosine ops**.
//!
//! Sprint 3 (mathematical heart of the Ephemeral Fluid Swarm):
//!
//! 1. [`cone_project`] — pull latents into the alignment cone
//! 2. [`weighted_mean`] — aggregate contributor latents into a consensus
//! 3. [`moe_reconcile`] — route via top-k gate softmax
//!
//! Sprint 5 (cross-module consolidation):
//!
//! 4. [`cosine_similarity_pair`] — `a·b / (‖a‖·‖b‖)`, deterministic
//!    fallback to 0 on near-zero norms.
//! 5. [`cosine_anchor_blend`] — soft projection of a vector onto the
//!    convex hull of anchors weighted by ReLU(cosine).
//!
//! All ops are implemented with numerical robustness in mind: Kahan
//! summation where accumulation matters, explicit NaN/Inf checks, and
//! squared-distance comparisons (to avoid `sqrt` until strictly needed).

use super::shared::{kahan_sum_slice, topk_row};
use crate::compute::error::{ComputeError, Result};
use crate::compute::tensor::Tensor;

// =====================================================================
// cone_project — CORE EFE #1
// =====================================================================

/// Project a batch of latents onto the alignment cone defined by anchors.
///
/// # Definition
///
/// Given latents `L = {zᵢ}` and anchors `A = {aⱼ}`, for each `zᵢ`:
///
/// 1. Find the nearest anchor `j*(i) = argmin_j ‖zᵢ − aⱼ‖₂`
/// 2. Let `d = ‖zᵢ − a_{j*(i)}‖₂`
/// 3. If `d ≤ max_dist`, keep `zᵢ` unchanged.
/// 4. Otherwise, pull `zᵢ` radially toward `a_{j*(i)}` until the
///    distance equals `max_dist`.
///
/// The alignment cone is thus the union of closed L2 balls of radius
/// `max_dist` centred on each anchor.
///
/// # Shape contract
///
/// * `latents : (B, d)` rank 2
/// * `anchors : (N, d)` rank 2, same `d`, at least one row
/// * `max_dist > 0 ∧ finite`
///
/// # Complexity
///
/// `O(B · N · d)` — one squared-distance per (latent, anchor) pair.
///
/// # Numerical notes
///
/// * Squared distances are compared to avoid an unnecessary `sqrt`.
///   Only the single selected distance gets square-rooted for the
///   projection scaling factor.
/// * When `d_min == 0` (latent exactly on an anchor), the function
///   returns the latent unchanged ; the projection branch is never
///   entered in this case because `0 ≤ max_dist` always holds.
///
/// # NaN / Inf propagation
///
/// * If a latent contains `NaN`, the corresponding row in the output
///   contains `NaN` (squared distance is NaN, compare-to-best is false,
///   best stays at `INFINITY`, projection formula multiplies `NaN` by
///   a finite scale and yields `NaN`). This matches PyTorch behaviour
///   and is the caller's responsibility to detect downstream.
/// * If a latent is `+Inf` or `-Inf`, squared distance is `+Inf`,
///   `d_min = +Inf`, `scale = max_dist / +Inf = 0`, so the output
///   formula becomes `anchor + Inf · 0 = NaN` on that row. Same
///   guidance as above.
/// * Anchors with NaN / Inf produce similar NaN pollution.
pub fn cone_project(
    latents: &Tensor<f32>,
    anchors: &Tensor<f32>,
    max_dist: f32,
) -> Result<Tensor<f32>> {
    // 1. Validation
    if !max_dist.is_finite() || max_dist <= 0.0 {
        return Err(ComputeError::NumericalInstability {
            op: "cone_project",
            detail: format!("max_dist must be finite and > 0, got {max_dist}"),
        });
    }
    let l_dims = latents.shape().dims();
    let a_dims = anchors.shape().dims();
    if l_dims.len() != 2 || a_dims.len() != 2 {
        return Err(ComputeError::ShapeMismatch {
            op: "cone_project",
            expected: "both tensors rank 2".into(),
            actual: format!(
                "latents.rank={}, anchors.rank={}",
                l_dims.len(),
                a_dims.len()
            ),
        });
    }
    if l_dims[1] != a_dims[1] {
        return Err(ComputeError::ShapeMismatch {
            op: "cone_project",
            expected: format!("matching last dim (latents.d={})", l_dims[1]),
            actual: format!("anchors.d={}", a_dims[1]),
        });
    }
    let (b, d) = (l_dims[0], l_dims[1]);
    let n = a_dims[0];
    if n == 0 {
        return Err(ComputeError::ShapeMismatch {
            op: "cone_project",
            expected: "at least 1 anchor".into(),
            actual: "0 anchors".into(),
        });
    }

    let lat = latents.as_slice()?;
    let anch = anchors.as_slice()?;
    let mut out = vec![0.0f32; b * d];
    let max_dist_sq = max_dist * max_dist;

    for bi in 0..b {
        // 2a. Find argmin squared distance
        let mut best_sq = f32::INFINITY;
        let mut best_j = 0usize;
        for j in 0..n {
            let mut sq = 0.0f32;
            for ki in 0..d {
                let diff = lat[bi * d + ki] - anch[j * d + ki];
                sq += diff * diff;
            }
            if sq < best_sq {
                best_sq = sq;
                best_j = j;
            }
        }

        // 2b. Project if outside cone
        if best_sq <= max_dist_sq {
            // Keep unchanged
            for ki in 0..d {
                out[bi * d + ki] = lat[bi * d + ki];
            }
        } else {
            // best_sq > max_dist_sq > 0, so sqrt is safe.
            let d_min = best_sq.sqrt();
            let scale = max_dist / d_min;
            for ki in 0..d {
                let a = anch[best_j * d + ki];
                out[bi * d + ki] = a + (lat[bi * d + ki] - a) * scale;
            }
        }
    }

    Tensor::from_slice(&out, [b, d].into())
}

// =====================================================================
// weighted_mean — CORE EFE #2
// =====================================================================

/// Weighted mean along batch axis, with optional L2 renormalisation.
///
/// # Definition
///
/// ```text
///     μ_k = (Σ_b w_b · v_{b,k}) / (Σ_b w_b)      for k ∈ [0, d)
/// ```
///
/// If `renormalize_l2 == true` :
///
/// ```text
///     μ_k ← μ_k / max(‖μ‖₂, 1e-9)
/// ```
///
/// # Shape contract
///
/// * `values : (B, d)` rank 2
/// * `weights : (B,)` rank 1
/// * `Σ w_b > 0` (non-zero weights, else `Err(NumericalInstability)`)
///
/// # Numerical stability
///
/// * Weight sum uses Kahan compensated summation.
/// * Per-dimension weighted sum uses an inlined Kahan accumulator.
///
/// # Invariants
///
/// * `weighted_mean(v, w=1_B)` ≈ plain mean over batch axis
/// * `weighted_mean(v, w)` is invariant to positive scaling of `w`
/// * Permutation-invariant (up to floating-point drift)
pub fn weighted_mean(
    values: &Tensor<f32>,
    weights: &Tensor<f32>,
    renormalize_l2: bool,
) -> Result<Tensor<f32>> {
    let v_dims = values.shape().dims();
    let w_dims = weights.shape().dims();
    if v_dims.len() != 2 {
        return Err(ComputeError::ShapeMismatch {
            op: "weighted_mean",
            expected: "values rank 2 (B, d)".into(),
            actual: format!("rank {}", v_dims.len()),
        });
    }
    if w_dims.len() != 1 {
        return Err(ComputeError::ShapeMismatch {
            op: "weighted_mean",
            expected: "weights rank 1 (B,)".into(),
            actual: format!("rank {}", w_dims.len()),
        });
    }
    if v_dims[0] != w_dims[0] {
        return Err(ComputeError::ShapeMismatch {
            op: "weighted_mean",
            expected: format!("B matches: values.B={}", v_dims[0]),
            actual: format!("weights.B={}", w_dims[0]),
        });
    }
    let (b, d) = (v_dims[0], v_dims[1]);

    let vals = values.as_slice()?;
    let ws = weights.as_slice()?;

    // Sum of weights (Kahan)
    let w_sum = kahan_sum_slice(ws);
    if w_sum <= 0.0 || !w_sum.is_finite() {
        return Err(ComputeError::NumericalInstability {
            op: "weighted_mean",
            detail: format!("sum of weights is {w_sum}, must be > 0 and finite"),
        });
    }

    // Weighted sum per dimension with Kahan compensation
    let mut out = vec![0.0f32; d];
    for k in 0..d {
        let mut sum = 0.0f32;
        let mut comp = 0.0f32;
        for bi in 0..b {
            let y = ws[bi] * vals[bi * d + k] - comp;
            let t = sum + y;
            comp = (t - sum) - y;
            sum = t;
        }
        out[k] = sum / w_sum;
    }

    if renormalize_l2 {
        let norm_sq: f32 = out.iter().map(|v| v * v).sum();
        let norm = norm_sq.sqrt().max(1e-9);
        for v in &mut out {
            *v /= norm;
        }
    }

    Tensor::from_slice(&out, [d].into())
}

// =====================================================================
// moe_reconcile — CORE EFE #3
// =====================================================================

/// Deterministic mixture-of-experts reconciliation with top-k selection.
///
/// # Definition
///
/// Per batch `b`:
///
/// 1. Select the top-`top_k` indices of `gate_logits_b` (ties broken by
///    smallest index).
/// 2. Apply softmax to the selected logits → weights `w_k`.
/// 3. Output `out_b = Σ_k w_k · candidates_{b, idx_k}` (shape `d`).
///
/// # Shape contract
///
/// * `candidates : (B, K, d)` rank 3
/// * `gate_logits : (B, K)` rank 2
/// * `0 < top_k ≤ K`
///
/// # Invariants
///
/// * `top_k = K` reduces to `softmax(gate_logits)`-weighted sum over all.
/// * `top_k = 1` reduces to `candidates[argmax(gate_logits)]`.
/// * Fully deterministic: no RNG.
pub fn moe_reconcile(
    candidates: &Tensor<f32>,
    gate_logits: &Tensor<f32>,
    top_k: u32,
) -> Result<Tensor<f32>> {
    let c_dims = candidates.shape().dims();
    let g_dims = gate_logits.shape().dims();
    if c_dims.len() != 3 {
        return Err(ComputeError::ShapeMismatch {
            op: "moe_reconcile",
            expected: "candidates rank 3 (B, K, d)".into(),
            actual: format!("rank {}", c_dims.len()),
        });
    }
    if g_dims.len() != 2 {
        return Err(ComputeError::ShapeMismatch {
            op: "moe_reconcile",
            expected: "gate_logits rank 2 (B, K)".into(),
            actual: format!("rank {}", g_dims.len()),
        });
    }
    if c_dims[0] != g_dims[0] || c_dims[1] != g_dims[1] {
        return Err(ComputeError::ShapeMismatch {
            op: "moe_reconcile",
            expected: format!(
                "candidates and gate_logits share (B, K): ({}, {})",
                c_dims[0], c_dims[1]
            ),
            actual: format!(
                "gate_logits = ({}, {})",
                g_dims[0], g_dims[1]
            ),
        });
    }
    let (b, k_total, d) = (c_dims[0], c_dims[1], c_dims[2]);
    let top_k_usize = top_k as usize;
    if top_k_usize == 0 || top_k_usize > k_total {
        return Err(ComputeError::ShapeMismatch {
            op: "moe_reconcile",
            expected: format!("0 < top_k ≤ {k_total}"),
            actual: format!("top_k = {top_k_usize}"),
        });
    }

    let cands = candidates.as_slice()?;
    let gl = gate_logits.as_slice()?;
    let mut out = vec![0.0f32; b * d];

    for bi in 0..b {
        let row = &gl[bi * k_total..(bi + 1) * k_total];
        let (top_values, top_indices) = topk_row(row, top_k_usize);

        // Softmax over selected top_k values (LogSumExp for stability)
        let max_val = top_values
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        if !max_val.is_finite() {
            return Err(ComputeError::NumericalInstability {
                op: "moe_reconcile",
                detail: format!(
                    "batch {bi}: top gate logits contain no finite max"
                ),
            });
        }
        let exps: Vec<f32> = top_values
            .iter()
            .map(|v| (v - max_val).exp())
            .collect();
        let sum_exp: f32 = exps.iter().sum();
        if sum_exp <= 0.0 || !sum_exp.is_finite() {
            return Err(ComputeError::NumericalInstability {
                op: "moe_reconcile",
                detail: format!("batch {bi}: softmax denominator = {sum_exp}"),
            });
        }
        let weights: Vec<f32> = exps.iter().map(|e| e / sum_exp).collect();

        // Weighted sum of selected candidates
        for dim in 0..d {
            let mut acc = 0.0f32;
            for (ki, &ci) in top_indices.iter().enumerate() {
                let candidate_offset =
                    bi * k_total * d + (ci as usize) * d + dim;
                acc += weights[ki] * cands[candidate_offset];
            }
            out[bi * d + dim] = acc;
        }
    }

    Tensor::from_slice(&out, [b, d].into())
}

// =====================================================================
// cosine_similarity_pair — Sprint 5
// =====================================================================

/// Cosine similarity between two rank-1 vectors of equal length.
///
/// # Definition
///
/// ```text
///     cos(a, b) = (a · b) / (‖a‖₂ · ‖b‖₂)
/// ```
///
/// Returns `0.0` if either vector has near-zero L2 norm
/// (`‖a‖·‖b‖ < f32::EPSILON`) — the geometric limit of undefined
/// orientations. This matches the historical contract of
/// `els::metrics::cosine_similarity`.
///
/// # Errors
///
/// * [`ComputeError::ShapeMismatch`] — either input is not rank 1, or
///   shapes differ.
/// * [`ComputeError::NumericalInstability`] — any element non-finite.
pub fn cosine_similarity_pair(
    a: &Tensor<f32>,
    b: &Tensor<f32>,
) -> Result<f32> {
    if a.shape().rank() != 1 || b.shape().rank() != 1 {
        return Err(ComputeError::ShapeMismatch {
            op: "cosine_similarity_pair",
            expected: "both rank 1".into(),
            actual: format!(
                "a.rank={}, b.rank={}",
                a.shape().rank(),
                b.shape().rank()
            ),
        });
    }
    if a.shape().dims() != b.shape().dims() {
        return Err(ComputeError::ShapeMismatch {
            op: "cosine_similarity_pair",
            expected: format!("matching length {}", a.shape().dims()[0]),
            actual: format!("{}", b.shape().dims()[0]),
        });
    }
    let sa = a.as_slice()?;
    let sb = b.as_slice()?;
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (&x, &y) in sa.iter().zip(sb.iter()) {
        if !x.is_finite() || !y.is_finite() {
            return Err(ComputeError::NumericalInstability {
                op: "cosine_similarity_pair",
                detail: format!("non-finite element (a={x}, b={y})"),
            });
        }
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let denom = (na * nb).sqrt();
    if denom < f32::EPSILON {
        Ok(0.0)
    } else {
        Ok(dot / denom)
    }
}

// =====================================================================
// cosine_anchor_blend — Sprint 5
// =====================================================================

/// Soft projection of `vector` onto the convex hull of `anchors`,
/// blended with the input by `strength`.
///
/// # Algorithm
///
/// ```text
///     w_j = max(0, cos(vector, anchors[j]))     for j in 0..N
///     Σw  = Σ_j w_j
///     if Σw < ε:
///         return (vector, 0.0)                  # geometric identity
///     w'_j = w_j / Σw
///     centroid = Σ_j w'_j · anchors[j]
///     result   = (1 − strength) · vector + strength · centroid
///     delta    = ‖vector − result‖₂
/// ```
///
/// # Shape contract
///
/// * `vector : (D,)` rank 1.
/// * `anchors : (N, D)` rank 2 with `N ≥ 1`.
/// * `strength ∈ [0, 1]` finite.
///
/// # Equivalence with the legacy `els::projection`
///
/// This op is the strict numerical equivalent of
/// `els::projection::project_vector_onto_anchors` (Sprint 5 audit
/// captured this redundancy across `els`, `dissolution::alignment`,
/// `fusion::kernels::project`).
#[allow(clippy::too_many_lines)] // 3 validations + 2 passes + Kahan blend = ~100 lines, splitting hurts readability
pub fn cosine_anchor_blend(
    vector: &Tensor<f32>,
    anchors: &Tensor<f32>,
    strength: f32,
) -> Result<(Tensor<f32>, f32)> {
    if !strength.is_finite() || !(0.0..=1.0).contains(&strength) {
        return Err(ComputeError::NumericalInstability {
            op: "cosine_anchor_blend",
            detail: format!("strength must be in [0, 1] and finite, got {strength}"),
        });
    }
    if vector.shape().rank() != 1 {
        return Err(ComputeError::ShapeMismatch {
            op: "cosine_anchor_blend",
            expected: "vector rank 1".into(),
            actual: format!("rank {}", vector.shape().rank()),
        });
    }
    let a_dims = anchors.shape().dims();
    if a_dims.len() != 2 {
        return Err(ComputeError::ShapeMismatch {
            op: "cosine_anchor_blend",
            expected: "anchors rank 2 (N, D)".into(),
            actual: format!("rank {}", a_dims.len()),
        });
    }
    let dim = vector.shape().dims()[0];
    if dim == 0 || a_dims[1] != dim {
        return Err(ComputeError::ShapeMismatch {
            op: "cosine_anchor_blend",
            expected: format!("anchors.D = vector.D = {dim}, both > 0"),
            actual: format!("anchors.D = {}", a_dims[1]),
        });
    }
    let n = a_dims[0];
    if n == 0 {
        return Err(ComputeError::ShapeMismatch {
            op: "cosine_anchor_blend",
            expected: "at least 1 anchor".into(),
            actual: "0 anchors".into(),
        });
    }

    let v = vector.as_slice()?;
    let a = anchors.as_slice()?;

    // Validate finite inputs upfront (audit-5 hardening).
    for (idx, &x) in v.iter().enumerate() {
        if !x.is_finite() {
            return Err(ComputeError::NumericalInstability {
                op: "cosine_anchor_blend",
                detail: format!("non-finite vector[{idx}] = {x}"),
            });
        }
    }
    // We compute squared norms inline (cosine = dot/√(na·nb)).
    let mut na = 0.0f32;
    for &x in v {
        na += x * x;
    }

    // Pass 1: compute positive cosine weights.
    let mut weights = vec![0.0f32; n];
    let mut total = 0.0f32;
    for j in 0..n {
        let row = &a[j * dim..(j + 1) * dim];
        let mut dot = 0.0f32;
        let mut nb = 0.0f32;
        for (k, &y) in row.iter().enumerate() {
            if !y.is_finite() {
                return Err(ComputeError::NumericalInstability {
                    op: "cosine_anchor_blend",
                    detail: format!("non-finite anchors[{j}][{k}] = {y}"),
                });
            }
            dot += v[k] * y;
            nb += y * y;
        }
        let denom = (na * nb).sqrt();
        let cos = if denom < f32::EPSILON { 0.0 } else { dot / denom };
        let w = cos.max(0.0);
        weights[j] = w;
        total += w;
    }

    if total < f32::EPSILON {
        // No anchor pulls the vector; geometric identity.
        let mut out = vec![0.0f32; dim];
        out.copy_from_slice(v);
        let result = Tensor::from_slice(&out, [dim].into())?;
        return Ok((result, 0.0));
    }

    // Pass 2: weighted centroid.
    let mut centroid = vec![0.0f32; dim];
    for j in 0..n {
        let w_norm = weights[j] / total;
        if w_norm == 0.0 {
            continue;
        }
        let row = &a[j * dim..(j + 1) * dim];
        for (k, &y) in row.iter().enumerate() {
            centroid[k] += w_norm * y;
        }
    }

    // Blend + delta. Kahan-compensated accumulation for `delta_sq` so
    // the returned `delta` matches `l2_distance(vector, result)` to
    // within `1e-7` even for `D ≥ 10⁴`. Without Kahan compensation, the
    // two ops would drift by `O(D · ε)` and the debug-assert in the
    // `els::projection` façade would risk firing for very long vectors.
    let mut result_buf = vec![0.0f32; dim];
    let mut delta_sq = 0.0f32;
    let mut comp = 0.0f32;
    for k in 0..dim {
        let r = (1.0 - strength) * v[k] + strength * centroid[k];
        result_buf[k] = r;
        let d = v[k] - r;
        let term = d * d - comp;
        let next = delta_sq + term;
        comp = (next - delta_sq) - term;
        delta_sq = next;
    }
    let result = Tensor::from_slice(&result_buf, [dim].into())?;
    Ok((result, delta_sq.sqrt()))
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    // =====================================================================
    // cone_project tests
    // =====================================================================

    #[test]
    fn cone_project_inside_cone_is_noop() {
        // Latents all within max_dist of anchor 0
        let latents = Tensor::<f32>::from_slice(
            &[0.1, 0.1, -0.2, 0.05],
            [2, 2].into(),
        )
        .unwrap();
        let anchors = Tensor::<f32>::from_slice(&[0.0, 0.0], [1, 2].into()).unwrap();
        let out = cone_project(&latents, &anchors, 1.0).unwrap();
        for (a, b) in out
            .as_slice()
            .unwrap()
            .iter()
            .zip(latents.as_slice().unwrap())
        {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn cone_project_outside_projects_to_boundary() {
        // Latent at (3, 4) from anchor (0, 0), distance 5. max_dist 1.0
        // → project to (3/5, 4/5)
        let latents = Tensor::<f32>::from_slice(&[3.0, 4.0], [1, 2].into()).unwrap();
        let anchors = Tensor::<f32>::from_slice(&[0.0, 0.0], [1, 2].into()).unwrap();
        let out = cone_project(&latents, &anchors, 1.0).unwrap();
        let s = out.as_slice().unwrap();
        assert!((s[0] - 0.6).abs() < 1e-5);
        assert!((s[1] - 0.8).abs() < 1e-5);
        // Verify distance is now exactly max_dist
        let d = (s[0] * s[0] + s[1] * s[1]).sqrt();
        assert!((d - 1.0).abs() < 1e-5);
    }

    #[test]
    fn cone_project_idempotent() {
        let latents = Tensor::<f32>::from_slice(
            &[3.0, 4.0, 0.1, 0.0, -0.2, 0.3],
            [3, 2].into(),
        )
        .unwrap();
        let anchors = Tensor::<f32>::from_slice(&[0.0, 0.0], [1, 2].into()).unwrap();
        let once = cone_project(&latents, &anchors, 1.0).unwrap();
        let twice = cone_project(&once, &anchors, 1.0).unwrap();
        let s1 = once.as_slice().unwrap();
        let s2 = twice.as_slice().unwrap();
        for i in 0..s1.len() {
            assert!((s1[i] - s2[i]).abs() < 1e-5);
        }
    }

    #[test]
    fn cone_project_multiple_anchors_picks_nearest() {
        // Latent at (2, 0). Anchors: A0=(0,0) dist 2, A1=(3,0) dist 1.
        // max_dist = 0.5 : nearest is A1, project toward A1 at 0.5
        // Direction from A1 to latent: (-1, 0), normalized = (-1, 0),
        // projected point : A1 + 0.5 * (-1, 0) = (2.5, 0)
        let latents = Tensor::<f32>::from_slice(&[2.0, 0.0], [1, 2].into()).unwrap();
        let anchors = Tensor::<f32>::from_slice(
            &[0.0, 0.0, 3.0, 0.0],
            [2, 2].into(),
        )
        .unwrap();
        let out = cone_project(&latents, &anchors, 0.5).unwrap();
        let s = out.as_slice().unwrap();
        assert!((s[0] - 2.5).abs() < 1e-5);
        assert!(s[1].abs() < 1e-5);
    }

    #[test]
    fn cone_project_rejects_non_positive_max_dist() {
        let latents = Tensor::<f32>::zeros([1, 2].into()).unwrap();
        let anchors = Tensor::<f32>::zeros([1, 2].into()).unwrap();
        assert!(cone_project(&latents, &anchors, 0.0).is_err());
        assert!(cone_project(&latents, &anchors, -1.0).is_err());
        assert!(cone_project(&latents, &anchors, f32::NAN).is_err());
    }

    #[test]
    fn cone_project_rejects_mismatched_dim() {
        let latents = Tensor::<f32>::zeros([1, 2].into()).unwrap();
        let anchors = Tensor::<f32>::zeros([1, 3].into()).unwrap();
        assert!(cone_project(&latents, &anchors, 1.0).is_err());
    }

    #[test]
    fn cone_project_rejects_no_anchors() {
        let latents = Tensor::<f32>::zeros([1, 2].into()).unwrap();
        let anchors = Tensor::<f32>::zeros([0, 2].into()).unwrap();
        assert!(cone_project(&latents, &anchors, 1.0).is_err());
    }

    #[test]
    fn cone_project_shape_preservation() {
        let latents = Tensor::<f32>::zeros([5, 3].into()).unwrap();
        let anchors = Tensor::<f32>::zeros([2, 3].into()).unwrap();
        let out = cone_project(&latents, &anchors, 1.0).unwrap();
        assert_eq!(out.shape().dims(), &[5, 3]);
    }

    #[test]
    fn cone_project_boundary_case_exactly_at_max_dist() {
        // Latent at distance exactly = max_dist → kept
        let latents = Tensor::<f32>::from_slice(&[1.0, 0.0], [1, 2].into()).unwrap();
        let anchors = Tensor::<f32>::from_slice(&[0.0, 0.0], [1, 2].into()).unwrap();
        let out = cone_project(&latents, &anchors, 1.0).unwrap();
        let s = out.as_slice().unwrap();
        assert!((s[0] - 1.0).abs() < 1e-5);
        assert!(s[1].abs() < 1e-5);
    }

    #[test]
    fn cone_project_nan_input_propagates_nan_on_that_row() {
        // Bug-fix coverage : documented behaviour — NaN in latents
        // produces NaN in the corresponding output row.
        let latents = Tensor::<f32>::from_slice(
            &[1.0, 2.0, f32::NAN, 0.5],
            [2, 2].into(),
        )
        .unwrap();
        let anchors = Tensor::<f32>::from_slice(&[0.0, 0.0], [1, 2].into()).unwrap();
        let out = cone_project(&latents, &anchors, 1.0).unwrap();
        let s = out.as_slice().unwrap();
        // Row 0 : finite (≤ 1.0)
        let norm0 = (s[0] * s[0] + s[1] * s[1]).sqrt();
        assert!(norm0.is_finite());
        assert!(norm0 <= 1.0 + 1e-5);
        // Row 1 : contains NaN (propagated)
        assert!(s[2].is_nan() || s[3].is_nan());
    }

    #[test]
    fn cone_project_inf_input_produces_nan_on_that_row() {
        // Bug-fix coverage : documented behaviour — +Inf latent
        // multiplies 0 in the projection step → NaN.
        let latents = Tensor::<f32>::from_slice(
            &[f32::INFINITY, 0.0],
            [1, 2].into(),
        )
        .unwrap();
        let anchors = Tensor::<f32>::from_slice(&[0.0, 0.0], [1, 2].into()).unwrap();
        let out = cone_project(&latents, &anchors, 1.0).unwrap();
        let s = out.as_slice().unwrap();
        // Expect at least one NaN
        assert!(s[0].is_nan() || s[1].is_nan());
    }

    #[test]
    fn cone_project_output_bounded_by_max_dist() {
        // For any input, output must be within max_dist of some anchor.
        let latents = Tensor::<f32>::from_slice(
            &[10.0, 0.0, -10.0, 5.0, 0.0, 100.0],
            [3, 2].into(),
        )
        .unwrap();
        let anchors = Tensor::<f32>::from_slice(
            &[0.0, 0.0, 1.0, 1.0],
            [2, 2].into(),
        )
        .unwrap();
        let max_dist = 0.5f32;
        let out = cone_project(&latents, &anchors, max_dist).unwrap();
        let s = out.as_slice().unwrap();
        let a = anchors.as_slice().unwrap();
        // For each output point, closest anchor distance ≤ max_dist + tiny
        for bi in 0..3 {
            let mut best = f32::INFINITY;
            for ai in 0..2 {
                let dx = s[bi * 2] - a[ai * 2];
                let dy = s[bi * 2 + 1] - a[ai * 2 + 1];
                let dist = (dx * dx + dy * dy).sqrt();
                if dist < best {
                    best = dist;
                }
            }
            assert!(
                best <= max_dist + 1e-5,
                "batch {bi} distance {best} > max_dist {max_dist}"
            );
        }
    }

    // =====================================================================
    // weighted_mean tests
    // =====================================================================

    #[test]
    fn weighted_mean_uniform_weights_equals_mean() {
        let values = Tensor::<f32>::from_slice(
            &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            [3, 2].into(),
        )
        .unwrap();
        let weights = Tensor::<f32>::from_slice(&[1.0, 1.0, 1.0], [3].into()).unwrap();
        let out = weighted_mean(&values, &weights, false).unwrap();
        let s = out.as_slice().unwrap();
        // Mean : [(1+3+5)/3, (2+4+6)/3] = [3, 4]
        assert!((s[0] - 3.0).abs() < 1e-5);
        assert!((s[1] - 4.0).abs() < 1e-5);
    }

    #[test]
    fn weighted_mean_single_nonzero_weight_selects() {
        let values = Tensor::<f32>::from_slice(
            &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            [3, 2].into(),
        )
        .unwrap();
        let weights = Tensor::<f32>::from_slice(&[0.0, 1.0, 0.0], [3].into()).unwrap();
        let out = weighted_mean(&values, &weights, false).unwrap();
        let s = out.as_slice().unwrap();
        // Only row 1 counts : [3, 4]
        assert!((s[0] - 3.0).abs() < 1e-5);
        assert!((s[1] - 4.0).abs() < 1e-5);
    }

    #[test]
    fn weighted_mean_scale_invariant() {
        let values = Tensor::<f32>::from_slice(
            &[1.0, 2.0, 3.0, 4.0],
            [2, 2].into(),
        )
        .unwrap();
        let w1 = Tensor::<f32>::from_slice(&[0.5, 0.5], [2].into()).unwrap();
        let w2 = Tensor::<f32>::from_slice(&[50.0, 50.0], [2].into()).unwrap();
        let o1 = weighted_mean(&values, &w1, false).unwrap();
        let o2 = weighted_mean(&values, &w2, false).unwrap();
        let s1 = o1.as_slice().unwrap();
        let s2 = o2.as_slice().unwrap();
        for i in 0..2 {
            assert!((s1[i] - s2[i]).abs() < 1e-5);
        }
    }

    #[test]
    fn weighted_mean_renormalize_gives_unit_norm() {
        let values = Tensor::<f32>::from_slice(&[3.0, 4.0, 6.0, 8.0], [2, 2].into()).unwrap();
        let weights = Tensor::<f32>::from_slice(&[1.0, 1.0], [2].into()).unwrap();
        let out = weighted_mean(&values, &weights, true).unwrap();
        let norm: f32 = out.as_slice().unwrap().iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }

    #[test]
    fn weighted_mean_rejects_zero_weights() {
        let values = Tensor::<f32>::zeros([2, 2].into()).unwrap();
        let weights = Tensor::<f32>::zeros([2].into()).unwrap();
        let err = weighted_mean(&values, &weights, false).unwrap_err();
        assert!(matches!(err, ComputeError::NumericalInstability { .. }));
    }

    #[test]
    fn weighted_mean_rejects_shape_mismatch() {
        let values = Tensor::<f32>::zeros([3, 2].into()).unwrap();
        let weights = Tensor::<f32>::zeros([4].into()).unwrap();
        assert!(weighted_mean(&values, &weights, false).is_err());
    }

    #[test]
    fn weighted_mean_output_shape_is_d() {
        let values = Tensor::<f32>::zeros([5, 7].into()).unwrap();
        let weights = Tensor::<f32>::from_slice(&[1.0; 5], [5].into()).unwrap();
        let out = weighted_mean(&values, &weights, false).unwrap();
        assert_eq!(out.shape().dims(), &[7]);
    }

    // =====================================================================
    // moe_reconcile tests
    // =====================================================================

    #[test]
    fn moe_reconcile_top_k_1_equals_argmax_candidate() {
        // Candidates: (B=1, K=3, d=2)
        // cands[0, 0, :] = [1, 0]
        // cands[0, 1, :] = [0, 1]
        // cands[0, 2, :] = [1, 1]
        // gate: [0.1, 0.9, 0.5] → argmax = 1
        let cands = Tensor::<f32>::from_slice(
            &[1.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            [1, 3, 2].into(),
        )
        .unwrap();
        let gate = Tensor::<f32>::from_slice(&[0.1, 0.9, 0.5], [1, 3].into()).unwrap();
        let out = moe_reconcile(&cands, &gate, 1).unwrap();
        let s = out.as_slice().unwrap();
        // Output = candidates[argmax=1] = [0, 1]
        assert!((s[0]).abs() < 1e-5);
        assert!((s[1] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn moe_reconcile_top_k_equals_k_equals_softmax_sum() {
        // top_k = K : should equal softmax-weighted sum over all candidates.
        let cands = Tensor::<f32>::from_slice(
            &[1.0, 0.0, 0.0, 1.0],
            [1, 2, 2].into(),
        )
        .unwrap();
        let gate = Tensor::<f32>::from_slice(&[0.0, 0.0], [1, 2].into()).unwrap();
        // Uniform gate → 0.5 · [1,0] + 0.5 · [0,1] = [0.5, 0.5]
        let out = moe_reconcile(&cands, &gate, 2).unwrap();
        let s = out.as_slice().unwrap();
        assert!((s[0] - 0.5).abs() < 1e-5);
        assert!((s[1] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn moe_reconcile_batched() {
        // B=2 batches, each with K=2 candidates of d=2
        let cands = Tensor::<f32>::from_slice(
            &[
                // batch 0 : cands [1,0], [0,1]
                1.0, 0.0, 0.0, 1.0,
                // batch 1 : cands [2,0], [0,2]
                2.0, 0.0, 0.0, 2.0,
            ],
            [2, 2, 2].into(),
        )
        .unwrap();
        let gate = Tensor::<f32>::from_slice(
            &[
                10.0, 0.0, // batch 0 : strong preference for cand 0
                0.0, 10.0, // batch 1 : strong preference for cand 1
            ],
            [2, 2].into(),
        )
        .unwrap();
        let out = moe_reconcile(&cands, &gate, 1).unwrap();
        let s = out.as_slice().unwrap();
        // batch 0 → [1, 0], batch 1 → [0, 2]
        assert!((s[0] - 1.0).abs() < 1e-5);
        assert!((s[1]).abs() < 1e-5);
        assert!((s[2]).abs() < 1e-5);
        assert!((s[3] - 2.0).abs() < 1e-5);
    }

    #[test]
    fn moe_reconcile_rejects_zero_top_k() {
        let cands = Tensor::<f32>::zeros([1, 3, 2].into()).unwrap();
        let gate = Tensor::<f32>::zeros([1, 3].into()).unwrap();
        assert!(moe_reconcile(&cands, &gate, 0).is_err());
    }

    #[test]
    fn moe_reconcile_rejects_top_k_larger_than_k() {
        let cands = Tensor::<f32>::zeros([1, 3, 2].into()).unwrap();
        let gate = Tensor::<f32>::zeros([1, 3].into()).unwrap();
        assert!(moe_reconcile(&cands, &gate, 5).is_err());
    }

    #[test]
    fn moe_reconcile_rejects_shape_mismatch() {
        let cands = Tensor::<f32>::zeros([1, 3, 2].into()).unwrap();
        let gate = Tensor::<f32>::zeros([1, 4].into()).unwrap(); // K mismatch
        assert!(moe_reconcile(&cands, &gate, 1).is_err());
    }

    #[test]
    fn moe_reconcile_rejects_nan_in_gate() {
        let cands = Tensor::<f32>::zeros([1, 2, 2].into()).unwrap();
        let gate = Tensor::<f32>::from_slice(
            &[f32::NAN, f32::NAN],
            [1, 2].into(),
        )
        .unwrap();
        // Using total_cmp in topk_row, NaN > +Inf, so max_val = NaN
        // → should fail NumericalInstability
        let err = moe_reconcile(&cands, &gate, 1).unwrap_err();
        assert!(matches!(err, ComputeError::NumericalInstability { .. }));
    }

    #[test]
    fn moe_reconcile_output_shape_is_b_d() {
        let cands = Tensor::<f32>::zeros([3, 5, 7].into()).unwrap();
        let gate = Tensor::<f32>::zeros([3, 5].into()).unwrap();
        let out = moe_reconcile(&cands, &gate, 2).unwrap();
        assert_eq!(out.shape().dims(), &[3, 7]);
    }

    #[test]
    fn moe_reconcile_deterministic() {
        // Same input → same output (no RNG)
        let cands = Tensor::<f32>::from_slice(
            &[0.1, 0.2, 0.3, 0.4, 0.5, 0.6],
            [1, 3, 2].into(),
        )
        .unwrap();
        let gate = Tensor::<f32>::from_slice(&[0.3, 0.5, 0.2], [1, 3].into()).unwrap();
        let o1 = moe_reconcile(&cands, &gate, 2).unwrap();
        let o2 = moe_reconcile(&cands, &gate, 2).unwrap();
        assert_eq!(o1.as_slice().unwrap(), o2.as_slice().unwrap());
    }

    #[test]
    fn moe_reconcile_top_1_ties_pick_smallest_index() {
        // Two candidates, same gate logit → pick smallest index
        let cands = Tensor::<f32>::from_slice(
            &[1.0, 0.0, 0.0, 1.0],
            [1, 2, 2].into(),
        )
        .unwrap();
        let gate = Tensor::<f32>::from_slice(&[0.5, 0.5], [1, 2].into()).unwrap();
        let out = moe_reconcile(&cands, &gate, 1).unwrap();
        let s = out.as_slice().unwrap();
        // Tie → smallest index 0 → output = candidates[0] = [1, 0]
        assert!((s[0] - 1.0).abs() < 1e-5);
        assert!((s[1]).abs() < 1e-5);
    }

    // =====================================================================
    // cosine_similarity_pair tests (Sprint 5)
    // =====================================================================

    #[test]
    fn cosine_similarity_pair_identical_returns_one() {
        let a = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0], [3].into()).unwrap();
        let cos = cosine_similarity_pair(&a, &a).unwrap();
        assert!((cos - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_pair_opposite_returns_minus_one() {
        let a = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0], [3].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[-1.0, -2.0, -3.0], [3].into()).unwrap();
        let cos = cosine_similarity_pair(&a, &b).unwrap();
        assert!((cos + 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_pair_orthogonal_returns_zero() {
        let a = Tensor::<f32>::from_slice(&[1.0, 0.0], [2].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[0.0, 1.0], [2].into()).unwrap();
        let cos = cosine_similarity_pair(&a, &b).unwrap();
        assert!(cos.abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_pair_zero_vector_returns_zero() {
        // Geometric limit: ‖a‖·‖b‖ < ε → returns 0.0.
        let a = Tensor::<f32>::from_slice(&[0.0, 0.0, 0.0], [3].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0], [3].into()).unwrap();
        let cos = cosine_similarity_pair(&a, &b).unwrap();
        assert!(cos.abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_pair_rejects_shape_mismatch() {
        let a = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0], [3].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[1.0, 2.0], [2].into()).unwrap();
        assert!(matches!(
            cosine_similarity_pair(&a, &b).unwrap_err(),
            ComputeError::ShapeMismatch { .. }
        ));
    }

    #[test]
    fn cosine_similarity_pair_rejects_rank_2() {
        let a = Tensor::<f32>::zeros([2, 2].into()).unwrap();
        let b = Tensor::<f32>::zeros([2, 2].into()).unwrap();
        assert!(matches!(
            cosine_similarity_pair(&a, &b).unwrap_err(),
            ComputeError::ShapeMismatch { .. }
        ));
    }

    #[test]
    fn cosine_similarity_pair_rejects_nan() {
        let a = Tensor::<f32>::from_slice(&[f32::NAN, 1.0], [2].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[1.0, 1.0], [2].into()).unwrap();
        assert!(matches!(
            cosine_similarity_pair(&a, &b).unwrap_err(),
            ComputeError::NumericalInstability { .. }
        ));
    }

    #[test]
    fn cosine_similarity_pair_bounded_in_unit_range() {
        // Random-looking but deterministic vectors — cos must be in [-1, 1].
        let a = Tensor::<f32>::from_slice(&[0.3, -0.7, 1.2, -0.4], [4].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[1.1, 0.5, -0.3, 0.8], [4].into()).unwrap();
        let cos = cosine_similarity_pair(&a, &b).unwrap();
        assert!(cos.abs() <= 1.0 + 1e-6, "cos out of bounds: {cos}");
    }

    // =====================================================================
    // cosine_anchor_blend tests (Sprint 5)
    // =====================================================================

    #[test]
    fn cosine_anchor_blend_strength_zero_returns_input() {
        let v = Tensor::<f32>::from_slice(&[1.0, 0.5, -0.5], [3].into()).unwrap();
        let a = Tensor::<f32>::from_slice(&[0.3, 0.3, 0.3], [1, 3].into()).unwrap();
        let (out, delta) = cosine_anchor_blend(&v, &a, 0.0).unwrap();
        let s = out.as_slice().unwrap();
        assert_eq!(s, v.as_slice().unwrap());
        assert!(delta.abs() < f32::EPSILON);
    }

    #[test]
    fn cosine_anchor_blend_orthogonal_anchors_identity() {
        // v = [1,0], anchors orthogonal to v → all weights = 0 → identity.
        let v = Tensor::<f32>::from_slice(&[1.0, 0.0], [2].into()).unwrap();
        let a = Tensor::<f32>::from_slice(&[0.0, 1.0, 0.0, 2.0], [2, 2].into()).unwrap();
        let (out, delta) = cosine_anchor_blend(&v, &a, 1.0).unwrap();
        let s = out.as_slice().unwrap();
        assert_eq!(s, v.as_slice().unwrap());
        assert!(delta.abs() < f32::EPSILON);
    }

    #[test]
    fn cosine_anchor_blend_parallel_anchor_pulls_toward() {
        // v = [1,0], single parallel anchor at [0.5,0] → centroid [0.5,0].
        // Blend with strength=0.5 → [0.75, 0].
        let v = Tensor::<f32>::from_slice(&[1.0, 0.0], [2].into()).unwrap();
        let a = Tensor::<f32>::from_slice(&[0.5, 0.0], [1, 2].into()).unwrap();
        let (out, delta) = cosine_anchor_blend(&v, &a, 0.5).unwrap();
        let s = out.as_slice().unwrap();
        assert!((s[0] - 0.75).abs() < 1e-5);
        assert!(s[1].abs() < 1e-5);
        assert!(delta > 0.0);
    }

    #[test]
    fn cosine_anchor_blend_rejects_strength_out_of_range() {
        let v = Tensor::<f32>::from_slice(&[1.0, 0.0], [2].into()).unwrap();
        let a = Tensor::<f32>::from_slice(&[0.5, 0.0], [1, 2].into()).unwrap();
        assert!(matches!(
            cosine_anchor_blend(&v, &a, -0.1).unwrap_err(),
            ComputeError::NumericalInstability { .. }
        ));
        assert!(matches!(
            cosine_anchor_blend(&v, &a, 1.5).unwrap_err(),
            ComputeError::NumericalInstability { .. }
        ));
        assert!(matches!(
            cosine_anchor_blend(&v, &a, f32::NAN).unwrap_err(),
            ComputeError::NumericalInstability { .. }
        ));
    }

    #[test]
    fn cosine_anchor_blend_rejects_dim_mismatch() {
        let v = Tensor::<f32>::from_slice(&[1.0, 0.0, 0.0], [3].into()).unwrap();
        let a = Tensor::<f32>::from_slice(&[0.5, 0.0], [1, 2].into()).unwrap();
        assert!(matches!(
            cosine_anchor_blend(&v, &a, 0.5).unwrap_err(),
            ComputeError::ShapeMismatch { .. }
        ));
    }

    #[test]
    fn cosine_anchor_blend_rejects_zero_anchors() {
        let v = Tensor::<f32>::from_slice(&[1.0, 0.0], [2].into()).unwrap();
        let a = Tensor::<f32>::zeros([0, 2].into()).unwrap();
        assert!(matches!(
            cosine_anchor_blend(&v, &a, 0.5).unwrap_err(),
            ComputeError::ShapeMismatch { .. }
        ));
    }

    #[test]
    fn cosine_anchor_blend_rejects_nan_in_vector() {
        let v = Tensor::<f32>::from_slice(&[1.0, f32::NAN], [2].into()).unwrap();
        let a = Tensor::<f32>::from_slice(&[0.5, 0.0], [1, 2].into()).unwrap();
        assert!(matches!(
            cosine_anchor_blend(&v, &a, 0.5).unwrap_err(),
            ComputeError::NumericalInstability { .. }
        ));
    }

    #[test]
    fn cosine_anchor_blend_delta_bounded_by_strength_one() {
        // delta(strength=0.9) ≥ delta(strength=0.1) for partial alignment.
        let v = Tensor::<f32>::from_slice(&[1.0, 0.1], [2].into()).unwrap();
        let a = Tensor::<f32>::from_slice(&[0.5, 0.5], [1, 2].into()).unwrap();
        let (_, d_small) = cosine_anchor_blend(&v, &a, 0.1).unwrap();
        let (_, d_large) = cosine_anchor_blend(&v, &a, 0.9).unwrap();
        assert!(d_large > d_small);
    }
}
