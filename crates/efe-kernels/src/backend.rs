//! [`CudaBackend`] — the GPU implementation of
//! `efe_core::compute::ComputeBackend` — Sprint D28-D29.
//!
//! Cf. `docs/EFE_KERNELS_ARCHITECTURE.md` § 13.
//!
//! Each method follows the same recipe :
//!
//!   1. Validate input shapes ; return `ComputeError::ShapeMismatch`
//!      / `InvalidAxis` if KO.
//!   2. Pick a stream from [`crate::streams::StreamPool`].
//!   3. Allocate device buffers via [`crate::memory::DeviceAllocator`].
//!   4. h2d-copy inputs.
//!   5. Launch the matching `extern "C"` kernel from
//!      [`crate::kernels`].
//!   6. d2h-copy the output (zero-copy is a post-D32 optimisation,
//!      DT-K-Z-1).
//!   7. Wrap into a host-side [`Tensor<f32>`] and return.
//!
//! The whole flow is wrapped in a [`crate::observability::record_kernel_call`]
//! guard so every op contributes to `efe_gpu_kernel_duration_seconds`.

use std::sync::Arc;

use cudarc::cublas::CudaBlas;
use cudarc::driver::CudaDevice;
use efe_core::compute::{
    ComputeBackend, ComputeError, Device, Result as ComputeResult, Shape, Tensor,
};

use crate::error::CudaError;
use crate::kernels;
use crate::memory::DeviceAllocator;
use crate::observability::record_kernel_call;
use crate::streams::StreamPool;

/// CUDA implementation of [`ComputeBackend`].
///
/// One instance per active GPU. The `Arc<CudaDevice>` is shared
/// with the cuBLAS handle and the stream pool so they all run on
/// the same device context.
pub struct CudaBackend {
    /// Device ordinal (e.g. 0 for the first GPU).
    ordinal: u32,
    /// Compute capability detected at construction (e.g. 80 for sm_80).
    compute_capability: u32,
    /// Shared device handle.
    device: Arc<CudaDevice>,
    /// cuBLAS handle for matmul / dot / nrm2.
    blas: Arc<CudaBlas>,
    /// Round-robin stream pool (4 streams by default).
    streams: StreamPool,
    /// VRAM-aware allocator with high-water-mark guard.
    allocator: DeviceAllocator,
}

impl std::fmt::Debug for CudaBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CudaBackend")
            .field("ordinal", &self.ordinal)
            .field("compute_capability", &self.compute_capability)
            .field("streams", &self.streams.len())
            .field("vram_high_water_bytes", &self.allocator.high_water_bytes())
            .finish()
    }
}

impl CudaBackend {
    /// Open a CUDA backend on the device with the given ordinal.
    ///
    /// Performs at boot :
    ///   * `CudaDevice::new(ordinal)` — driver init.
    ///   * Compute capability check (≥ sm_75).
    ///   * cuBLAS handle creation.
    ///   * 4-stream pool allocation.
    ///   * VRAM allocator setup with 80 % high-water-mark.
    ///   * ABI version probe via `efe_kernels_version()`.
    pub fn new(ordinal: u32) -> Result<Self, CudaError> {
        let device = CudaDevice::new(ordinal as usize).map_err(|_| {
            CudaError::DeviceUnavailable(ordinal)
        })?;

        // Compute capability check.
        let compute_capability = read_compute_capability(ordinal)?;
        if compute_capability < 75 {
            return Err(CudaError::UnsupportedCapability {
                device: ordinal,
                got_cc: compute_capability,
                min_cc: 75,
            });
        }

        // ABI version probe via `cuda/stub.cu::efe_kernels_version`.
        // Surfaces a clear error if the linked .a was built from a
        // different version of the source tree (toolchain drift).
        // SAFETY: `efe_kernels_version` is a pure const-int function.
        let abi_version = unsafe { kernels::efe_kernels_version() };
        let expected = pkg_abi_version();
        if (abi_version as u32) != expected {
            return Err(CudaError::InitFailed(format!(
                "kernel ABI version mismatch: linked .a reports {abi_version}, \
                 crate expected {expected} — rebuild with --features cuda"
            )));
        }

        let blas = CudaBlas::new(device.clone()).map_err(|e| {
            CudaError::InitFailed(format!("cuBLAS init on device {ordinal}: {e}"))
        })?;

        let streams = StreamPool::with_default_size(device.clone())?;
        let allocator = DeviceAllocator::new(device.clone())?;

        tracing::info!(
            target: "efe.cuda.backend",
            ordinal,
            compute_capability,
            n_streams = streams.len(),
            vram_high_water_gib = allocator.high_water_bytes() / (1024 * 1024 * 1024),
            "CudaBackend ready"
        );

        Ok(Self {
            ordinal,
            compute_capability,
            device,
            blas: Arc::new(blas),
            streams,
            allocator,
        })
    }

