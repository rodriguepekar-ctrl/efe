//! Production participant adapter — wraps ONE of two engines and exposes
//! the gRPC-friendly accessors `ParticipantService` needs :
//!
//! * **mock** — the Phase-2 [`MockParticipant`] (`Φ(x) = κ·x`), built from a
//!   manifest row WITHOUT a `controller` block.  Byte-identical to the
//!   historical behaviour.
//! * **efe-core** (Couche 3 M1.d, feature `core-integration`) — the REAL
//!   bridge `efe_meta_swarm::LocalParticipant` over an
//!   `efe_core::SwarmController` built by
//!   `efe_server::mode1::build_mvp_controller` (Voie A) : Mode-1 numerical
//!   fusion on stub templates — real efe-core latent swarms, **not** SLMs
//!   (no LM weights / tokenizer on the step path).  Its Φ is deterministic
//!   (M1.b) and its H1 non-expansiveness is property-tested at boot by the
//!   M1.c Lipschitz canary ([`LocalParticipant::h1_boot_gate`]) — a failed
//!   canary refuses to serve (fail-closed).
//!
//! Without the feature, an efe-core manifest row **fails construction**
//! ("built without core-integration") — never a silent mock fallback.
//!
//! # Lock model
//!
//! Both engines hold their state behind a single internal `Mutex<Tensor>`.
//! All accessors serialise on that lock — no nested locks, no deadlock risk.
//! The efe-core bridge's `step` additionally calls
//! `runtime.block_on(controller.step_with_pull(..))` — which is exactly why
//! every call site runs under `tokio::task::spawn_blocking` (see `step`'s
//! SAFETY note ; the served `Step` handler already complies).
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

/// Wire token advertised in `ParticipantDescriptor.features` by efe-core
/// rows (honest engine identification ; mock rows advertise nothing extra).
pub const ENGINE_FEATURE_EFE_CORE: &str = "engine:efe-core-mode1";

/// Production participant.  Cheap to clone (`Arc`s inside).
#[derive(Clone)]
pub struct LocalParticipant {
    inner: Arc<LocalParticipantInner>,
}

/// The two engines a served participant can wrap.  The enum is PRIVATE —
/// `ParticipantService`/`ParticipantState` only see the engine-agnostic
/// accessors below, so the Step/Reset/State handlers are engine-blind.
enum Engine {
    /// Phase-2 synthetic participant (`Φ(x) = κ·x`).
    Mock(MockParticipant),
    /// Couche 3 M1.d — the REAL efe-core bridge.
    #[cfg(feature = "core-integration")]
    EfeCore(efe_meta_swarm::participants::LocalParticipant),
}

