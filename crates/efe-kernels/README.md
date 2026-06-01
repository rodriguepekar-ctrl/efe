# `efe-kernels` — GPU substrate (M2 / Sprint Φ-D)

> **Audit fix K-A9** — README added during the post-D28 chirurgical audit.

`efe-kernels` is the CUDA-backed implementation of the
`efe_core::compute::ComputeBackend` trait. It is the GPU half of
the EFE compute substrate ; the CPU half (`CpuBackend`) lives in
`efe-core/src/compute/cpu/` and is the reference against which
every CUDA kernel must agree numerically.

Cf. `docs/EFE_KERNELS_ARCHITECTURE.md` for the full architecture.

## Status (post-D28 audit)

| Item | Status |
|------|:------:|
| Trait coverage `ComputeBackend` (23 methods) | ✅ all 23 wired (12 fully ; 11 `NotYetWired` pending D29) |
| 19 CUDA `.cu` source files | ✅ shipped under `cuda/` |
| `build.rs` (nvcc + cc + multi-arch) | ✅ |
| Cross-backend equivalence tests | ✅ 7 `tests/cross_backend_*.rs` files |
| Telemetry (6 GPU instruments) | ✅ wired via `KernelCallGuard` |
| Multi-GPU sharding (NCCL) | ⚠️ skeleton — D30 |
| Streaming RPC | ⚠️ deferred — D32 |
| Hybrid fusion | ⚠️ deferred — D32 |

## Cargo features

| Feature | Default | Description |
|---------|:-------:|-------------|
| `cuda` | OFF | The real `CudaBackend`. Pulls `cudarc` + `cc`. |
| `nccl` | OFF | Multi-GPU collectives via NCCL (implies `cuda`). |
| `fp16` | OFF | Half-precision support (post-GA). |
| `auto_check_finite` | OFF | Debug-only NaN/Inf scan after each kernel. |
| `mock_gpu` | OFF | CI escape hatch when no GPU runner is available. |

## Build

```bash
# Default workspace build — no CUDA, builds on Mac/Linux/Windows.
cargo build -p efe-kernels

# Real CUDA build — requires NVIDIA toolkit ≥ 12.4 and a host with `nvcc`.
cargo build -p efe-kernels --features cuda

# Multi-GPU build — additionally requires NCCL ≥ 2.18.
cargo build -p efe-kernels --features cuda,nccl
```

Environment overrides honoured by `build.rs` :

* `EFE_CUDA_HOME=/opt/cuda-12.4` — explicit CUDA install root.
* `EFE_CUDA_ARCH=80,90` — comma-separated SM list (default `75,80,86,89,90`).
* `EFE_CUDA_DEBUG=1` — compile with `-G -lineinfo` for `nsight-compute`.

## Quick example

```rust
# #[cfg(feature = "cuda")] fn _example() -> Result<(), Box<dyn std::error::Error>> {
use efe_core::compute::{ComputeBackend, Tensor, Shape};
use efe_kernels::CudaBackend;

let backend = CudaBackend::new(0)?;        // first GPU
let x = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0], Shape::from_slice(&[4]))?;
let y = backend.softmax(&x, -1)?;          // GPU softmax
assert_eq!(y.numel(), 4);
# Ok(()) }
```

## Runtime configuration (consumed by `efe-server`)

`efe-server` reads the following env vars at boot to decide whether
to instantiate a `CudaBackend`. They are surfaced in the Helm chart
as `runtime.gpu*` values.

| Env var | Default | Description |
|---------|:-------:|-------------|
| `EFE_RUNTIME_GPU_ENABLED` | `false` | Master switch. |
| `EFE_RUNTIME_GPU_DEVICE_ORDINAL` | `0` | First visible GPU. |
| `EFE_RUNTIME_GPU_MEMORY_HWM_PERCENT` | `80` | VRAM ceiling (1-95). |
| `EFE_RUNTIME_GPU_STREAMS` | `4` | Stream pool size. |
| `EFE_RUNTIME_GPU_SHARD_ORDINALS` | _(empty)_ | Comma-separated for multi-GPU (D30). |
| `EFE_RUNTIME_GPU_FALLBACK_POLICY` | `strict` | `strict` or `cpu_on_error`. |
| `EFE_RUNTIME_GPU_CHECK_FINITE` | `false` | Debug NaN/Inf scan (perf cost). |

## Determinism guarantees

* **No `atomicAdd<f32>`** anywhere — every reduction goes through
  the deterministic warp/block helpers in `cuda/include/efe_warp.cuh`.
* `cuBLAS` calls go through `CUBLAS_PEDANTIC_MATH` mode (planned
  D29 — wired in `CudaBackend::matmul` once cuBLAS path is active).
* Multi-stream is round-robin but **the trait does not promise
  cross-call determinism** ; an operator chasing bit-identical
  outputs across runs must call `synchronize()` between calls
  (already the rule for the CPU backend).

## Testing

```bash
# Default (no GPU required)
cargo test -p efe-kernels --lib

# GPU smoke (requires a CUDA-capable runner)
cargo test -p efe-kernels --features cuda --tests \
  -- --test-threads=1
```

The CI matrix `.github/workflows/test-gpu-smoke.yml` runs these
nightly on a self-hosted T4 runner and additionally exercises
`cuda-memcheck` to catch device-side memory errors.

## Defect Trace (D28-D32)

| ID | Sprint | Defect / Follow-up |
|----|:------:|--------------------|
| DT-K-D28-MATMUL-1 | D29 | Wire `cublasSgemm` in `CudaBackend::matmul` |
| DT-K-D28-DOT-1 | D29 | Wire `cublasSdot` + `cublasSnrm2` |
| DT-K-D28-SDPA-1 | D29 | Multi-head + multi-batch SDPA via `cublasSgemmStridedBatched` |
| DT-K-D29-MIGR-1 | D29 | Migrate the `efe-core::compute::cuda::CudaBackend` stub to point at `efe_kernels::CudaBackend` (currently parallel impls) |
| DT-K-D29-FINITE-1 | D29 | Wire `efe_check_finite_launch` from `CudaBackend` hot path when `EFE_RUNTIME_GPU_CHECK_FINITE=true` |
| DT-K-D30-NCCL-1 | D30 | Real `ShardedBackend::all_reduce_centroid` impl |
| DT-K-D31-OPT-1 | D31 | Profiling-driven kernel fusion (softmax + entropy → 1 kernel) |
| DT-K-D32-STREAM-1 | D32 | Streaming RPC + Hybrid fusion (CPU shadow + GPU primary) |
| DT-K-Z-1 | Post-GA | Zero-copy device-resident `Tensor` (avoid h2d/d2h round-trips) |

## License

Proprietary — see top-level `LICENSE`.
