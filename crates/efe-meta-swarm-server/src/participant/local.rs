//! Production participant adapter — wraps a Phase-1 [`MockParticipant`]
//! and exposes the gRPC-friendly accessors `ParticipantService` needs.
//!
//! In PR #14 this will be replaced by a sidecar-backed
//! `LocalParticipant` reading weights from disk.  Until then,
//! `MockParticipant::with_contraction(id, dim, kappa, seed)` is the
//! deterministic equivalent.
//!
//! # Lock model
//!
//! The inner `MockParticipant` holds a `Mutex<Tensor>` for its
//! state.  All accessors here serialise on that single lock — no
//! nested locks, no deadlock risk.
//!
//! The `step_index_last` and `reset_gen` counters are `AtomicU64`
//! with `Relaxed` ordering : they are *advisory* (cf. PR #9
//! architecture § 14) and never used for synchronisation.
//!
//! # Phase-2 simplification : γ and α come from the manifest
//!
//! Phase-1's `step_with_pull(pull, gamma, alpha)` takes the
//! coupling coefficients as parameters.  The wire `StepRequest`
//! does NOT carry them ; both the controller and the participant
//! read them from the same manifest's `iteration` block.  The L2
//! graduation soak protocol enforces same-manifest agreement.
//!
//! [`MockParticipant`]: efe_meta_swarm::participants::MockParticipant

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use efe_meta_swarm::participants::{MockParticipant, SwarmParticipant};
use efe_meta_swarm::prelude::Tensor;
use efe_meta_swarm::MetaSwarmError;

use crate::manifest::ParticipantManifest;

/// Production participant.  Cheap to clone (`Arc`s inside).
#[derive(Clone)]
pub struct LocalParticipant {
    inner: Arc<LocalParticipantInner>,
}

struct LocalParticipantInner {
    id: String,
    dim: usize,
    kappa: f32,
    seed: u64,
    /// Phase-1 in-process participant.  Holds `state: Mutex<Tensor>`.
    mock: MockParticipant,
    /// `pulled.step_index` from the most recent successful Step.
    /// Returned in `StateResponse.step_index_last`.  Advisory only.
    step_index_last: AtomicU64,
    /// Bumped on every successful Reset ; returned in
    /// `ResetParticipantResponse.reset_gen`.  Advisory only.
    reset_gen: AtomicU64,
}

impl LocalParticipant {
    /// Build from a [`ParticipantManifest`] row.  Returns Err if
    /// `dim == 0` or `MockParticipant::with_contraction` rejects the
    /// kappa range.
    ///
    /// # Defence in depth
    ///
    /// `manifest::validate_manifest` already rejects `dim == 0` at
    /// load time ; the extra check here lets callers use
    /// `LocalParticipant::from_manifest_row` standalone (e.g. unit
    /// tests with synthetic rows) without first running the
    /// manifest validator.  Phase-1's `MockParticipant` accepts
    /// `dim == 0` natively, which would yield a degenerate
    /// 0-element state — we reject it here to keep the production
    /// surface coherent.
    ///
    /// # Errors
    ///
    /// Returns [`MetaSwarmError::Internal`] when `dim == 0` ;
    /// bubbles up Phase-1's [`MetaSwarmError`] verbatim otherwise
    /// (kappa out of range, etc.).  The boundary layer
    /// (`bin/participant.rs`) is expected to wrap any error in a
    /// [`crate::error::ServerError::Config`].
    pub fn from_manifest_row(row: &ParticipantManifest) -> Result<Self, MetaSwarmError> {
        if row.dim == 0 {
            return Err(MetaSwarmError::Internal(format!(
                "LocalParticipant::from_manifest_row : dim must be > 0 \
                 for participant {:?} (manifest validation should have \
                 caught this)",
                row.id,
            )));
        }
        let mock = MockParticipant::with_contraction(
            row.id.clone(),
            row.dim as usize,
            row.kappa,
            row.seed,
        )?;
        Ok(Self {
            inner: Arc::new(LocalParticipantInner {
                id: row.id.clone(),
                dim: row.dim as usize,
                kappa: row.kappa,
                seed: row.seed,
                mock,
                step_index_last: AtomicU64::new(0),
                reset_gen: AtomicU64::new(0),
            }),
        })
    }