    /// Compute capability of the underlying GPU (`75` for T4, `80`
    /// for A10/A100, etc.).
    #[must_use]
    pub fn compute_capability(&self) -> u32 {
        self.compute_capability
    }

    /// Synchronise every stream in the pool. Convenience wrapper.
    pub fn synchronize_streams(&self) -> Result<(), CudaError> {
        self.streams.synchronize_all()
    }

    /// Borrow the underlying [`DeviceAllocator`] (e.g. for
    /// `multi_gpu.rs` to provide manual h2d/d2h paths).
    #[must_use]
    pub fn allocator(&self) -> &DeviceAllocator {
        &self.allocator
    }
}

// ─────────────────────────────────────────────────────────────────────
// Kernel-launch helpers (private)
// ─────────────────────────────────────────────────────────────────────

impl CudaBackend {
    /// Generic host-tensor-in / host-tensor-out flow for rowwise
    /// kernels. Used by `softmax`, `log_softmax`, `mean`, `std_dev`,
    /// `shannon_entropy`, `l2_normalize` (variants).
    fn launch_rowwise<F>(
        &self,
        op: &'static str,
        x: &Tensor<f32>,
        axis: i32,
        launch: F,
    ) -> ComputeResult<Tensor<f32>>
    where
        F: FnOnce(
            kernels::cudaStream_t,
            *const f32,
            *mut f32,
            i32,
            i32,
        ) -> cudarc::driver::sys::cudaError,
    {
        let mut g = record_kernel_call(op);

        let resolved_axis = x.shape().resolve_axis(axis)?;
        if resolved_axis + 1 != x.rank() {
            // Architecture defines rowwise = last-axis. Other axes
            // require a transpose ; left for D31.
            return Err(ComputeError::InvalidAxis {
                axis,
                rank: x.rank(),
            });
        }
        let axis_size = x.shape().dims()[resolved_axis];
        let n_rows = x.numel() / axis_size.max(1);

        let host_in = x.as_slice()?;

        let dev_in = self
            .allocator
            .h2d_copy(host_in)
            .map_err(ComputeError::from)?;
        let mut dev_out = self
            .allocator
            .alloc_zeros::<f32>(x.numel())
            .map_err(ComputeError::from)?;

        let stream = self.streams.next();
        // SAFETY: `dev_in` and `dev_out` are valid for the kernel's
        // pointer arithmetic (size matches `n_rows * axis_size`),
        // the launch is queued on a non-default stream, and the
        // launcher returns a `cudaError_t` we forward.
        let status = unsafe {
            launch(
                crate::streams::raw_stream(&stream),
                dev_in.as_slice().as_kernel_param() as *const f32,
                dev_out.as_mut_slice().as_kernel_param() as *mut f32,
                n_rows as i32,
                axis_size as i32,
            )
        };
        check_kernel_status(&mut g, op, status)?;

        let host_out = self
            .allocator
            .d2h_copy(dev_out.as_slice())
            .map_err(ComputeError::from)?;

        Tensor::from_slice(&host_out, x.shape().clone())
    }
}

// ─────────────────────────────────────────────────────────────────────
// `ComputeBackend` impl
// ─────────────────────────────────────────────────────────────────────

impl ComputeBackend for CudaBackend {
    fn device(&self) -> Device {
        Device::Cuda(self.ordinal)
    }

