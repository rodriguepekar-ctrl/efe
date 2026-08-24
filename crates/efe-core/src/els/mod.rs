//! Epistemic Latent Space (M4).
//!
//! M4 is the typed-access layer over the raw allocations produced by
//! M3's spawner. It exposes borrowed views (`LatentView`, `MaskView`,
//! `AnchorsView`, `EmbeddingView`), population statistics, soft
//! projection onto the alignment cone, and a lightweight snapshot for
//! M11 telemetry.
//!
//! # Safety model
//!
//! See [`views`]. All accessors take `&Arena` and validate the raw
//! allocation's generation before dereferencing; the returned views
//! carry the arena's lifetime, so the borrow checker prevents use
//! after the arena is reset or dropped.

pub mod metrics;
pub mod projection;
pub mod snapshot;
pub mod stats;
pub mod views;

pub use metrics::{
    cosine_similarity, dot_product, l2_distance, l2_distance_mixed, l2_distance_sq_mixed, l2_norm,
};
pub use projection::{project_onto_anchors, project_vector_onto_anchors};
pub use snapshot::{fnv1a_hash, ElsSnapshot};
pub use stats::{compute_stats, compute_stats_inner, ElsStats};
pub use views::{AnchorsView, EmbeddingView, LatentView, LatentViewMut, MaskView, MaskViewMut};

use crate::errors::Result;
use crate::memory::{Arena, RawAllocation};
use crate::types::{LatentDim, SwarmId};

/// Shared latent space for one swarm.
///
/// The struct owns metadata and opaque `RawAllocation` handles into an
/// arena held by the parent [`crate::memory::NanoAgentBatch`]. View
/// accessors take `&Arena` to bind their lifetime to the arena's.
///
/// # M5 double-buffering
///
/// The optional `scratch_latent` slot enables M5's fusion kernel to
/// ping-pong between two allocations without calling `alloc_zeroed_raw`
/// every round (which would leak the old latent bytes into the arena on
/// each round, saturating after ~N/slot_count rounds). Lazy-initialized
/// on first use via `ensure_scratch_latent` (crate-private); swapped
/// with the "current" latent via `swap_latent_with_scratch` (also
/// crate-private). See `fusion::mvp_cpu` for the orchestration.
#[derive(Debug)]
pub struct EpistemicLatentSpace {
    swarm_id: SwarmId,
    dim: LatentDim,
    agent_count: u32,
    anchor_count: u32,
    /// Cached `AlignmentCone::max_projection_distance` from the spec that
    /// spawned this ELS. Used by M8 (`dissolution`) to gate the final
    /// extraction with strict breach detection. Zero on a stub ELS.
    alignment_cone_radius: f32,
    latent: Option<RawAllocation>,
    mask: Option<RawAllocation>,
    anchors: Option<RawAllocation>,
    problem_emb: Option<RawAllocation>,
    /// Secondary latent allocation for M5 double-buffering. Allocated
    /// lazily on first fusion round; persists across rounds, re-used
    /// via pointer swap.
    scratch_latent: Option<RawAllocation>,
    /// **Couche 3 (M1.c)** — per-execute carrier for the fused consensus
    /// centroid, used by the meta-swarm *latent* controller to make `Φ`
    /// (`SwarmController::phi_once`) pure **by construction** rather than
    /// by single-flight discipline.
    ///
    /// The ELS is created fresh by genesis on every `execute()` and flows
    /// (by `&mut`) through fusion and (by `&`) into dissolution *within a
    /// single call stack*, so it is the only per-request object both the
    /// fusion strategy and the [`crate::swarm::strategies::Dissolver`]
    /// share. Threading the centroid here — the fusion round (which holds
    /// the arena) writes it, an ELS-reading dissolver takes it — removes
    /// the controller-wide `Mutex<Option<Vec<f32>>>` publication slot that
    /// two concurrent `phi_once` calls on a *shared* controller would race
    /// on (B's publish overwriting A's slot before A's take).
    ///
    /// **Left `None` on the efe-server Mode-1 request path**, which keeps
    /// its `PassThroughDissolver` + shared-slot wiring untouched (the
    /// "byte-identical server binary" invariant). Only the latent
    /// controller (`efe_server::mode1::build_mvp_controller`) populates and
    /// reads it. Not part of any snapshot / hash (see [`snapshot`]), so an
    /// unpopulated slot is invisible to telemetry and reproducibility pins.
    dissolution_centroid: Option<Vec<f32>>,
}

