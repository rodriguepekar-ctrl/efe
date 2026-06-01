// cuda/linalg/l2_normalize.cu — Sprint D28.
//
// Rowwise L2 normalize : y = x / max(||x||₂, eps).
// Each block processes one row.

#include "efe.cuh"
#include "efe_warp.cuh"

namespace {

__global__ void l2_normalize_rowwise(
    const float* __restrict__ input,
    float* __restrict__ output,
    int axis_size,
    float eps
) {
    extern __shared__ float smem[];
    const int row = blockIdx.x;
    const int tid = threadIdx.x;

    const float* row_in  = input  + row * axis_size;
    float*       row_out = output + row * axis_size;

    // Phase 1 : compute Σ x²
    float local_sq = 0.0f;
    for (int i = tid; i < axis_size; i += blockDim.x) {
        const float v = row_in[i];
        local_sq += v * v;
    }
    float sum_sq = efe::block_reduce_sum(local_sq, smem);

    __shared__ float s_inv_norm;
    if (tid == 0) {
        const float norm = sqrtf(fmaxf(sum_sq, 0.0f));
        s_inv_norm = 1.0f / fmaxf(norm, eps);
    }
    __syncthreads();

    // Phase 2 : write y = x · inv_norm
    for (int i = tid; i < axis_size; i += blockDim.x) {
        row_out[i] = row_in[i] * s_inv_norm;
    }
}

}  // anonymous

extern "C" cudaError_t efe_l2_normalize_launch(
    cudaStream_t stream,
    const float* input,
    float* output,
    int n_rows,
    int axis_size,
    float eps
) {
    if (n_rows <= 0 || axis_size <= 0 || !(eps > 0.0f)) {
        return cudaErrorInvalidValue;
    }
    int threads = axis_size < 256 ? axis_size : 256;
    threads = efe_next_pow2(threads);
    if (threads > 1024) { threads = 1024; }
    const int n_warps = (threads + 31) / 32;
    const size_t smem  = n_warps * sizeof(float);
    l2_normalize_rowwise<<<n_rows, threads, smem, stream>>>(
        input, output, axis_size, eps);
    return cudaGetLastError();
}
