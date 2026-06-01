// cuda/reductions/shannon_entropy.cu — Sprint D28.
//
// Rowwise Shannon entropy : H(p) = -Σ p_i log p_i.
// Inputs are expected to be valid probability distributions
// (non-negative, sum to 1) ; the kernel does NOT renormalize.
//
// Numerical safety : 0·log(0) is defined as 0 ; values < EFE_EPS
// contribute 0 to the sum (matches CpuBackend reference).

#include "efe.cuh"
#include "efe_warp.cuh"

namespace {

__global__ void shannon_entropy_rowwise(
    const float* __restrict__ input,
    float* __restrict__ output,
    int axis_size
) {
    extern __shared__ float smem[];
    const int row = blockIdx.x;
    const int tid = threadIdx.x;

    const float* row_in = input + row * axis_size;

    float local_h = 0.0f;
    for (int i = tid; i < axis_size; i += blockDim.x) {
        const float p = row_in[i];
        if (p > EFE_EPS) {
            local_h -= p * logf(p);
        }
    }
    float h = efe::block_reduce_sum(local_h, smem);

    if (tid == 0) {
        // Clamp tiny negative values arising from floating-point
        // round-off (entropy is mathematically ≥ 0).
        output[row] = fmaxf(h, 0.0f);
    }
}

}  // anonymous

extern "C" cudaError_t efe_shannon_entropy_launch(
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
    shannon_entropy_rowwise<<<n_rows, threads, smem, stream>>>(
        input, output, axis_size);
    return cudaGetLastError();
}