impl EpistemicLatentSpace {
    /// Construct a minimal stub without any arena-backed state.
    #[must_use]
    pub fn new_stub(swarm_id: SwarmId, dim: LatentDim, agent_count: u32) -> Self {
        Self {
            swarm_id,
            dim,
            agent_count,
            anchor_count: 0,
            alignment_cone_radius: 0.0,
            latent: None,
            mask: None,
            anchors: None,
            problem_emb: None,
            scratch_latent: None,
            dissolution_centroid: None,
        }
    }

    /// Construct a populated space backed by an arena's raw allocations.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new_populated(
        swarm_id: SwarmId,
        dim: LatentDim,
        agent_count: u32,
        latent: RawAllocation,
        mask: RawAllocation,
        anchors: RawAllocation,
        problem_emb: RawAllocation,
        anchor_count: u32,
    ) -> Self {
        Self::new_populated_with_cone(
            swarm_id,
            dim,
            agent_count,
            latent,
            mask,
            anchors,
            problem_emb,
            anchor_count,
            0.0,
        )
    }

    /// Like [`Self::new_populated`] but takes the alignment-cone radius
    /// (`AlignmentCone::max_projection_distance`) so M8 can gate
    /// extraction with strict breach detection.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new_populated_with_cone(
        swarm_id: SwarmId,
        dim: LatentDim,
        agent_count: u32,
        latent: RawAllocation,
        mask: RawAllocation,
        anchors: RawAllocation,
        problem_emb: RawAllocation,
        anchor_count: u32,
        alignment_cone_radius: f32,
    ) -> Self {
        Self {
            swarm_id,
            dim,
            agent_count,
            anchor_count,
            alignment_cone_radius,
            latent: Some(latent),
            mask: Some(mask),
            anchors: Some(anchors),
            problem_emb: Some(problem_emb),
            scratch_latent: None,
            dissolution_centroid: None,
        }
    }

    // ---------- Metadata ----------

    /// Swarm identifier.
    #[must_use]
    pub fn swarm_id(&self) -> SwarmId {
        self.swarm_id
    }

    /// Latent space dimension.
    #[must_use]
    pub fn dim(&self) -> LatentDim {
        self.dim
    }

    /// Total declared number of agents.
    #[must_use]
    pub fn agent_count(&self) -> u32 {
        self.agent_count
    }

    /// Number of alignment anchors.
    #[must_use]
    pub fn anchor_count(&self) -> u32 {
        self.anchor_count
    }

    /// Radius of the alignment cone (max projection distance tolerated
    /// by M8). Zero on stub ELSs.
    #[must_use]
    pub fn alignment_cone_radius(&self) -> f32 {
        self.alignment_cone_radius
    }

    /// `true` iff all four allocations are populated.
    #[must_use]
    pub fn is_populated(&self) -> bool {
        self.latent.is_some()
            && self.mask.is_some()
            && self.anchors.is_some()
            && self.problem_emb.is_some()
    }

    // ---------- Raw allocation accessors (legacy: used by M5 kernels) ----------

    /// Raw latent tensor allocation, if populated.
    #[must_use]
    pub fn latent_raw(&self) -> Option<RawAllocation> {
        self.latent
    }

    /// Raw active-mask allocation, if populated.
    #[must_use]
    pub fn mask_raw(&self) -> Option<RawAllocation> {
        self.mask
    }

    /// Raw anchors allocation, if populated.
    #[must_use]
    pub fn anchors_raw(&self) -> Option<RawAllocation> {
        self.anchors
    }

    /// Raw problem-embedding allocation, if populated.
    #[must_use]
    pub fn problem_embedding_raw(&self) -> Option<RawAllocation> {
        self.problem_emb
    }

    // ---------- Dissolution-centroid carrier (Couche 3 M1.c) ----------

    /// Stash the fused consensus centroid on this (per-execute) ELS.
    ///
    /// Called by the fusion strategy — which holds the arena needed to
    /// read the latent/mask views — at the end of a round on the latent
    /// meta-swarm path. Overwrites any previously stashed value so the
    /// last round to run wins, matching the legacy "last round wins"
    /// dissolution semantics but scoped to *this* ELS instead of a
    /// controller-wide slot. See the field docstring for the concurrency
    /// rationale ([`dissolution_centroid`](Self::dissolution_centroid)).
    pub fn set_dissolution_centroid(&mut self, centroid: Vec<f32>) {
        self.dissolution_centroid = Some(centroid);
    }

    /// The centroid stashed by [`Self::set_dissolution_centroid`], if any.
    ///
    /// Read by the latent controller's ELS-carrying dissolver. Returns
    /// `None` on the server request path (which never stashes) and until
    /// the first fusion round has run.
    #[must_use]
    pub fn dissolution_centroid(&self) -> Option<&[f32]> {
        self.dissolution_centroid.as_deref()
    }

    // ---------- View accessors ----------

    /// Immutable view over the latent tensor.
    ///
    /// Returns `Ok(None)` if the ELS is a stub (no latent populated),
    /// `Ok(Some(..))` for a normal ELS, and `Err(_)` on generation
    /// mismatch.
    ///
    /// # Errors
    ///
    /// Propagates [`RawAllocation::validate_against`] failures.
    pub fn latent_view<'a>(&self, arena: &'a Arena) -> Result<Option<LatentView<'a>>> {
        self.latent
            .map(|raw| LatentView::from_raw(raw, self.agent_count, self.dim.get(), arena))
            .transpose()
    }

    /// Mutable view over the latent tensor (exclusive borrow of `self`).
    ///
    /// # Errors
    ///
    /// Same as [`Self::latent_view`].
    pub fn latent_view_mut<'a>(&mut self, arena: &'a Arena) -> Result<Option<LatentViewMut<'a>>> {
        let n = self.agent_count;
        let d = self.dim.get();
        self.latent
            .map(|raw| LatentViewMut::from_raw(raw, n, d, arena))
            .transpose()
    }

    /// Immutable view over the active mask.
    ///
    /// # Errors
    ///
    /// Propagates [`RawAllocation::validate_against`] failures.
    pub fn mask_view<'a>(&self, arena: &'a Arena) -> Result<Option<MaskView<'a>>> {
        self.mask
            .map(|raw| MaskView::from_raw(raw, self.agent_count, arena))
            .transpose()
    }

    /// Mutable view over the active mask (exclusive borrow).
    ///
    /// # Errors
    ///
    /// Same as [`Self::mask_view`].
    pub fn mask_view_mut<'a>(&mut self, arena: &'a Arena) -> Result<Option<MaskViewMut<'a>>> {
        let n = self.agent_count;
        self.mask
            .map(|raw| MaskViewMut::from_raw(raw, n, arena))
            .transpose()
    }

    /// Immutable view over the alignment anchors.
    ///
    /// # Errors
    ///
    /// Propagates [`RawAllocation::validate_against`] failures.
    pub fn anchors_view<'a>(&self, arena: &'a Arena) -> Result<Option<AnchorsView<'a>>> {
        self.anchors
            .map(|raw| AnchorsView::from_raw(raw, self.anchor_count, self.dim.get(), arena))
            .transpose()
    }

    /// Immutable view over the problem embedding.
    ///
    /// # Errors
    ///
    /// Propagates [`RawAllocation::validate_against`] failures.
    pub fn problem_embedding_view<'a>(
        &self,
        arena: &'a Arena,
    ) -> Result<Option<EmbeddingView<'a>>> {
        self.problem_emb
            .map(|raw| EmbeddingView::from_raw(raw, self.dim.get(), arena))
            .transpose()
    }

    /// Ensure a scratch latent allocation exists and return it.
    ///
    /// Lazy-initialization: if `scratch_latent` is `None`, allocate one
    /// of the same byte size as the main latent (N * D * 2 for bf16).
    /// Subsequent calls return the cached allocation without reallocating,
    /// provided the arena is still alive (same generation).
    ///
    /// This is the backbone of M5's double-buffering strategy — the
    /// fusion kernel aggregates into the scratch, then swaps pointers
    /// with the main latent (see [`Self::swap_latent_with_scratch`]).
    ///
    /// # Errors
    ///
    /// - [`crate::EfeError::Internal`] if the ELS is a stub.
    /// - Propagates [`Arena::alloc_zeroed_raw`] failures.
    pub(crate) fn ensure_scratch_latent(&mut self, arena: &Arena) -> Result<RawAllocation> {
        if self.latent.is_none() {
            return Err(crate::EfeError::Internal(
                "ensure_scratch_latent: ELS is a stub",
            ));
        }
        // If we have a scratch and it's still valid (generation matches),
        // reuse it. Otherwise (None or stale), allocate fresh.
        if let Some(s) = self.scratch_latent {
            if s.validate_against(arena).is_ok() {
                return Ok(s);
            }
        }
        let size_bytes = u64::from(self.agent_count) * u64::from(self.dim.get()) * 2;
        let fresh = arena.alloc_zeroed_raw(size_bytes)?;
        self.scratch_latent = Some(fresh);
        Ok(fresh)
    }

    /// Swap the latent allocation with the scratch allocation.
    ///
    /// After this call:
    /// - `self.latent`  ← the previous scratch (just aggregated into).
    /// - `self.scratch` ← the previous latent (stale bytes, to be overwritten next round).
    ///
    /// The arena reclaims nothing — both allocations persist until the
    /// arena is reset — but M5 no longer needs to allocate a new scratch
    /// per round, so the total footprint is bounded at 2×(N*D*2) bytes
    /// independently of the number of rounds.
    ///
    /// # Errors
    ///
    /// - [`crate::EfeError::Internal`] if either slot is `None` (should
    ///   not happen if [`Self::ensure_scratch_latent`] was called first).
    pub(crate) fn swap_latent_with_scratch(&mut self) -> Result<()> {
        let (Some(latent), Some(scratch)) = (self.latent, self.scratch_latent) else {
            return Err(crate::EfeError::Internal(
                "swap_latent_with_scratch: latent or scratch missing",
            ));
        };
        self.latent = Some(scratch);
        self.scratch_latent = Some(latent);
        Ok(())
    }

    /// Split-borrow all four views, with `latent` mutable.
    ///
    /// This is the preferred entry point for M5's fusion kernels, which
    /// need simultaneous write access to the latent and read access to
    /// the mask / anchors / problem embedding.
    ///
    /// # Errors
    ///
    /// - [`crate::EfeError::Internal`] if the ELS is not fully populated.
    /// - Generation mismatch on any allocation.
    pub fn split_mut<'a>(&mut self, arena: &'a Arena) -> Result<ElsSplitMut<'a>> {
        let latent_raw = self
            .latent
            .ok_or(crate::EfeError::Internal("split_mut: ELS missing latent"))?;
        let mask_raw = self
            .mask
            .ok_or(crate::EfeError::Internal("split_mut: ELS missing mask"))?;
        let anchors_raw = self
            .anchors
            .ok_or(crate::EfeError::Internal("split_mut: ELS missing anchors"))?;
        let problem_raw = self.problem_emb.ok_or(crate::EfeError::Internal(
            "split_mut: ELS missing problem_embedding",
        ))?;

        let latent = LatentViewMut::from_raw(latent_raw, self.agent_count, self.dim.get(), arena)?;
        let mask = MaskView::from_raw(mask_raw, self.agent_count, arena)?;
        let anchors = AnchorsView::from_raw(anchors_raw, self.anchor_count, self.dim.get(), arena)?;
        let problem_embedding = EmbeddingView::from_raw(problem_raw, self.dim.get(), arena)?;
        Ok(ElsSplitMut {
            latent,
            mask,
            anchors,
            problem_embedding,
        })
    }
}