struct LocalParticipantInner {
    id: String,
    dim: usize,
    /// Mock contraction constant ;  `0.0` sentinel for efe-core rows (the
    /// concept does not exist there — see `config_hash`).
    kappa: f32,
    /// Mock initial-state seed ;  `0` sentinel for efe-core rows.
    seed: u64,
    /// The wrapped engine.  Holds `state: Mutex<Tensor>` internally.
    engine: Engine,
    /// Couche 3 M1.d — hex SHA-256 of the manifest `controller` block
    /// (canonical JSON).  `Some` ⟺ efe-core engine ;  advertised on the wire
    /// (`ParticipantDescriptor.config_hash`) instead of the mock kappa/seed.
    config_hash: Option<String>,
    /// The manifest `controller` block this participant was booted with
    /// (`None` for mock).  Compared verbatim on SIGHUP reload — any drift is
    /// rejected fail-closed (`participant::reload`).
    controller_manifest: Option<crate::manifest::ControllerManifest>,
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
        match &row.controller {
            // ── Mock row (historical path, byte-identical behaviour) ──────
            None => {
                let (Some(kappa), Some(seed)) = (row.kappa, row.seed) else {
                    return Err(MetaSwarmError::Internal(format!(
                        "LocalParticipant::from_manifest_row : participant {:?} \
                         is a mock row (no `controller` block) but kappa/seed \
                         are missing (manifest validation should have caught \
                         this)",
                        row.id,
                    )));
                };
                let mock = MockParticipant::with_contraction(
                    row.id.clone(),
                    row.dim as usize,
                    kappa,
                    seed,
                )?;
                Ok(Self {
                    inner: Arc::new(LocalParticipantInner {
                        id: row.id.clone(),
                        dim: row.dim as usize,
                        kappa,
                        seed,
                        engine: Engine::Mock(mock),
                        config_hash: None,
                        controller_manifest: None,
                        step_index_last: AtomicU64::new(0),
                        reset_gen: AtomicU64::new(0),
                    }),
                })
            }
            // ── efe-core row (Couche 3 M1.d) ──────────────────────────────
            #[cfg(feature = "core-integration")]
            Some(block) => {
                let (bridge, config_hash) = core_engine::build(row, block)?;
                Ok(Self {
                    inner: Arc::new(LocalParticipantInner {
                        id: row.id.clone(),
                        dim: row.dim as usize,
                        kappa: 0.0, // sentinel — mock-only concept
                        seed: 0,    // sentinel — mock-only concept
                        engine: Engine::EfeCore(bridge),
                        config_hash: Some(config_hash),
                        controller_manifest: Some(block.clone()),
                        step_index_last: AtomicU64::new(0),
                        reset_gen: AtomicU64::new(0),
                    }),
                })
            }
            // ── efe-core row on a binary WITHOUT the feature : fail-closed —
            // never a silent mock fallback (architecture doc §6 item 4).
            #[cfg(not(feature = "core-integration"))]
            Some(_) => Err(MetaSwarmError::Internal(format!(
                "LocalParticipant::from_manifest_row : participant {:?} \
                 declares an efe-core `controller` block but this participant \
                 binary was built WITHOUT the `core-integration` feature — \
                 refusing to serve a mock in its place (fail-closed)",
                row.id,
            ))),
        }
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

    /// Mock contraction constant κ.  **`0.0` sentinel for efe-core rows** —
    /// the concept does not exist there (see [`Self::config_hash`]).
    #[must_use]
    pub fn kappa(&self) -> f32 {
        self.inner.kappa
    }

    /// Mock initial-state seed.  **`0` sentinel for efe-core rows.**
    #[must_use]
    pub fn seed(&self) -> u64 {
        self.inner.seed
    }

    /// Couche 3 M1.d — hex SHA-256 of the manifest `controller` block.
    /// `Some` ⟺ this participant serves a REAL efe-core engine.
    #[must_use]
    pub fn config_hash(&self) -> Option<&str> {
        self.inner.config_hash.as_deref()
    }

    /// The manifest `controller` block this participant booted with (`None`
    /// for mock).  Used by the SIGHUP reload drift check.
    #[must_use]
    pub fn controller_manifest(&self) -> Option<&crate::manifest::ControllerManifest> {
        self.inner.controller_manifest.as_ref()
    }

    /// True iff this participant wraps the REAL efe-core engine.
    #[must_use]
    pub fn is_efe_core(&self) -> bool {
        self.inner.config_hash.is_some()
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
        let new_state = match &self.inner.engine {
            Engine::Mock(m) => m.step_with_pull(pulled, gamma, alpha)?,
            #[cfg(feature = "core-integration")]
            Engine::EfeCore(b) => b.step_with_pull(pulled, gamma, alpha)?,
        };
        // Record only on success ; failed steps don't bump the counter.
        self.inner
            .step_index_last
            .store(u64::from(step_index), Ordering::Relaxed);
        Ok(new_state)
    }

    /// Read current state without mutating.  Acquires the engine's
    /// internal state mutex briefly.
    ///
    /// # Errors
    ///
    /// Bubbles up the engine's lock-poisoning error if applicable.
    pub fn current_state(&self) -> Result<Tensor, MetaSwarmError> {
        match &self.inner.engine {
            Engine::Mock(m) => m.current_state(),
            #[cfg(feature = "core-integration")]
            Engine::EfeCore(b) => b.current_state(),
        }
    }

