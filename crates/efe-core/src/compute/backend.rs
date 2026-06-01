//! The [`ComputeBackend`] trait — single surface exposing the 23 numerical
//! primitives EFE relies on.
//!
//! # Groups
//!
//! 1. **Metadata** — `device`, `name`, `synchronize`
//! 2. **Linear algebra** — `matmul`, `matmul_fused_gelu`, `dot`,
//!    `l2_norm`, `l2_normalize`, `l2_distance` (6 ops)
//! 3. **Activations** — `softmax`, `log_softmax`, `gelu`, `sigmoid` (4 ops)
//! 4. **Attention & reduction** — `scaled_dot_product_attention`, `topk`,
//!    `argmax` (3 ops)
//! 5. **Consensus / cone** — `cone_project`, `weighted_mean`,
//!    `moe_reconcile`, `cosine_similarity_pair`, `cosine_anchor_blend`
//!    (5 ops, core EFE)
//! 6. **Probabilities & sampling** — `multinomial_sample`, `kl_divergence`,
//!    `shannon_entropy` (3 ops)
//! 7. **Statistics** — `mean`, `std_dev` (2 ops)
//!
//! Total: **23 operations**.
//!
//! # Implementation status by sprint
//!
//! | Sprint | Operations |
//! |--------|------------|
//! | **S1** | `l2_norm`, `l2_normalize`, `mean`, `argmax` |
//! | **S2** | `matmul`, `matmul_fused_gelu`, `dot`, `softmax`, `log_softmax`, `gelu` |
//! | **S3** | `scaled_dot_product_attention`, `topk`, `cone_project`, `weighted_mean`, `moe_reconcile`, `multinomial_sample`, `kl_divergence`, `std_dev` |
//! | **S4** | Property tests + fuzz + benchmarks + docs |
//! | **S5** | `l2_distance`, `cosine_similarity_pair`, `cosine_anchor_blend`, `shannon_entropy`, `sigmoid` (cross-module refactor) |

use super::device::Device;
use super::error::Result;
use super::tensor::Tensor;

/// The device-agnostic compute backend.
///
/// See module-level docs for the operation inventory.
pub trait ComputeBackend: Send + Sync + std::fmt::Debug {
    // ========================================================
    // 1. Metadata
    // ========================================================

    /// Device on which this backend operates.
    fn device(&self) -> Device;

    /// Human-readable backend name, e.g. `"cpu"` or `"cuda"`.
    fn name(&self) -> &'static str;

    /// Block until all pending async work on this backend completes.
    ///
    /// On CPU this is a no-op; on CUDA this calls `cudaStreamSynchronize`.
    fn synchronize(&self) -> Result<()>;

    // ========================================================
    // 2. Linear algebra  (5 ops)
    // ========================================================

    /// Matrix multiplication: `C = A · B`.
    ///
    /// `a: (M, K)`, `b: (K, N)` → `(M, N)`. Batch dims supported via axis 0.
    fn matmul(&self, a: &Tensor<f32>, b: &Tensor<f32>) -> Result<Tensor<f32>>;

    /// Fused matmul + optional bias + GELU: `GELU(A · B + bias)`.
    fn matmul_fused_gelu(
        &self,
        a: &Tensor<f32>,
        b: &Tensor<f32>,
        bias: Option<&Tensor<f32>>,
    ) -> Result<Tensor<f32>>;

    /// Dot product of two 1-D tensors.
    fn dot(&self, a: &Tensor<f32>, b: &Tensor<f32>) -> Result<f32>;

    /// L2 norm along an axis.
    fn l2_norm(&self, x: &Tensor<f32>, axis: i32) -> Result<Tensor<f32>>;

    /// L2 normalization along an axis: `x / max(||x||, eps)`.
    fn l2_normalize(&self, x: &Tensor<f32>, axis: i32, eps: f32) -> Result<Tensor<f32>>;

    /// L2 distance between two same-shaped rank-1 or rank-2 tensors.
    ///
    /// **Sprint 5 op.** Computes `‖a − b‖₂`. For rank-2 inputs, returns a
    /// rank-1 tensor of per-row distances. Replaces the artisanal
    /// `els::metrics::l2_distance` and the per-call-site loops in
    /// `dissolution::alignment` and `consensus::signals`.
    fn l2_distance(&self, a: &Tensor<f32>, b: &Tensor<f32>) -> Result<Tensor<f32>>;

    // ========================================================
    // 3. Activations  (4 ops)
    // ========================================================

    /// Numerically stable softmax along an axis.
    fn softmax(&self, x: &Tensor<f32>, axis: i32) -> Result<Tensor<f32>>;

    /// Numerically stable log-softmax along an axis.
    fn log_softmax(&self, x: &Tensor<f32>, axis: i32) -> Result<Tensor<f32>>;

    /// Exact GELU: `0.5 · x · (1 + erf(x / √2))`.
    fn gelu(&self, x: &Tensor<f32>) -> Result<Tensor<f32>>;

    /// Logistic sigmoid: `σ(x) = 1 / (1 + exp(−x))`, element-wise.
    ///
    /// **Sprint 5 op.** Numerically stable for both signs of `x`. Replaces
    /// the artisanal `consensus::signals::sigmoid` (and downstream
    /// scoring code).
    fn sigmoid(&self, x: &Tensor<f32>) -> Result<Tensor<f32>>;

    // ========================================================
    // 4. Attention & reduction  (3 ops)
    // ========================================================

