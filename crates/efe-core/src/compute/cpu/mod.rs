//! [`CpuBackend`] — pure-Rust implementation of [`ComputeBackend`].
//!
//! * **Sprint 1** : `l2_norm`, `l2_normalize`, `mean`, `argmax`
//! * **Sprint 2** : `softmax`, `log_softmax`, `gelu`, `dot`, `matmul`,
//!   `matmul_fused_gelu`
//! * **Sprint 3** : `scaled_dot_product_attention`, `topk`,
//!   `cone_project`, `weighted_mean`, `moe_reconcile`,
//!   `multinomial_sample`, `kl_divergence`, `std_dev`.
//! * **Sprint 4 Light** : BLAS dispatch for `matmul` and
//!   `matmul_fused_gelu` (feature-gated `accelerate` / `openblas`).
//! * **Sprint 5** : `l2_distance`, `cosine_similarity_pair`,
//!   `cosine_anchor_blend`, `shannon_entropy`, `sigmoid` —
//!   the cross-module-refactor surface that lets `els`, `genesis`,
//!   `alignment`, `consensus`, `dissolution`, `fusion::kernels`
//!   delegate their inner arithmetic here.
//!
//! # Clippy overrides (rationale)
//!
//! * `clippy::needless_range_loop` — gather/scatter along a moving
//!   axis requires parallel indexing of `coords[a]` and the input/output
//!   buffer at the same position `k`. Rewriting with iterators would
//!   force a `zip` over the coords slot + data slot and obscure the
//!   mathematical intent.
//! * `clippy::many_single_char_names` — matmul and axis walks use the
//!   canonical `M/K/N/i/j/k` from standard linear algebra literature.
//!   Longer names here hurt readability.

#![allow(clippy::needless_range_loop)]
#![allow(clippy::many_single_char_names)]

use super::backend::ComputeBackend;
use super::device::Device;
use super::error::Result;
use super::tensor::Tensor;

mod activ;
mod attention;
mod consensus;
mod linalg;
#[cfg(any(feature = "accelerate", feature = "openblas"))]
mod linalg_blas;
mod sampling;
mod shared;
mod stats;

/// CPU backend.
///
/// Backed by plain Rust + (eventually) BLAS via `ndarray-linalg`.
/// In Sprint 1 the implementations use hand-rolled loops to minimise
/// dependencies; BLAS is introduced in Sprint 2 for `matmul`.
#[derive(Debug, Clone)]
pub struct CpuBackend {
    // Reserved for Sprint 4: thread pool handle, allocator hint, ...
    _private: (),
}

impl CpuBackend {
    /// Create a new CPU backend.
    pub fn new() -> Self {
        CpuBackend { _private: () }
    }
}

impl Default for CpuBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl ComputeBackend for CpuBackend {
    // ============ Metadata ============
    fn device(&self) -> Device {
        Device::Cpu
    }

