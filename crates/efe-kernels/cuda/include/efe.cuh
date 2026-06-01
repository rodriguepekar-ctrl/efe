// EFE CUDA shared definitions — Sprint D28 (Φ-D).
//
// Cf. `docs/EFE_KERNELS_ARCHITECTURE.md` § 12.
//
// This header is included by every `.cu` source file under
// `cuda/`. It carries :
//
//   * The version macros injected by `build.rs` (used for ABI
//     verification at runtime).
//   * Numerical constants shared across kernels (epsilon, init
//     sigma, alignment).
//   * Tile-size hints per compute capability (T4, A10, L4, A100,
//     H100). Each `.cu` file picks the right tile size based on
//     `EFE_TILE_<OP>_<CC>` macros — keeps the kernels GPU-aware
//     without runtime branching.
//   * Macros wrapping `cudaError_t` checks so launchers can
//     return early on driver errors.

#pragma once

#include <cuda_runtime.h>
#include <cuda_bf16.h>
#include <cuda_fp16.h>

// ────────────────────────────────────────────────────────────────────
// 1. Build-time injected version macros (defined by `build.rs` via
//    `-DEFE_VERSION_*`). Guarded so a missing definition surfaces a
//    clear compile error instead of a confusing 0.
// ────────────────────────────────────────────────────────────────────

#ifndef EFE_VERSION_MAJOR
#error "EFE_VERSION_MAJOR must be defined by the build system (-DEFE_VERSION_MAJOR=...)"
#endif

#ifndef EFE_VERSION_MINOR
#error "EFE_VERSION_MINOR must be defined by the build system"
#endif

#ifndef EFE_VERSION_PATCH
#error "EFE_VERSION_PATCH must be defined by the build system"
#endif

// ────────────────────────────────────────────────────────────────────
// 2. Numerical constants (must match CpuBackend tolerances ; cf.
//    `docs/compute/tolerances.md`).
// ────────────────────────────────────────────────────────────────────

// Tensor base alignment — coalesced loads on H100 (128B) but we
// over-align to 256 for SHM safety on H100+.
constexpr size_t EFE_TENSOR_ALIGNMENT = 256;

// Numerical-stability epsilon shared across softmax /
// l2_normalize / kl_divergence.
constexpr float EFE_EPS = 1e-6f;

// Default initial diversification noise standard deviation
// (referenced by `genesis::DefaultSpawner` on the CPU side ; here
// for parity).
constexpr float EFE_INIT_SIGMA = 0.02f;

// Maximum value capped by `kl_divergence` to avoid log overflow.
constexpr float EFE_KL_CAP = 50.0f;

// ────────────────────────────────────────────────────────────────────
// 3. Tile-size hints per compute capability.
// ────────────────────────────────────────────────────────────────────

#if   __CUDA_ARCH__ >= 900   // H100 (sm_90)
  #define EFE_TILE_MATMUL    256
  #define EFE_TILE_SOFTMAX   1024
  #define EFE_TILE_ATTENTION 1024
#elif __CUDA_ARCH__ >= 890   // L4 / RTX 4000 (sm_89)
  #define EFE_TILE_MATMUL    256
  #define EFE_TILE_SOFTMAX   512
  #define EFE_TILE_ATTENTION 512
#elif __CUDA_ARCH__ >= 800   // A10 / A100 (sm_80, sm_86)
  #define EFE_TILE_MATMUL    128
  #define EFE_TILE_SOFTMAX   256
  #define EFE_TILE_ATTENTION 256
#elif __CUDA_ARCH__ >= 750   // T4 (sm_75)
  #define EFE_TILE_MATMUL    64
  #define EFE_TILE_SOFTMAX   256
  #define EFE_TILE_ATTENTION 128
#else                         // V100 / older — best effort
  #define EFE_TILE_MATMUL    64
  #define EFE_TILE_SOFTMAX   128
  #define EFE_TILE_ATTENTION 128
#endif

// ────────────────────────────────────────────────────────────────────
// 4. Helper macros.
// ────────────────────────────────────────────────────────────────────

// Round `x` up to the next power of two (host-side helper used by
// launchers when sizing block dimensions).
__host__ inline int efe_next_pow2(int x) {
    if (x <= 1) return 1;
    int p = 1;
    while (p < x) { p <<= 1; }
    return p;
}

// Clamp value within [lo, hi].
template <typename T>
__device__ __host__ inline T efe_clamp(T v, T lo, T hi) {
    return v < lo ? lo : (v > hi ? hi : v);
}

// Numerically stable `expf` clamping to avoid overflow when the
// caller did not max-shift the input.
__device__ inline float efe_safe_expf(float x) {
    if (x > 80.0f) return expf(80.0f);
    if (x < -80.0f) return expf(-80.0f);
    return expf(x);
}

// CUDA error propagation macro for use inside `extern "C"` launchers.
#define EFE_CUDA_RETURN_IF_ERR(call)                              \
    do {                                                          \
        cudaError_t _efe_err = (call);                            \
        if (_efe_err != cudaSuccess) { return _efe_err; }         \
    } while (0)

// Semantic version encoded as MAJOR*10000 + MINOR*100 + PATCH.
constexpr int EFE_KERNELS_ABI_VERSION =
    EFE_VERSION_MAJOR * 10000 + EFE_VERSION_MINOR * 100 + EFE_VERSION_PATCH;
