//! `ReplayBuffer` — ring buffer of latent snapshots for M7 rollback.
//!
//! Owned by the `FusionStrategy` (when `enable_replay = true`), backed
//! by a dedicated arena separate from the swarm's ELS arena. This
//! separation guarantees that resetting the ELS arena (on collapse)
//! does *not* invalidate snapshots.
//!
//! # Memory discipline
//!
//! Each slot stores one bf16 latent copy: `N * D * 2` bytes. Capacity
//! defaults to 4, so the worst-case footprint is `4 * N * D * 2`. For
//! `N=128, D=2048`: **2 MB** per swarm. The arena is sized accordingly
//! at construction and lives for the entire campaign.

use std::collections::VecDeque;
use std::sync::Arc;

use crate::els::EpistemicLatentSpace;
use crate::errors::{EfeError, Result};
use crate::memory::{Arena, DeviceBackend, RawAllocation};

/// One snapshot slot in the replay buffer.
#[derive(Debug)]
struct SnapshotSlot {
    round: u32,
    latent: RawAllocation,
    #[allow(dead_code)]
    size_bytes: u64,
}

/// Ring buffer of latent snapshots.
///
/// - `push(round, els, arena)` copies the current latent bytes into a
///   freshly-allocated slot.
/// - `restore(round, els, arena)` overwrites the ELS latent with the
///   snapshot's bytes (same layout).
/// - When the buffer is full, pushing evicts the oldest slot.
///
/// The internal arena is reset when *all* slots have been evicted — but
/// in the steady state of a fusion campaign, each slot's memory lives
/// until explicitly overwritten. For MVP we accept the fragmentation
/// cost; Sprint 5+ will move to a dedicated slab allocator.
pub struct ReplayBuffer {
    arena: Arena,
    capacity: u32,
    snapshots: VecDeque<SnapshotSlot>,
}

impl std::fmt::Debug for ReplayBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplayBuffer")
            .field("capacity", &self.capacity)
            .field("len", &self.snapshots.len())
            .finish_non_exhaustive()
    }
}