    /// Participant id (matches the manifest row).
    #[must_use]
    pub fn id(&self) -> &str {
        &self.inner.id
    }

    /// Native dimension of this participant.
    #[must_use]
    pub fn dim(&self) -> usize {
        self.inner.dim
    }

    /// Contraction constant κ.
    #[must_use]
    pub fn kappa(&self) -> f32 {
        self.inner.kappa
    }

    /// Seed for the initial state.
    #[must_use]
    pub fn seed(&self) -> u64 {
        self.inner.seed
    }

    /// Last `step_index` successfully processed.  Lock-free read.
    #[must_use]
    pub fn step_index_last(&self) -> u64 {
        self.inner.step_index_last.load(Ordering::Relaxed)
    }

    /// Current reset generation.  Lock-free read.
    #[must_use]
    pub fn reset_gen(&self) -> u64 {
        self.inner.reset_gen.load(Ordering::Relaxed)
    }

    /// Apply one coupled iteration step.  `gamma` and `alpha` come
    /// from the manifest's `iteration` block (NOT the wire — cf.
    /// module docs).
    ///
    /// # SAFETY
    ///
    /// MUST be called from `spawn_blocking` (Phase-1 is sync ; running
    /// it directly on a tokio worker would block the runtime).
    ///
    /// # Errors
    ///
    /// Phase-1 may return `DimensionMismatch` if `pulled.dim() ≠ self.dim()`.
    pub fn step(
        &self,
        pulled: &Tensor,
        gamma: f64,
        alpha: f64,
        step_index: u32,
    ) -> Result<Tensor, MetaSwarmError> {
        let new_state = self.inner.mock.step_with_pull(pulled, gamma, alpha)?;
        // Record only on success ; failed steps don't bump the counter.
        self.inner
            .step_index_last
            .store(u64::from(step_index), Ordering::Relaxed);
        Ok(new_state)
    }

    /// Read current state without mutating.  Phase-1 acquires the
    /// internal state mutex briefly.
    ///
    /// # Errors
    ///
    /// Bubbles up Phase-1's lock-poisoning error if applicable.
    pub fn current_state(&self) -> Result<Tensor, MetaSwarmError> {
        self.inner.mock.current_state()
    }

    /// Reset to the seeded initial state (if `initial` is `None`) or
    /// to the provided tensor (if `Some`).  Returns the new
    /// `reset_gen` value.
    ///
    /// # Errors
    ///
    /// Phase-1 rejects `set_state` if `initial.dim() ≠ self.dim()`.
    pub fn reset(&self, initial: Option<&Tensor>) -> Result<u64, MetaSwarmError> {
        if let Some(t) = initial {
            self.inner.mock.set_state(t.clone())?;
        } else {
            self.inner.mock.reset()?;
        }
        let new_gen = self.inner.reset_gen.fetch_add(1, Ordering::Relaxed) + 1;
        Ok(new_gen)
    }
}

