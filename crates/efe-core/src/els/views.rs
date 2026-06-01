//! Typed borrowed views over the raw allocations of an ELS.
//!
//! # Lifetime model
//!
//! Every view carries a lifetime `'a` tied to a `&'a Arena`. The
//! constructors validate the allocation's generation against the
//! arena; once constructed, views dereference raw pointers directly,
//! trusting the lifetime contract.
//!
//! # Alignment invariant
//!
//! The arena guarantees every allocation is aligned to
//! [`crate::memory::config::TENSOR_ALIGNMENT`] (256 bytes, far larger
//! than the natural alignment of `bf16` / `f32` / `u8`). Therefore
//! `slice::from_raw_parts[_mut]` is always sound here — we never hit
//! the under-aligned pointer UB path.
//!
//! # Exclusivity invariant (Mut views)
//!
//! `LatentViewMut::from_raw` and `MaskViewMut::from_raw` are
//! `pub(crate)`. Callers must ensure at most one mutable view exists
//! per `RawAllocation` at any time. The public entry points
//! (`EpistemicLatentSpace::split_mut`, `latent_view_mut`,
//! `mask_view_mut`) all take `&mut self`, which the borrow checker
//! uses to enforce this invariant for us. Do **not** construct Mut
//! views manually from outside the `els` module.
//!
//! # Backend assumptions
//!
//! MVP assumes the `HostBackend`: raw device pointers are
//! host-dereferenceable. A CUDA view type will be added in Sprint 3+
//! behind a feature flag; it exposes the same API but copies to host
//! on first read.

use std::marker::PhantomData;

use half::bf16;

use crate::errors::{EfeError, Result};
use crate::memory::{Arena, RawAllocation};

// =====================================================================
// LatentView — immutable
// =====================================================================

/// Immutable view over the `[N, D]` latent tensor (bf16).
pub struct LatentView<'a> {
    data: &'a [bf16],
    n_agents: u32,
    dim: u32,
}

impl<'a> LatentView<'a> {
    /// Construct from a validated raw allocation.
    ///
    /// # Errors
    ///
    /// - [`EfeError::Internal`] if the allocation's generation has
    ///   drifted from the arena's current one, or if the declared
    ///   `n * d` exceeds `usize::MAX`.
    pub(crate) fn from_raw(raw: RawAllocation, n: u32, d: u32, arena: &'a Arena) -> Result<Self> {
        raw.validate_against(arena)?;
        let len = (n as usize)
            .checked_mul(d as usize)
            .ok_or(EfeError::Internal("LatentView: n*d overflow"))?;
        // SAFETY:
        //  - `raw` is a host-backend allocation of at least len*2 bytes.
        //  - `arena`'s lifetime `'a` outlives this function's return.
        //  - Generation check ensures the arena was not reset between
        //    spawn and now.
        let ptr = unsafe { raw.ptr.as_raw().cast::<bf16>().as_ptr() };
        let data = unsafe { std::slice::from_raw_parts(ptr, len) };
        Ok(Self {
            data,
            n_agents: n,
            dim: d,
        })
    }

    /// Row `agent` as `&[bf16]` of length `dim`.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] if `agent >= n_agents`.
    pub fn row(&self, agent: u32) -> Result<&[bf16]> {
        if agent >= self.n_agents {
            return Err(EfeError::InvalidConfig(format!(
                "LatentView: row {agent} out of range (n={})",
                self.n_agents
            )));
        }
        let start = (agent as usize) * (self.dim as usize);
        let end = start + (self.dim as usize);
        Ok(&self.data[start..end])
    }

    /// Iterate over every row (in `0..n_agents` order).
    pub fn iter_rows(&self) -> impl Iterator<Item = &[bf16]> {
        let d = self.dim as usize;
        self.data.chunks_exact(d)
    }

    /// Iterate over rows where the corresponding mask bit is `1`,
    /// yielding `(agent_index, row)`.
    pub fn iter_active_rows<'b>(
        &'b self,
        mask: &'b MaskView<'_>,
    ) -> impl Iterator<Item = (u32, &'b [bf16])> {
        let d = self.dim as usize;
        self.data
            .chunks_exact(d)
            .enumerate()
            .filter_map(move |(i, row)| {
                let agent = i as u32;
                if mask.is_active(agent) {
                    Some((agent, row))
                } else {
                    None
                }
            })
    }

    /// Number of agents.
    #[must_use]
    pub fn n_agents(&self) -> u32 {
        self.n_agents
    }

    /// Latent dimension.
    #[must_use]
    pub fn dim(&self) -> u32 {
        self.dim
    }

    /// Raw slice (read-only, `[N*D]` row-major).
    #[must_use]
    pub fn as_flat(&self) -> &[bf16] {
        self.data
    }
}

