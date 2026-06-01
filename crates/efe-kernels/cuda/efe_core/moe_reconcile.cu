// cuda/efe_core/moe_reconcile.cu — Sprint D29.
//
// Mixture of Experts reconciliation : top-k softmax + weighted sum
// of candidates. Used by Mode-2 routing (M9 doc).
//
// candidates : [B × K_total × D]
// gate_logits: [B × K_total]
// output     : [B × D]
//
// For each b in 0..B :
//   1. select top_k indices in gate_logits[b, :]
//   2. compute softmax over the selected values
//   3. output[b] = Σ_{j in top_k} softmax[j] · candidates[b, j, :]
//
// One block per batch element. Threads cooperate via shared
// memory tree-reduction for the softmax + weighted sum.

#include "efe.cuh"
#include "efe_warp.cuh"
#include <cfloat>
#include <climits>

namespace {

// Per-batch top-k softmax + weighted sum. Limits :
//   K_total ≤ 256 (typical Mode-2 router top_k=5..16, full K=64..256)
//   top_k   ≤ 32  (matches the topk.cu kernel ceiling)
__global__ void moe_reconcile_block(
    const float* __restrict__ candidates,
    const float* __restrict__ gate_logits,
    float* __restrict__ output,
    int k_total,
    int d,
    int top_k
) {
    constexpr int K_MAX = 32;
    __shared__ float s_top_v[K_MAX];
    __shared__ int   s_top_i[K_MAX];
    __shared__ float s_softmax[K_MAX];

    const int b = blockIdx.x;
    const int tid = threadIdx.x;

    const float* batch_logits = gate_logits + b * k_total;
    const float* batch_cand   = candidates  + b * k_total * d;
    float*       batch_out    = output      + b * d;

    // Phase 1 : top-k indices (single-thread for K small ≤ 32).
    if (tid == 0) {
        for (int j = 0; j < top_k; ++j) {
            s_top_v[j] = -FLT_MAX;
            s_top_i[j] = INT_MAX;
        }
        for (int i = 0; i < k_total; ++i) {
            const float v = batch_logits[i];
            // Insert sorted descending, tie-break by lower index.
            int pos = top_k - 1;
            if (v > s_top_v[pos] || (v == s_top_v[pos] && i < s_top_i[pos])) {
                while (pos > 0 && (v > s_top_v[pos - 1] ||
                                   (v == s_top_v[pos - 1] && i < s_top_i[pos - 1]))) {
                    s_top_v[pos] = s_top_v[pos - 1];
                    s_top_i[pos] = s_top_i[pos - 1];
                    --pos;
                }
                s_top_v[pos] = v;
                s_top_i[pos] = i;
            }
        }
    }
    __syncthreads();

    // Phase 2 : softmax over the selected top-k values (one thread).
    if (tid == 0) {
        float max_v = s_top_v[0];
        for (int j = 1; j < top_k; ++j) {
            if (s_top_v[j] > max_v) { max_v = s_top_v[j]; }
        }
        float sum = 0.0f;
        for (int j = 0; j < top_k; ++j) {
            s_softmax[j] = efe_safe_expf(s_top_v[j] - max_v);
            sum += s_softmax[j];
        }
        const float inv_sum = 1.0f / fmaxf(sum, EFE_EPS);
        for (int j = 0; j < top_k; ++j) {
            s_softmax[j] *= inv_sum;
        }
    }
    __syncthreads();

    // Phase 3 : weighted sum on candidate vectors.
    for (int i = tid; i < d; i += blockDim.x) {
        float acc = 0.0f;
        for (int j = 0; j < top_k; ++j) {
            const int cand_idx = s_top_i[j];
            acc += s_softmax[j] * batch_cand[cand_idx * d + i];
        }
        batch_out[i] = acc;
    }
}

}  // anonymous

extern "C" cudaError_t efe_moe_reconcile_launch(
    cudaStream_t stream,
    const float* candidates,
    const float* gate_logits,
    float* output,
    int b,
    int k_total,
    int d,
    int top_k
) {
    if (b <= 0 || k_total <= 0 || d <= 0 ||
        top_k <= 0 || top_k > 32 || top_k > k_total) {
        return cudaErrorInvalidValue;
    }
    int threads = d < 256 ? d : 256;
    threads = efe_next_pow2(threads);
    if (threads > 1024) { threads = 1024; }
    moe_reconcile_block<<<b, threads, 0, stream>>>(
        candidates, gate_logits, output, k_total, d, top_k);
    return cudaGetLastError();
}
