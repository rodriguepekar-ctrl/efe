//! Shared controller state.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use efe_meta_swarm_proto::meta_swarm::{InfoResponse, ParticipantDescriptor};
use tokio::sync::RwLock;

use crate::config::controller::ControllerConfig;
use crate::controller::inflight::InflightSemaphore;
use crate::manifest::{Manifest, ParticipantManifest};
use crate::observability::health::HealthState;
use crate::observability::metrics::MetricsRegistry;
use crate::tls::TlsReloader;

/// Shared controller state.  Cheap to clone (Arcs inside).
///
/// # Lock hierarchy
///
/// Two concurrent paths share these locks ; deadlock freedom requires
/// acquiring them in this strict order (updated for PR #12) :
///
///   1. [`InflightSemaphore::try_acquire`] — controller-level,
///      bounded ; Iterate / `StepOnce` / Reset acquire one permit at
///      handler entry.
///   2. [`Self::manifest`] **read** (Iterate / `StepOnce` / Reset
///      path : `state.manifest.read().await.clone()` then drop the
///      guard) **OR** [`Self::manifest`] **write** (reload path :
///      `state.manifest.write().await` for the atomic swap).
///   3. Per-participant client mutex — inside `RemoteParticipant`'s
///      tonic client wrapper.
///
/// The reload path (`controller::reload::apply_reload`) acquires the
/// `manifest.write()` lock **while holding nothing else** :
///
///   * it does NOT take an inflight permit (reload is independent
///     of the throttle ; operators must always be able to reload
///     even when `MAX_INFLIGHT` is saturated) ;
///   * it does NOT hold any participant mutex (participants are
///     stateless from the controller's perspective during reload).
///
/// In-flight Iterates that hold a *cloned* `Arc<Manifest>` snapshot
/// (PR #8 pattern) are unaffected by the write-lock swap : their
/// snapshot's lifetime extends past the lock release.  This is the
/// load-bearing **T20 mitigation** (cf.
/// `EFE_A1_PHASE2_PR12_ARCHITECTURE.md` § 5.2 + § 18).
///
/// Original PR #8 design : `EFE_A1_PHASE2_PR08_ARCHITECTURE.md` § 17.
/// PR #12 update : `EFE_A1_PHASE2_PR12_ARCHITECTURE.md` § 18.
#[derive(Clone)]
pub struct ControllerState {
    pub config: Arc<ControllerConfig>,
    pub health: HealthState,
    pub metrics: MetricsRegistry,
    pub start_time: Instant,
    /// Loaded manifest.  PR #8 loads at startup ; PR #12 swaps on
    /// SIGHUP via a write-lock.
    pub manifest: Arc<RwLock<Manifest>>,
    /// Concurrent Iterate gate (T7 mitigation).
    pub inflight: InflightSemaphore,
    /// PR #16 — TLS reload coordinator.  `None` when
    /// `tls_required=false` (the default) ; `Some` once
    /// `bin/controller.rs::bootstrap_tls` finishes.  Carries the
    /// current [`crate::tls::TlsClientConfig`] used to dial each
    /// participant via mTLS.  See `controller/iterate.rs::
    /// connect_participants` for the consumer.
    pub tls_reloader: Option<Arc<TlsReloader>>,
    /// Couche 3 M1.d — **lifetime engine-identity lineage**.  Maps every
    /// participant id the controller has EVER served → the row it was first
    /// seen with.  Consulted on reload by
    /// [`crate::manifest::validate_engine_lineage`] so a *removed-then-re-added*
    /// id cannot swap its engine identity across two reloads — the immutability
    /// invariant the per-manifest drift check alone cannot uphold (it only sees
    /// ids present in both manifests).  Populated at boot, monotonically
    /// extended after each successful reload (first identity per id wins ; a
    /// process restart resets it, which is the sanctioned way to change an
    /// engine).
    pub engine_lineage: Arc<Mutex<HashMap<String, ParticipantManifest>>>,
}

impl std::fmt::Debug for ControllerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControllerState")
            .field("config", &self.config)
            .field("uptime_s", &self.start_time.elapsed().as_secs())
            .finish_non_exhaustive()
    }
}