    /// Reset to the engine's initial state (if `initial` is `None` ;  the
    /// mock re-seeds from `seed`, the efe-core bridge zeroes) or to the
    /// provided tensor (if `Some`).  Returns the new `reset_gen` value.
    ///
    /// # Errors
    ///
    /// The engine rejects `set_state` if `initial.dim() ≠ self.dim()`.
    pub fn reset(&self, initial: Option<&Tensor>) -> Result<u64, MetaSwarmError> {
        match &self.inner.engine {
            Engine::Mock(m) => {
                if let Some(t) = initial {
                    m.set_state(t.clone())?;
                } else {
                    m.reset()?;
                }
            }
            #[cfg(feature = "core-integration")]
            Engine::EfeCore(b) => {
                if let Some(t) = initial {
                    b.set_state(t.clone())?;
                } else {
                    b.reset()?;
                }
            }
        }
        let new_gen = self.inner.reset_gen.fetch_add(1, Ordering::Relaxed) + 1;
        Ok(new_gen)
    }

    /// **Couche 3 M1.d — the H1 boot gate (fail-closed).**  Runs the M1.c
    /// adversarial Lipschitz canary on the efe-core engine, per the manifest
    /// `h1_canary` policy, and returns `Err` when the participant must NOT
    /// serve :  the certificate is inconclusive, or measured `L̂ ≥ 1 − margin`
    /// (H1 falsified — "non-expansiveness property-tested, NOT certified" ;
    /// a pass is a falsification failure, never a proof).
    ///
    /// * Mock engine → `Ok` immediately (Phase-2 lenient legacy — the mock is
    ///   analytically contractive by construction).
    /// * efe-core + `h1_canary.mode == "off"` → `Ok` with a LOUD warning (the
    ///   manifest explicitly opted out ;  the participant serves unverified).
    ///
    /// The canary drives `phi_once` internally via `block_on`, so this method
    /// wraps it in `spawn_blocking` — call it from async context (the
    /// participant binary calls it after construction, before serving).
    /// Expect ~0.5-3 s per Φ evaluation : the default probe (~60 evals) costs
    /// roughly half a minute to a few minutes of boot time.
    pub async fn h1_boot_gate(&self) -> Result<(), MetaSwarmError> {
        #[cfg(feature = "core-integration")]
        {
            if let Engine::EfeCore(_) = &self.inner.engine {
                let policy = self
                    .inner
                    .controller_manifest
                    .as_ref()
                    .map(|c| c.h1_canary.clone())
                    .unwrap_or_default();
                if policy.mode == "off" {
                    tracing::warn!(
                        participant = %self.inner.id,
                        "H1 boot canary DISABLED by manifest (h1_canary.mode = off) — \
                         this efe-core participant serves with UNVERIFIED \
                         non-expansiveness ; run the Lipschitz canary out-of-band"
                    );
                    return Ok(());
                }
                let this = self.clone();
                let cert = tokio::task::spawn_blocking(move || {
                    let Engine::EfeCore(bridge) = &this.inner.engine else {
                        unreachable!("checked EfeCore above ; enum is immutable");
                    };
                    bridge.verify_non_expansive()
                })
                .await
                .map_err(|e| {
                    MetaSwarmError::Internal(format!("H1 boot canary panicked : {e}"))
                })??;
                return match &cert {
                    efe_meta_swarm::participants::NonExpansiveCertificate::Empirical {
                        l_hat,
                        margin,
                        n_pairs_scored,
                        inconclusive,
                        ..
                    } => {
                        if cert.accepts(false) {
                            tracing::info!(
                                participant = %self.inner.id,
                                l_hat, margin, n_pairs_scored,
                                "H1 boot canary PASSED — non-expansiveness \
                                 property-tested (NOT certified) on the safety box"
                            );
                            Ok(())
                        } else {
                            Err(MetaSwarmError::HypothesisCheckFailed { hypothesis: "H1" }
                                .tap_trace(&self.inner.id, *l_hat, *margin, *inconclusive))
                        }
                    }
                    other => Err(MetaSwarmError::Internal(format!(
                        "H1 boot canary returned an unexpected certificate \
                         ({other:?}) for efe-core participant {:?}",
                        self.inner.id,
                    ))),
                };
            }
        }
        // Mock engine (or feature off — where an efe-core row cannot exist).
        Ok(())
    }
}

