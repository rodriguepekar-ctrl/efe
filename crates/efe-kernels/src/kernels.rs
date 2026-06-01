//! `extern "C"` bindings to the CUDA kernels compiled from `cuda/`.
//!
//! Cf. `docs/EFE_KERNELS_ARCHITECTURE.md` § 11.2.
//!
//! Every `extern "C"` declared here matches a function defined in a
//! `.cu` file. The naming convention is :
//!
//!   * `efe_<op>_launch(stream, ...)` for single-output kernels.
//!   * `efe_<op>_<variant>_launch` for kernels with multiple
//!     entry points (e.g. softmax-rowwise vs softmax-blockwise).
//!
//! All bindings are `unsafe` — the type-safety layer lives in
//! [`crate::backend::CudaBackend`].

#![allow(non_camel_case_types)]
#![allow(clippy::missing_safety_doc)]

use cudarc::driver::sys::{cudaError, CUstream_st};

/// Re-export the CUDA stream pointer type for convenience inside
/// `backend.rs` launchers.
pub type cudaStream_t = *mut CUstream_st;

extern "C" {
    // ── ABI version probe (`cuda/stub.cu`) ─────────────────────────

    /// Returns the kernel ABI version encoded as MAJOR*10000 +
    /// MINOR*100 + PATCH. Used at `CudaBackend::new` to verify the
    /// linked CUDA static lib matches the Rust crate version.
    pub fn efe_kernels_version() -> std::os::raw::c_int;

    /// Trivial noop kernel — exercises the build pipeline.
    pub fn efe_noop_launch(stream: cudaStream_t) -> cudaError;

    // ── Activations (cuda/activations/*.cu) ────────────────────────

    /// `softmax(x, -1)` rowwise. Each block processes one row of
    /// length `axis_size`. Output and input may overlap (the kernel
    /// uses a 3-pass max → exp/sum → div pattern).
    pub fn efe_softmax_launch(
        stream: cudaStream_t,
        input: *const f32,
        output: *mut f32,
        n_rows: i32,
        axis_size: i32,
    ) -> cudaError;

    /// `log_softmax(x, -1)` rowwise. Same layout as `softmax`.
    pub fn efe_log_softmax_launch(
        stream: cudaStream_t,
        input: *const f32,
        output: *mut f32,
        n_rows: i32,
        axis_size: i32,
    ) -> cudaError;

    /// `gelu(x)` element-wise (exact erf path, matches CpuBackend).
    pub fn efe_gelu_launch(
        stream: cudaStream_t,
        input: *const f32,
        output: *mut f32,
        n: i32,
    ) -> cudaError;

    /// `sigmoid(x)` element-wise.
    pub fn efe_sigmoid_launch(
        stream: cudaStream_t,
        input: *const f32,
        output: *mut f32,
        n: i32,
    ) -> cudaError;

    // ── Reductions (cuda/reductions/*.cu) ──────────────────────────

    /// `mean(x, axis)` rowwise.
    pub fn efe_mean_launch(
        stream: cudaStream_t,
        input: *const f32,
        output: *mut f32,
        n_rows: i32,
        axis_size: i32,
    ) -> cudaError;

    /// `std_dev(x, axis, unbiased)` rowwise via Welford's online
    /// algorithm.
    pub fn efe_std_dev_launch(
        stream: cudaStream_t,
        input: *const f32,
        output: *mut f32,
        n_rows: i32,
        axis_size: i32,
        unbiased: i32, // 0 or 1
    ) -> cudaError;

    /// `shannon_entropy(p, axis)` — `−Σ p log p`.
    pub fn efe_shannon_entropy_launch(
        stream: cudaStream_t,
        input: *const f32,
        output: *mut f32,
        n_rows: i32,
        axis_size: i32,
    ) -> cudaError;

    /// `argmax(x, axis)` rowwise. Returns `i32` indices.
    pub fn efe_argmax_launch(
        stream: cudaStream_t,
        input: *const f32,
        output: *mut i32,
        n_rows: i32,
        axis_size: i32,
    ) -> cudaError;

    /// `topk(x, k, axis)` rowwise. Outputs `(values, indices)` of
    /// shape `(n_rows, k)`. Tie-break by lowest index.
    pub fn efe_topk_launch(
        stream: cudaStream_t,
        input: *const f32,
        values: *mut f32,
        indices: *mut i32,
        n_rows: i32,
        axis_size: i32,
        k: i32,
    ) -> cudaError;

    /// `kl_divergence(p, q, axis, eps)` smoothed + capped at 50.
    pub fn efe_kl_divergence_launch(
        stream: cudaStream_t,
        p: *const f32,
        q: *const f32,
        output: *mut f32,
        n_rows: i32,
        axis_size: i32,
        eps: f32,
    ) -> cudaError;

    // ── Linalg (cuda/linalg/*.cu) ──────────────────────────────────

    /// `l2_normalize(x, axis, eps)` rowwise.
    pub fn efe_l2_normalize_launch(
        stream: cudaStream_t,
        input: *const f32,
        output: *mut f32,
        n_rows: i32,
        axis_size: i32,
        eps: f32,
    ) -> cudaError;

    /// `l2_distance(a, b)` paired ; `out[i] = ||a[i]-b[i]||₂`.
    pub fn efe_l2_distance_launch(
        stream: cudaStream_t,
        a: *const f32,
        b: *const f32,
        output: *mut f32,
        n_rows: i32,
        d: i32,
    ) -> cudaError;

    /// `cosine_similarity_pair(a, b)` — single scalar `cos(a, b)`.
    pub fn efe_cosine_similarity_launch(
        stream: cudaStream_t,
        a: *const f32,
        b: *const f32,
        output: *mut f32,
        n: i32,
    ) -> cudaError;

    /// `cosine_anchor_blend(vector, anchors, strength)`. Returns
    /// `(blended_vector, similarity_to_anchor)`.
    pub fn efe_cosine_anchor_blend_launch(
        stream: cudaStream_t,
        vector: *const f32,
        anchors: *const f32,
        out_vector: *mut f32,
        out_similarity: *mut f32,
        n_anchors: i32,
        d: i32,
        strength: f32,
    ) -> cudaError;

    // ── EFE-core (cuda/efe_core/*.cu) ──────────────────────────────

    /// `cone_project(latents, anchors, max_dist)`. For each
    /// latent : find nearest anchor, project to cone if outside.
    pub fn efe_cone_project_launch(
        stream: cudaStream_t,
        latents: *const f32,
        anchors: *const f32,
        output: *mut f32,
        b: i32,
        n_anchors: i32,
        d: i32,
        max_dist: f32,
    ) -> cudaError;

    /// `weighted_mean(values, weights, renormalize_l2)`. Normalises
    /// weights to sum=1 internally, then computes `Σ wn[i]·v[i]`.
    pub fn efe_weighted_mean_launch(
        stream: cudaStream_t,
        values: *const f32,
        weights: *const f32,
        output: *mut f32,
        n: i32,
        d: i32,
        renormalize_l2: i32,
    ) -> cudaError;

    /// `moe_reconcile(candidates, gate_logits, top_k)`. Top-k
    /// softmax + weighted sum. Used by Mode-2.
    pub fn efe_moe_reconcile_launch(
        stream: cudaStream_t,
        candidates: *const f32,
        gate_logits: *const f32,
        output: *mut f32,
        b: i32,
        k_total: i32,
        d: i32,
        top_k: i32,
    ) -> cudaError;

    // ── Attention (cuda/attention/*.cu) ────────────────────────────

    /// `scaled_dot_product_attention(q, k, v, mask)`. Composed
    /// kernel : matmul Q×Kᵀ + scale + (optional) mask add + softmax
    /// + matmul ×V. `mask` may be null (no masking).
    pub fn efe_sdpa_launch(
        stream: cudaStream_t,
        q: *const f32,
        k: *const f32,
        v: *const f32,
        mask: *const f32, // nullable
        output: *mut f32,
        b: i32,
        h: i32,
        t: i32,
        d_k: i32,
    ) -> cudaError;

    // ── Sampling (cuda/sampling/*.cu) ──────────────────────────────

    /// `multinomial_sample(probs, n, seed)`. Inverse-CDF method
    /// driven by cuRAND. Same seed → same samples bit-exact.
    pub fn efe_multinomial_sample_launch(
        stream: cudaStream_t,
        probs: *const f32,
        output: *mut i32,
        b: i32,
        k_classes: i32,
        n_samples: i32,
        seed: u64,
    ) -> cudaError;

    // ── Debug (cuda/debug/*.cu) ────────────────────────────────────

    /// **Audit fix K-A7** — `check_finite` debug-only kernel.
    /// Scans `input[0..n]` and atomically increments `counter` for
    /// every non-finite (NaN, Inf) value found. Caller pre-zeroes
    /// `counter` (1-element u32 device buffer) and reads it back
    /// after kernel completion. A non-zero result triggers a
    /// `ComputeError::NumericalInstability` on the Rust side.
    pub fn efe_check_finite_launch(
        stream: cudaStream_t,
        input: *const f32,
        n: i32,
        counter: *mut u32,
    ) -> cudaError;
}