impl ControllerState {
    /// Construct a controller state from a config + a pre-loaded
    /// manifest.  `bin/controller.rs` calls this after loading the
    /// manifest from disk.
    #[must_use]
    pub fn new(config: ControllerConfig, manifest: Manifest) -> Self {
        let max_inflight = config.max_inflight;
        let metrics = MetricsRegistry::new();
        // PR #17 — pre-touch every per-participant label combination so
        // /metrics emits the per-id series at value `0` from boot
        // (AC6 / § 15).  Dashboards bound to e.g.
        // `step_total{participant_id="supplier-scoring"}` render with
        // `0` rather than "no data" before any traffic.
        metrics.pretouch_for_manifest(&manifest);
        // Seed the engine lineage with every boot-time participant identity.
        let engine_lineage: HashMap<String, ParticipantManifest> = manifest
            .participants
            .iter()
            .map(|p| (p.id.clone(), p.clone()))
            .collect();
        Self {
            config: Arc::new(config),
            health: HealthState::serving(),
            metrics,
            start_time: Instant::now(),
            manifest: Arc::new(RwLock::new(manifest)),
            inflight: InflightSemaphore::new(max_inflight),
            tls_reloader: None,
            engine_lineage: Arc::new(Mutex::new(engine_lineage)),
        }
    }

    /// PR #16 — attach a TLS reloader after construction.
    ///
    /// Called from `bin/controller.rs::async_main` once
    /// [`crate::tls::TlsReloader::bootstrap`] has completed
    /// successfully.  After this call, `connect_participants` will
    /// dial each participant over mTLS (using the reloader's
    /// `current_client_config()`).
    pub fn set_tls_reloader(&mut self, reloader: Arc<TlsReloader>) {
        self.tls_reloader = Some(reloader);
    }

    /// Build a minimal state for tests (empty manifest with a single
    /// stub participant ; never used in production).
    #[doc(hidden)]
    #[must_use]
    pub fn new_for_tests(config: ControllerConfig) -> Self {
        use crate::manifest::{
            IterationManifest, MetaSpaceManifest, ParticipantManifest, SafetyManifest,
        };
        let manifest = Manifest {
            version: "v0.2.0-test".into(),
            meta: MetaSpaceManifest {
                dim: 1,
                safety: SafetyManifest::Box { radius: 1.0 },
            },
            participants: vec![ParticipantManifest {
                id: "stub".into(),
                dim: 1,
                kappa: Some(0.5),
                seed: Some(0),
                blas_backend: "none".into(),
                controller: None,
            }],
            iteration: IterationManifest::default(),
        };
        Self::new(config, manifest)
    }

    /// Pre-built `InfoResponse` populated with everything the
    /// controller knows.  PR #8 fills in `configured_participants`
    /// from the manifest snapshot.
    ///
    /// Uses [`tokio::sync::RwLock::try_read`] so the call is
    /// non-blocking from sync contexts ; if the manifest is being
    /// reloaded (PR #12 SIGHUP path holds the write lock briefly),
    /// this returns an empty list rather than blocking.
    pub fn info_snapshot(&self) -> InfoResponse {
        let configured_participants = match self.manifest.try_read() {
            Ok(m) => m
                .participants
                .iter()
                .map(|p| ParticipantDescriptor {
                    id: p.id.clone(),
                    dim: p.dim,
                    // M1.d : Option in the manifest → wire sentinels (0.0/0
                    // identify "not a mock" together with the row's
                    // `controller` block ; the authoritative config_hash
                    // comes from the participant's own Capabilities).
                    kappa: p.kappa.unwrap_or(0.0),
                    seed: p.seed.unwrap_or(0),
                    build_sha: String::new(),
                    blas_backend: p.blas_backend.clone(),
                    features: Vec::new(),
                    config_hash: None,
                })
                .collect(),
            Err(_) => Vec::<ParticipantDescriptor>::new(),
        };

        InfoResponse {
            build_sha: crate::GIT_SHA.into(),
            proto_version: crate::proto::PROTO_VERSION.into(),
            configured_participants,
            leader_pod_name: std::env::var("HOSTNAME").unwrap_or_default(),
            uptime_s: self.start_time.elapsed().as_secs(),
            // PR #12 : mirror the live `HealthState.reload_gen` so
            // operators querying `Info` see the same monotonic counter
            // as `Health.reload_gen`.  Bumped by
            // `controller::reload::apply_reload` on every successful
            // SIGHUP / `ReloadConfig`.  Was hard-coded to 0 in PR #8
            // pre-PR-#12 (no reload path existed yet).
            last_reload_gen: self.health.snapshot().reload_gen,
        }
    }
}
