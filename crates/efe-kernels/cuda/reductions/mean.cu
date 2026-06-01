// cuda/reductions/mean.cu — Sprint D28.
//
// Rowwise mean = (1/N) Σ x_i. One block per row, tree-reduction
// in shared memory for determinism (no atomicAdd on f32).

#include "efe.cuh"
#include "efe_warp.cuh"

namespace {

__global__ void mean_rowwise(
    const float* __restrict__ input,
    float* __restrict__ output,
    int axis_size
) {
    extern __shared__ float smem[];
    const int row = blockIdx.x;
    const int tid = threadIdx.x;

    const float* row_in = input + row * axis_size;

    float local_sum = 0.0f;
    for (int i = tid; i < axis_size; i += blockDim.x) {
        local_sum += row_in[i];
    }
    float row_sum = efe::block_reduce_sum(local_sum, smem);

    if (tid == 0) {
        output[row] = row_sum / static_cast<float>(axis_size);
    }
}

}  // anonymous

extern "C" cudaError_t efe_mean_launch(
    cudaStream_t stream,
    const float* input,
    float* output,
    int n_rows,
    int axis_size
) {
    if (n_rows <= 0 || axis_size <= 0) {
        return cudaErrorInvalidValue;
    }
    int threads = axis_size < 256 ? axis_size : 256;
    threads = efe_next_pow2(threads);
    if (threads > 1024) { threads = 1024; }
    const int n_warps = (threads + 31) / 32;
    const size_t smem  = n_warps * sizeof(float);
    mean_rowwise<<<n_rows, threads, smem, stream>>>(
        input, output, axis_size);
    return cudaGetLastError();
}
