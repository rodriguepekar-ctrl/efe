//! CUDA backend — Phase 3.
//!
//! This module is compiled only when the `cuda` feature is enabled.
//! On macOS (no CUDA toolkit available) this module never builds, which is
//! by design.
//!
//! Sprint 1 & 2 leave every method returning
//! [`crate::compute::error::ComputeError::NotImplemented`]. The actual
//! kernels live in `crates/efe-kernels/cuda/*.cu` and are wired through
//! `cudarc` bindings in Sprint 3.

use crate::compute::backend::ComputeBackend;
use crate::compute::device::Device;
use crate::compute::error::{ComputeError, Result};
use crate::compute::tensor::Tensor;

/// CUDA backend (feature-gated).
#[derive(Debug)]
pub struct CudaBackend {
    ordinal: u32,
}

impl CudaBackend {
    /// Create a CUDA backend targeting the given device ordinal.
    ///
    /// In Sprint 3 this will allocate a `CudaStream` and a `CudaBlas`
    /// handle via `cudarc`. For now it simply stores the ordinal.
    pub fn new(ordinal: u32) -> Result<Self> {
        Ok(CudaBackend { ordinal })
    }
}

impl ComputeBackend for CudaBackend {
    fn device(&self) -> Device {
        Device::Cuda(self.ordinal)
    }

    fn name(&self) -> &'static str {
        "cuda"
    }

    fn synchronize(&self) -> Result<()> {
        Err(ComputeError::NotImplemented {
            op: "synchronize",
            backend: "cuda",
        })
    }

    fn matmul(&self, _a: &Tensor<f32>, _b: &Tensor<f32>) -> Result<Tensor<f32>> {
        Err(not_impl("matmul"))
    }

    fn matmul_fused_gelu(
        &self,
        _a: &Tensor<f32>,
        _b: &Tensor<f32>,
        _bias: Option<&Tensor<f32>>,
    ) -> Result<Tensor<f32>> {
        Err(not_impl("matmul_fused_gelu"))
    }

    fn dot(&self, _a: &Tensor<f32>, _b: &Tensor<f32>) -> Result<f32> {
        Err(not_impl("dot"))
    }

    fn l2_norm(&self, _x: &Tensor<f32>, _axis: i32) -> Result<Tensor<f32>> {
        Err(not_impl("l2_norm"))
    }

    fn l2_normalize(&self, _x: &Tensor<f32>, _axis: i32, _eps: f32) -> Result<Tensor<f32>> {
        Err(not_impl("l2_normalize"))
    }

    fn l2_distance(&self, _a: &Tensor<f32>, _b: &Tensor<f32>) -> Result<Tensor<f32>> {
        Err(not_impl("l2_distance"))
    }

    fn softmax(&self, _x: &Tensor<f32>, _axis: i32) -> Result<Tensor<f32>> {
        Err(not_impl("softmax"))
    }

    fn log_softmax(&self, _x: &Tensor<f32>, _axis: i32) -> Result<Tensor<f32>> {
        Err(not_impl("log_softmax"))
    }

    fn gelu(&self, _x: &Tensor<f32>) -> Result<Tensor<f32>> {
        Err(not_impl("gelu"))
    }

    fn sigmoid(&self, _x: &Tensor<f32>) -> Result<Tensor<f32>> {
        Err(not_impl("sigmoid"))
    }

    fn scaled_dot_product_attention(
        &self,
        _q: &Tensor<f32>,
        _k: &Tensor<f32>,
        _v: &Tensor<f32>,
        _mask: Option<&Tensor<f32>>,
    ) -> Result<Tensor<f32>> {
        Err(not_impl("scaled_dot_product_attention"))
    }

    fn topk(
        &self,
        _x: &Tensor<f32>,
        _k: u32,
        _axis: i32,
    ) -> Result<(Tensor<f32>, Tensor<i32>)> {
        Err(not_impl("topk"))
    }

    fn argmax(&self, _x: &Tensor<f32>, _axis: i32) -> Result<Tensor<i32>> {
        Err(not_impl("argmax"))
    }

    fn cone_project(
        &self,
        _latents: &Tensor<f32>,
        _anchors: &Tensor<f32>,
        _max_dist: f32,
    ) -> Result<Tensor<f32>> {
        Err(not_impl("cone_project"))
    }

    fn weighted_mean(
        &self,
        _values: &Tensor<f32>,
        _weights: &Tensor<f32>,
        _renormalize_l2: bool,
    ) -> Result<Tensor<f32>> {
        Err(not_impl("weighted_mean"))
    }

    fn moe_reconcile(
        &self,
        _candidates: &Tensor<f32>,
        _gate_logits: &Tensor<f32>,
        _top_k: u32,
    ) -> Result<Tensor<f32>> {
        Err(not_impl("moe_reconcile"))
    }

    fn cosine_similarity_pair(
        &self,
        _a: &Tensor<f32>,
        _b: &Tensor<f32>,
    ) -> Result<f32> {
        Err(not_impl("cosine_similarity_pair"))
    }

    fn cosine_anchor_blend(
        &self,
        _vector: &Tensor<f32>,
        _anchors: &Tensor<f32>,
        _strength: f32,
    ) -> Result<(Tensor<f32>, f32)> {
        Err(not_impl("cosine_anchor_blend"))
    }

    fn multinomial_sample(
        &self,
        _probs: &Tensor<f32>,
        _n: u32,
        _seed: u64,
    ) -> Result<Tensor<i32>> {
        Err(not_impl("multinomial_sample"))
    }

    fn kl_divergence(
        &self,
        _p: &Tensor<f32>,
        _q: &Tensor<f32>,
        _axis: i32,
        _eps: f32,
    ) -> Result<Tensor<f32>> {
        Err(not_impl("kl_divergence"))
    }

    fn shannon_entropy(&self, _p: &Tensor<f32>, _axis: i32) -> Result<Tensor<f32>> {
        Err(not_impl("shannon_entropy"))
    }

    fn mean(&self, _x: &Tensor<f32>, _axis: i32) -> Result<Tensor<f32>> {
        Err(not_impl("mean"))
    }

    fn std_dev(&self, _x: &Tensor<f32>, _axis: i32, _unbiased: bool) -> Result<Tensor<f32>> {
        Err(not_impl("std_dev"))
    }
}

fn not_impl(op: &'static str) -> ComputeError {
    ComputeError::NotImplemented {
        op,
        backend: "cuda",
    }
}
