//! Shared participant state.
//!
//! # Lock hierarchy
//!
//! Only one lock in play : the `Mutex<Tensor>` inside the wrapped
//! Phase-1 `MockParticipant` (accessed via [`LocalParticipant`]).  All
//! accessors hold it briefly during the sync Phase-1 call.  No nested
//! locks ; no deadlock risk.
//!
//! # PR #12 atomic `IterationParams` swap
//!
//! `iteration: Arc<ArcSwap<IterationParams>>` is the swap surface for
//! the participant-side reload (architecture § 12).  Readers (the
//! `Step` handler) call `iteration.load()` which is **lock-free** ;
//! the swap path calls `iteration_swap(new)` which is a single atomic
//! store.  A reader that already captured an `Arc<IterationParams>`
//! snapshot keeps it valid for the duration of the call — identical
//! correctness guarantee to PR #8's `Arc<RwLock<Manifest>>` +
//! `.clone()` pattern on the controller side, but lock-free.
//!
//! Documented in `EFE_A1_PHASE2_PR09_ARCHITECTURE.md` § 9 / § 17 and
//! `EFE_A1_PHASE2_PR12_ARCHITECTURE.md` § 12.

use std::sync::Arc;
use std::time::Instant;

use arc_swap::ArcSwap;
use efe_meta_swarm_proto::meta_swarm::ParticipantDescriptor;

use crate::config::participant::ParticipantConfig;
use crate::observability::health::HealthState;
use crate::observability::metrics::MetricsRegistry;
use crate::participant::local::LocalParticipant;

/// Iteration knobs the participant applies inside `step_with_pull`.
///
/// Snapshot at boot from the manifest's `iteration` block ; PR #12
/// swaps it atomically on SIGHUP via the surrounding `ArcSwap`.
#[derive(Debug, Clone)]
pub struct IterationParams {
    pub alpha_default: f64,
    pub coupling_gamma: f64,
}

/// Shared participant state.  Cheap to clone (`Arc`s inside).
#[derive(Clone)]
pub struct ParticipantState {
    pub config: Arc<ParticipantConfig>,
    pub health: HealthState,
    pub metrics: MetricsRegistry,
    pub start_time: Instant,
    /// Phase-1 in-process participant.  Holds the mutable tensor state.
    pub local: LocalParticipant,
    /// `iteration` block snapshot from the manifest.  Read on every
    /// `Step` to retrieve γ and α.  PR #12 swaps it via
    /// [`Self::iteration_swap`] on SIGHUP-driven reload.
    pub iteration: Arc<ArcSwap<IterationParams>>,
}

impl std::fmt::Debug for ParticipantState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let iter_snap = self.iteration.load();
        f.debug_struct("ParticipantState")
            .field("config", &self.config)
            .field("local", &self.local)
            .field("iteration", &*iter_snap)
            .field("uptime_s", &self.start_time.elapsed().as_secs())
            .finish_non_exhaustive()
    }
}

impl ParticipantState {
    /// Build a participant state from a fully-resolved config + a
    /// pre-built [`LocalParticipant`] + the iteration knobs.
    ///
    /// `bin/participant.rs` is the only production caller ; tests
    /// use [`Self::new_for_tests`] for convenience.
    #[must_use]
    pub fn new(
        config: ParticipantConfig,
        local: LocalParticipant,
        iteration: IterationParams,
    ) -> Self {
        Self {
            config: Arc::new(config),
            health: HealthState::serving(),
            metrics: MetricsRegistry::new(),
            start_time: Instant::now(),
            local,
            iteration: Arc::new(ArcSwap::from_pointee(iteration)),
        }
    }

    /// Replace the iteration knobs atomically.  Called by the
    /// crate-private `participant::reload::apply_reload` (which lives
    /// alongside this module) under the SIGHUP handler.  Single atomic
    /// store ; lock-free ; in-flight Step handlers that already called
    /// `iteration.load()` keep their snapshot for the duration of the
    /// call (architecture § 12.6).
    pub fn iteration_swap(&self, new: IterationParams) {
        self.iteration.store(Arc::new(new));
    }

    /// Convenience accessor : load a cheap snapshot of the current
    /// iteration knobs.  Identical to `state.iteration.load()` but
    /// returns the `Arc<IterationParams>` directly (the `Guard`
    /// from `ArcSwap::load` is harder to pass across `.await`).
    #[must_use]
    pub fn iteration_snapshot(&self) -> Arc<IterationParams> {
        // `load_full` clones the inner Arc ; cheap (refcount bump).
        self.iteration.load_full()
    }

    /// Build a minimal state for tests : a single 8-dim participant
    /// with κ = 0.6, seed = 11, and the default `IterationParams`.
    ///
    /// **Never** used in production — the descriptor's `id` matches
    /// `config.id` rather than `local.id()` to keep the test
    /// helpers ergonomic.
    #[doc(hidden)]
    #[must_use]
    pub fn new_for_tests(config: ParticipantConfig) -> Self {
        use crate::manifest::ParticipantManifest;
        let row = ParticipantManifest {
            id: config.id.clone(),
            dim: 8,
            kappa: Some(0.6),
            seed: Some(11),
            blas_backend: "none".into(),
            controller: None,
        };
        let local = LocalParticipant::from_manifest_row(&row).expect("test LocalParticipant build");
        let iteration = IterationParams {
            alpha_default: 0.5,
            coupling_gamma: 0.2,
        };
        Self::new(config, local, iteration)
    }