    fn name(&self) -> &'static str {
        "cpu"
    }

    fn synchronize(&self) -> Result<()> {
        // CPU operations are synchronous; nothing to wait for.
        Ok(())
    }

    // ============ Linear algebra ============
    fn matmul(&self, a: &Tensor<f32>, b: &Tensor<f32>) -> Result<Tensor<f32>> {
        linalg::matmul(a, b)
    }

    fn matmul_fused_gelu(
        &self,
        a: &Tensor<f32>,
        b: &Tensor<f32>,
        bias: Option<&Tensor<f32>>,
    ) -> Result<Tensor<f32>> {
        linalg::matmul_fused_gelu(a, b, bias)
    }

    fn dot(&self, a: &Tensor<f32>, b: &Tensor<f32>) -> Result<f32> {
        linalg::dot(a, b)
    }

    fn l2_norm(&self, x: &Tensor<f32>, axis: i32) -> Result<Tensor<f32>> {
        linalg::l2_norm(x, axis)
    }

    fn l2_normalize(&self, x: &Tensor<f32>, axis: i32, eps: f32) -> Result<Tensor<f32>> {
        linalg::l2_normalize(x, axis, eps)
    }

    fn l2_distance(&self, a: &Tensor<f32>, b: &Tensor<f32>) -> Result<Tensor<f32>> {
        linalg::l2_distance(a, b)
    }

    // ============ Activations ============
    fn softmax(&self, x: &Tensor<f32>, axis: i32) -> Result<Tensor<f32>> {
        activ::softmax(x, axis)
    }

    fn log_softmax(&self, x: &Tensor<f32>, axis: i32) -> Result<Tensor<f32>> {
        activ::log_softmax(x, axis)
    }

    fn gelu(&self, x: &Tensor<f32>) -> Result<Tensor<f32>> {
        activ::gelu(x)
    }

    fn sigmoid(&self, x: &Tensor<f32>) -> Result<Tensor<f32>> {
        activ::sigmoid(x)
    }

    // ============ Attention & reduction ============
    fn scaled_dot_product_attention(
        &self,
        q: &Tensor<f32>,
        k: &Tensor<f32>,
        v: &Tensor<f32>,
        mask: Option<&Tensor<f32>>,
    ) -> Result<Tensor<f32>> {
        attention::scaled_dot_product_attention(q, k, v, mask)
    }

    fn topk(
        &self,
        x: &Tensor<f32>,
        k: u32,
        axis: i32,
    ) -> Result<(Tensor<f32>, Tensor<i32>)> {
        attention::topk(x, k, axis)
    }

    fn argmax(&self, x: &Tensor<f32>, axis: i32) -> Result<Tensor<i32>> {
        stats::argmax(x, axis)
    }

    // ============ Consensus / cone ============
    fn cone_project(
        &self,
        latents: &Tensor<f32>,
        anchors: &Tensor<f32>,
        max_dist: f32,
    ) -> Result<Tensor<f32>> {
        consensus::cone_project(latents, anchors, max_dist)
    }

    fn weighted_mean(
        &self,
        values: &Tensor<f32>,
        weights: &Tensor<f32>,
        renormalize_l2: bool,
    ) -> Result<Tensor<f32>> {
        consensus::weighted_mean(values, weights, renormalize_l2)
    }

    fn moe_reconcile(
        &self,
        candidates: &Tensor<f32>,
        gate_logits: &Tensor<f32>,
        top_k: u32,
    ) -> Result<Tensor<f32>> {
        consensus::moe_reconcile(candidates, gate_logits, top_k)
    }

    fn cosine_similarity_pair(
        &self,
        a: &Tensor<f32>,
        b: &Tensor<f32>,
    ) -> Result<f32> {
        consensus::cosine_similarity_pair(a, b)
    }

    fn cosine_anchor_blend(
        &self,
        vector: &Tensor<f32>,
        anchors: &Tensor<f32>,
        strength: f32,
    ) -> Result<(Tensor<f32>, f32)> {
        consensus::cosine_anchor_blend(vector, anchors, strength)
    }

    // ============ Probabilities & sampling ============
    fn multinomial_sample(
        &self,
        probs: &Tensor<f32>,
        n: u32,
        seed: u64,
    ) -> Result<Tensor<i32>> {
        sampling::multinomial_sample(probs, n, seed)
    }

    fn kl_divergence(
        &self,
        p: &Tensor<f32>,
        q: &Tensor<f32>,
        axis: i32,
        eps: f32,
    ) -> Result<Tensor<f32>> {
        sampling::kl_divergence(p, q, axis, eps)
    }

    fn shannon_entropy(&self, p: &Tensor<f32>, axis: i32) -> Result<Tensor<f32>> {
        sampling::shannon_entropy(p, axis)
    }

    // ============ Statistics ============
    fn mean(&self, x: &Tensor<f32>, axis: i32) -> Result<Tensor<f32>> {
        stats::mean(x, axis)
    }

    fn std_dev(&self, x: &Tensor<f32>, axis: i32, unbiased: bool) -> Result<Tensor<f32>> {
        stats::std_dev(x, axis, unbiased)
    }
}
