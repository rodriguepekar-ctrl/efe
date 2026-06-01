// cuda/linalg/cosine.cu — Sprint D28-D29.
//
// Two related kernels :
//   1. `efe_cosine_similarity_launch` — single-pair cos(a, b).
//   2. `efe_cosine_anchor_blend_launch` — blend a vector toward
//       its nearest anchor with weight `strength`. Returns the
//       blended vector + the cosine similarity to the chosen
//       anchor (used by Mode-2 router).

#include "efe.cuh"
#include "efe_warp.cuh"

// ────────────────────────────────────────────────────────────────────
// 1. Cosine similarity (single pair)
// ────────────────────────────────────────────────────────────────────

namespace {

__global__ void cosine_similarity_single(
    const float* __restrict__ a,
    const float* __restrict__ b,
    float* __restrict__ out,
    int n
) {
    extern __shared__ float smem[];
    const int tid = threadIdx.x;

    float dot = 0.0f, norm_a = 0.0f, norm_b = 0.0f;
    for (int i = tid; i < n; i += blockDim.x) {
        const float ai = a[i];
        const float bi = b[i];
        dot    += ai * bi;
        norm_a += ai * ai;
        norm_b += bi * bi;
    }
    // Reduce all three within the block. Re-uses smem 3× via offsets.
    float* smem_dot = smem;
    float* smem_a   = smem + 32;
    float* smem_b   = smem + 64;

    dot    = efe::block_reduce_sum(dot,    smem_dot);
    norm_a = efe::block_reduce_sum(norm_a, smem_a);
    norm_b = efe::block_reduce_sum(norm_b, smem_b);

    if (tid == 0) {
        const float den = sqrtf(fmaxf(norm_a, EFE_EPS)) *
                          sqrtf(fmaxf(norm_b, EFE_EPS));
        out[0] = dot / fmaxf(den, EFE_EPS);
    }
}

}  // anonymous

extern "C" cudaError_t efe_cosine_similarity_launch(
    cudaStream_t stream,
    const float* a,
    const float* b,
    float* output,
    int n
) {
    if (n <= 0) { return cudaErrorInvalidValue; }
    int threads = n < 256 ? n : 256;
    threads = efe_next_pow2(threads);
    if (threads > 1024) { threads = 1024; }
    const size_t smem = 96 * sizeof(float);  // 3 × 32-warp slots
    cosine_similarity_single<<<1, threads, smem, stream>>>(a, b, output, n);
    return cudaGetLastError();
}

// ────────────────────────────────────────────────────────────────────
// 2. Cosine anchor blend
// ────────────────────────────────────────────────────────────────────

namespace {

// One block does : pick the anchor with max cosine sim, then
// blend the input vector toward that anchor with weight `strength`.
__global__ void cosine_anchor_blend(
    const float* __restrict__ vector,
    const float* __restrict__ anchors,
    float* __restrict__ out_vector,
    float* __restrict__ out_similarity,
    int n_anchors,
    int d,
    float strength
) {
    __shared__ float s_best_sim;
    __shared__ int   s_best_idx;
    __shared__ float s_v_norm;

    const int tid = threadIdx.x;

    // Compute ||vector||₂ once.
    float local_v2 = 0.0f;
    for (int i = tid; i < d; i += blockDim.x) {
        const float v = vector[i];
        local_v2 += v * v;
    }
    extern __shared__ float smem_pool[];
    float v_norm_sq = efe::block_reduce_sum(local_v2, smem_pool);
    if (tid == 0) {
        s_v_norm = sqrtf(fmaxf(v_norm_sq, EFE_EPS));
        s_best_sim = -INFINITY;
        s_best_idx = 0;
    }
    __syncthreads();

    // For each anchor, compute cos sim against vector.
    for (int j = 0; j < n_anchors; ++j) {
        float local_dot = 0.0f, local_a2 = 0.0f;
        for (int i = tid; i < d; i += blockDim.x) {
            const float ai = anchors[j * d + i];
            local_dot += vector[i] * ai;
            local_a2  += ai * ai;
        }
        float dot   = efe::block_reduce_sum(local_dot, smem_pool);
        float a_sq  = efe::block_reduce_sum(local_a2,  smem_pool);
        if (tid == 0) {
            const float sim = dot /
                              fmaxf(s_v_norm * sqrtf(fmaxf(a_sq, EFE_EPS)), EFE_EPS);
            if (sim > s_best_sim) {
                s_best_sim = sim;
                s_best_idx = j;
            }
        }
        __syncthreads();
    }

    // Blend : out = (1 - strength) * vector + strength * anchor[best_idx].
    const float w_v = 1.0f - strength;
    const float w_a = strength;
    for (int i = tid; i < d; i += blockDim.x) {
        out_vector[i] = w_v * vector[i] +
                        w_a * anchors[s_best_idx * d + i];
    }
    if (tid == 0) {
        *out_similarity = s_best_sim;
    }
}

}  // anonymous

extern "C" cudaError_t efe_cosine_anchor_blend_launch(
    cudaStream_t stream,
    const float* vector,
    const float* anchors,
    float* out_vector,
    float* out_similarity,
    int n_anchors,
    int d,
    float strength
) {
    if (n_anchors <= 0 || d <= 0 ||
        !(strength >= 0.0f && strength <= 1.0f)) {
        return cudaErrorInvalidValue;
    }
    int threads = d < 256 ? d : 256;
    threads = efe_next_pow2(threads);
    if (threads > 1024) { threads = 1024; }
    const int n_warps = (threads + 31) / 32;
    const size_t smem  = n_warps * sizeof(float);
    cosine_anchor_blend<<<1, threads, smem, stream>>>(
        vector, anchors, out_vector, out_similarity,
        n_anchors, d, strength);
    return cudaGetLastError();
}