    fn name(&self) -> &'static str {
        "cuda"
    }

    fn synchronize(&self) -> ComputeResult<()> {
        let mut g = record_kernel_call("synchronize");
        self.streams
            .synchronize_all()
            .map_err(ComputeError::from)?;
        self.device
            .synchronize()
            .map_err(|e| ComputeError::NumericalInstability {
                op: "cuda.synchronize",
                detail: format!("{e}"),
            })
    }

    // ── P2 — Linalg via cuBLAS ─────────────────────────────────────

    fn matmul(&self, a: &Tensor<f32>, b: &Tensor<f32>) -> ComputeResult<Tensor<f32>> {
        let mut g = record_kernel_call("matmul");
        // For D28, route through a generic rowwise approach is not
        // applicable. The cuBLAS impl lives in this method directly.
        // We validate shapes (M×K · K×N → M×N) and call sgemm.

        if a.rank() != 2 || b.rank() != 2 {
            return Err(ComputeError::ShapeMismatch {
                op: "matmul",
                expected: "rank 2 tensors".into(),
                actual: format!("a.rank={}, b.rank={}", a.rank(), b.rank()),
            });
        }
        let m = a.shape().dims()[0];
        let k = a.shape().dims()[1];
        let k2 = b.shape().dims()[0];
        let n = b.shape().dims()[1];
        if k != k2 {
            return Err(ComputeError::ShapeMismatch {
                op: "matmul",
                expected: format!("a={}x{}, b={}x{}", m, k, k, n),
                actual: format!("a={}x{}, b={}x{}", m, k, k2, n),
            });
        }

        let host_a = a.as_slice()?;
        let host_b = b.as_slice()?;
        let dev_a = self.allocator.h2d_copy(host_a).map_err(ComputeError::from)?;
        let dev_b = self.allocator.h2d_copy(host_b).map_err(ComputeError::from)?;
        let mut dev_c = self
            .allocator
            .alloc_zeros::<f32>(m * n)
            .map_err(ComputeError::from)?;

        // cuBLAS column-major: compute C = A * B by computing
        // Cᵀ = Bᵀ * Aᵀ in column-major layout. The cudarc Gemm
        // helper handles the row-major → col-major conversion ;
        // we call the deterministic-mode variant.
        // For D28 we route through a placeholder — the actual
        // sgemm wiring lives in DT-K-D28-MATMUL-1, deferred to D29.
        let _ = (&dev_a, &dev_b, &mut dev_c, &self.blas);
        Err(ComputeError::NumericalInstability {
            op: "cuda.matmul",
            detail: "cuBLAS sgemm wiring deferred to D29 (DT-K-D28-MATMUL-1)".into(),
        })
    }

    fn matmul_fused_gelu(
        &self,
        _a: &Tensor<f32>,
        _b: &Tensor<f32>,
        _bias: Option<&Tensor<f32>>,
    ) -> ComputeResult<Tensor<f32>> {
        let mut g = record_kernel_call("matmul_fused_gelu");
        Err(CudaError::NotYetWired("matmul_fused_gelu").into())
    }

    fn dot(&self, a: &Tensor<f32>, b: &Tensor<f32>) -> ComputeResult<f32> {
        let mut g = record_kernel_call("dot");
        if a.numel() != b.numel() || a.rank() != 1 || b.rank() != 1 {
            return Err(ComputeError::ShapeMismatch {
                op: "dot",
                expected: format!("rank 1 same len"),
                actual: format!("a={:?}, b={:?}", a.shape().dims(), b.shape().dims()),
            });
        }
        // cuBLAS sdot wiring: DT-K-D28-DOT-1, deferred to D29.
        Err(CudaError::NotYetWired("dot").into())
    }

    fn l2_norm(&self, _x: &Tensor<f32>, _axis: i32) -> ComputeResult<Tensor<f32>> {
        let mut g = record_kernel_call("l2_norm");
        Err(CudaError::NotYetWired("l2_norm").into())
    }

    fn l2_normalize(
        &self,
        x: &Tensor<f32>,
        axis: i32,
        eps: f32,
    ) -> ComputeResult<Tensor<f32>> {
        let mut g = record_kernel_call("l2_normalize");

        let resolved_axis = x.shape().resolve_axis(axis)?;
        if resolved_axis + 1 != x.rank() {
            return Err(ComputeError::InvalidAxis {
                axis,
                rank: x.rank(),
            });
        }
        let axis_size = x.shape().dims()[resolved_axis];
        let n_rows = x.numel() / axis_size.max(1);

        let host_in = x.as_slice()?;
        let dev_in = self.allocator.h2d_copy(host_in).map_err(ComputeError::from)?;
        let mut dev_out = self
            .allocator
            .alloc_zeros::<f32>(x.numel())
            .map_err(ComputeError::from)?;

        let stream = self.streams.next();
        let status = unsafe {
            kernels::efe_l2_normalize_launch(
                crate::streams::raw_stream(&stream),
                dev_in.as_slice().as_kernel_param() as *const f32,
                dev_out.as_mut_slice().as_kernel_param() as *mut f32,
                n_rows as i32,
                axis_size as i32,
                eps,
            )
        };
        check_kernel_status(&mut g, "l2_normalize", status)?;

        let host_out = self
            .allocator
            .d2h_copy(dev_out.as_slice())
            .map_err(ComputeError::from)?;
        Tensor::from_slice(&host_out, x.shape().clone())
    }

    fn l2_distance(
        &self,
        _a: &Tensor<f32>,
        _b: &Tensor<f32>,
    ) -> ComputeResult<Tensor<f32>> {
        let mut g = record_kernel_call("l2_distance");
        Err(CudaError::NotYetWired("l2_distance").into())
    }

    fn cosine_similarity_pair(
        &self,
        _a: &Tensor<f32>,
        _b: &Tensor<f32>,
    ) -> ComputeResult<f32> {
        let mut g = record_kernel_call("cosine_similarity_pair");
        Err(CudaError::NotYetWired("cosine_similarity_pair").into())
    }

    fn cosine_anchor_blend(
        &self,
        _vector: &Tensor<f32>,
        _anchors: &Tensor<f32>,
        _strength: f32,
    ) -> ComputeResult<(Tensor<f32>, f32)> {
        let mut g = record_kernel_call("cosine_anchor_blend");
        Err(CudaError::NotYetWired("cosine_anchor_blend").into())
    }

    // ── P1 — Activations ───────────────────────────────────────────

    fn softmax(&self, x: &Tensor<f32>, axis: i32) -> ComputeResult<Tensor<f32>> {
        self.launch_rowwise("softmax", x, axis, |stream, in_p, out_p, n_rows, axis_size| {
            unsafe { kernels::efe_softmax_launch(stream, in_p, out_p, n_rows, axis_size) }
        })
    }

    fn log_softmax(&self, x: &Tensor<f32>, axis: i32) -> ComputeResult<Tensor<f32>> {
        self.launch_rowwise("log_softmax", x, axis, |stream, in_p, out_p, n_rows, axis_size| {
            unsafe {
                kernels::efe_log_softmax_launch(stream, in_p, out_p, n_rows, axis_size)
            }
        })
    }

    fn gelu(&self, x: &Tensor<f32>) -> ComputeResult<Tensor<f32>> {
        let mut g = record_kernel_call("gelu");
        let host_in = x.as_slice()?;
        let dev_in = self.allocator.h2d_copy(host_in).map_err(ComputeError::from)?;
        let mut dev_out = self
            .allocator
            .alloc_zeros::<f32>(x.numel())
            .map_err(ComputeError::from)?;
        let stream = self.streams.next();
        let status = unsafe {
            kernels::efe_gelu_launch(
                crate::streams::raw_stream(&stream),
                dev_in.as_slice().as_kernel_param() as *const f32,
                dev_out.as_mut_slice().as_kernel_param() as *mut f32,
                x.numel() as i32,
            )
        };
        check_kernel_status(&mut g, "gelu", status)?;
        let host_out = self
            .allocator
            .d2h_copy(dev_out.as_slice())
            .map_err(ComputeError::from)?;
        Tensor::from_slice(&host_out, x.shape().clone())
    }

    fn sigmoid(&self, x: &Tensor<f32>) -> ComputeResult<Tensor<f32>> {
        let mut g = record_kernel_call("sigmoid");
        let host_in = x.as_slice()?;
        let dev_in = self.allocator.h2d_copy(host_in).map_err(ComputeError::from)?;
        let mut dev_out = self
            .allocator
            .alloc_zeros::<f32>(x.numel())
            .map_err(ComputeError::from)?;
        let stream = self.streams.next();
        let status = unsafe {
            kernels::efe_sigmoid_launch(
                crate::streams::raw_stream(&stream),
                dev_in.as_slice().as_kernel_param() as *const f32,
                dev_out.as_mut_slice().as_kernel_param() as *mut f32,
                x.numel() as i32,
            )
        };
        check_kernel_status(&mut g, "sigmoid", status)?;
        let host_out = self
            .allocator
            .d2h_copy(dev_out.as_slice())
            .map_err(ComputeError::from)?;
        Tensor::from_slice(&host_out, x.shape().clone())
    }

    // ── P1 — Attention ─────────────────────────────────────────────

    fn scaled_dot_product_attention(
        &self,
        _q: &Tensor<f32>,
        _k: &Tensor<f32>,
        _v: &Tensor<f32>,
        _mask: Option<&Tensor<f32>>,
    ) -> ComputeResult<Tensor<f32>> {
        let mut g = record_kernel_call("scaled_dot_product_attention");
        // SDPA is composed (matmul + softmax + matmul). The full
        // wiring lives in `cuda/attention/scaled_dot_product_attention.cu`
        // and is exposed in D29 once matmul is fully wired.
        Err(CudaError::NotYetWired("scaled_dot_product_attention").into())
    }

    // ── P3 — Reductions ────────────────────────────────────────────

    fn topk(
        &self,
        _x: &Tensor<f32>,
        _k: u32,
        _axis: i32,
    ) -> ComputeResult<(Tensor<f32>, Tensor<i32>)> {
        let mut g = record_kernel_call("topk");
        Err(CudaError::NotYetWired("topk").into())
    }

    fn argmax(&self, x: &Tensor<f32>, axis: i32) -> ComputeResult<Tensor<i32>> {
        let mut g = record_kernel_call("argmax");
        let resolved_axis = x.shape().resolve_axis(axis)?;
        if resolved_axis + 1 != x.rank() {
            return Err(ComputeError::InvalidAxis { axis, rank: x.rank() });
        }
        let axis_size = x.shape().dims()[resolved_axis];
        let n_rows = x.numel() / axis_size.max(1);

        let host_in = x.as_slice()?;
        let dev_in = self.allocator.h2d_copy(host_in).map_err(ComputeError::from)?;
        let mut dev_out = self
            .allocator
            .alloc_zeros::<i32>(n_rows)
            .map_err(ComputeError::from)?;
        let stream = self.streams.next();
        let status = unsafe {
            kernels::efe_argmax_launch(
                crate::streams::raw_stream(&stream),
                dev_in.as_slice().as_kernel_param() as *const f32,
                dev_out.as_mut_slice().as_kernel_param() as *mut i32,
                n_rows as i32,
                axis_size as i32,
            )
        };
        check_kernel_status(&mut g, "argmax", status)?;
        let host_out = self
            .allocator
            .d2h_copy(dev_out.as_slice())
            .map_err(ComputeError::from)?;
        // Output shape = input shape minus the reduced axis.
        let mut out_dims: Vec<usize> = x.shape().dims().to_vec();
        out_dims.remove(resolved_axis);
        Tensor::from_slice(&host_out, Shape::from_slice(&out_dims))
    }

    // ── P1/P2 — EFE-core ───────────────────────────────────────────

    fn cone_project(
        &self,
        latents: &Tensor<f32>,
        anchors: &Tensor<f32>,
        max_dist: f32,
    ) -> ComputeResult<Tensor<f32>> {
        let mut g = record_kernel_call("cone_project");
        if latents.rank() != 2 || anchors.rank() != 2 {
            return Err(ComputeError::ShapeMismatch {
                op: "cone_project",
                expected: "rank 2 tensors".into(),
                actual: format!("latents.rank={}, anchors.rank={}", latents.rank(), anchors.rank()),
            });
        }
        let b = latents.shape().dims()[0];
        let d = latents.shape().dims()[1];
        let n = anchors.shape().dims()[0];
        if anchors.shape().dims()[1] != d {
            return Err(ComputeError::ShapeMismatch {
                op: "cone_project",
                expected: format!("anchors last dim = {d}"),
                actual: format!("got {}", anchors.shape().dims()[1]),
            });
        }
        if !max_dist.is_finite() || max_dist < 0.0 {
            return Err(ComputeError::NumericalInstability {
                op: "cone_project",
                detail: format!("max_dist must be finite ≥ 0 ; got {max_dist}"),
            });
        }

        let dev_lat = self
            .allocator
            .h2d_copy(latents.as_slice()?)
            .map_err(ComputeError::from)?;
        let dev_anch = self
            .allocator
            .h2d_copy(anchors.as_slice()?)
            .map_err(ComputeError::from)?;
        let mut dev_out = self
            .allocator
            .alloc_zeros::<f32>(b * d)
            .map_err(ComputeError::from)?;
        let stream = self.streams.next();
        let status = unsafe {
            kernels::efe_cone_project_launch(
                crate::streams::raw_stream(&stream),
                dev_lat.as_slice().as_kernel_param() as *const f32,
                dev_anch.as_slice().as_kernel_param() as *const f32,
                dev_out.as_mut_slice().as_kernel_param() as *mut f32,
                b as i32,
                n as i32,
                d as i32,
                max_dist,
            )
        };
        check_kernel_status(&mut g, "cone_project", status)?;

        let host_out = self
            .allocator
            .d2h_copy(dev_out.as_slice())
            .map_err(ComputeError::from)?;
        Tensor::from_slice(&host_out, latents.shape().clone())
    }

    fn weighted_mean(
        &self,
        values: &Tensor<f32>,
        weights: &Tensor<f32>,
        renormalize_l2: bool,
    ) -> ComputeResult<Tensor<f32>> {
        let mut g = record_kernel_call("weighted_mean");
        if values.rank() != 2 {
            return Err(ComputeError::ShapeMismatch {
                op: "weighted_mean",
                expected: "values rank 2".into(),
                actual: format!("rank {}", values.rank()),
            });
        }
        let n = values.shape().dims()[0];
        let d = values.shape().dims()[1];
        if weights.rank() != 1 || weights.shape().dims()[0] != n {
            return Err(ComputeError::ShapeMismatch {
                op: "weighted_mean",
                expected: format!("weights rank 1 len {n}"),
                actual: format!("rank {} len {:?}", weights.rank(), weights.shape().dims()),
            });
        }

        let dev_vals = self
            .allocator
            .h2d_copy(values.as_slice()?)
            .map_err(ComputeError::from)?;
        let dev_w = self
            .allocator
            .h2d_copy(weights.as_slice()?)
            .map_err(ComputeError::from)?;
        let mut dev_out = self
            .allocator
            .alloc_zeros::<f32>(d)
            .map_err(ComputeError::from)?;
        let stream = self.streams.next();
        let status = unsafe {
            kernels::efe_weighted_mean_launch(
                crate::streams::raw_stream(&stream),
                dev_vals.as_slice().as_kernel_param() as *const f32,
                dev_w.as_slice().as_kernel_param() as *const f32,
                dev_out.as_mut_slice().as_kernel_param() as *mut f32,
                n as i32,
                d as i32,
                if renormalize_l2 { 1 } else { 0 },
            )
        };
        check_kernel_status(&mut g, "weighted_mean", status)?;

        let host_out = self
            .allocator
            .d2h_copy(dev_out.as_slice())
            .map_err(ComputeError::from)?;
        Tensor::from_slice(&host_out, Shape::from_slice(&[d]))
    }

    fn moe_reconcile(
        &self,
        _candidates: &Tensor<f32>,
        _gate_logits: &Tensor<f32>,
        _top_k: u32,
    ) -> ComputeResult<Tensor<f32>> {
        let mut g = record_kernel_call("moe_reconcile");
        Err(CudaError::NotYetWired("moe_reconcile").into())
    }

    // ── P3 — Sampling & info-theory ────────────────────────────────

    fn multinomial_sample(
        &self,
        _probs: &Tensor<f32>,
        _n: u32,
        _seed: u64,
    ) -> ComputeResult<Tensor<i32>> {
        let mut g = record_kernel_call("multinomial_sample");
        Err(CudaError::NotYetWired("multinomial_sample").into())
    }

    fn kl_divergence(
        &self,
        _p: &Tensor<f32>,
        _q: &Tensor<f32>,
        _axis: i32,
        _eps: f32,
    ) -> ComputeResult<Tensor<f32>> {
        let mut g = record_kernel_call("kl_divergence");
        Err(CudaError::NotYetWired("kl_divergence").into())
    }

    fn shannon_entropy(&self, x: &Tensor<f32>, axis: i32) -> ComputeResult<Tensor<f32>> {
        let mut g = record_kernel_call("shannon_entropy");
        let resolved_axis = x.shape().resolve_axis(axis)?;
        if resolved_axis + 1 != x.rank() {
            return Err(ComputeError::InvalidAxis { axis, rank: x.rank() });
        }
        let axis_size = x.shape().dims()[resolved_axis];
        let n_rows = x.numel() / axis_size.max(1);

        let dev_in = self
            .allocator
            .h2d_copy(x.as_slice()?)
            .map_err(ComputeError::from)?;
        let mut dev_out = self
            .allocator
            .alloc_zeros::<f32>(n_rows)
            .map_err(ComputeError::from)?;
        let stream = self.streams.next();
        let status = unsafe {
            kernels::efe_shannon_entropy_launch(
                crate::streams::raw_stream(&stream),
                dev_in.as_slice().as_kernel_param() as *const f32,
                dev_out.as_mut_slice().as_kernel_param() as *mut f32,
                n_rows as i32,
                axis_size as i32,
            )
        };
        check_kernel_status(&mut g, "shannon_entropy", status)?;
        let host_out = self
            .allocator
            .d2h_copy(dev_out.as_slice())
            .map_err(ComputeError::from)?;
        let mut out_dims: Vec<usize> = x.shape().dims().to_vec();
        out_dims.remove(resolved_axis);
        Tensor::from_slice(&host_out, Shape::from_slice(&out_dims))
    }

    // ── P1 — Stats ─────────────────────────────────────────────────

    fn mean(&self, x: &Tensor<f32>, axis: i32) -> ComputeResult<Tensor<f32>> {
        let mut g = record_kernel_call("mean");
        let resolved_axis = x.shape().resolve_axis(axis)?;
        if resolved_axis + 1 != x.rank() {
            return Err(ComputeError::InvalidAxis { axis, rank: x.rank() });
        }
        let axis_size = x.shape().dims()[resolved_axis];
        let n_rows = x.numel() / axis_size.max(1);

        let dev_in = self.allocator.h2d_copy(x.as_slice()?).map_err(ComputeError::from)?;
        let mut dev_out = self
            .allocator
            .alloc_zeros::<f32>(n_rows)
            .map_err(ComputeError::from)?;
        let stream = self.streams.next();
        let status = unsafe {
            kernels::efe_mean_launch(
                crate::streams::raw_stream(&stream),
                dev_in.as_slice().as_kernel_param() as *const f32,
                dev_out.as_mut_slice().as_kernel_param() as *mut f32,
                n_rows as i32,
                axis_size as i32,
            )
        };
        check_kernel_status(&mut g, "mean", status)?;
        let host_out = self
            .allocator
            .d2h_copy(dev_out.as_slice())
            .map_err(ComputeError::from)?;
        let mut out_dims: Vec<usize> = x.shape().dims().to_vec();
        out_dims.remove(resolved_axis);
        Tensor::from_slice(&host_out, Shape::from_slice(&out_dims))
    }

    fn std_dev(
        &self,
        x: &Tensor<f32>,
        axis: i32,
        unbiased: bool,
    ) -> ComputeResult<Tensor<f32>> {
        let mut g = record_kernel_call("std_dev");
        let resolved_axis = x.shape().resolve_axis(axis)?;
        if resolved_axis + 1 != x.rank() {
            return Err(ComputeError::InvalidAxis { axis, rank: x.rank() });
        }
        let axis_size = x.shape().dims()[resolved_axis];
        let n_rows = x.numel() / axis_size.max(1);

        let dev_in = self.allocator.h2d_copy(x.as_slice()?).map_err(ComputeError::from)?;
        let mut dev_out = self
            .allocator
            .alloc_zeros::<f32>(n_rows)
            .map_err(ComputeError::from)?;
        let stream = self.streams.next();
        let status = unsafe {
            kernels::efe_std_dev_launch(
                crate::streams::raw_stream(&stream),
                dev_in.as_slice().as_kernel_param() as *const f32,
                dev_out.as_mut_slice().as_kernel_param() as *mut f32,
                n_rows as i32,
                axis_size as i32,
                if unbiased { 1 } else { 0 },
            )
        };
        check_kernel_status(&mut g, "std_dev", status)?;
        let host_out = self
            .allocator
            .d2h_copy(dev_out.as_slice())
            .map_err(ComputeError::from)?;
        let mut out_dims: Vec<usize> = x.shape().dims().to_vec();
        out_dims.remove(resolved_axis);
        Tensor::from_slice(&host_out, Shape::from_slice(&out_dims))
    }
}