#[allow(clippy::missing_docs_in_private_items)]
impl std::fmt::Debug for LatentView<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LatentView")
            .field("n_agents", &self.n_agents)
            .field("dim", &self.dim)
            .field("bytes", &self.data.len().saturating_mul(2))
            .finish_non_exhaustive()
    }
}

// =====================================================================
// LatentViewMut — exclusive mutable
// =====================================================================

/// Mutable view over the `[N, D]` latent tensor (bf16).
pub struct LatentViewMut<'a> {
    data: &'a mut [bf16],
    n_agents: u32,
    dim: u32,
}

impl<'a> LatentViewMut<'a> {
    /// Construct from a validated raw allocation with exclusive access.
    ///
    /// # Errors
    ///
    /// See [`LatentView::from_raw`].
    pub(crate) fn from_raw(raw: RawAllocation, n: u32, d: u32, arena: &'a Arena) -> Result<Self> {
        raw.validate_against(arena)?;
        let len = (n as usize)
            .checked_mul(d as usize)
            .ok_or(EfeError::Internal("LatentViewMut: n*d overflow"))?;
        // SAFETY: same contract as `LatentView::from_raw`. Exclusivity
        // is enforced by the caller: `EpistemicLatentSpace::split_mut`
        // takes `&mut self`, so only one mutable view can be alive at
        // a time.
        let ptr = unsafe { raw.ptr.as_raw().cast::<bf16>().as_ptr() };
        let data = unsafe { std::slice::from_raw_parts_mut(ptr, len) };
        Ok(Self {
            data,
            n_agents: n,
            dim: d,
        })
    }

    /// Mutable access to row `agent`.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] if `agent >= n_agents`.
    pub fn row_mut(&mut self, agent: u32) -> Result<&mut [bf16]> {
        if agent >= self.n_agents {
            return Err(EfeError::InvalidConfig(format!(
                "LatentViewMut: row {agent} out of range (n={})",
                self.n_agents
            )));
        }
        let start = (agent as usize) * (self.dim as usize);
        let end = start + (self.dim as usize);
        Ok(&mut self.data[start..end])
    }

    /// Iterate mutably over every row.
    pub fn iter_rows_mut(&mut self) -> impl Iterator<Item = &mut [bf16]> {
        let d = self.dim as usize;
        self.data.chunks_exact_mut(d)
    }

    /// Number of agents.
    #[must_use]
    pub fn n_agents(&self) -> u32 {
        self.n_agents
    }

    /// Latent dimension.
    #[must_use]
    pub fn dim(&self) -> u32 {
        self.dim
    }

    /// Mutable raw flat slice.
    #[must_use]
    pub fn as_flat_mut(&mut self) -> &mut [bf16] {
        self.data
    }

    /// Immutable borrow of the same region (for passing to read-only
    /// kernels while retaining mutable ownership).
    #[must_use]
    pub fn as_immut(&self) -> LatentView<'_> {
        LatentView {
            data: self.data,
            n_agents: self.n_agents,
            dim: self.dim,
        }
    }
}

// =====================================================================
// MaskView — immutable
// =====================================================================

/// Immutable view over the `[N]` active mask (u8, 0 = inactive, 1 = active).
pub struct MaskView<'a> {
    data: &'a [u8],
    n_agents: u32,
}

impl<'a> MaskView<'a> {
    pub(crate) fn from_raw(raw: RawAllocation, n: u32, arena: &'a Arena) -> Result<Self> {
        raw.validate_against(arena)?;
        // SAFETY: raw is a host-backend allocation of at least n bytes.
        let ptr = unsafe { raw.ptr.as_raw().as_ptr() };
        let data = unsafe { std::slice::from_raw_parts(ptr, n as usize) };
        Ok(Self { data, n_agents: n })
    }

    /// Whether agent `i` is active. Returns `false` for out-of-range.
    #[must_use]
    pub fn is_active(&self, agent: u32) -> bool {
        if agent >= self.n_agents {
            return false;
        }
        self.data[agent as usize] != 0
    }