impl LocalParticipant {
    /// **Test/ops introspection (doc-hidden)** — run the M1.c Lipschitz
    /// canary on the efe-core engine and return the raw certificate
    /// (`None` for a mock engine).  Same spawn_blocking discipline as
    /// [`Self::h1_boot_gate`] ;  used by the Palier-2 measurement test to
    /// compare `L̂` across topology classes.
    #[cfg(feature = "core-integration")]
    #[doc(hidden)]
    pub async fn h1_certificate(
        &self,
    ) -> Result<Option<efe_meta_swarm::participants::NonExpansiveCertificate>, MetaSwarmError>
    {
        if !matches!(&self.inner.engine, Engine::EfeCore(_)) {
            return Ok(None);
        }
        let this = self.clone();
        let cert = tokio::task::spawn_blocking(move || {
            let Engine::EfeCore(bridge) = &this.inner.engine else {
                unreachable!("checked EfeCore above ; enum is immutable");
            };
            bridge.verify_non_expansive()
        })
        .await
        .map_err(|e| MetaSwarmError::Internal(format!("H1 canary panicked : {e}")))??;
        Ok(Some(cert))
    }
}

/// Small extension so the H1 failure logs its evidence before bubbling.
/// Only used by the feature-gated `h1_boot_gate` body.
#[cfg(feature = "core-integration")]
trait TapTrace {
    fn tap_trace(self, id: &str, l_hat: f64, margin: f64, inconclusive: bool) -> Self;
}
#[cfg(feature = "core-integration")]
impl TapTrace for MetaSwarmError {
    fn tap_trace(self, id: &str, l_hat: f64, margin: f64, inconclusive: bool) -> Self {
        tracing::error!(
            participant = %id,
            l_hat, margin, inconclusive,
            "H1 boot canary FAILED — refusing to serve (fail-closed) : the \
             Lipschitz canary falsified non-expansiveness or was inconclusive"
        );
        self
    }
}

/// Couche 3 M1.d — construction of the REAL efe-core engine from a manifest
/// `controller` block.  Isolated in a feature-gated module so the default
/// build carries zero efe-core/efe-server code.
#[cfg(feature = "core-integration")]
mod core_engine {
    use std::sync::Arc;

    use efe_core::consensus::ConsensusConfig;
    use efe_core::fusion::MvpCpuFusion;
    use efe_core::genesis::{
        DefaultSpawner, FixedCardinality, FixedTemplateRouter, GenesisEngine, SpawnerConfig,
    };
    use efe_core::genesis::registry::{StubTemplateRegistry, TemplateRegistry};
    use efe_core::memory::{DeviceBackend, HostBackend, MemoryPool, PoolConfig};
    use efe_core::streams::{StreamPool, StreamPoolConfig};
    use efe_core::swarm::controller::{ControllerConfig, SwarmController};
    use efe_core::swarm::strategies::{
        AlignmentProjector, ConsensusDetector, Dissolver, FusionStrategy, GenesisStrategy,
        NullTelemetryCollector, TelemetryCollector,
    };
    use efe_core::types::{
        AlignmentCone, LatentDim, PopulationMix, ResourceBudget, SwarmSpec, TemplateCode,
    };
    use efe_meta_swarm::participants::LipschitzProbe;
    use efe_meta_swarm::safety::{BoxSafety, SafetySet};
    use efe_meta_swarm::MetaSwarmError;
    use efe_server::mode1::{
        build_mvp_controller, AlignmentAdapter, ConsensusAdapter, FusionAdapter, LastCentroid,
        LastSpecTrace, Mode1Config, PassThroughDissolver,
    };
    use sha2::{Digest, Sha256};

