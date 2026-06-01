// cuda/efe_core/cone_project.cu — Sprint D28.
//
// Cone projection — the cornerstone of EFE alignment.
// For each latent vector z_i (B total) :
//   1. find the nearest anchor j* = argmin_j ||z_i - anchors_j||₂ ;
//   2. if ||z_i - anchors_{j*}||₂ ≤ max_dist, keep z_i ;
//   3. else, project : z'_i = anchors_{j*} + (z_i - anchors_{j*}) · max_dist / dist.
//
// One block per latent. Threads cooperate to compute distances
// and reductions for argmin. Tile shape :
//   blockDim.x = next_pow2(min(d, 256))
//   gridDim.x  = B
//
// Numerical safety : `max_dist` is validated upstream (CudaBackend
// rejects negative / NaN). Tie-break by lowest anchor index.

#include "efe.cuh"
#include "efe_warp.cuh"
#include <cfloat>
#include <climits>

namespace {

__global__ void cone_project_block(
    const float* __restrict__ latents,
    const float* __restrict__ anchors,
    float* __restrict__ output,
    int n_anchors,
    int d,
    float max_dist
) {
    __shared__ float s_best_d2;
    __shared__ int   s_best_j;

    const int row = blockIdx.x;
    const int tid = threadIdx.x;

    const float* z = latents + row * d;
    float* z_out = output + row * d;

    if (tid == 0) {
        s_best_d2 = FLT_MAX;
        s_best_j  = 0;
    }
    __syncthreads();

    extern __shared__ float smem[];

    // Phase 1 : find nearest anchor.
    for (int j = 0; j < n_anchors; ++j) {
        float local_d2 = 0.0f;
        for (int i = tid; i < d; i += blockDim.x) {
            const float diff = z[i] - anchors[j * d + i];
            local_d2 += diff * diff;
        }
        float d2 = efe::block_reduce_sum(local_d2, smem);
        if (tid == 0) {
            if (d2 < s_best_d2 || (d2 == s_best_d2 && j < s_best_j)) {
                s_best_d2 = d2;
                s_best_j  = j;
            }
        }
        __syncthreads();
    }

    const int   j_star = s_best_j;
    const float dist   = sqrtf(fmaxf(s_best_d2, 0.0f));

    // Phase 2 : either copy z verbatim, or project onto cone.
    if (dist <= max_dist) {
        for (int i = tid; i < d; i += blockDim.x) {
            z_out[i] = z[i];
        }
    } else {
        const float scale = max_dist / fmaxf(dist, EFE_EPS);
        for (int i = tid; i < d; i += blockDim.x) {
            const float a_i = anchors[j_star * d + i];
            z_out[i] = a_i + (z[i] - a_i) * scale;
        }
    }
}

}  // anonymous

extern "C" cudaError_t efe_cone_project_launch(
    cudaStream_t stream,
    const float* latents,
    const float* anchors,
    float* output,
    int b,
    int n_anchors,
    int d,
    float max_dist
) {
    if (b <= 0 || n_anchors <= 0 || d <= 0 ||
        !(max_dist >= 0.0f) || isnan(max_dist) || isinf(max_dist)) {
        return cudaErrorInvalidValue;
    }
    int threads = d < 256 ? d : 256;
    threads = efe_next_pow2(threads);
    if (threads > 1024) { threads = 1024; }
    const int n_warps = (threads + 31) / 32;
    const size_t smem  = n_warps * sizeof(float);
    cone_project_block<<<b, threads, smem, stream>>>(
        latents, anchors, output, n_anchors, d, max_dist);
    return cudaGetLastError();
}