    /// Number of active agents (sum of 1s).
    #[must_use]
    pub fn active_count(&self) -> u32 {
        self.data.iter().filter(|&&b| b != 0).count() as u32
    }

    /// Number of agents in the mask.
    #[must_use]
    pub fn n_agents(&self) -> u32 {
        self.n_agents
    }

    /// Raw slice access for kernels.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        self.data
    }
}

// =====================================================================
// MaskViewMut — exclusive mutable
// =====================================================================

/// Mutable view over the active mask.
pub struct MaskViewMut<'a> {
    data: &'a mut [u8],
    n_agents: u32,
}

impl<'a> MaskViewMut<'a> {
    pub(crate) fn from_raw(raw: RawAllocation, n: u32, arena: &'a Arena) -> Result<Self> {
        raw.validate_against(arena)?;
        // SAFETY: same as `MaskView::from_raw`, exclusivity via `&mut self` on ELS.
        let ptr = unsafe { raw.ptr.as_raw().as_ptr() };
        let data = unsafe { std::slice::from_raw_parts_mut(ptr, n as usize) };
        Ok(Self { data, n_agents: n })
    }

    /// Set the activation flag for `agent`.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] if `agent >= n_agents`.
    pub fn set(&mut self, agent: u32, active: bool) -> Result<()> {
        if agent >= self.n_agents {
            return Err(EfeError::InvalidConfig(format!(
                "MaskViewMut: agent {agent} out of range (n={})",
                self.n_agents
            )));
        }
        self.data[agent as usize] = u8::from(active);
        Ok(())
    }

    /// Mark every agent active.
    pub fn activate_all(&mut self) {
        for b in self.data.iter_mut() {
            *b = 1;
        }
    }

    /// Borrow as an immutable `MaskView`.
    #[must_use]
    pub fn as_immut(&self) -> MaskView<'_> {
        MaskView {
            data: self.data,
            n_agents: self.n_agents,
        }
    }

    /// Number of agents in the mask.
    #[must_use]
    pub fn n_agents(&self) -> u32 {
        self.n_agents
    }
}

// =====================================================================
// AnchorsView — immutable (anchors are constitutionally immutable)
// =====================================================================

/// Immutable view over the `[K, D]` anchors (f32).
pub struct AnchorsView<'a> {
    data: &'a [f32],
    n_anchors: u32,
    dim: u32,
}

impl<'a> AnchorsView<'a> {
    pub(crate) fn from_raw(raw: RawAllocation, n: u32, d: u32, arena: &'a Arena) -> Result<Self> {
        raw.validate_against(arena)?;
        let len = (n as usize)
            .checked_mul(d as usize)
            .ok_or(EfeError::Internal("AnchorsView: n*d overflow"))?;
        // SAFETY: host-backend allocation of at least n*d*4 bytes.
        let ptr = unsafe { raw.ptr.as_raw().cast::<f32>().as_ptr() };
        let data = unsafe { std::slice::from_raw_parts(ptr, len) };
        Ok(Self {
            data,
            n_anchors: n,
            dim: d,
        })
    }

    /// Anchor `idx` as `&[f32]` of length `dim`.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] if `idx >= n_anchors`.
    pub fn get(&self, idx: u32) -> Result<&[f32]> {
        if idx >= self.n_anchors {
            return Err(EfeError::InvalidConfig(format!(
                "AnchorsView: index {idx} out of range (n_anchors={})",
                self.n_anchors
            )));
        }
        let start = (idx as usize) * (self.dim as usize);
        let end = start + (self.dim as usize);
        Ok(&self.data[start..end])
    }

    /// Iterate over all anchors.
    pub fn iter(&self) -> impl Iterator<Item = &[f32]> {
        let d = self.dim as usize;
        self.data.chunks_exact(d)
    }

    /// Number of anchors.
    #[must_use]
    pub fn n_anchors(&self) -> u32 {
        self.n_anchors
    }

    /// Dimensionality of each anchor.
    #[must_use]
    pub fn dim(&self) -> u32 {
        self.dim
    }

    /// Raw flat slice access (`[K*D]` row-major).
    #[must_use]
    pub fn as_flat(&self) -> &[f32] {
        self.data
    }

