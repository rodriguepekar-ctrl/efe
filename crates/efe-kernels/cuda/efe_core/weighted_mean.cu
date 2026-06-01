// cuda/efe_core/weighted_mean.cu — Sprint D28.
//
// Weighted mean : out[d] = Σ_i wn[i] · values[i, d]
// where wn[i] = weights[i] / Σ_j weights[j].
//
// Optionally renormalises the output to unit L2 norm (used by
// the dissolution::WeightedMeanExtractor with renormalize=true).
//
// Cross-backend tolerance : rtol=1e-5 (Kahan compensation absent
// on GPU ; FMA suffices for d ≤ 1024 typical sizes).

#include "efe.cuh"
#include "efe_warp.cuh"

namespace {

// Kernel 1 : sum the weights to produce the normaliser.
__global__ void sum_weights_kernel(
    const float* __restrict__ weights,
    float* __restrict__ out_sum,
    int n
) {
    extern __shared__ float smem[];
    const int tid = threadIdx.x;
    float local = 0.0f;
    for (int i = tid; i < n; i += blockDim.x) {
        local += weights[i];
    }
    float total = efe::block_reduce_sum(local, smem);
    if (tid == 0) {
        out_sum[0] = fmaxf(total, EFE_EPS);
    }
}

// Kernel 2 : compute the weighted sum + optional L2 renorm.
// One block per output dimension (d blocks total).
__global__ void weighted_mean_kernel(
    const float* __restrict__ values,
    const float* __restrict__ weights,
    const float* __restrict__ weight_sum,
    float* __restrict__ output,
    int n,
    int d,
    int renormalize_l2
) {
    extern __shared__ float smem[];

    const int dim = blockIdx.x;
    const int tid = threadIdx.x;

    const float inv_w_sum = 1.0f / weight_sum[0];

    float local = 0.0f;
    for (int i = tid; i < n; i += blockDim.x) {
        local += weights[i] * values[i * d + dim];
    }
    float total = efe::block_reduce_sum(local, smem);
    if (tid == 0) {
        output[dim] = total * inv_w_sum;
    }

    if (renormalize_l2) {
        // We need the L2 norm of the full output vector, but each
        // block only sees one dim. So we sync via cooperative
        // groups would be needed — for D28 we punt the renorm to
        // a 3rd kernel launched by the host.
    }
}

// Kernel 3 : optional L2 renorm of the full output vector (length d).
__global__ void l2_renormalize_inplace(
    float* __restrict__ output,
    int d
) {
    extern __shared__ float smem[];
    const int tid = threadIdx.x;

    float local_sq = 0.0f;
    for (int i = tid; i < d; i += blockDim.x) {
        const float v = output[i];
        local_sq += v * v;
    }
    float sum_sq = efe::block_reduce_sum(local_sq, smem);

    __shared__ float s_inv_norm;
    if (tid == 0) {
        s_inv_norm = 1.0f / fmaxf(sqrtf(sum_sq), EFE_EPS);
    }
    __syncthreads();

    for (int i = tid; i < d; i += blockDim.x) {
        output[i] *= s_inv_norm;
    }
}

}  // anonymous

extern "C" cudaError_t efe_weighted_mean_launch(
    cudaStream_t stream,
    const float* values,
    const float* weights,
    float* output,
    int n,
    int d,
    int renormalize_l2
) {
    if (n <= 0 || d <= 0) { return cudaErrorInvalidValue; }

    // Allocate a 1-element scratch on the device for weight_sum.
    // We use cudaMallocAsync so the lifetime ends at the stream.
    float* dev_w_sum = nullptr;
    EFE_CUDA_RETURN_IF_ERR(cudaMallocAsync((void**)&dev_w_sum, sizeof(float), stream));

    // Launch 1 : sum weights.
    {
        int threads = n < 256 ? n : 256;
        threads = efe_next_pow2(threads);
        if (threads > 1024) { threads = 1024; }
        const int n_warps = (threads + 31) / 32;
        const size_t smem  = n_warps * sizeof(float);
        sum_weights_kernel<<<1, threads, smem, stream>>>(
            weights, dev_w_sum, n);
        EFE_CUDA_RETURN_IF_ERR(cudaGetLastError());
    }

    // Launch 2 : weighted sum.
    {
        int threads = n < 256 ? n : 256;
        threads = efe_next_pow2(threads);
        if (threads > 1024) { threads = 1024; }
        const int n_warps = (threads + 31) / 32;
        const size_t smem  = n_warps * sizeof(float);
        weighted_mean_kernel<<<d, threads, smem, stream>>>(
            values, weights, dev_w_sum, output, n, d, renormalize_l2);
        EFE_CUDA_RETURN_IF_ERR(cudaGetLastError());
    }

    // Launch 3 (conditional) : L2 renormalise output.
    if (renormalize_l2) {
        int threads = d < 256 ? d : 256;
        threads = efe_next_pow2(threads);
        if (threads > 1024) { threads = 1024; }
        const int n_warps = (threads + 31) / 32;
        const size_t smem  = n_warps * sizeof(float);
        l2_renormalize_inplace<<<1, threads, smem, stream>>>(output, d);
        EFE_CUDA_RETURN_IF_ERR(cudaGetLastError());
    }

    EFE_CUDA_RETURN_IF_ERR(cudaFreeAsync(dev_w_sum, stream));
    return cudaSuccess;
}
