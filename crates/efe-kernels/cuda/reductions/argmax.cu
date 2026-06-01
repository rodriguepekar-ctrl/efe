// cuda/reductions/argmax.cu — Sprint D28.
//
// Rowwise argmax. Tie-break by lowest index — matches CpuBackend.
// Each block processes one row ; warp + block argmax via the
// helpers in `efe_warp.cuh`.

#include "efe.cuh"
#include "efe_warp.cuh"
#include <cfloat>
#include <climits>

namespace {

__global__ void argmax_rowwise(
    const float* __restrict__ input,
    int* __restrict__ output,
    int axis_size
) {
    __shared__ float smem_v[32];
    __shared__ int   smem_i[32];

    const int row = blockIdx.x;
    const int tid = threadIdx.x;

    const float* row_in = input + row * axis_size;

    float best_v = -FLT_MAX;
    int   best_i = INT_MAX;
    for (int i = tid; i < axis_size; i += blockDim.x) {
        const float v = row_in[i];
        if (v > best_v || (v == best_v && i < best_i)) {
            best_v = v;
            best_i = i;
        }
    }

    efe::block_reduce_argmax(best_v, best_i, smem_v, smem_i);

    if (tid == 0) {
        output[row] = best_i;
    }
}

}  // anonymous

extern "C" cudaError_t efe_argmax_launch(
    cudaStream_t stream,
    const float* input,
    int* output,
    int n_rows,
    int axis_size
) {
    if (n_rows <= 0 || axis_size <= 0) {
        return cudaErrorInvalidValue;
    }
    int threads = axis_size < 256 ? axis_size : 256;
    threads = efe_next_pow2(threads);
    if (threads > 1024) { threads = 1024; }
    argmax_rowwise<<<n_rows, threads, 0, stream>>>(
        input, output, axis_size);
    return cudaGetLastError();
}