    /// Materialize the cone as a host-side `Vec<Vec<f32>>`.
    ///
    /// Used by code that needs to cache the cone outside the arena's
    /// lifetime (e.g. M9 requests, telemetry payloads).
    #[must_use]
    pub fn to_host(&self) -> Vec<Vec<f32>> {
        self.iter().map(<[f32]>::to_vec).collect()
    }
}

// =====================================================================
// EmbeddingView — immutable (problem embedding)
// =====================================================================

/// Immutable view over the `[D]` problem embedding (f32).
pub struct EmbeddingView<'a> {
    data: &'a [f32],
    _phantom: PhantomData<&'a Arena>,
}

impl<'a> EmbeddingView<'a> {
    pub(crate) fn from_raw(raw: RawAllocation, d: u32, arena: &'a Arena) -> Result<Self> {
        raw.validate_against(arena)?;
        // SAFETY: host-backend allocation of at least d*4 bytes.
        let ptr = unsafe { raw.ptr.as_raw().cast::<f32>().as_ptr() };
        let data = unsafe { std::slice::from_raw_parts(ptr, d as usize) };
        Ok(Self {
            data,
            _phantom: PhantomData,
        })
    }

    /// Raw slice access.
    #[must_use]
    pub fn as_slice(&self) -> &[f32] {
        self.data
    }

