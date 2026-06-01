// cuda/activations/softmax.cu — Sprint D28 (Φ-D).
//
// Numerically-stable rowwise softmax (LogSumExp trick).
// Maps to `ComputeBackend::softmax(x, axis=-1)`.
// Cross-backend tolerance: rtol=1e-5, atol=1e-6.

#include "efe.cuh"
#include "efe_warp.cuh"

namespace {

// Each block processes one row of length `axis_size`. Threads in
// the block cooperate via shared memory tree-reduction to compute
// the row-max, then the sum of exponentials, then the final divide.
//
// Determinism : tree-reduction on shared memory yields the same
// summation order across runs (no atomicAdd on f32) → bit-identical
// outputs for the same input.
__global__ void softmax_rowwise(
    const float* __restrict__ input,
    float* __restrict__ output,
    int axis_size
) {
    extern __shared__ float smem[];
    const int row = blockIdx.x;
    const int tid = threadIdx.x;

    const float* row_in  = input  + row * axis_size;
    float*       row_out = output + row * axis_size;

    // Phase 1 : row max via block reduction.
    float local_max = -INFINITY;
    for (int i = tid; i < axis_size; i += blockDim.x) {
        local_max = fmaxf(local_max, row_in[i]);
    }
    float row_max = efe::block_reduce_max(local_max, smem);
    __shared__ float s_max;
    if (tid == 0) { s_max = row_max; }
    __syncthreads();

    // Phase 2 : sum of exp(x - max).
    float local_sum = 0.0f;
    for (int i = tid; i < axis_size; i += blockDim.x) {
        local_sum += efe_safe_expf(row_in[i] - s_max);
    }
    float row_sum = efe::block_reduce_sum(local_sum, smem);
    __shared__ float s_sum;
    if (tid == 0) { s_sum = row_sum; }
    __syncthreads();

    const float inv_sum = 1.0f / fmaxf(s_sum, EFE_EPS);

    // Phase 3 : write outputs.
    for (int i = tid; i < axis_size; i += blockDim.x) {
        row_out[i] = efe_safe_expf(row_in[i] - s_max) * inv_sum;
    }
}

}  // anonymous

extern "C" cudaError_t efe_softmax_launch(
    cudaStream_t stream,
    const float* input,
    float* output,
    int n_rows,
    int axis_size
) {
    if (n_rows <= 0 || axis_size <= 0) {
        return cudaErrorInvalidValue;
    }
    int threads = axis_size < EFE_TILE_SOFTMAX ? axis_size : EFE_TILE_SOFTMAX;
    threads = efe_next_pow2(threads);
    if (threads > 1024) { threads = 1024; }
    const int n_warps = (threads + 31) / 32;
    const size_t smem  = n_warps * sizeof(float);
    softmax_rowwise<<<n_rows, threads, smem, stream>>>(
        input, output, axis_size);
    return cudaGetLastError();
}