/// Bundle of views returned by [`EpistemicLatentSpace::split_mut`].
///
/// The struct is the anchor of Rust's disjoint-field borrow: the
/// caller can use `latent` mutably while reading `mask`, `anchors`,
/// and `problem_embedding` concurrently.
pub struct ElsSplitMut<'a> {
    /// Mutable view over the latent tensor.
    pub latent: LatentViewMut<'a>,
    /// Read-only view over the active mask.
    pub mask: MaskView<'a>,
    /// Read-only view over the alignment anchors.
    pub anchors: AnchorsView<'a>,
    /// Read-only view over the problem embedding.
    pub problem_embedding: EmbeddingView<'a>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{DeviceBackend, HostBackend};
    use std::sync::Arc;

    #[test]
    fn stub_els_records_basic_fields() {
        let dim = LatentDim::new(128).expect("valid");
        let els = EpistemicLatentSpace::new_stub(SwarmId::new(), dim, 256);
        assert_eq!(els.dim().get(), 128);
        assert_eq!(els.agent_count(), 256);
        assert!(!els.is_populated());
        assert!(els.latent_raw().is_none());
    }

    #[test]
    fn stub_views_return_none() {
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let arena = Arena::new(1 << 14, backend, 0).unwrap();
        let dim = LatentDim::new(8).unwrap();
        let els = EpistemicLatentSpace::new_stub(SwarmId::new(), dim, 4);
        assert!(els.latent_view(&arena).unwrap().is_none());
        assert!(els.mask_view(&arena).unwrap().is_none());
        assert!(els.anchors_view(&arena).unwrap().is_none());
        assert!(els.problem_embedding_view(&arena).unwrap().is_none());
    }

    #[test]
    fn split_mut_on_stub_returns_internal_error() {
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let arena = Arena::new(1 << 14, backend, 0).unwrap();
        let dim = LatentDim::new(8).unwrap();
        let mut els = EpistemicLatentSpace::new_stub(SwarmId::new(), dim, 4);
        // ElsSplitMut doesn't derive Debug (views over slices can't
        // derive it cheaply), so unwrap_err isn't available here —
        // assert via explicit matching.
        match els.split_mut(&arena) {
            Err(crate::EfeError::Internal(_)) => {}
            other => panic!("expected Internal error, got: {:?}", other.err()),
        }
    }
}