    /// Embedding dimensionality.
    #[must_use]
    pub fn dim(&self) -> u32 {
        self.data.len() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{DeviceBackend, HostBackend};
    use std::sync::Arc;

    /// Build an arena and a raw allocation of `bytes` zeroed bytes. The
    /// returned `Arena` must outlive any view built on `raw`.
    fn mk_arena_raw(bytes: u64) -> (Arena, RawAllocation) {
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let arena = Arena::new(bytes.max(4096), backend, 0).unwrap();
        let raw = arena.alloc_zeroed_raw(bytes).unwrap();
        (arena, raw)
    }

    #[test]
    fn latent_view_row_access() {
        let (arena, raw) = mk_arena_raw(8 * 4 * 2); // n=8, d=4, bf16
        let view = LatentView::from_raw(raw, 8, 4, &arena).unwrap();
        assert_eq!(view.n_agents(), 8);
        assert_eq!(view.dim(), 4);
        let first_row = view.row(0).unwrap();
        assert_eq!(first_row.len(), 4);
    }

    #[test]
    fn latent_view_rejects_oob_row() {
        let (arena, raw) = mk_arena_raw(4 * 4 * 2);
        let view = LatentView::from_raw(raw, 4, 4, &arena).unwrap();
        assert!(view.row(100).is_err());
    }

    #[test]
    fn mask_view_active_count() {
        let (arena, raw) = mk_arena_raw(8);
        // Write pattern: first 5 active, last 3 inactive.
        unsafe {
            let ptr = raw.ptr.as_raw().as_ptr();
            for i in 0..8u32 {
                *ptr.add(i as usize) = u8::from(i < 5);
            }
        }
        let view = MaskView::from_raw(raw, 8, &arena).unwrap();
        assert_eq!(view.active_count(), 5);
        assert!(view.is_active(0));
        assert!(view.is_active(4));
        assert!(!view.is_active(5));
        assert!(!view.is_active(100));
    }

    #[test]
    fn mask_view_mut_set() {
        let (arena, raw) = mk_arena_raw(4);
        let mut mut_view = MaskViewMut::from_raw(raw, 4, &arena).unwrap();
        mut_view.set(0, true).unwrap();
        mut_view.set(2, true).unwrap();
        let immut = mut_view.as_immut();
        assert!(immut.is_active(0));
        assert!(!immut.is_active(1));
        assert!(immut.is_active(2));
        assert!(mut_view.set(99, true).is_err());
    }

    #[test]
    fn anchors_view_iter() {
        let (arena, raw) = mk_arena_raw(3 * 4 * 4); // 3 anchors x 4 f32
        let view = AnchorsView::from_raw(raw, 3, 4, &arena).unwrap();
        assert_eq!(view.n_anchors(), 3);
        assert_eq!(view.iter().count(), 3);
    }

    #[test]
    fn embedding_view_len() {
        let (arena, raw) = mk_arena_raw(16 * 4);
        let view = EmbeddingView::from_raw(raw, 16, &arena).unwrap();
        assert_eq!(view.as_slice().len(), 16);
        assert_eq!(view.dim(), 16);
    }

    #[test]
    fn latent_view_rejects_stale_generation() {
        let (mut arena, raw) = mk_arena_raw(8 * 2 * 2);
        // First use: OK.
        let _v = LatentView::from_raw(raw, 8, 2, &arena).unwrap();
        // Reset arena → raw is now stale.
        arena.reset();
        let err = LatentView::from_raw(raw, 8, 2, &arena).unwrap_err();
        assert!(matches!(err, EfeError::Internal(_)));
    }

    #[test]
    fn latent_view_mut_writes_visible() {
        let (arena, raw) = mk_arena_raw(4 * 2 * 2);
        {
            let mut mut_view = LatentViewMut::from_raw(raw, 4, 2, &arena).unwrap();
            let mutable_row = mut_view.row_mut(0).unwrap();
            mutable_row[0] = bf16::from_f32(2.5);
        }
        let view = LatentView::from_raw(raw, 4, 2, &arena).unwrap();
        let readback_row = view.row(0).unwrap();
        assert!((readback_row[0].to_f32() - 2.5).abs() < 0.05);
    }

    #[test]
    fn iter_active_rows_skips_inactive() {
        let (arena, raw_l) = mk_arena_raw(4 * 2 * 2);
        let raw_m = arena.alloc_zeroed_raw(4).unwrap();
        unsafe {
            let ptr = raw_m.ptr.as_raw().as_ptr();
            *ptr.add(0) = 1;
            *ptr.add(1) = 0;
            *ptr.add(2) = 1;
            *ptr.add(3) = 0;
        }
        let latent = LatentView::from_raw(raw_l, 4, 2, &arena).unwrap();
        let mask = MaskView::from_raw(raw_m, 4, &arena).unwrap();
        let active: Vec<u32> = latent.iter_active_rows(&mask).map(|(i, _)| i).collect();
        assert_eq!(active, vec![0, 2]);
    }

    #[test]
    fn anchors_view_as_flat_has_expected_length() {
        let (arena, raw) = mk_arena_raw(3 * 4 * 4);
        let view = AnchorsView::from_raw(raw, 3, 4, &arena).unwrap();
        assert_eq!(view.as_flat().len(), 3 * 4);
    }

    #[test]
    fn anchors_view_to_host_produces_expected_shape() {
        let (arena, raw) = mk_arena_raw(2 * 4 * 4);
        let view = AnchorsView::from_raw(raw, 2, 4, &arena).unwrap();
        let host = view.to_host();
        assert_eq!(host.len(), 2);
        for row in &host {
            assert_eq!(row.len(), 4);
        }
    }

    #[test]
    fn views_are_send_and_sync() {
        // Compile-time assertion — views over `&[T]` with `T: Send + Sync`
        // must themselves be Send + Sync so M5 kernels can pass them
        // across thread boundaries if needed.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<LatentView<'_>>();
        assert_send_sync::<MaskView<'_>>();
        assert_send_sync::<AnchorsView<'_>>();
        assert_send_sync::<EmbeddingView<'_>>();
    }

    #[test]
    fn latent_view_mut_sendable_across_threads() {
        // `LatentViewMut` owns a `&mut [bf16]` — its `Send` bound
        // follows from `bf16: Send`.
        fn assert_send<T: Send>() {}
        assert_send::<LatentViewMut<'_>>();
        assert_send::<MaskViewMut<'_>>();
    }

    #[test]
    fn latent_view_n_eq_zero_is_empty_slice() {
        let (arena, raw) = mk_arena_raw(4 * 4 * 2);
        let view = LatentView::from_raw(raw, 0, 4, &arena).unwrap();
        assert_eq!(view.n_agents(), 0);
        assert_eq!(view.as_flat().len(), 0);
        assert_eq!(view.iter_rows().count(), 0);
    }

    #[test]
    fn mask_view_oob_returns_false() {
        let (arena, raw) = mk_arena_raw(4);
        let view = MaskView::from_raw(raw, 4, &arena).unwrap();
        assert!(!view.is_active(4)); // equal to n_agents
        assert!(!view.is_active(u32::MAX));
    }

    #[test]
    fn latent_view_debug_prints_metadata() {
        let (arena, raw) = mk_arena_raw(4 * 4 * 2);
        let view = LatentView::from_raw(raw, 4, 4, &arena).unwrap();
        let s = format!("{view:?}");
        assert!(s.contains("n_agents"));
        assert!(s.contains("dim"));
    }
}
