// cuda/reductions/std_dev.cu — Sprint D28.
//
// Welford's online algorithm — numerically stable single-pass
// mean + variance. Less prone to catastrophic cancellation than
// `Σ(x - mean)²`. Matches `CpuBackend::std_dev` reference values
// to rtol=1e-5.
//
// Each block processes one row. Threads compute partial Welford
// triples (n, mean, M2), then merge them via tree-reduction
// using the parallel Welford update formula.

#include "efe.cuh"
#include "efe_warp.cuh"

namespace {

struct Welford {
    int n;
    float mean;
    float m2;
};

__device__ __forceinline__ Welford welford_zero() {
    return Welford{0, 0.0f, 0.0f};
}

__device__ __forceinline__ Welford welford_add(Welford w, float x) {
    int new_n   = w.n + 1;
    float delta = x - w.mean;
    float new_mean = w.mean + delta / static_cast<float>(new_n);
    float new_m2   = w.m2 + delta * (x - new_mean);
    return Welford{new_n, new_mean, new_m2};
}

__device__ __forceinline__ Welford welford_merge(Welford a, Welford b) {
    if (a.n == 0) { return b; }
    if (b.n == 0) { return a; }
    int new_n     = a.n + b.n;
    float delta   = b.mean - a.mean;
    float new_mean = a.mean + delta * static_cast<float>(b.n) /
                              static_cast<float>(new_n);
    float new_m2  = a.m2 + b.m2 +
                    delta * delta *
                    static_cast<float>(a.n) * static_cast<float>(b.n) /
                    static_cast<float>(new_n);
    return Welford{new_n, new_mean, new_m2};
}

__global__ void std_dev_rowwise(
    const float* __restrict__ input,
    float* __restrict__ output,
    int axis_size,
    int unbiased
) {
    // Two arrays of shared memory : one float for mean, one int
    // for n (we pack them into a single Welford via static smem).
    __shared__ Welford smem[32];

    const int row = blockIdx.x;
    const int tid = threadIdx.x;
    const int lane    = tid % 32;
    const int warp_id = tid / 32;
    const int n_warps = (blockDim.x + 31) / 32;

    const float* row_in = input + row * axis_size;

    // Phase 1 : per-thread Welford reduction.
    Welford w = welford_zero();
    for (int i = tid; i < axis_size; i += blockDim.x) {
        w = welford_add(w, row_in[i]);
    }

    // Phase 2 : warp reduction via shuffle.
    #pragma unroll
    for (int offset = 16; offset > 0; offset >>= 1) {
        Welford other;
        other.n    = __shfl_down_sync(0xffffffff, w.n,    offset);
        other.mean = __shfl_down_sync(0xffffffff, w.mean, offset);
        other.m2   = __shfl_down_sync(0xffffffff, w.m2,   offset);
        w = welford_merge(w, other);
    }

    // Phase 3 : cross-warp via shared memory.
    if (lane == 0) { smem[warp_id] = w; }
    __syncthreads();

    if (warp_id == 0) {
        Welford x = (lane < n_warps) ? smem[lane] : welford_zero();
        #pragma unroll
        for (int offset = 16; offset > 0; offset >>= 1) {
            Welford other;
            other.n    = __shfl_down_sync(0xffffffff, x.n,    offset);
            other.mean = __shfl_down_sync(0xffffffff, x.mean, offset);
            other.m2   = __shfl_down_sync(0xffffffff, x.m2,   offset);
            x = welford_merge(x, other);
        }
        if (lane == 0) {
            const float divisor = unbiased ? static_cast<float>(x.n - 1)
                                           : static_cast<float>(x.n);
            const float variance = (x.n > 1) ? (x.m2 / fmaxf(divisor, 1.0f)) : 0.0f;
            output[row] = sqrtf(fmaxf(variance, 0.0f));
        }
    }
}

}  // anonymous

extern "C" cudaError_t efe_std_dev_launch(
    cudaStream_t stream,
    const float* input,
    float* output,
    int n_rows,
    int axis_size,
    int unbiased
) {
    if (n_rows <= 0 || axis_size <= 0) {
        return cudaErrorInvalidValue;
    }
    int threads = axis_size < 256 ? axis_size : 256;
    threads = efe_next_pow2(threads);
    if (threads > 1024) { threads = 1024; }
    std_dev_rowwise<<<n_rows, threads, 0, stream>>>(
        input, output, axis_size, unbiased);
    return cudaGetLastError();
}
