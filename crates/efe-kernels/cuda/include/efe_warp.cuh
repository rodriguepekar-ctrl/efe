// EFE CUDA warp-level primitives — Sprint D28 (Φ-D).
//
// Cf. `docs/EFE_KERNELS_ARCHITECTURE.md` § 12.2.
//
// Helpers reused across reductions and attention kernels.
// All primitives use `__shfl_*_sync` with full warp mask 0xffffffff
// — assumes block-size multiple of 32. The launchers in each `.cu`
// must enforce that ; cf. `efe_next_pow2` host helper.

#pragma once

#include <cuda_runtime.h>
#include <cfloat>
#include <climits>

namespace efe {

// ────────────────────────────────────────────────────────────────────
// Warp-level reductions (32 threads per warp).
// ────────────────────────────────────────────────────────────────────

/// Sum reduction across a warp. Returns the result on lane 0 of
/// the warp ; other lanes hold partial sums (caller must broadcast
/// if needed).
__device__ __forceinline__ float warp_reduce_sum(float v) {
    #pragma unroll
    for (int offset = 16; offset > 0; offset >>= 1) {
        v += __shfl_down_sync(0xffffffff, v, offset);
    }
    return v;
}

/// Max reduction across a warp.
__device__ __forceinline__ float warp_reduce_max(float v) {
    #pragma unroll
    for (int offset = 16; offset > 0; offset >>= 1) {
        float other = __shfl_down_sync(0xffffffff, v, offset);
        v = v > other ? v : other;
    }
    return v;
}

/// Argmax reduction across a warp. Each lane carries `(value, idx)`.
/// Returns the lane-0 result `(max_value, argmax_idx)`. Tie-break
/// is by lowest index (matching CpuBackend deterministic behaviour).
__device__ __forceinline__ void warp_reduce_argmax(
    float& v, int& idx
) {
    #pragma unroll
    for (int offset = 16; offset > 0; offset >>= 1) {
        float other_v   = __shfl_down_sync(0xffffffff, v,   offset);
        int   other_idx = __shfl_down_sync(0xffffffff, idx, offset);
        // Keep the strictly-greater one ; tie → keep lower index.
        if (other_v > v || (other_v == v && other_idx < idx)) {
            v   = other_v;
            idx = other_idx;
        }
    }
}

// ────────────────────────────────────────────────────────────────────
// Block-level reductions (`blockDim.x` threads).
//
// The block is reduced via warp-reduce → cross-warp via shared
// memory → final warp-reduce. Caller must pass an `extern __shared__`
// buffer sized at `(blockDim.x / 32) * sizeof(float)`.
// ────────────────────────────────────────────────────────────────────

/// Block-level sum.
__device__ __forceinline__ float block_reduce_sum(
    float v, float* smem
) {
    int lane    = threadIdx.x % 32;
    int warp_id = threadIdx.x / 32;

    v = warp_reduce_sum(v);
    if (lane == 0) {
        smem[warp_id] = v;
    }
    __syncthreads();

    if (warp_id == 0) {
        int n_warps = (blockDim.x + 31) / 32;
        v = (lane < n_warps) ? smem[lane] : 0.0f;
        v = warp_reduce_sum(v);
    }
    return v;
}

/// Block-level max.
__device__ __forceinline__ float block_reduce_max(
    float v, float* smem
) {
    int lane    = threadIdx.x % 32;
    int warp_id = threadIdx.x / 32;

    v = warp_reduce_max(v);
    if (lane == 0) {
        smem[warp_id] = v;
    }
    __syncthreads();

    if (warp_id == 0) {
        int n_warps = (blockDim.x + 31) / 32;
        v = (lane < n_warps) ? smem[lane] : -FLT_MAX;
        v = warp_reduce_max(v);
    }
    return v;
}

/// Block-level argmax.
__device__ __forceinline__ void block_reduce_argmax(
    float& v, int& idx, float* smem_v, int* smem_i
) {
    int lane    = threadIdx.x % 32;
    int warp_id = threadIdx.x / 32;

    warp_reduce_argmax(v, idx);
    if (lane == 0) {
        smem_v[warp_id] = v;
        smem_i[warp_id] = idx;
    }
    __syncthreads();

    if (warp_id == 0) {
        int n_warps = (blockDim.x + 31) / 32;
        v   = (lane < n_warps) ? smem_v[lane] : -FLT_MAX;
        idx = (lane < n_warps) ? smem_i[lane] : INT_MAX;
        warp_reduce_argmax(v, idx);
    }
}

}  // namespace efe