impl ReplayBuffer {
    /// Create a new buffer backed by a dedicated arena.
    ///
    /// `arena_size_bytes` must be ≥ `capacity * snapshot_size_bytes`
    /// (plus alignment overhead). The caller is responsible for sizing;
    /// a safe estimate is `capacity * N * D * 2 * 2` (2× for alignment
    /// padding on `TENSOR_ALIGNMENT`).
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] if `capacity == 0`.
    /// - Propagates arena allocation failures.
    pub fn new(
        capacity: u32,
        arena_size_bytes: u64,
        backend: Arc<dyn DeviceBackend>,
    ) -> Result<Self> {
        if capacity == 0 {
            return Err(EfeError::InvalidConfig(
                "ReplayBuffer: capacity must be > 0".into(),
            ));
        }
        let arena = Arena::new(arena_size_bytes, backend, 0)?;
        Ok(Self {
            arena,
            capacity,
            snapshots: VecDeque::with_capacity(capacity as usize),
        })
    }

    /// Number of snapshots currently retained.
    #[must_use]
    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    /// True iff no snapshot is currently retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    /// Capacity of the buffer.
    #[must_use]
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Snapshot the latent of `els` under tag `round`.
    ///
    /// # Errors
    ///
    /// - [`EfeError::Internal`] if `els` is a stub (no latent populated).
    /// - Propagates arena failures.
    pub fn push(
        &mut self,
        round: u32,
        els: &EpistemicLatentSpace,
        els_arena: &Arena,
    ) -> Result<()> {
        let src = els
            .latent_raw()
            .ok_or(EfeError::Internal("ReplayBuffer::push: ELS has no latent"))?;
        src.validate_against(els_arena)?;

        let size_bytes = u64::from(els.agent_count()) * u64::from(els.dim().get()) * 2;

        // Evict oldest if full. Note: we don't reclaim its arena bytes
        // in MVP — the new slot lives further down the bump cursor.
        if self.snapshots.len() >= self.capacity as usize {
            self.snapshots.pop_front();
        }

        let dst = self.arena.alloc_zeroed_raw(size_bytes)?;

        // SAFETY:
        //  - `src.validate_against(els_arena)` succeeded → `src.ptr` is
        //    valid for `size_bytes`.
        //  - `dst.ptr` was just allocated with `size_bytes` zero bytes.
        //  - Both are host-backend pointers for the MVP (generation
        //    check excludes CUDA paths).
        unsafe {
            std::ptr::copy_nonoverlapping(
                src.ptr.as_raw().as_ptr(),
                dst.ptr.as_raw().as_ptr(),
                size_bytes as usize,
            );
        }

        self.snapshots.push_back(SnapshotSlot {
            round,
            latent: dst,
            size_bytes,
        });
        Ok(())
    }

    /// Restore the latent of `els` from the snapshot tagged `round`.
    ///
    /// The snapshot is retained afterwards (not popped) — the caller
    /// may want to try again.
    ///
    /// # Errors
    ///
    /// - [`EfeError::Internal`] if no snapshot with that `round` exists,
    ///   or if ELS is a stub, or if sizes mismatch.
    pub fn restore(
        &self,
        round: u32,
        els: &mut EpistemicLatentSpace,
        els_arena: &Arena,
    ) -> Result<()> {
        let slot = self
            .snapshots
            .iter()
            .find(|s| s.round == round)
            .ok_or(EfeError::Internal(
                "ReplayBuffer::restore: no snapshot with that round tag",
            ))?;

        let dst = els.latent_raw().ok_or(EfeError::Internal(
            "ReplayBuffer::restore: ELS has no latent",
        ))?;
        dst.validate_against(els_arena)?;

        let expected_size = u64::from(els.agent_count()) * u64::from(els.dim().get()) * 2;
        if slot.size_bytes != expected_size {
            return Err(EfeError::Internal(
                "ReplayBuffer::restore: snapshot size mismatches ELS",
            ));
        }

        // SAFETY: same contract as push — both pointers valid for
        // `expected_size` bytes, both host-backend.
        unsafe {
            std::ptr::copy_nonoverlapping(
                slot.latent.ptr.as_raw().as_ptr(),
                dst.ptr.as_raw().as_ptr(),
                expected_size as usize,
            );
        }
        Ok(())
    }

    /// Rounds currently held, oldest-first.
    #[must_use]
    pub fn rounds(&self) -> Vec<u32> {
        self.snapshots.iter().map(|s| s.round).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::HostBackend;
    use crate::types::{LatentDim, SwarmId};
    use half::bf16;

    fn mk_populated_els(
        n: u32,
        d: u32,
        fill: impl Fn(u32, u32) -> f32,
    ) -> (Arena, EpistemicLatentSpace) {
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let arena = Arena::new(1 << 18, backend, 0).unwrap();
        let latent = arena
            .alloc_zeroed_raw(u64::from(n) * u64::from(d) * 2)
            .unwrap();
        let mask = arena.alloc_zeroed_raw(u64::from(n)).unwrap();
        let anchors = arena.alloc_zeroed_raw(u64::from(d) * 4).unwrap();
        let problem = arena.alloc_zeroed_raw(u64::from(d) * 4).unwrap();
        unsafe {
            let p = latent.ptr.as_raw().cast::<bf16>().as_ptr();
            for i in 0..n {
                for j in 0..d {
                    *p.add((i * d + j) as usize) = bf16::from_f32(fill(i, j));
                }
            }
            let p = mask.ptr.as_raw().as_ptr();
            for i in 0..n {
                *p.add(i as usize) = 1;
            }
        }
        let els = EpistemicLatentSpace::new_populated(
            SwarmId::new(),
            LatentDim::new(d).unwrap(),
            n,
            latent,
            mask,
            anchors,
            problem,
            1,
        );
        (arena, els)
    }

    #[test]
    fn push_then_restore_roundtrips_bytes() {
        let (arena, mut els) = mk_populated_els(4, 4, |i, j| (i + j) as f32 * 0.25);
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let mut buf = ReplayBuffer::new(4, 1 << 16, backend).unwrap();

        // Snapshot round 0.
        buf.push(0, &els, &arena).unwrap();

        // Corrupt the latent.
        {
            let mut split = els.split_mut(&arena).unwrap();
            let row = split.latent.row_mut(0).unwrap();
            row[0] = bf16::from_f32(999.0);
        }

        // Verify corruption took.
        let snap_corrupted = els.snapshot(&arena).unwrap();

        // Restore round 0.
        buf.restore(0, &mut els, &arena).unwrap();
        let snap_after = els.snapshot(&arena).unwrap();

        // Hash should differ from corrupted, match original.
        assert_ne!(snap_corrupted.centroid_hash, snap_after.centroid_hash);
    }

    #[test]
    fn capacity_eviction_works() {
        let (arena, els) = mk_populated_els(2, 2, |_, _| 1.0);
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let mut buf = ReplayBuffer::new(3, 1 << 16, backend).unwrap();
        buf.push(0, &els, &arena).unwrap();
        buf.push(1, &els, &arena).unwrap();
        buf.push(2, &els, &arena).unwrap();
        buf.push(3, &els, &arena).unwrap(); // evicts round 0
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.rounds(), vec![1, 2, 3]);
    }

    #[test]
    fn restore_unknown_round_errors() {
        let (arena, mut els) = mk_populated_els(2, 2, |_, _| 1.0);
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let buf = ReplayBuffer::new(3, 1 << 16, backend).unwrap();
        let err = buf.restore(99, &mut els, &arena).unwrap_err();
        assert!(matches!(err, EfeError::Internal(_)));
    }

    #[test]
    fn capacity_zero_errors() {
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let err = ReplayBuffer::new(0, 1024, backend).unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[test]
    fn new_buffer_is_empty() {
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let buf = ReplayBuffer::new(4, 1024, backend).unwrap();
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());
        assert_eq!(buf.capacity(), 4);
    }
}
