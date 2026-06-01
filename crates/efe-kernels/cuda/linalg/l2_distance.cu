// cuda/linalg/l2_distance.cu — Sprint D28.
//
// Paired L2 distance : out[i] = ||a[i] - b[i]||₂ for B rows.
// One block per row, tree reduction in shared memory.

#include "efe.cuh"
#include "efe_warp.cuh"

namespace {

__global__ void l2_distance_paired(
    const float* __restrict__ a,
    const float* __restrict__ b,
    float* __restrict__ output,
    int d
) {
    extern __shared__ float smem[];
    const int row = blockIdx.x;
    const int tid = threadIdx.x;

    const float* row_a = a + row * d;
    const float* row_b = b + row * d;

    float local_sq = 0.0f;
    for (int i = tid; i < d; i += blockDim.x) {
        const float diff = row_a[i] - row_b[i];
        local_sq += diff * diff;
    }
    float sum_sq = efe::block_reduce_sum(local_sq, smem);

    if (tid == 0) {
        output[row] = sqrtf(fmaxf(sum_sq, 0.0f));
    }
}

}  // anonymous

extern "C" cudaError_t efe_l2_distance_launch(
    cudaStream_t stream,
    const float* a,
    const float* b,
    float* output,
    int n_rows,
    int d
) {
    if (n_rows <= 0 || d <= 0) { return cudaErrorInvalidValue; }
    int threads = d < 256 ? d : 256;
    threads = efe_next_pow2(threads);
    if (threads > 1024) { threads = 1024; }
    const int n_warps = (threads + 31) / 32;
    const size_t smem  = n_warps * sizeof(float);
    l2_distance_paired<<<n_rows, threads, smem, stream>>>(
        a, b, output, d);
    return cudaGetLastError();
}