// ─────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────

fn read_compute_capability(ordinal: u32) -> Result<u32, CudaError> {
    let major = cudarc::driver::result::device::get_attribute(
        ordinal as i32,
        cudarc::driver::sys::CUdevice_attribute::CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MAJOR,
    )
    .map_err(|e| CudaError::InitFailed(format!("get_attribute MAJOR: {e}")))?;
    let minor = cudarc::driver::result::device::get_attribute(
        ordinal as i32,
        cudarc::driver::sys::CUdevice_attribute::CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MINOR,
    )
    .map_err(|e| CudaError::InitFailed(format!("get_attribute MINOR: {e}")))?;
    Ok((major as u32) * 10 + (minor as u32))
}

fn pkg_abi_version() -> u32 {
    let major = env!("CARGO_PKG_VERSION_MAJOR").parse::<u32>().unwrap_or(0);
    let minor = env!("CARGO_PKG_VERSION_MINOR").parse::<u32>().unwrap_or(0);
    let patch = env!("CARGO_PKG_VERSION_PATCH").parse::<u32>().unwrap_or(0);
    major * 10000 + minor * 100 + patch
}

/// **Audit fix K-A16** — pre-fix `check_kernel_status` returned an
/// error and silently relied on `KernelCallGuard::Drop` to bump the
/// outcome counter ; but the guard observed `failed=false` at that
/// point so the failure was recorded as `outcome="ok"`. The caller
/// also did `.with_observability(kernel)` on the CudaError which
/// then double-counted as `"fail"`. The correct semantics are :
///
///   * On success : do nothing (the guard records `"ok"` at drop).
///   * On failure : flip the guard's `failed` flag (so drop records
///     `"fail"`) AND return Err. No `.with_observability` (single
///     count).
///
/// Helpers for the two paths.
fn check_kernel_status(
    guard: &mut crate::observability::KernelCallGuard,
    kernel: &'static str,
    status: cudarc::driver::sys::cudaError,
) -> ComputeResult<()> {
    if status as i32 == 0 {
        Ok(())
    } else {
        guard.fail();
        Err(CudaError::LaunchFailed {
            kernel,
            detail: format!("cudaError {}", status as i32),
        }
        .into())
    }
}

#[cfg(test)]
mod tests {
    // GPU-only tests live in `tests/cross_backend_*.rs`. Here we
    // cover constructor invariants reachable without a GPU.

    use super::pkg_abi_version;

    #[test]
    fn pkg_abi_version_non_zero() {
        // CARGO_PKG_VERSION_* is always defined at compile time.
        assert!(pkg_abi_version() > 0);
    }
}
