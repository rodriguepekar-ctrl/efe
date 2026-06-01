//! Multi-GPU sharding via NCCL — Sprint D30 (skeleton in D28).
//!
//! Cf. `docs/EFE_KERNELS_ARCHITECTURE.md` § 15.
//!
//! For essaims with `N > 256`, the ELS `[N × D]` is partitioned
//! across `K` GPUs. Each GPU runs a local fusion round on its
//! shard and the partial centroids are summed via NCCL all-reduce.
//!
//! **D28 status** : the module ships the public API skeleton +
//! topology validation. The real NCCL all-reduce path lands in
//! D30 — until then, [`ShardedBackend::all_reduce_centroid`]
//! returns `CudaError::NotYetWired`.

use std::sync::Arc;

use crate::backend::CudaBackend;
use crate::error::CudaError;
use crate::observability;

/// Sharded multi-GPU backend.
///
/// Construct with [`ShardedBackend::new`] passing the ordinals of
/// the GPUs to use (typically `[0, 1, 2, 3]` for a 4-GPU pod).
pub struct ShardedBackend {
    backends: Vec<Arc<CudaBackend>>,
    nccl_available: bool,
}

impl std::fmt::Debug for ShardedBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShardedBackend")
            .field("n_shards", &self.backends.len())
            .field("nccl_available", &self.nccl_available)
            .finish()
    }
}

impl ShardedBackend {
    /// Construct a sharded backend over the listed device ordinals.
    /// Each GPU gets its own [`CudaBackend`] instance.
    pub fn new(ordinals: &[u32]) -> Result<Self, CudaError> {
        if ordinals.is_empty() {
            return Err(CudaError::InitFailed(
                "ShardedBackend requires ≥ 1 GPU ordinal".into(),
            ));
        }

        let mut backends = Vec::with_capacity(ordinals.len());
        for &o in ordinals {
            let b = CudaBackend::new(o)?;
            backends.push(Arc::new(b));
        }

        let nccl_available = cfg!(feature = "nccl");

        Ok(Self {
            backends,
            nccl_available,
        })
    }

    /// Number of shards (= GPUs).
    #[must_use]
    pub fn n_shards(&self) -> usize {
        self.backends.len()
    }

    /// Borrow shard `i`.
    #[must_use]
    pub fn shard(&self, i: usize) -> Option<&Arc<CudaBackend>> {
        self.backends.get(i)
    }

    /// All-reduce sum of partial centroids across shards.
    ///
    /// **D28 stub** — returns `CudaError::NotYetWired("all_reduce_centroid")`
    /// until D30 wires the actual NCCL collective. The signature
    /// is final ; only the body is deferred.
    ///
    /// `partials[i]` is a host-side buffer for shard `i` of length
    /// `D`. On success, every shard observes the **summed** centroid
    /// in its own copy of `partials[i]`.
    pub fn all_reduce_centroid(&self, _partials: &mut [Vec<f32>]) -> Result<(), CudaError> {
        let _t = observability::record_kernel_call("nccl_allreduce_centroid");
        if !self.nccl_available {
            tracing::warn!(
                target: "efe.cuda.multi_gpu",
                "NCCL feature off — falling back to TCP all-reduce (DT-K-D30-A1, deferred)"
            );
        }
        Err(CudaError::NotYetWired("ShardedBackend::all_reduce_centroid"))
    }

    /// Synchronise every shard. Used as a barrier between rounds.
    pub fn synchronize_all(&self) -> Result<(), CudaError> {
        for (i, b) in self.backends.iter().enumerate() {
            b.synchronize_streams().map_err(|e| {
                CudaError::StreamError(format!("shard {i}: {e}"))
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sharded_backend_rejects_empty_ordinal_list() {
        let r = ShardedBackend::new(&[]);
        assert!(matches!(r, Err(CudaError::InitFailed(_))));
    }

    // Real GPU tests live in `tests/multi_gpu.rs`, gated on the
    // `[self-hosted, gpu, multi-a10]` runner label.
}
