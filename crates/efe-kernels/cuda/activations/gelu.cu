// cuda/activations/gelu.cu — Sprint D28.
//
// Exact GELU activation matching CpuBackend (uses erff to match
// PyTorch reference at 1e-5 tolerance) :
//   GELU(x) = 0.5 · x · (1 + erf(x / √2))
//
// CUDA's `erff` is documented to be IEEE-754 correctly rounded for
// the entire f32 domain ; cross-backend tolerance: rtol=1e-5.

#include "efe.cuh"

namespace {

__global__ void gelu_elementwise(
    const float* __restrict__ input,
    float* __restrict__ output,
    int n
) {
    const int idx = blockIdx.x * blockDim.x + threadIdx.x;
    if (idx >= n) { return; }
    constexpr float SQRT2_INV = 0.70710678118654752440f;  // 1/√2
    const float x = input[idx];
    output[idx] = 0.5f * x * (1.0f + erff(x * SQRT2_INV));
}

}  // anonymous

extern "C" cudaError_t efe_gelu_launch(
    cudaStream_t stream,
    const float* input,
    float* output,
    int n
) {
    if (n <= 0) { return cudaErrorInvalidValue; }
    const int block = 256;
    const int grid  = (n + block - 1) / block;
    gelu_elementwise<<<grid, block, 0, stream>>>(input, output, n);
    return cudaGetLastError();
}
