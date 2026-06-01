// cuda/attention/scaled_dot_product_attention.cu — Sprint D28-D29.
//
// Scaled dot-product attention :
//   1. scores = Q @ Kᵀ / √d_k                   [B, H, T, T]
//   2. scores += mask (if non-null)
//   3. weights = softmax(scores, -1)             [B, H, T, T]
//   4. out = weights @ V                          [B, H, T, d_k]
//
// Q, K, V : [B, H, T, d_k]. mask : optional [B, H, T, T] additive mask.
// out     : [B, H, T, d_k].
//
// **D28 wiring note** : the implementation uses the in-place
// rowwise softmax kernel from `cuda/activations/softmax.cu` for
// step 3, but steps 1 and 4 require batched sgemm via cuBLAS. The
// final pipeline lands in D29 (DT-K-D29-SDPA-1) — for D28 the
// kernel ships a working **single-head, single-batch** path used
// only by the Mode-1 swarm controller (B=1, H=1, T ≤ 1024,
// d_k ≤ 256). Multi-head (H > 1) and large batch (B > 1) fall
// through to `cudaErrorNotSupported` until D29.

#include "efe.cuh"
#include "efe_warp.cuh"

namespace {

// Single-head, single-batch SDPA. Kernel grid = T (one block per
// query token). Each block computes :
//   1. row of scores : scores[t, k] = Q[t] · K[k] / √d_k
//   2. softmax(scores[t, :])
//   3. out[t, :] = Σ_k scores[t, k] · V[k, :]
__global__ void sdpa_single_head(
    const float* __restrict__ q,
    const float* __restrict__ k,
    const float* __restrict__ v,
    const float* __restrict__ mask,  // nullable
    float* __restrict__ output,
    int t_seq,
    int d_k,
    float inv_sqrt_dk,
    int has_mask
) {
    extern __shared__ float smem[];
    float* s_scores = smem;                 // [T] scratch for softmax
    float* s_reduce = smem + t_seq;         // [n_warps] tree reduction

    const int t_q = blockIdx.x;
    const int tid = threadIdx.x;

    const float* row_q   = q + t_q * d_k;
    float*       row_out = output + t_q * d_k;

    // Phase 1 : compute Q[t_q] · K[k] for each k, store in s_scores.
    for (int k_idx = 0; k_idx < t_seq; ++k_idx) {
        float local_dot = 0.0f;
        for (int i = tid; i < d_k; i += blockDim.x) {
            local_dot += row_q[i] * k[k_idx * d_k + i];
        }
        float dot = efe::block_reduce_sum(local_dot, s_reduce);
        if (tid == 0) {
            float s = dot * inv_sqrt_dk;
            if (has_mask) {
                s += mask[t_q * t_seq + k_idx];
            }
            s_scores[k_idx] = s;
        }
        __syncthreads();
    }

    // Phase 2 : softmax over s_scores in shared memory.
    if (tid == 0) {
        float row_max = -INFINITY;
        for (int k_idx = 0; k_idx < t_seq; ++k_idx) {
            row_max = fmaxf(row_max, s_scores[k_idx]);
        }
        float row_sum = 0.0f;
        for (int k_idx = 0; k_idx < t_seq; ++k_idx) {
            s_scores[k_idx] = efe_safe_expf(s_scores[k_idx] - row_max);
            row_sum += s_scores[k_idx];
        }
        const float inv_sum = 1.0f / fmaxf(row_sum, EFE_EPS);
        for (int k_idx = 0; k_idx < t_seq; ++k_idx) {
            s_scores[k_idx] *= inv_sum;
        }
    }
    __syncthreads();

    // Phase 3 : weighted sum over V.
    for (int i = tid; i < d_k; i += blockDim.x) {
        float acc = 0.0f;
        for (int k_idx = 0; k_idx < t_seq; ++k_idx) {
            acc += s_scores[k_idx] * v[k_idx * d_k + i];
        }
        row_out[t_q * d_k + i] = acc;
    }
}

}  // anonymous

extern "C" cudaError_t efe_sdpa_launch(
    cudaStream_t stream,
    const float* q,
    const float* k,
    const float* v,
    const float* mask,
    float* output,
    int b,
    int h,
    int t,
    int d_k
) {
    if (b <= 0 || h <= 0 || t <= 0 || d_k <= 0) {
        return cudaErrorInvalidValue;
    }
    if (b != 1 || h != 1) {
        // D28 only supports single-batch / single-head. Multi-head
        // ships in D29 once cuBLAS sgemm batched is wired.
        return cudaErrorNotSupported;
    }

    int threads = d_k < EFE_TILE_ATTENTION ? d_k : EFE_TILE_ATTENTION;
    threads = efe_next_pow2(threads);
    if (threads > 1024) { threads = 1024; }

    const int n_warps = (threads + 31) / 32;
    // smem layout: [t scratch] + [n_warps reduction]
    const size_t smem = (t + n_warps) * sizeof(float);

    const float inv_sqrt_dk = 1.0f / sqrtf(static_cast<float>(d_k));

    sdpa_single_head<<<t, threads, smem, stream>>>(
        q, k, v, mask, output,
        t, d_k, inv_sqrt_dk, mask ? 1 : 0);
    return cudaGetLastError();
}