    /// Scaled dot-product attention: `softmax(Q·Kᵀ / √d_k) · V`.
    fn scaled_dot_product_attention(
        &self,
        q: &Tensor<f32>,
        k: &Tensor<f32>,
        v: &Tensor<f32>,
        mask: Option<&Tensor<f32>>,
    ) -> Result<Tensor<f32>>;

    /// Top-k values and their indices along an axis.
    fn topk(
        &self,
        x: &Tensor<f32>,
        k: u32,
        axis: i32,
    ) -> Result<(Tensor<f32>, Tensor<i32>)>;

    /// Argmax along an axis.
    fn argmax(&self, x: &Tensor<f32>, axis: i32) -> Result<Tensor<i32>>;

    // ========================================================
    // 5. Consensus / cone  (3 ops — EFE core)
    // ========================================================

    /// Project latents onto the alignment cone defined by anchors.
    ///
    /// For each `z_i`:
    /// * Find `j* = argmin_j ||z_i - anchors_j||₂`
    /// * If `d_min ≤ max_dist`, `z_i` is kept;
    /// * else `z_i` is pulled toward `anchors_{j*}` to distance `max_dist`.
    fn cone_project(
        &self,
        latents: &Tensor<f32>,
        anchors: &Tensor<f32>,
        max_dist: f32,
    ) -> Result<Tensor<f32>>;

    /// Weighted mean with optional L2 renormalization.
    fn weighted_mean(
        &self,
        values: &Tensor<f32>,
        weights: &Tensor<f32>,
        renormalize_l2: bool,
    ) -> Result<Tensor<f32>>;

    /// Deterministic mixture-of-experts reconciliation.
    ///
    /// Top-k softmax over `gate_logits`, then weighted sum of `candidates`.
    fn moe_reconcile(
        &self,
        candidates: &Tensor<f32>,
        gate_logits: &Tensor<f32>,
        top_k: u32,
    ) -> Result<Tensor<f32>>;

    /// Cosine similarity between two rank-1 vectors of equal length.
    ///
    /// **Sprint 5 op.** Returns `a · b / (‖a‖₂ · ‖b‖₂)` capped in
    /// magnitude by 1, or `0.0` when either vector has near-zero norm
    /// (`‖a‖·‖b‖ < f32::EPSILON`). Replaces the artisanal arithmetic
    /// previously inlined in `els::metrics::cosine_similarity`,
    /// `genesis::meta_analyzer::cosine_similarity`,
    /// `alignment::policy::EffectivePolicy::validate_against` (I5),
    /// and `alignment::compose::intersect_cones` — all of which now
    /// route through this op. Errors on shape mismatch or non-finite
    /// inputs.
    fn cosine_similarity_pair(
        &self,
        a: &Tensor<f32>,
        b: &Tensor<f32>,
    ) -> Result<f32>;

    /// Soft projection of a vector onto the convex hull of `anchors`,
    /// blended with the input by `strength`.
    ///
    /// **Sprint 5 op.** For each anchor `a_j`, computes `w_j = max(0, cos(v, a_j))`.
    /// If all weights are below `f32::EPSILON`, returns `(v, 0.0)` — the
    /// geometric identity. Otherwise normalises the weights to a probability
    /// vector, builds the centroid `c = Σ_j w_j' · a_j`, and returns
    /// `((1 − strength) · v + strength · c, ‖v − result‖₂)`.
    ///
    /// Replaces `els::projection::project_vector_onto_anchors`,
    /// `dissolution::alignment::project` (anchor-weights branch), and
    /// `fusion::kernels::project::project_rows`.
    fn cosine_anchor_blend(
        &self,
        vector: &Tensor<f32>,
        anchors: &Tensor<f32>,
        strength: f32,
    ) -> Result<(Tensor<f32>, f32)>;

    // ========================================================
    // 6. Probabilities & sampling  (3 ops)
    // ========================================================

    /// Reproducible multinomial sampling.
    fn multinomial_sample(
        &self,
        probs: &Tensor<f32>,
        n: u32,
        seed: u64,
    ) -> Result<Tensor<i32>>;

    /// Smoothed KL divergence `Σ p · log((p+ε)/(q+ε))`, capped at 50.
    fn kl_divergence(
        &self,
        p: &Tensor<f32>,
        q: &Tensor<f32>,
        axis: i32,
        eps: f32,
    ) -> Result<Tensor<f32>>;

    /// Shannon entropy `H(p) = − Σ p_i · ln(max(p_i, ε))` along an axis.
    ///
    /// **Sprint 5 op.** `ε = 1e-12` floor avoids `ln(0)`. Inputs must be
    /// non-negative and finite (validated upfront, like `kl_divergence`
    /// after the audit-5 hardening). Replaces the artisanal Shannon
    /// computations in `fusion::kernels::entropy` and `genesis::dag`.
    fn shannon_entropy(&self, p: &Tensor<f32>, axis: i32) -> Result<Tensor<f32>>;

    // ========================================================
    // 7. Statistics  (2 ops)
    // ========================================================

    /// Mean along an axis.
    fn mean(&self, x: &Tensor<f32>, axis: i32) -> Result<Tensor<f32>>;

    /// Standard deviation along an axis.
    ///
    /// If `unbiased` is `true`, uses Bessel's correction (divides by N-1).
    fn std_dev(&self, x: &Tensor<f32>, axis: i32, unbiased: bool) -> Result<Tensor<f32>>;
}