impl std::fmt::Debug for LocalParticipant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalParticipant")
            .field("id", &self.inner.id)
            .field("dim", &self.inner.dim)
            .field("kappa", &self.inner.kappa)
            .field("seed", &self.inner.seed)
            .field("step_index_last", &self.step_index_last())
            .field("reset_gen", &self.reset_gen())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good_row() -> ParticipantManifest {
        ParticipantManifest {
            id: "p-1".into(),
            dim: 8,
            kappa: 0.6,
            seed: 11,
            blas_backend: "none".into(),
        }
    }

    #[test]
    fn assert_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<LocalParticipant>();
    }

    #[test]
    fn from_manifest_row_happy_path() {
        let lp = LocalParticipant::from_manifest_row(&good_row()).expect("build");
        assert_eq!(lp.id(), "p-1");
        assert_eq!(lp.dim(), 8);
        assert!((lp.kappa() - 0.6).abs() < 1e-6);
        assert_eq!(lp.seed(), 11);
        assert_eq!(lp.step_index_last(), 0);
        assert_eq!(lp.reset_gen(), 0);
    }

    #[test]
    fn from_manifest_row_rejects_kappa_out_of_range() {
        let mut row = good_row();
        row.kappa = 1.5;
        let err = LocalParticipant::from_manifest_row(&row).unwrap_err();
        assert!(matches!(err, MetaSwarmError::Internal(_)));
    }

    #[test]
    fn from_manifest_row_rejects_zero_dim() {
        // Defence in depth : `manifest::validate_manifest` already
        // rejects dim == 0, but `LocalParticipant::from_manifest_row`
        // must be safe to call on a raw row.
        let mut row = good_row();
        row.dim = 0;
        let err = LocalParticipant::from_manifest_row(&row).unwrap_err();
        assert!(matches!(err, MetaSwarmError::Internal(ref m) if m.contains("dim must be > 0")));
    }

    #[test]
    fn step_bumps_step_index_last_on_success() {
        let lp = LocalParticipant::from_manifest_row(&good_row()).unwrap();
        let pull = Tensor::from_slice(&[1.0_f32; 8]);
        let _ = lp.step(&pull, 0.2, 0.5, 7).expect("step");
        assert_eq!(lp.step_index_last(), 7);
        let _ = lp.step(&pull, 0.2, 0.5, 42).expect("step");
        assert_eq!(lp.step_index_last(), 42);
    }

    #[test]
    fn step_failure_does_not_bump_step_index() {
        let lp = LocalParticipant::from_manifest_row(&good_row()).unwrap();
        // Wrong dim → Phase-1 returns DimensionMismatch.
        let pull = Tensor::from_slice(&[1.0_f32; 4]);
        let err = lp.step(&pull, 0.2, 0.5, 99).unwrap_err();
        assert!(matches!(err, MetaSwarmError::DimensionMismatch { .. }));
        assert_eq!(lp.step_index_last(), 0);
    }

    #[test]
    fn reset_default_bumps_reset_gen() {
        let lp = LocalParticipant::from_manifest_row(&good_row()).unwrap();
        assert_eq!(lp.reset(None).unwrap(), 1);
        assert_eq!(lp.reset(None).unwrap(), 2);
        assert_eq!(lp.reset_gen(), 2);
    }

    #[test]
    fn reset_with_initial_validates_dim() {
        let lp = LocalParticipant::from_manifest_row(&good_row()).unwrap();
        let bad = Tensor::from_slice(&[0.0_f32; 4]); // expected 8
        let err = lp.reset(Some(&bad)).unwrap_err();
        assert!(matches!(err, MetaSwarmError::DimensionMismatch { .. }));
        // reset_gen unchanged on error.
        assert_eq!(lp.reset_gen(), 0);
    }

    #[test]
    fn reset_with_initial_succeeds_on_matching_dim() {
        let lp = LocalParticipant::from_manifest_row(&good_row()).unwrap();
        let t = Tensor::from_slice(&[0.25_f32; 8]);
        assert_eq!(lp.reset(Some(&t)).unwrap(), 1);
        let state = lp.current_state().unwrap();
        assert_eq!(state.dim(), 8);
        assert!((state.as_slice()[0] - 0.25).abs() < 1e-6);
    }

    #[test]
    fn current_state_returns_seeded_value_at_start() {
        let lp = LocalParticipant::from_manifest_row(&good_row()).unwrap();
        let state = lp.current_state().unwrap();
        assert_eq!(state.dim(), 8);
    }

    #[test]
    fn clones_share_state() {
        // Two clones see the same underlying state — they share an Arc.
        let lp = LocalParticipant::from_manifest_row(&good_row()).unwrap();
        let clone = lp.clone();
        let _ = lp
            .step(&Tensor::from_slice(&[1.0_f32; 8]), 0.2, 0.5, 5)
            .unwrap();
        assert_eq!(clone.step_index_last(), 5);
    }
}
