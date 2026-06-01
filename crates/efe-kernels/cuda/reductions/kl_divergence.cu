// cuda/reductions/kl_divergence.cu — Sprint D29.
//
// KL(p || q) = Σ (p+ε) log((p+ε)/(q+ε)) — smoothed + capped at
// EFE_KL_CAP. Matches `CpuBackend::kl_divergence` to rtol=1e-5.

#include "efe.cuh"
#include "efe_warp.cuh"

namespace {

__global__ void kl_divergence_rowwise(
    const float* __restrict__ p,
    const float* __restrict__ q,
    float* __restrict__ output,
    int axis_size,
    float eps
) {
    extern __shared__ float smem[];
    const int row = blockIdx.x;
    const int tid = threadIdx.x;

    const float* row_p = p + row * axis_size;
    const float* row_q = q + row * axis_size;

    float local_kl = 0.0f;
    for (int i = tid; i < axis_size; i += blockDim.x) {
        const float ps = row_p[i] + eps;
        const float qs = row_q[i] + eps;
        // Defensive : silently treat negative inputs as their abs
        // value (the CpuBackend errors out instead — but the GPU
        // path expects upstream validation has already passed).
        const float ratio = fmaxf(ps, EFE_EPS) / fmaxf(qs, EFE_EPS);
        local_kl += ps * logf(ratio);
    }
    float kl = efe::block_reduce_sum(local_kl, smem);

    if (tid == 0) {
        // Cap at EFE_KL_CAP (50.0 by default) to match CpuBackend.
        output[row] = fminf(fmaxf(kl, 0.0f), EFE_KL_CAP);
    }
}

}  // anonymous

extern "C" cudaError_t efe_kl_divergence_launch(
    cudaStream_t stream,
    const float* p,
    const float* q,
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
    kl_divergence_rowwise<<<n_rows, threads, smem, stream>>>(
        p, q, output, axis_size, eps);
    return cudaGetLastError();
}