    use crate::manifest::{ControllerManifest, ParticipantManifest, TopologyManifest};

    /// A generous wall-clock budget for the participant spec : a real
    /// `phi_once` costs ~0.5-3 s (arena zeroing dominates) — far above
    /// efe-core's 500 ms default, which would fail steps spuriously under
    /// load.  30 s matches the controller's `MAX_WALL_MS_HARD_CAP`.
    const SPEC_WALL_CLOCK_MS: u32 = 30_000;

    /// Build the real bridge + its config hash.  Fail-closed on any
    /// unparseable template code or invalid spec.
    pub(super) fn build(
        row: &ParticipantManifest,
        block: &ControllerManifest,
    ) -> Result<(efe_meta_swarm::participants::LocalParticipant, String), MetaSwarmError> {
        let dim = row.dim as usize;

        // 1. Population — template strings parsed fail-closed.
        let mut mix = Vec::with_capacity(block.population.len());
        for p in &block.population {
            let code = TemplateCode::from_str(&p.template).ok_or_else(|| {
                MetaSwarmError::Internal(format!(
                    "participant {:?} : unknown template code {:?} in \
                     controller.population (expected e.g. \"na-sem\")",
                    row.id, p.template,
                ))
            })?;
            mix.push((code, p.count));
        }
        let population = PopulationMix::new(mix.clone())
            .map_err(|e| into_internal(&row.id, "population", &e))?;

        // 2. Spec pieces.
        let dim_latent = LatentDim::new(row.dim)
            .map_err(|e| into_internal(&row.id, "dim_latent", &e))?;
        let cone = AlignmentCone::new(
            block.alignment_cone.anchors.clone(),
            block.alignment_cone.max_projection_distance,
        )
        .map_err(|e| into_internal(&row.id, "alignment_cone", &e))?;
        let mut budget = ResourceBudget::default();
        budget.max_wall_clock_ms = SPEC_WALL_CLOCK_MS;
        let spec = SwarmSpec::builder()
            .problem_embedding(vec![0.0; dim]) // overridden per step by phi_once
            .dim_latent(dim_latent)
            .population(population)
            .max_rounds(block.max_rounds)
            .map_err(|e| into_internal(&row.id, "max_rounds", &e))?
            .alignment_cone(cone)
            .budget(budget)
            .build()
            .map_err(|e| into_internal(&row.id, "SwarmSpec", &e))?;

        // 3. The REAL controller (M1.b : deterministic, noise-free Φ),
        //    per the manifest topology class (M1 Palier-2).
        let controller = match block.topology {
            // Palier-2 (default) : x-independent topology ⟹ Φ continuous by
            // construction — the genesis-routing discontinuity (the one H1
            // killer) is eliminated.
            TopologyManifest::Fixed => build_fixed_topology_controller(row, &mix)?,
            // Explicit opt-out : heuristic (x-dependent) genesis routing.
            // H1 then rests SOLELY on the empirical canary — logged loudly.
            TopologyManifest::Dynamic => {
                tracing::warn!(
                    participant = %row.id,
                    "topology = DYNAMIC : the genesis routing varies with x, so Φ \
                     may be discontinuous at routing-cell boundaries — H1 is \
                     unproven by construction and rests solely on the empirical \
                     canary (property-tested, NOT certified)"
                );
                build_mvp_controller(Mode1Config::for_dim(row.dim))
                    .map_err(|e| into_internal(&row.id, "build_mvp_controller", &e))?
            }
        };

        // 4. Participant safety box = state projection + H1 canary domain.
        let safety: Box<dyn SafetySet> = Box::new(
            BoxSafety::new(
                vec![block.safety_box.lo; dim],
                vec![block.safety_box.hi; dim],
            )
            .map_err(|e| MetaSwarmError::Internal(format!(
                "participant {:?} : safety_box : {e}",
                row.id,
            )))?,
        );

        // 5. The bridge.  `Handle::try_current()` : construction happens in
        //    the binary's async_main (multi-thread runtime) — fail-closed
        //    with a clear message when misused outside a runtime.
        let handle = tokio::runtime::Handle::try_current().map_err(|_| {
            MetaSwarmError::Internal(format!(
                "participant {:?} : an efe-core participant must be built \
                 inside a tokio runtime (its step path bridges sync→async)",
                row.id,
            ))
        })?;
        let probe = probe_from_manifest(block);
        let bridge = efe_meta_swarm::participants::LocalParticipant::new(
            row.id.clone(),
            controller,
            spec,
            block.tenant_id.clone(),
            safety,
            handle,
        )?
        .with_lipschitz_probe(probe);

        // 6. config_hash — canonical JSON of the block, SHA-256, hex.
        let canonical = serde_json::to_string(block).map_err(|e| {
            MetaSwarmError::Internal(format!(
                "participant {:?} : controller block serialization : {e}",
                row.id,
            ))
        })?;
        let hash = hex_sha256(canonical.as_bytes());
        Ok((bridge, hash))
    }

