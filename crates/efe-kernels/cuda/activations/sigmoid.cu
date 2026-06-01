// cuda/activations/sigmoid.cu — Sprint D28.
//
// Numerically-stable sigmoid using two-branch formulation :
//   x ≥ 0 :  1 / (1 + exp(-x))
//   x < 0 :  exp(x) / (1 + exp(x))
//
// Avoids overflow when x is very negative (the naïve form
// exp(-x) overflows for x < -88).

#include "efe.cuh"

namespace {

__device__ __forceinline__ float stable_sigmoid(float x) {
    if (x >= 0.0f) {
        const float z = efe_safe_expf(-x);
        return 1.0f / (1.0f + z);
    } else {
        const float z = efe_safe_expf(x);
        return z / (1.0f + z);
    }
}

__global__ void sigmoid_elementwise(
    const float* __restrict__ input,
    float* __restrict__ output,
    int n
) {
    const int idx = blockIdx.x * blockDim.x + threadIdx.x;
    if (idx >= n) { return; }
    output[idx] = stable_sigmoid(input[idx]);
}

}  // anonymous

extern "C" cudaError_t efe_sigmoid_launch(
    cudaStream_t stream,
    const float* input,
    float* output,
    int n
) {
    if (n <= 0) { return cudaErrorInvalidValue; }
    const int block = 256;
    const int grid  = (n + block - 1) / block;
    sigmoid_elementwise<<<grid, block, 0, stream>>>(input, output, n);
    return cudaGetLastError();
}