    /// Test-only constructor that lets the caller pass arbitrary
    /// iteration knobs.  Used by `participant::reload::tests`.
    #[doc(hidden)]
    #[must_use]
    pub fn new_for_tests_with_iteration(
        config: ParticipantConfig,
        iteration: IterationParams,
    ) -> Self {
        use crate::manifest::ParticipantManifest;
        let row = ParticipantManifest {
            id: config.id.clone(),
            dim: 8,
            kappa: Some(0.6),
            seed: Some(11),
            blas_backend: "none".into(),
            controller: None,
        };
        let local = LocalParticipant::from_manifest_row(&row).expect("test LocalParticipant build");
        Self::new(config, local, iteration)
    }

    /// Pre-built descriptor.  PR #9 fills in real values from the
    /// manifest row (was placeholders dim/kappa/seed = 0 in PR #6).
    ///
    /// **Couche 3 M1.d — honest wire.**  For an efe-core participant,
    /// `kappa`/`seed` are mock-only concepts : the wire carries the sentinels
    /// `0.0`/`0` (frozen field numbers — deprecated, never removed) and the
    /// engine is identified by `config_hash` (SHA-256 of the manifest
    /// `controller` block) plus the `engine:efe-core-mode1` feature token.
    /// The `determinism` claim is kept for BOTH engines : the mock is
    /// deterministic by construction, and the efe-core latent controller runs
    /// the M1.b zero-noise Φ (bit-reproducible, seed-invariant).
    pub fn descriptor(&self) -> ParticipantDescriptor {
        let mut features = vec!["determinism".into(), "step_index".into()];
        if self.local.is_efe_core() {
            features.push(crate::participant::local::ENGINE_FEATURE_EFE_CORE.into());
        }
        ParticipantDescriptor {
            id: self.local.id().to_string(),
            dim: u32::try_from(self.local.dim()).unwrap_or(u32::MAX),
            kappa: self.local.kappa(),
            seed: self.local.seed(),
            build_sha: crate::GIT_SHA.into(),
            blas_backend: "none".into(),
            features,
            config_hash: self.local.config_hash().map(str::to_string),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::participant::ParticipantConfig;

    fn test_config() -> ParticipantConfig {
        ParticipantConfig {
            id: "test-participant".into(),
            grpc_bind: "127.0.0.1:0".parse().unwrap(),
            metrics_bind: "127.0.0.1:0".parse().unwrap(),
            worker_threads: 1,
            tls_required: false,
            tls_cert_path: None,
            tls_key_path: None,
            tls_ca_path: None,
            // PR #16 — TLS knobs off in state unit tests.
            tls_mutual: false,
            tls_min_protocol: crate::tls::ProtocolVersion::Tls13,
            tls_allowed_peer_sans: Vec::new(),
            tls_peer_san: None,
        }
    }

    #[test]
    fn descriptor_uses_local_participant_values() {
        let state = ParticipantState::new_for_tests(test_config());
        let d = state.descriptor();
        assert_eq!(d.id, "test-participant");
        assert_eq!(d.dim, 8);
        assert!((d.kappa - 0.6).abs() < 1e-6);
        assert_eq!(d.seed, 11);
        assert_eq!(d.blas_backend, "none");
        assert!(d.features.iter().any(|f| f == "determinism"));
        assert!(d.features.iter().any(|f| f == "step_index"));
    }

    #[test]
    fn iteration_params_default_values() {
        let state = ParticipantState::new_for_tests(test_config());
        // PR #12 : `iteration` is now `Arc<ArcSwap<IterationParams>>`.
        // `load()` is lock-free and returns a Guard ; deref it to
        // inspect the fields.
        let snap = state.iteration.load();
        assert!((snap.alpha_default - 0.5).abs() < 1e-9);
        assert!((snap.coupling_gamma - 0.2).abs() < 1e-9);
    }

    #[test]
    fn iteration_swap_replaces_params_atomically() {
        let state = ParticipantState::new_for_tests(test_config());
        // Pre-swap snapshot (would represent an in-flight Step reader).
        let pre = state.iteration_snapshot();
        assert!((pre.alpha_default - 0.5).abs() < 1e-9);

        state.iteration_swap(IterationParams {
            alpha_default: 0.9,
            coupling_gamma: 0.05,
        });

        // Post-swap snapshot reflects the new values.
        let post = state.iteration_snapshot();
        assert!((post.alpha_default - 0.9).abs() < 1e-9);
        assert!((post.coupling_gamma - 0.05).abs() < 1e-9);

        // Pre-swap snapshot is **still** the old values (architecture
        // § 12.6 in-flight reader correctness).
        assert!((pre.alpha_default - 0.5).abs() < 1e-9);
        assert!((pre.coupling_gamma - 0.2).abs() < 1e-9);
    }

    #[test]
    fn iteration_snapshot_returns_independent_arc() {
        let state = ParticipantState::new_for_tests(test_config());
        let snap_a = state.iteration_snapshot();
        let snap_b = state.iteration_snapshot();
        // Same underlying contents.
        assert!((snap_a.alpha_default - snap_b.alpha_default).abs() < 1e-12);
        // But after a swap, A and B both observe the OLD values.
        state.iteration_swap(IterationParams {
            alpha_default: 0.123,
            coupling_gamma: 0.456,
        });
        assert!((snap_a.alpha_default - 0.5).abs() < 1e-9);
        assert!((snap_b.alpha_default - 0.5).abs() < 1e-9);
        let snap_c = state.iteration_snapshot();
        assert!((snap_c.alpha_default - 0.123).abs() < 1e-9);
    }
}
