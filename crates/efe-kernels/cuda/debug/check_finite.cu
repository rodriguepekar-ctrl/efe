// cuda/debug/check_finite.cu — Sprint D28 (Φ-D).
//
// **Audit fix K-A7** — pre-fix the `auto_check_finite` Cargo
// feature was declared in `efe-kernels/Cargo.toml` and the env
// var `EFE_RUNTIME_GPU_CHECK_FINITE=true` was documented in the
// architecture doc § 21.1, but no kernel actually scanned for
// NaN/Inf. The hook below closes the loop : a single-block kernel
// counts non-finite values into a u32 device-side counter ; the
// Rust side reads it back and surfaces a `NumericalInstability`
// error if > 0.
//
// Cost when enabled : 1 extra kernel + 1 device-to-host copy per
// hot kernel (~ 5-10 µs on T4). Recommended for CI / debug only.

#include "efe.cuh"

namespace {

// One block of `blockDim.x` threads scans the input strided by
// `blockDim.x`. Each warp uses `__ballot_sync` to count how many
// of its 32 lanes hold a non-finite value, then lane 0 atomicAdds
// the warp count to the global counter.
//
// Determinism note : `atomicAdd<u32>` is deterministic on integer
// types — the counter ends with the same value regardless of warp
// scheduling order. This is the ONE place we use atomicAdd.
__global__ void check_finite_kernel(
    const float* __restrict__ input,
    int n,
    unsigned int* __restrict__ counter
) {
    const int idx = blockIdx.x * blockDim.x + threadIdx.x;
    bool bad = false;
    if (idx < n) {
        const float v = input[idx];
        bad = !isfinite(v);
    }
    // Warp-level vote — count lanes with `bad == true`.
    unsigned int mask = __ballot_sync(0xffffffff, bad);
    if ((threadIdx.x & 31) == 0) {
        const unsigned int n_bad = __popc(mask);
        if (n_bad > 0) {
            atomicAdd(counter, n_bad);
        }
    }
}

}  // anonymous

extern "C" cudaError_t efe_check_finite_launch(
    cudaStream_t stream,
    const float* input,
    int n,
    unsigned int* counter  // 1-element u32 device buffer, pre-zeroed
) {
    if (n <= 0) {
        // Empty input → trivially finite ; counter stays 0.
        return cudaSuccess;
    }
    if (counter == nullptr) {
        return cudaErrorInvalidValue;
    }
    const int block = 256;
    const int grid  = (n + block - 1) / block;
    check_finite_kernel<<<grid, block, 0, stream>>>(input, n, counter);
    return cudaGetLastError();
}
