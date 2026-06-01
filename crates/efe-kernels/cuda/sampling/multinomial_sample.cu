// cuda/sampling/multinomial_sample.cu — Sprint D29.
//
// Multinomial sampling via inverse-CDF method, driven by cuRAND.
//
// probs  : [B × K] (assumed normalized, sums to 1)
// output : [B × n_samples]
// seed   : same seed → same samples bit-exact (cuRAND philox).
//
// Each block samples for one batch row. CDF is constructed by a
// thread-cooperative prefix-sum in shared memory.

#include "efe.cuh"
#include <curand_kernel.h>

namespace {

// Threads in a block cooperate to build the CDF then sample.
// CDF[k] = Σ_{j ≤ k} probs[k]. Computed via Hillis-Steele scan
// in shared memory ; deterministic for fixed thread count.
__global__ void multinomial_sample_block(
    const float* __restrict__ probs,
    int* __restrict__ output,
    int k_classes,
    int n_samples,
    unsigned long long seed
) {
    extern __shared__ float smem_cdf[];

    const int b   = blockIdx.x;
    const int tid = threadIdx.x;

    const float* row_p = probs + b * k_classes;

    // Hillis-Steele inclusive prefix-sum (assumes blockDim.x ≥ k_classes
    // — caller enforces). For larger k_classes we'd partition into
    // tiles ; D29 limits us to k_classes ≤ blockDim.x ≤ 1024.
    if (tid < k_classes) {
        smem_cdf[tid] = row_p[tid];
    }
    __syncthreads();

    for (int offset = 1; offset < k_classes; offset <<= 1) {
        float v = 0.0f;
        if (tid >= offset && tid < k_classes) {
            v = smem_cdf[tid - offset];
        }
        __syncthreads();
        if (tid >= offset && tid < k_classes) {
            smem_cdf[tid] += v;
        }
        __syncthreads();
    }
    // smem_cdf[k] now holds Σ_{j ≤ k} probs[j].

    // Initialise per-thread cuRAND state with `seed` salted by
    // (b, sample_idx) so the same seed across runs gives the
    // same samples regardless of GPU.
    curandStatePhilox4_32_10_t state;
    curand_init(seed, b * n_samples + tid, 0, &state);

    // Each thread draws (n_samples / blockDim.x) uniform values
    // and binary-searches the CDF. Tie-break by lower index
    // (matches CpuBackend reference).
    int* batch_out = output + b * n_samples;
    for (int s = tid; s < n_samples; s += blockDim.x) {
        const float u = curand_uniform(&state);
        // Binary search in smem_cdf[0..k_classes-1].
        int lo = 0, hi = k_classes - 1;
        while (lo < hi) {
            const int mid = (lo + hi) / 2;
            if (smem_cdf[mid] < u) {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        batch_out[s] = lo;
    }
}

}  // anonymous

extern "C" cudaError_t efe_multinomial_sample_launch(
    cudaStream_t stream,
    const float* probs,
    int* output,
    int b,
    int k_classes,
    int n_samples,
    unsigned long long seed
) {
    if (b <= 0 || k_classes <= 0 || n_samples <= 0 ||
        k_classes > 1024) {
        return cudaErrorInvalidValue;
    }

    int threads = k_classes < 256 ? 256 : k_classes;
    threads = efe_next_pow2(threads);
    if (threads > 1024) { threads = 1024; }

    const size_t smem = k_classes * sizeof(float);
    multinomial_sample_block<<<b, threads, smem, stream>>>(
        probs, output, k_classes, n_samples, seed);
    return cudaGetLastError();
}