    /// **M1 Palier-2 — the fixed-topology latent controller.**
    ///
    /// Mirrors `efe_server::mode1::build_mvp_parts`'s LATENT recipe (the
    /// converge-now single-round profile of M1.a/M1.b) with ONE deliberate
    /// difference : genesis routing/cardinality are the CONSTANTS
    /// [`FixedTemplateRouter`]/[`FixedCardinality`] built from the manifest
    /// population plan — so the spawned population (count, mix, mask) is
    /// independent of the input `x` and `Φ` is continuous by construction.
    ///
    /// DELIBERATE DUPLICATION : this assembly is a mirror of mode1's latent
    /// branch, kept HERE (server crate) instead of extending `mode1.rs`,
    /// which a concurrent session currently holds (its uncommitted dissolver
    /// rework touches exactly that construction).  Consolidate into a
    /// `build_mvp_controller_fixed(cfg, plan)` in mode1 once that lands.
    /// Until then this path intentionally uses only HEAD-committed APIs.
    fn build_fixed_topology_controller(
        row: &ParticipantManifest,
        plan: &[(TemplateCode, u32)],
    ) -> Result<std::sync::Arc<SwarmController>, MetaSwarmError> {
        let cfg = Mode1Config::for_dim(row.dim);
        let ii = |what: &str, e: &dyn std::fmt::Display| into_internal(&row.id, what, e);

        // 1. Device + pools (HostBackend, CPU-only — identical to mode1).
        let device: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let mut pool_cfg = PoolConfig::default();
        pool_cfg.prealloc_arenas = cfg.prealloc_arenas;
        pool_cfg.max_arenas = cfg.max_arenas;
        let pool = Arc::new(MemoryPool::new(pool_cfg, device.clone()).map_err(|e| ii("MemoryPool", &e))?);
        let mut stream_cfg = StreamPoolConfig::default();
        stream_cfg.size = cfg.n_streams;
        let streams = Arc::new(StreamPool::new(stream_cfg).map_err(|e| ii("StreamPool", &e))?);
        let registry: Arc<dyn TemplateRegistry> =
            Arc::new(StubTemplateRegistry::mvp_defaults(row.dim));

        // 2. FIXED genesis : constant router + cardinality from the manifest
        //    plan, zero-noise spawner (M1.b determinism).
        let router = Arc::new(FixedTemplateRouter::from_population(plan).map_err(|e| ii("fixed router", &e))?);
        let cardinality = Arc::new(FixedCardinality::from_population(plan).map_err(|e| ii("fixed cardinality", &e))?);
        let mut spawner_cfg = SpawnerConfig::default();
        spawner_cfg.init_sigma = 0.0; // deterministic genesis — no noise draw
        let spawner = Arc::new(DefaultSpawner::new(
            pool.clone(),
            registry.clone(),
            streams.clone(),
            spawner_cfg,
        ));
        let genesis: Arc<dyn GenesisStrategy> = Arc::new(
            GenesisEngine::builder()
                .router(router)
                .cardinality(cardinality)
                .spawner(spawner)
                .build_mvp(pool, registry, streams)
                .map_err(|e| ii("GenesisEngine", &e))?,
        );

        // 3. Fusion + centroid channel (single-flight discipline as mode1).
        let fusion_core = Arc::new(
            MvpCpuFusion::builder()
                .with_backend(device)
                .with_replay_enabled(false)
                .build()
                .map_err(|e| ii("MvpCpuFusion", &e))?,
        );
        let last_centroid = LastCentroid::new();
        let last_spec_trace = LastSpecTrace::new();
        let fusion: Arc<dyn FusionStrategy> = Arc::new(FusionAdapter::new(
            fusion_core,
            cfg.fusion.clone(),
            last_centroid.clone(),
            last_spec_trace,
        ));
        let dissolver: Arc<dyn Dissolver> = Arc::new(PassThroughDissolver::new(last_centroid));
        let alignment: Arc<dyn AlignmentProjector> =
            Arc::new(AlignmentAdapter::strict_default().map_err(|e| ii("AlignmentAdapter", &e))?);
        let telemetry: Arc<dyn TelemetryCollector> = Arc::new(NullTelemetryCollector);

        // 4. Converge-now consensus (single-round Φ profile — M1.a) : same
        //    signal weights as Mode-1, threshold 0 + hysteresis 1, and
        //    ControllerConfig::default() (min_rounds = 0).
        let converge_now: ConsensusConfig = cfg
            .consensus
            .clone()
            .with_confidence_threshold(0.0)
            .with_hysteresis(1);
        let consensus_factory = move || -> Box<dyn ConsensusDetector> {
            Box::new(
                ConsensusAdapter::new(converge_now.clone())
                    .expect("converge-now consensus config is valid"),
            )
        };

        SwarmController::builder()
            .genesis(genesis)
            .fusion(fusion)
            .dissolver(dissolver)
            .alignment(alignment)
            .telemetry(telemetry)
            .consensus_factory(consensus_factory)
            .config(ControllerConfig::default())
            .build()
            .map(std::sync::Arc::new)
            .map_err(|e| ii("SwarmController", &e))
    }

    /// Manifest → canary probe (bounded, boot-sized).
    fn probe_from_manifest(block: &ControllerManifest) -> LipschitzProbe {
        let c = &block.h1_canary;
        let mut p = LipschitzProbe::small();
        p.n_seeds = c.n_seeds as usize;
        p.n_pairs = c.n_pairs as usize;
        p.n_directions = c.n_directions as usize;
        p.n_bisect_pairs = c.n_bisect_pairs as usize;
        p.bisect_depth = c.bisect_depth as usize;
        p.max_evals = c.max_evals as usize;
        p.margin = c.margin;
        p
    }

    fn hex_sha256(bytes: &[u8]) -> String {
        let digest = Sha256::digest(bytes);
        let mut out = String::with_capacity(64);
        for b in digest {
            use std::fmt::Write;
            let _ = write!(out, "{b:02x}");
        }
        out
    }

    fn into_internal(id: &str, what: &str, e: &dyn std::fmt::Display) -> MetaSwarmError {
        MetaSwarmError::Internal(format!("participant {id:?} : {what} : {e}"))
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
            kappa: Some(0.6),
            seed: Some(11),
            blas_backend: "none".into(),
            controller: None,
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
        row.kappa = Some(1.5);
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
