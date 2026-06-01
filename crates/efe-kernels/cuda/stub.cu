// EFE CUDA kernels — Sprint 0 stub.
//
// Real kernels arrive in Sprint 1+:
//   - spawn_batch.cu (Sprint 2)
//   - swarm_attention.cu (Sprint 3, may be Triton only)
//   - langevin_sde.cu (Sprint 3)
//   - frechet_mean.cu (Sprint 5)
//   - projection.cu (Sprint 5)

#include "efe.cuh"

// A trivial function to validate the CUDA build pipeline.
// Returns the EFE version encoded as MAJOR*10000 + MINOR*100 + PATCH.
extern "C" int efe_kernels_version() {
    return EFE_VERSION_MAJOR * 10000
         + EFE_VERSION_MINOR * 100
         + EFE_VERSION_PATCH;
}

// Simple device-side noop kernel to ensure nvcc compiles and links.
__global__ void efe_noop_kernel() {
    // nothing
}

extern "C" cudaError_t efe_noop_launch(cudaStream_t stream) {
    efe_noop_kernel<<<1, 1, 0, stream>>>();
    return cudaGetLastError();
}
