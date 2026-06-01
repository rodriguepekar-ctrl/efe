// cuda/reductions/topk.cu — Sprint D28.
//
// Rowwise top-k for small k (k ≤ 32). Each block handles one row ;
// per-thread local heap of size `k` then warp-merge. Tie-break by
// lowest index — matches CpuBackend.
//
// For k > 32, fall back to a 2-pass radix select (D31 optimisation
// — DT-K-D31-TOPK-1).

#include "efe.cuh"
#include <cfloat>
#include <climits>

namespace {

// In-place insertion sort on `vals[0..k]` keeping the array sorted
// in descending order. Indices are tracked in `idxs`.
__device__ __forceinline__ void insert_sorted(
    float v, int i, float* vals, int* idxs, int k
) {
    if (v < vals[k - 1] || (v == vals[k - 1] && i >= idxs[k - 1])) {
        return;
    }
    int pos = k - 1;
    while (pos > 0 && (v > vals[pos - 1] ||
                       (v == vals[pos - 1] && i < idxs[pos - 1]))) {
        vals[pos] = vals[pos - 1];
        idxs[pos] = idxs[pos - 1];
        --pos;
    }
    vals[pos] = v;
    idxs[pos] = i;
}

__global__ void topk_rowwise(
    const float* __restrict__ input,
    float* __restrict__ values,
    int* __restrict__ indices,
    int axis_size,
    int k
) {
    // Per-thread top-k with a small fixed buffer (k ≤ 32 here).
    constexpr int K_MAX = 32;
    float local_v[K_MAX];
    int   local_i[K_MAX];
    for (int j = 0; j < k; ++j) {
        local_v[j] = -FLT_MAX;
        local_i[j] = INT_MAX;
    }

    const int row = blockIdx.x;
    const int tid = threadIdx.x;

    const float* row_in = input + row * axis_size;

    for (int i = tid; i < axis_size; i += blockDim.x) {
        insert_sorted(row_in[i], i, local_v, local_i, k);
    }

    // Warp-level merge : reduce K threads → 1.
    extern __shared__ float smem_pool[];
    float* smem_v = smem_pool;
    int* smem_i = reinterpret_cast<int*>(smem_pool + blockDim.x * k);

    for (int j = 0; j < k; ++j) {
        smem_v[tid * k + j] = local_v[j];
        smem_i[tid * k + j] = local_i[j];
    }
    __syncthreads();

    // Single-thread final merge (k * blockDim small for k ≤ 32).
    if (tid == 0) {
        // Reset local_v / local_i to merge across all threads.
        for (int j = 0; j < k; ++j) {
            local_v[j] = -FLT_MAX;
            local_i[j] = INT_MAX;
        }
        for (int t = 0; t < blockDim.x; ++t) {
            for (int j = 0; j < k; ++j) {
                insert_sorted(
                    smem_v[t * k + j],
                    smem_i[t * k + j],
                    local_v, local_i, k);
            }
        }
        // Write to global memory.
        for (int j = 0; j < k; ++j) {
            values[row * k + j]  = local_v[j];
            indices[row * k + j] = local_i[j];
        }
    }
}

}  // anonymous

extern "C" cudaError_t efe_topk_launch(
    cudaStream_t stream,
    const float* input,
    float* values,
    int* indices,
    int n_rows,
    int axis_size,
    int k
) {
    if (n_rows <= 0 || axis_size <= 0 || k <= 0 || k > 32 || k > axis_size) {
        return cudaErrorInvalidValue;
    }
    const int threads = 64;  // small block — top-k merge is O(k·N)
    const size_t smem = threads * k * (sizeof(float) + sizeof(int));
    topk_rowwise<<<n_rows, threads, smem, stream>>>(
        input, values, indices, axis_size, k);
    return cudaGetLastError();
}
