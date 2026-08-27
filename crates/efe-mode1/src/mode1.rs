//! **Mode 1** — câblage d'un `SwarmController` **réel** pour le cas
//! d'usage « EFE comme outil de classification par ancres » (voir
//! `MODE1_ARCHITECTURE.md`).
//!
//! Ce module fournit trois choses :
//!
//! 1. Des **adapters** qui réconcilient les signatures entre les traits
//!    publics de `efe-core::swarm::strategies` (vue controller) et les
//!    impls concrètes de `efe-core::fusion`, `efe-core::consensus`,
//!    `efe-core::genesis`, etc.
//!
//!    *Pourquoi ? Le controller voit un trait `FusionStrategy` qui prend
//!    `(els, batch, round) -> FusionStats`, alors que l'impl MVP
//!    (`MvpCpuFusion`) implémente un trait plus riche `run_round(els,
//!    arena, cfg, round) -> FusionRound`. Le `FusionAdapter` ci-dessous
//!    fait le pont et capture au passage le centroïde du round pour que
//!    le dissolver y ait accès.*
//!
//! 2. Un `PassThroughDissolver` qui, sur demande du controller, lit le
//!    centroïde capturé et le renvoie brut au client. Le client fait
//!    ensuite le post-traitement (nearest-anchor, seuil de confiance)
//!    — voir `clients/python/efe_tool/` et
//!    `MODE1_ARCHITECTURE.md §13`.
//!
//! 3. Une fonction `build_mvp_backend(dim, fusion_cfg, consensus_cfg)`
//!    qui assemble le tout et retourne un `Arc<dyn ExecuteBackend>`
//!    prêt à être branché dans `main.rs` à la place du stub
//!    `UnimplementedBackend`.
//!
//! # Pourquoi un `Mutex<Option<Vec<f32>>>` partagé ?
//!
//! Le trait `swarm::strategies::Dissolver::extract_centroid(&els)` ne
//! reçoit pas l'arène mémoire nécessaire aux views typées de l'ELS,
//! tandis que `FusionStrategy::run_round(els, batch, round)` y a accès
//! via `batch.arena_handle()`. On publie donc, à la fin de chaque
//! round, le centroïde extrait — le dernier round qui termine gagne
//! (typiquement `Verdict::Converged`), et c'est le résultat qui
//! parvient au dissolver. Cette approche :
//!
//!   * évite de modifier le trait public (risque de cascade sur le
//!     reste du workspace);
//!   * préserve la sémantique « un essaim = un execute, pas d'état
//!     résiduel entre requêtes » (la cellule est remise à zéro par
//!     `build_mvp_backend` à chaque construction, et *per-request*
//!     via un wrapper `Mode1Backend` — voir plus bas).

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use efe_core::consensus::{
    detector::ConsensusDetector as CoreConsensusDetector, verdict::ConsensusVerdict,
    ConsensusConfig, MultiCriteriaDetector,
};
use efe_core::dissolution::{
    alignment::{AlignmentProjector as CoreAlignmentProjector, StrictAlignmentProjector},
    config::ExtractionConfig,
    extractors::{CentroidExtractor, WeightedMeanExtractor},
};
use efe_core::els::EpistemicLatentSpace;
use efe_core::errors::{EfeError, Result};
use efe_core::fusion::{
    config::FusionConfig, mvp_cpu::MvpCpuFusion, stats::FusionStats,
    strategy::FusionStrategy as CoreFusionStrategy,
};
use efe_core::genesis::{
    engine::GenesisEngine,
    registry::{
        ActiveAnchorsSlot, AnchorAwareTemplateRegistry, StubTemplateRegistry, TemplateRegistry,
    },
    // Couche 3 M1.b — the latent controller injects a zero-noise spawner (init_sigma=0) so Φ is
    // a bit-reproducible pure function of x (see `build_mvp_parts`, `latent_init_sigma`).
    spawner::{DefaultSpawner, SpawnerConfig},
};
use efe_core::memory::{HostBackend, MemoryPool, NanoAgentBatch, PoolConfig};
use efe_core::streams::{StreamPool, StreamPoolConfig};
use efe_core::swarm::{
    controller::{ControllerConfig, SwarmController},
    request::{DecodeMode, DecodedOutput, Request, Response},
    strategies::{
        AlignmentProjector as SwarmAlignmentProjector, ConsensusDetector as SwarmConsensusDetector,
        Dissolver, FusionStrategy as SwarmFusionStrategy, GenesisStrategy, NullTelemetryCollector,
    },
    verdict::Verdict,
};
use efe_core::types::AlignmentCone;

use crate::backend::{ExecuteBackend, SwarmControllerBackend};

// =====================================================================
// LastCentroid — shared publication slot
// =====================================================================

/// Publication slot for the latest centroid extracted from the ELS.
///
/// Populated by [`FusionAdapter::run_round`] at the end of each fusion
/// round; consumed by [`PassThroughDissolver::extract_centroid`]. The
/// mutex section is minimal (clone of a `Vec<f32>`), so contention is
/// negligible in practice.
///
/// # Scope (Couche 3)
///
/// This slot backs the **server request path** only. Because it lives for
/// the whole life of the controller, it is safe only under **sequential**
/// `execute()` calls on a given controller (publish overwrites, take
/// drains). The meta-swarm **latent** controller — which the M1.c
/// Lipschitz canary evaluates *concurrently* — instead carries the
/// centroid on the per-execute ELS (see [`ElsCarriedDissolver`] and
/// [`FusionAdapter::with_centroid_carry`]) so `Φ` is pure by construction
/// rather than by single-flight discipline.
#[derive(Clone, Default, Debug)]
pub struct LastCentroid(Arc<Mutex<Option<Vec<f32>>>>);

impl LastCentroid {
    /// Construct an empty slot.
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(None)))
    }

    fn publish(&self, centroid: Vec<f32>) {
        let mut g = self.0.lock().expect("LastCentroid: mutex poisoned");
        *g = Some(centroid);
    }

    fn take(&self) -> Option<Vec<f32>> {
        let mut g = self.0.lock().expect("LastCentroid: mutex poisoned");
        g.take()
    }

    fn reset(&self) {
        let mut g = self.0.lock().expect("LastCentroid: mutex poisoned");
        *g = None;
    }
}

// =====================================================================
// SpecTraceCtx — per-request trace context for D7 centroid logging
// =====================================================================

/// **Sprint D7** — diagnostic snapshot of a request's spec, used by the
/// `EFE_LOG_CENTROID_TRACE` instrumentation to compute per-round cosine
/// similarities between the running centroid and (a) the original
/// problem embedding, (b) each anchor of the alignment cone.
///
/// Populated by [`Mode1Backend::execute`] at the start of each request
/// (the `Request` carries the `SwarmSpec`), and consumed by
/// [`FusionAdapter::run_round`] **only when the trace env var is set**
/// — the trace is otherwise zero-overhead (no clone).
#[derive(Clone, Debug)]
pub struct SpecTraceCtx {
    /// L2-normalised problem embedding, length = dim_latent.
    problem_embedding: Vec<f32>,
    /// Anchors that define the alignment cone, each length = dim_latent.
    /// We keep them as a flat list so the trace can attribute the
    /// centroid to a specific anchor by index.
    anchors: Vec<Vec<f32>>,
}

impl SpecTraceCtx {
    /// Build a snapshot from the parts the trace needs. Called from
    /// [`Mode1Backend::execute`] which has access to the full request.
    #[must_use]
    pub fn new(problem_embedding: Vec<f32>, anchors: Vec<Vec<f32>>) -> Self {
        Self {
            problem_embedding,
            anchors,
        }
    }
}

/// Shared slot for the per-request [`SpecTraceCtx`]. Mirrors the
/// [`LastCentroid`] pattern — populated by [`Mode1Backend`] before
/// delegation, consumed by [`FusionAdapter`] post-round, reset between
/// requests.
///
/// When the `EFE_LOG_CENTROID_TRACE` env var is unset the producer
/// (`Mode1Backend::execute`) refrains from populating this slot, so
/// the FusionAdapter's `peek`-or-skip path is a single mutex lock per
/// round on the `None` arm — negligible.
///
/// # Concurrent-request semantics (diagnostic only)
///
/// Multiple requests can be in flight on the same `Mode1Backend`
/// (the gRPC layer fans out N parallel tasks). They all share the
/// same `LastSpecTrace` instance, so request B's `publish()` can
/// overwrite request A's slot mid-fusion. The trace logged for
/// request A's later rounds may then carry request B's
/// problem-embedding / anchor list.
///
/// **This is intentional.** The trace is a **diagnostic
/// instrumentation** activated by the operator during a calibration
/// session, not a correctness-critical observation. The same
/// limitation exists for `LastCentroid` ; the existing module
/// docstring (top of file) explains that the dissolver inherits
/// "the last round which terminates wins" semantics. For an
/// authoritative per-request trace, future work would route the
/// spec via the `NanoAgentBatch` (out of scope for D7 — would
/// require widening the `swarm::FusionStrategy` trait, which
/// cascades into the entire workspace).
#[derive(Clone, Default, Debug)]
pub struct LastSpecTrace(Arc<Mutex<Option<SpecTraceCtx>>>);

impl LastSpecTrace {
    /// Construct an empty slot.
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(None)))
    }

    fn publish(&self, ctx: SpecTraceCtx) {
        let mut g = self.0.lock().expect("LastSpecTrace: mutex poisoned");
        *g = Some(ctx);
    }

    /// Peek without consuming — the trace must remain available for
    /// every round in the current campaign.
    fn peek(&self) -> Option<SpecTraceCtx> {
        let g = self.0.lock().expect("LastSpecTrace: mutex poisoned");
        g.clone()
    }

    fn reset(&self) {
        let mut g = self.0.lock().expect("LastSpecTrace: mutex poisoned");
        *g = None;
    }
}

/// **Sprint D7** — Returns `true` if `EFE_LOG_CENTROID_TRACE` is set
/// to a permissive truthy value (`1`, `true`, `yes`, `on`).
///
/// The result is cached at first call (atomic, lock-free) so the
/// hot fusion-round path pays only an `Acquire`-class load on every
/// subsequent call. **The cache is process-wide and one-shot** —
/// toggling the env var after the first call is *not* observed.
/// This is acceptable because :
///
/// 1. The env var is intended for *startup* configuration (operator
///    sets `EFE_LOG_CENTROID_TRACE=1` then launches the binary).
/// 2. Mid-process toggling is undefined for every other `EFE_*`
///    var in the server (`config.rs` reads them once via
///    `ServerConfig::from_env`).
/// 3. Reading the env var on every fusion round (a few µs syscall)
///    would noise up `tracing` overhead measurements.
///
/// If a future requirement needs dynamic toggle, switch the static
/// to an `AtomicBool` and arm a SIGHUP handler that re-reads the
/// env. For D7 the one-shot semantic is adequate.
fn centroid_trace_enabled() -> bool {
    use std::sync::atomic::{AtomicU8, Ordering};
    // 0 = uninitialised, 1 = false, 2 = true.
    static CACHE: AtomicU8 = AtomicU8::new(0);
    match CACHE.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let enabled = std::env::var("EFE_LOG_CENTROID_TRACE")
                .map(|v| {
                    matches!(
                        v.trim().to_ascii_lowercase().as_str(),
                        "1" | "true" | "yes" | "on"
                    )
                })
                .unwrap_or(false);
            CACHE.store(if enabled { 2 } else { 1 }, Ordering::Relaxed);
            enabled
        }
    }
}

/// Cosine similarity for two equal-length vectors.
///
/// Returns `0.0` if either vector has zero norm — the trace is a
/// diagnostic, not a numerical check, so we suppress NaN propagation
/// at this boundary.
fn cosine_sim(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let denom = (na.sqrt()) * (nb.sqrt());
    if denom <= 0.0 || !denom.is_finite() {
        0.0
    } else {
        (dot / denom).clamp(-1.0, 1.0)
    }
}

// =====================================================================
// FusionAdapter — bridges swarm::FusionStrategy ↔ fusion::FusionStrategy
// =====================================================================

/// Adapts the controller-facing [`SwarmFusionStrategy`] trait to the
/// richer `fusion::FusionStrategy` trait implemented by `MvpCpuFusion`.
///
/// Responsibilities:
///
/// * Retrieves the arena from the `NanoAgentBatch` (the core fusion
///   needs it to read/write the latent views).
/// * Invokes `MvpCpuFusion::run_round(els, arena, cfg, round)`.
/// * Extracts the centroid from the updated ELS and publishes it to a
///   shared `LastCentroid` slot.
/// * Returns the per-round `FusionStats` that the controller expects.
pub struct FusionAdapter {
    inner: Arc<MvpCpuFusion>,
    cfg: FusionConfig,
    last_centroid: LastCentroid,
    /// **Sprint D7** — Per-request snapshot of the spec used by the
    /// `EFE_LOG_CENTROID_TRACE` instrumentation. Always present in the
    /// adapter, but the trace logic only fires when the env var is set
    /// AND the slot has been populated by `Mode1Backend::execute`.
    last_spec_trace: LastSpecTrace,
    extractor: Arc<dyn CentroidExtractor>,
    /// **Couche 3 (M1.c)** — when `true`, the round's centroid is stashed
    /// on the per-execute ELS (`els.set_dissolution_centroid`) instead of
    /// the controller-wide [`LastCentroid`] slot, so `Φ` is pure under
    /// concurrent `phi_once` calls on a shared controller. Set **only** by
    /// the latent controller (`build_mvp_controller`); the server request
    /// path leaves it `false` and keeps the exact shared-slot behaviour
    /// (byte-identical). See [`FusionAdapter::with_centroid_carry`].
    carry_centroid_via_els: bool,
}

impl std::fmt::Debug for FusionAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FusionAdapter")
            .field("backend", &self.inner.backend_name())
            .field("extractor", &self.extractor.name())
            .finish()
    }
}

impl FusionAdapter {
    /// Construct with an existing `MvpCpuFusion` and a centroid slot.
    ///
    /// `last_spec_trace` is the slot the adapter reads at the end of
    /// each round to log the diagnostic cosine vector. Pass an empty
    /// `LastSpecTrace::new()` if the trace path is not used.
    #[must_use]
    pub fn new(
        inner: Arc<MvpCpuFusion>,
        cfg: FusionConfig,
        last_centroid: LastCentroid,
        last_spec_trace: LastSpecTrace,
    ) -> Self {
        Self {
            inner,
            cfg,
            last_centroid,
            last_spec_trace,
            extractor: Arc::new(WeightedMeanExtractor::default()),
            // Default: publish to the shared slot (server request path).
            carry_centroid_via_els: false,
        }
    }

    /// **Couche 3 (M1.c)** — opt into carrying the round's centroid on the
    /// per-execute ELS instead of the shared [`LastCentroid`] slot.
    ///
    /// Enabled by the latent controller so that `Φ` (`phi_once`) is a pure
    /// function of its input under **concurrent** evaluation on a shared
    /// `SwarmController` (the M1.c Lipschitz-canary wants `Φ(x)‖Φ(y)`).
    /// Pass `false` (the default) on the server request path to preserve
    /// the exact shared-slot behaviour bit-for-bit. Must be paired with an
    /// [`ElsCarriedDissolver`] on the controller (see `build_mvp_parts`).
    #[must_use]
    pub fn with_centroid_carry(mut self, on: bool) -> Self {
        self.carry_centroid_via_els = on;
        self
    }
}

#[async_trait]
impl SwarmFusionStrategy for FusionAdapter {
    async fn run_round(
        &self,
        els: &mut EpistemicLatentSpace,
        batch: &NanoAgentBatch,
        round: u32,
    ) -> Result<FusionStats> {
        // 1. Get the arena bound to this batch.
        let arena_handle = batch
            .arena_handle()
            .ok_or(EfeError::Internal("FusionAdapter: batch has no arena"))?;
        let arena = arena_handle.arena();

        // 2. Delegate to the core MVP fusion kernel.
        let fusion_round = self.inner.run_round(els, arena, &self.cfg, round).await?;
        let stats = fusion_round.stats;

        // 3. Capture centroid from the updated ELS for the dissolver.
        //    On the server path this publishes to the shared `LastCentroid`
        //    slot; on the latent path (`carry_centroid_via_els`) it is
        //    stashed on the per-execute ELS instead — pure under concurrency.
        //    Errors here are non-fatal for the round (stats already
        //    travelled), but we still log to help diagnose mismatches.
        if let Err(e) = self.capture_centroid(els, arena, round) {
            tracing::warn!(round, error = %e, "FusionAdapter: centroid capture failed");
        }

        Ok(stats)
    }

    async fn inject_noise(&self, _els: &mut EpistemicLatentSpace, sigma: f32) -> Result<()> {
        // `MvpCpuFusion` does not expose an inject_noise kernel in the
        // `fusion::FusionStrategy` trait. The MVP wiring disables
        // mode-collapse recovery (see `build_mvp_controller`), so the
        // controller should never call this path. If it does (e.g.
        // policy changes in a future build), we log but do not fail —
        // a subsequent verdict will still be Converged / Diverged.
        tracing::warn!(
            sigma,
            "FusionAdapter::inject_noise called but not implemented in MVP; continuing"
        );
        Ok(())
    }
}

impl FusionAdapter {
    /// Extract the round's centroid from the (updated) ELS and route it to
    /// the dissolver's channel.
    ///
    /// Takes `&mut els` because the latent path stashes the centroid **on
    /// the ELS itself** (`set_dissolution_centroid`) — the per-execute
    /// carrier that makes `Φ` pure by construction. The centroid is
    /// extracted through *immutable* views (scoped so their borrow ends
    /// before the stash), so the `&mut` is only used for the final write.
    ///
    /// Routing:
    ///   * `carry_centroid_via_els == false` (server request path) —
    ///     publish to the shared [`LastCentroid`] slot, **exactly** as
    ///     before (D7 trace included). Byte-identical.
    ///   * `carry_centroid_via_els == true` (latent controller) — stash on
    ///     the ELS; the shared slot is never touched, so two concurrent
    ///     `phi_once` calls on a shared controller cannot clobber each
    ///     other's centroid.
    fn capture_centroid(
        &self,
        els: &mut EpistemicLatentSpace,
        arena: &efe_core::memory::Arena,
        round: u32,
    ) -> Result<()> {
        // Scope the immutable views so the shared borrow of `els` ends
        // before the mutable stash below.
        let c = {
            let latent = els
                .latent_view(arena)?
                .ok_or(EfeError::Internal("FusionAdapter: ELS has no latent"))?;
            let mask = els
                .mask_view(arena)?
                .ok_or(EfeError::Internal("FusionAdapter: ELS has no mask"))?;
            self.extractor.extract(&latent, &mask)?
        };

        // Sprint D7 — diagnostic centroid trace. Activated by
        // `EFE_LOG_CENTROID_TRACE=1` ; gated on the env var being set
        // AND the per-request spec being available, so the cost is
        // a single atomic load + a `None` mutex peek when disabled.
        // (Inert on the latent path: `last_spec_trace` is only populated
        // by `Mode1Backend`, which is not in the latent controller's path.)
        if centroid_trace_enabled() {
            if let Some(ctx) = self.last_spec_trace.peek() {
                let cos_problem = cosine_sim(&c, &ctx.problem_embedding);
                let cos_anchors: Vec<f32> = ctx.anchors.iter().map(|a| cosine_sim(&c, a)).collect();
                let argmax_anchor = cos_anchors
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(i, v)| (i, *v));
                tracing::debug!(
                    // **Cette cible ne suit PAS le crate, et c'est délibéré (R1).** Le code vit
                    // désormais dans `efe-mode1`, mais `target:` est une étiquette de filtrage, pas
                    // un chemin résolu par le compilateur — et trois documents publient la commande
                    // `RUST_LOG=efe_server::mode1::trace=debug` (BENCHMARKS.md, l'architecture D7,
                    // le rapport de calibration D7). La renommer pour « faire propre » casserait
                    // silencieusement des runbooks écrits ; la laisser sans ce commentaire ferait
                    // qu'on la renomme au prochain passage. Une incohérence assumée et expliquée
                    // vaut mieux qu'une cohérence qui casse ce qui marche.
                    target: "efe_server::mode1::trace",
                    round,
                    cos_problem = cos_problem,
                    cos_anchors = ?cos_anchors,
                    argmax_anchor = ?argmax_anchor,
                    "D7 centroid trace"
                );
            }
        }

        if self.carry_centroid_via_els {
            // Latent path — pure by construction (per-execute carrier).
            els.set_dissolution_centroid(c);
        } else {
            // Server request path — shared publication slot (unchanged).
            self.last_centroid.publish(c);
        }
        Ok(())
    }
}

// =====================================================================
// ConsensusAdapter — bridges swarm::ConsensusDetector ↔ core trait
// =====================================================================

/// Adapts `consensus::detector::ConsensusDetector` (multi-criteria,
/// `ingest + verdict`) to the simpler `swarm::strategies::ConsensusDetector`
/// (`evaluate + reset`) that the controller consumes.
///
/// The core trait emits a richer `ConsensusVerdict` (Continue,
/// Converged, Diverged, Stalled, Anomaly). We map it down to the
/// three-variant `swarm::Verdict` in the only obvious way:
///
/// * `Continue`       → `Verdict::Continue`
/// * `Converged{c}`   → `Verdict::Converged { confidence: c }`
/// * `Diverged{r}`    → `Verdict::Divergence { reason: "diverged-..." }`
/// * `Stalled{..}`    → `Verdict::Continue` (controller will time out
///                       at `max_rounds` if no real convergence occurs)
/// * `Anomaly{..}`    → `Verdict::Divergence { reason: "anomaly-..." }`
pub struct ConsensusAdapter {
    inner: MultiCriteriaDetector,
}

impl ConsensusAdapter {
    /// Construct from a validated [`ConsensusConfig`].
    ///
    /// # Errors
    /// Propagates `ConsensusConfig::validate` failures.
    pub fn new(cfg: ConsensusConfig) -> Result<Self> {
        Ok(Self {
            inner: MultiCriteriaDetector::new(cfg)?,
        })
    }
}

impl SwarmConsensusDetector for ConsensusAdapter {
    fn evaluate(&mut self, _round: u32, stats: &FusionStats) -> Verdict {
        match self.inner.ingest(stats) {
            Ok(ConsensusVerdict::Continue) => Verdict::Continue,
            Ok(ConsensusVerdict::Converged { confidence }) => Verdict::Converged { confidence },
            Ok(ConsensusVerdict::Diverged { .. }) => Verdict::Divergence {
                reason: "multi-criteria-diverged",
            },
            Ok(ConsensusVerdict::Stalled { .. }) => Verdict::Continue,
            Ok(other) => {
                // Future-proof: any variant we do not know yet becomes
                // Divergence so the controller aborts rather than loops.
                tracing::warn!(verdict = ?other, "ConsensusAdapter: unhandled verdict");
                Verdict::Divergence {
                    reason: "unhandled-verdict",
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "ConsensusAdapter: ingest failed");
                Verdict::Divergence {
                    reason: "ingest-error",
                }
            }
        }
    }

    fn reset(&mut self) {
        self.inner.reset();
    }
}

// =====================================================================
// PassThroughDissolver
// =====================================================================

/// Minimal dissolver: returns the centroid captured by the last fusion
/// round. The projection is performed by the controller via the
/// `AlignmentProjector`; the decode step returns the raw vector so
/// the **client** can apply whatever decoder it wants (nearest-anchor
/// cosine, k-NN fallback, custom scoring).
///
/// This is the simplest, most composable choice for **Mode 1**: we do
/// not pre-bind an anchor list inside the server, so the same EFE
/// deployment can serve many catalogs / tenants.
#[derive(Clone, Debug)]
pub struct PassThroughDissolver {
    last_centroid: LastCentroid,
}

impl PassThroughDissolver {
    /// Construct from a shared centroid slot.
    #[must_use]
    pub fn new(last_centroid: LastCentroid) -> Self {
        Self { last_centroid }
    }
}

#[async_trait]
impl Dissolver for PassThroughDissolver {
    async fn extract_centroid(&self, _els: &EpistemicLatentSpace) -> Result<Vec<f32>> {
        self.last_centroid.take().ok_or(EfeError::Internal(
            "PassThroughDissolver: no centroid captured (no fusion round ran?)",
        ))
    }

    async fn decode(&self, centroid: &[f32], _mode: &DecodeMode) -> Result<DecodedOutput> {
        // Always return the raw projected centroid in Mode 1.
        // Client-side code does the nearest-anchor cosine + threshold
        // policy.
        Ok(DecodedOutput::Raw(centroid.to_vec()))
    }
}

// =====================================================================
// ElsCarriedDissolver — Couche 3 M1.c, pure-Φ-by-construction dissolver
// =====================================================================

/// Dissolver that reads the centroid **from the ELS it is handed**
/// (`els.dissolution_centroid()`), rather than from a controller-wide
/// [`LastCentroid`] slot.
///
/// # Why this exists
///
/// [`PassThroughDissolver`] and the [`FusionAdapter`] communicate the
/// fused centroid through a shared `Arc<Mutex<Option<Vec<f32>>>>` that
/// lives for the whole life of the controller. Under **sequential**
/// `execute()` calls that is self-correcting (publish overwrites, take
/// drains), but two `phi_once` calls running **concurrently** on the same
/// `Arc<SwarmController>` race on it: B's publish can overwrite the slot
/// before A's take, so A decodes B's centroid (or `None → Err`). `Φ` is
/// then pure only by single-flight discipline, not by construction.
///
/// This dissolver closes that hole for the meta-swarm *latent* controller.
/// The ELS is spawned fresh by genesis on every `execute()` and is the
/// only per-request object shared between fusion and dissolution, so a
/// centroid stashed on it (by [`FusionAdapter`] built with
/// `with_centroid_carry(true)`) cannot be seen by any other in-flight
/// call. `Φ` becomes a pure function of its input for concurrent
/// evaluators — the contract the M1.c empirical-Lipschitz canary needs.
///
/// Only wired on the latent path; the server request path keeps
/// [`PassThroughDissolver`] unchanged (byte-identical).
#[derive(Clone, Debug, Default)]
pub struct ElsCarriedDissolver;

#[async_trait]
impl Dissolver for ElsCarriedDissolver {
    async fn extract_centroid(&self, els: &EpistemicLatentSpace) -> Result<Vec<f32>> {
        els.dissolution_centroid().map(<[f32]>::to_vec).ok_or(
            EfeError::Internal(
                "ElsCarriedDissolver: no centroid on ELS (no fusion round ran?)",
            ),
        )
    }

    async fn decode(&self, centroid: &[f32], _mode: &DecodeMode) -> Result<DecodedOutput> {
        // Identical decode contract to `PassThroughDissolver`: return the
        // raw projected centroid so the client applies its own policy.
        Ok(DecodedOutput::Raw(centroid.to_vec()))
    }
}

// =====================================================================
// AlignmentAdapter — bridges swarm::AlignmentProjector ↔ core
// =====================================================================

/// Adapter between the controller-facing `swarm::strategies::AlignmentProjector`
/// (`project(centroid, cone) -> (Vec<f32>, f32)`) and the core
/// `dissolution::alignment::AlignmentProjector` (`project(centroid,
/// anchors, cone_radius, cfg) -> ProjectionOutcome`).
///
/// The extraction config is immutable and validated at build time; the
/// adapter is cheap to clone (stateless + `Arc`-wrapped upstream).
pub struct AlignmentAdapter {
    inner: Arc<dyn CoreAlignmentProjector>,
    extraction_cfg: ExtractionConfig,
}

impl AlignmentAdapter {
    /// Construct a Mode-1 adapter around the strict projector with the
    /// default extraction config.
    ///
    /// # Errors
    ///
    /// Propagates `ExtractionConfig::validate` if the default somehow
    /// fails structural checks (should not happen in practice).
    pub fn strict_default() -> Result<Self> {
        let cfg = ExtractionConfig::default();
        cfg.validate()?;
        Ok(Self {
            inner: Arc::new(StrictAlignmentProjector),
            extraction_cfg: cfg,
        })
    }
}

impl std::fmt::Debug for AlignmentAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlignmentAdapter")
            .field("inner", &self.inner.name())
            .finish()
    }
}

impl SwarmAlignmentProjector for AlignmentAdapter {
    fn project(&self, centroid: &[f32], cone: &AlignmentCone) -> Result<(Vec<f32>, f32)> {
        let outcome = self.inner.project(
            centroid,
            &cone.anchors,
            cone.max_projection_distance,
            &self.extraction_cfg,
        )?;
        Ok((outcome.projected_centroid, outcome.distance_from_raw))
    }
}

// =====================================================================
// Mode1Backend — per-request reset wrapper
// =====================================================================

/// `ExecuteBackend` wrapper that resets the `LastCentroid` slot at the
/// *start* of each request, so a previous swarm's centroid cannot leak
/// into the next request if the controller's internal state machine is
/// ever surprised by a genesis / fusion failure before any round has
/// run. Belt-and-suspenders against hard-to-diagnose cross-request
/// pollution.
///
/// **Sprint D7** — also publishes the per-request `SpecTraceCtx` that
/// the `EFE_LOG_CENTROID_TRACE` instrumentation reads, but **only when
/// the env var is set** to keep the hot path free of allocations.
pub struct Mode1Backend {
    inner: SwarmControllerBackend,
    last_centroid: LastCentroid,
    last_spec_trace: LastSpecTrace,
    /// **Sprint D8.5** — Per-request anchors slot consumed by the
    /// `AnchorAwareTemplateRegistry` (when activated). `None` when
    /// the backend was built with `template_registry = Stub` (default).
    anchor_slot: Option<ActiveAnchorsSlot>,
}

impl Mode1Backend {
    /// Wrap a controller-backed `ExecuteBackend` with the centroid-slot
    /// reset hook, the optional D7 spec-trace plumbing, and the
    /// optional D8.5 anchor-slot for per-request NaSem prior biasing.
    #[must_use]
    pub fn new(
        inner: SwarmControllerBackend,
        last_centroid: LastCentroid,
        last_spec_trace: LastSpecTrace,
        anchor_slot: Option<ActiveAnchorsSlot>,
    ) -> Self {
        Self {
            inner,
            last_centroid,
            last_spec_trace,
            anchor_slot,
        }
    }
}

#[async_trait]
impl ExecuteBackend for Mode1Backend {
    async fn execute(&self, req: Request) -> Result<Response> {
        self.last_centroid.reset();

        // **Sprint D8.5** — If the AnchorAware template registry is
        // active, populate the slot with this request's catalog
        // anchors. The genesis spawner will read them when generating
        // the NaSem prior. When `anchor_slot` is `None` (default Stub
        // registry), this branch is a single `if` with a `None`
        // discriminant — negligible cost.
        if let Some(slot) = self.anchor_slot.as_ref() {
            let anchors: Vec<Vec<f32>> = req.spec.alignment_cone.anchors.to_vec();
            slot.set(anchors);
        }

        // Sprint D7 — populate the trace slot only if the operator has
        // explicitly opted in via `EFE_LOG_CENTROID_TRACE=1`. The env
        // var is read once per process via the cached
        // `centroid_trace_enabled` (see its docstring) — subsequent
        // calls are a single relaxed atomic load.
        //
        // When the trace is *disabled*, the slot stays at `None`
        // forever, so the per-round `peek` in `FusionAdapter` returns
        // `None` and skips the cosine computation entirely.
        if centroid_trace_enabled() {
            let problem_embedding = req.spec.problem_embedding.clone();
            let anchors: Vec<Vec<f32>> = req.spec.alignment_cone.anchors.to_vec();
            self.last_spec_trace
                .publish(SpecTraceCtx::new(problem_embedding, anchors));
        } else {
            // The trace is disabled (env var unset at startup) — reset
            // the slot defensively. With the current one-shot cache
            // this branch leaves the slot at `None` (which is its
            // initial state), so the work is technically redundant.
            // We keep it so a *future* dynamic-toggle implementation
            // (e.g. SIGHUP-driven re-read of the env var) can flip
            // back to "trace off" cleanly without a stale spec being
            // logged for a request that isn't asking for traces.
            self.last_spec_trace.reset();
        }
        self.inner.execute(req).await
    }
}

// =====================================================================
// build_mvp_backend — entry point wired from main.rs
// =====================================================================

/// **Sprint D8.5** — Choice of template-registry kind.
///
/// The default is `Stub` which keeps backward compat with all
/// pre-D8 tests and benches (~1 711 tests). The `AnchorAware`
/// variant wraps the stub with a per-request anchor-conditioned
/// NaSem prior — see the architecture document
/// `docs/bench/d8_…architecture.md` §7 for rationale.
///
/// **D8.5 is opt-in only** : enabling AnchorAware is a deliberate
/// decision tied to the calibration audit's H3c-γ findings on a
/// specific catalog. For Mode 1 in production today, `Stub` is
/// the correct default.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum TemplateRegistryKind {
    /// Pre-D8 default — fixed 3-template registry (NaSem, NaLog, NaCrit).
    /// Deterministic, identical priors per template across all requests.
    Stub,
    /// **Sprint D8.5** — Per-request NaSem priors blended with the
    /// catalog anchors centroid. NaLog and NaCrit pass through unchanged.
    AnchorAware {
        /// Blend weight in [0, 1]. 0 = base prior unchanged,
        /// 1 = pure anchor centroid. Default 0.6.
        blend_strength: f32,
        /// Gaussian noise std-dev added to the blended prior. The
        /// noise is **deterministic per-dim** (seeded from the
        /// dimension index, NOT per-request or per-agent) — its
        /// only purpose is to break exact equality between the
        /// prior and the linear combination
        /// `(1-blend)*base + blend*centroid`. See
        /// [`AnchorAwareTemplateRegistry::pseudo_noise`] for the
        /// detailed semantic note. Default `0.05`.
        noise_sigma: f32,
    },
}

impl Default for TemplateRegistryKind {
    fn default() -> Self {
        // **Sprint D8.5** — backward-compat default. AnchorAware is
        // explicitly opt-in via `Mode1Config::with_anchor_aware_templates`.
        Self::Stub
    }
}

/// Configuration exposed to `main.rs`. All fields have MVP defaults.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Mode1Config {
    /// Latent dimension of the essaim (must match the embedder).
    pub dim_latent: u32,
    /// Fusion kernel config (temperature, alpha/beta schedules, …).
    pub fusion: FusionConfig,
    /// Consensus detector config (6-signal weights, thresholds, …).
    pub consensus: ConsensusConfig,
    /// **Sprint D6** — Controller config including the new
    /// `min_rounds` floor. The Mode 1 default sets `min_rounds = 4`
    /// so the fusion runs at least 4 sweeps before any `Converged`
    /// verdict is honoured — the safety belt against premature
    /// convergence (cf. architecture D6 §4.1).
    pub controller: ControllerConfig,
    /// Number of arenas pre-allocated in the memory pool.
    pub prealloc_arenas: usize,
    /// Max simultaneously-alive arenas (== max concurrent swarms).
    pub max_arenas: usize,
    /// Number of CUDA-like streams in the pool (unused on CPU, kept for
    /// forward compat with the GPU backend).
    pub n_streams: usize,
    /// **Sprint D8.5** — Template registry kind (Stub vs AnchorAware).
    /// Defaults to `Stub` for backward compat.
    pub template_registry: TemplateRegistryKind,
}

// =====================================================================
// Sprint D7 — Mode-1-specific FusionConfig (β/α tuning)
// =====================================================================

/// **Sprint D7** — Mode-1-specific [`FusionConfig`] with β/α schedules
/// re-tuned for *classification-by-anchors* usage.
///
/// The library default `FusionConfig::default()` follows an
/// "emancipation" doctrine where the input embedding decays
/// exponentially fast (β₀=0.20, λ_β=0.15) so the swarm self-organises.
/// In Mode 1 this is precisely the wrong contract : the input embedding
/// **is the signature of the class** and must remain dominant across
/// the full fusion campaign.
///
/// The architecture document
/// [`docs/bench/d7_genesis_fusion_bias_remediation_architecture.md`]
/// derives the analytical starting points (β₀=0.45, λ_β=0.05, α₀=0.30).
/// The values below are the **empirical winner** of the 27-point
/// calibration sweep — see
/// [`docs/audits/d7_calibration_report.md`] for the full ranking and
/// the per-catalog measurements.
///
/// | Field | Library default | Analytical D7 | **Empirical winner** | Effect at round 7 |
/// |-------|----------------:|--------------:|---------------------:|-------------------|
/// | `problem_anchor_init` (β₀)        | `0.20` | `0.45` | **`0.45`** | β = 0.45·exp(-0.56) ≈ 0.257 |
/// | `problem_anchor_decay` (λ_β)      | `0.15` | `0.05` | **`0.08`** | faster decay than analytical |
/// | `alignment_strength_init` (α₀)    | `0.50` | `0.30` | **`0.25`** | α = 0.25·(1 - 7/16) ≈ 0.141 |
/// | `alignment_strength_floor` (α_∞)  | `0.05` | `0.05` | `0.05` (unchanged) | preserves alignment-breach detection |
///
/// **Why the empirical winner differs from the analytical** : the
/// sweep on 100 cases × 3 catalogs found that a slightly faster β
/// decay (λ=0.08) combined with slightly less cone pull (α=0.25)
/// yields aggregate_macro_f1 = **0.8150** vs the analytical's
/// 0.7896 (+0.0254 absolute, +3.2 % relative). The improvement is
/// driven by `fraud_alert` (+0.0507 macro_f1) ; `support_triage`
/// (+0.0255) ; `billing_escalation` is unchanged (capped by the
/// residual H3c question→billing confusion).
///
/// The two layers still work in tandem :
/// * **Layer 1 (β)** keeps the input dominant — the swarm is now
///   genuinely conditioned on the user's text rather than drifting
///   toward the stable consensus of the templates.
/// * **Layer 2 (α)** relaxes the convex-hull projection — the cone
///   acts as a regulator, not as a gravity well that pulls every
///   centroid to the same anchor.
///
/// All fields not listed above remain at their library defaults so
/// validators (clip, replay capacity, deadline strictness, etc.) are
/// untouched.
///
/// # Why a private helper rather than a `FusionConfig::default()`
/// rewrite
///
/// `FusionConfig::default()` is consumed by **the entire workspace**,
/// including ~1 690 tests, all benches, and library examples. Rewiring
/// the library default would cascade into hundreds of unrelated
/// failures and would also be semantically wrong : the library default
/// is the right policy for "free swarm" use cases (Mode 2/3 future
/// modes). D7 narrowly carves a Mode-1 override.
///
/// # Anti-regression
///
/// The values are pinned by `tests::d7_fusion_config_pins_tuned_schedules`
/// and `tests::d7_default_uses_tuned_fusion`. Any PR that mutates them
/// without also updating the calibration audit
/// (`docs/audits/d7_calibration_report.md`) will trip the CI gate.
#[must_use]
pub fn build_mode1_fusion_config() -> FusionConfig {
    let mut cfg = FusionConfig::default();

    // -------- Layer 1 (β) — input remains dominant across rounds --
    // β₀ = 0.45 (was 0.20) : at round 0 the latents absorb 45 % of
    // the input embedding instead of 20 %. The post-genesis state
    // is no longer a near-clone of the templates. Empirical winner
    // matches the analytical here.
    cfg.problem_anchor_init = 0.45;
    // λ_β = 0.08 (was 0.15 in library default ; analytical D7 was
    // 0.05). The empirical sweep found that slightly faster β decay
    // (combined with the lower α below) lets the swarm find its
    // local cluster while the input still has 25-32 % weight on
    // the early rounds. At round 7, β = 0.45·exp(-0.56) ≈ 0.257,
    // vs library default's 0.070 (× 3.7 weight on input).
    cfg.problem_anchor_decay = 0.08;

    // -------- Layer 2 (α) — cone relaxed to a regulator role -----
    // α₀ = 0.25 (was 0.50 in library default ; analytical D7 was
    // 0.30). The empirical sweep found that even less cone pull
    // works better : the centroid follows the input first, then
    // the cone trims residual deviation. At round 7, α =
    // 0.25·(1 - 7/16) ≈ 0.141, vs library default's 0.281
    // (× 0.5 cone pull).
    cfg.alignment_strength_init = 0.25;
    // Floor is unchanged at 0.05 — alignment must remain non-zero
    // so the alignment-breach detection (`StrictAlignmentProjector`)
    // continues to work for adversarial inputs (e.g. weather query
    // submitted to a support_triage catalog).

    cfg
}

impl Default for Mode1Config {
    fn default() -> Self {
        // ─────────────────────────────────────────────────────────
        // Sprint D6 — Hardened Mode-1 consensus knobs.
        //
        // The pre-D6 values (confidence_threshold=0.50,
        // hysteresis_rounds=2, variance_threshold=0.05,
        // entropy_fraction_max=0.98, centroid_drift_max=0.20,
        // outlier_ratio_max=0.30) were chosen to *avoid* the
        // ConvergenceTimeout on hyper-converging essaims. They
        // succeeded at that — at the cost of converging *too*
        // early.
        //
        // The Sprint D6 audit on a real multilingual support-triage
        // golden set found that with those knobs :
        //
        //   * The detector emitted `Converged` after the minimum
        //     hysteresis (round 2), before the centroid had
        //     evolved meaningfully from its post-genesis state.
        //   * `confidence` was constant 0.5750 across 8 distinct
        //     inputs (the threshold + ε of the composite signal).
        //   * 4 / 8 multilingual support-triage cases were wrongly
        //     classified as `billing`, the convex-hull centre.
        //
        // The new values below tighten the knobs so a true
        // convergence is earned by several rounds of fusion,
        // and combine with `controller.min_rounds = 4` (also new
        // in D6) for a defence-in-depth that holds even if a
        // future tenant config relaxes the consensus thresholds
        // again.
        //
        // Calibration rationale (architecture D6 §4.2) :
        //   * confidence_threshold 0.50 → 0.65 :
        //         force the composite signal above the median.
        //   * hysteresis_rounds 2 → 3 :
        //         3 frames consecutively above threshold means
        //         the trajectory is stable, not a single noise
        //         spike.
        //   * variance_threshold 0.05 → 0.02 :
        //         the post-genesis variance is typically 0.04,
        //         which trivially satisfied the 0.05 bar.
        //   * entropy_fraction_max 0.98 → 0.95 :
        //         attention entropy must drop by 5 % (real
        //         clustering) not 2 % (noise).
        //   * centroid_drift_max 0.20 → 0.10 :
        //         the centroid must be stable to 10 % between
        //         rounds, indicating settled consensus.
        //   * outlier_ratio_max 0.30 → 0.20 :
        //         at most 20 % of agents in outlier ring, so the
        //         majority is genuinely aligned.
        let mut consensus = ConsensusConfig::default();
        consensus.confidence_threshold = 0.65; // D6 : was 0.50
        consensus.hysteresis_rounds = 3; // D6 : was 2
        consensus.variance_threshold = 0.02; // D6 : was 0.05
        consensus.entropy_fraction_max = 0.95; // D6 : was 0.98
        consensus.centroid_drift_max = 0.10; // D6 : was 0.20
        consensus.outlier_ratio_max = 0.20; // D6 : was 0.30

        // D6 controller floor — the safety belt that protects
        // against any future tenant config which might re-loosen
        // the consensus thresholds. `min_rounds = 4` guarantees
        // the centroid has been swept at least 4 times before
        // any `Converged` verdict is honoured. Cf. architecture
        // D6 §4.1 for the choice of 4.
        let mut controller = ControllerConfig::default();
        controller.min_rounds = 4;

        Self {
            dim_latent: 384, // matches sentence-transformers/all-MiniLM-L6-v2
            // **Sprint D7** — switch from the library `FusionConfig::default()`
            // (β₀=0.20, λ_β=0.15, α₀=0.50) to the Mode-1-tuned schedule.
            // The frozen winner is the empirical post-sweep tuple
            // `(β₀=0.45, λ_β=0.08, α₀=0.25)` (cf.
            // `docs/audits/d7_calibration_report.md` § 6.1, sub-grid
            // ranking #1). The analytical pre-sweep starting point
            // was `(0.45, 0.05, 0.30)` ; the sweep improved aggregate
            // macro_f1 by ~0.0254. See [`build_mode1_fusion_config`]
            // for the runtime values and
            // `docs/bench/d7_genesis_fusion_bias_remediation_architecture.md`
            // for the diagnostic that motivated the change.
            fusion: build_mode1_fusion_config(),
            consensus,
            controller,
            prealloc_arenas: 4,
            max_arenas: 8,
            n_streams: 4,
            // **Sprint D8.5** — Stub by default. AnchorAware is opt-in
            // via [`Self::with_anchor_aware_templates`].
            //
            // **D8 finalization 2026-04-27** : decision finalised to
            // **stay opt-in indefinitely** (DT-D8.1 closed in
            // `docs/audits/d8_sprint_closure.md` § 5.1). Activating by
            // default would conflict with D11 per-tenant isolation
            // because `ActiveAnchorsSlot` is backend-global (single-
            // flight semantics, see its docstring) — concurrent
            // requests from different tenants would overwrite each
            // other's anchors. A future per-request slot rework is
            // tracked as DT-D8.4. Until that lands, callers that want
            // anchor-modulated NaSem priors must opt in explicitly
            // (e.g. single-tenant dedicated deploys).
            template_registry: TemplateRegistryKind::default(),
        }
    }
}

impl Mode1Config {
    /// Convenience factory for a given embedder dimension.
    #[must_use]
    pub fn for_dim(dim_latent: u32) -> Self {
        Self {
            dim_latent,
            ..Self::default()
        }
    }

    /// **Sprint D8.5** — Builder that flips the template registry to
    /// `AnchorAware`, opt-in. Defaults : `blend_strength = 0.6`,
    /// `noise_sigma = 0.05`. Use `with_anchor_aware_templates_params`
    /// for explicit values.
    #[must_use]
    pub fn with_anchor_aware_templates(mut self) -> Self {
        self.template_registry = TemplateRegistryKind::AnchorAware {
            blend_strength: 0.6,
            noise_sigma: 0.05,
        };
        self
    }

    /// **Sprint D8.5** — Builder with explicit AnchorAware parameters.
    /// `blend_strength` is clamped to `[0, 1]` and `noise_sigma` is
    /// clamped to `≥ 0` by the underlying [`AnchorAwareTemplateRegistry`].
    #[must_use]
    pub fn with_anchor_aware_templates_params(
        mut self,
        blend_strength: f32,
        noise_sigma: f32,
    ) -> Self {
        self.template_registry = TemplateRegistryKind::AnchorAware {
            blend_strength,
            noise_sigma,
        };
        self
    }

    /// **Sprint D7** — Build a [`Mode1Config`] for `dim_latent` and
    /// apply any `EFE_FUSION_*` environment-variable overrides on top
    /// of the calibrated defaults.
    ///
    /// This factory is the entry point used by `main.rs` so the
    /// running server picks up calibration sweeps without a recompile.
    /// The four recognised variables are :
    ///
    /// * `EFE_FUSION_BETA_INIT`   — float in `[0, 1]` overriding `problem_anchor_init`
    /// * `EFE_FUSION_BETA_DECAY`  — non-negative float overriding `problem_anchor_decay`
    /// * `EFE_FUSION_ALPHA_INIT`  — float in `[0, 1]` overriding `alignment_strength_init`
    /// * `EFE_FUSION_ALPHA_FLOOR` — float in `[0, 1]` overriding `alignment_strength_floor`
    ///
    /// Empty / absent variables fall back to the calibrated defaults
    /// produced by [`build_mode1_fusion_config`]. Any malformed value
    /// is silently ignored with a `tracing::warn!` so the server
    /// still boots on a sane configuration. Out-of-range values are
    /// dropped (warning logged), inconsistent `floor > init` pairs
    /// are clamped (warning logged) — see
    /// [`apply_fusion_env_overrides`] for the exact policy.
    ///
    /// # Operational note
    ///
    /// These env vars are intended for **calibration sweeps only**
    /// (run via `eval/scripts/calibrate_mode1.py`). They MUST NOT be
    /// set in production deployments — once the calibration audit
    /// hardcodes the winner in [`build_mode1_fusion_config`], the
    /// production Helm chart should leave `EFE_FUSION_*` unset and
    /// rely on the default schedule.
    #[must_use]
    pub fn from_env_for_dim(dim_latent: u32) -> Self {
        Self::from_env_for_dim_with(dim_latent, std::env::var)
    }

    /// Test-friendly variant of [`Self::from_env_for_dim`] that takes
    /// a custom env lookup. Lets tests hermetically inject env vars
    /// without touching the process environment (which would make
    /// parallel test runs flaky).
    #[must_use]
    pub fn from_env_for_dim_with<F>(dim_latent: u32, lookup: F) -> Self
    where
        F: Fn(&'static str) -> std::result::Result<String, std::env::VarError>,
    {
        let mut cfg = Self::for_dim(dim_latent);
        apply_fusion_env_overrides(&mut cfg.fusion, &lookup);
        cfg
    }
}

/// Apply the 4 `EFE_FUSION_*` calibration env vars on top of an
/// existing `FusionConfig`. Any malformed value is logged and
/// ignored.
///
/// This helper is split out so unit tests can verify the parsing
/// logic in isolation, and so callers that build a `FusionConfig`
/// outside the Mode-1 path (e.g. future Mode-2 calibration tooling)
/// can reuse it.
fn apply_fusion_env_overrides<F>(cfg: &mut FusionConfig, lookup: &F)
where
    F: Fn(&'static str) -> std::result::Result<String, std::env::VarError>,
{
    if let Some(v) = read_unit_float(lookup, "EFE_FUSION_BETA_INIT") {
        cfg.problem_anchor_init = v;
    }
    if let Some(v) = read_nonneg_float(lookup, "EFE_FUSION_BETA_DECAY") {
        cfg.problem_anchor_decay = v;
    }
    if let Some(v) = read_unit_float(lookup, "EFE_FUSION_ALPHA_INIT") {
        cfg.alignment_strength_init = v;
    }
    if let Some(v) = read_unit_float(lookup, "EFE_FUSION_ALPHA_FLOOR") {
        cfg.alignment_strength_floor = v;
    }
    // The validator forbids floor > init ; clamp the floor down if
    // the operator supplied an inconsistent pair (we prefer to
    // enforce a sane invariant here rather than crash the server on
    // a calibration typo).
    if cfg.alignment_strength_floor > cfg.alignment_strength_init {
        tracing::warn!(
            floor = cfg.alignment_strength_floor,
            init = cfg.alignment_strength_init,
            "EFE_FUSION_ALPHA_FLOOR > EFE_FUSION_ALPHA_INIT ; clamping floor = init"
        );
        cfg.alignment_strength_floor = cfg.alignment_strength_init;
    }
}

fn read_unit_float<F>(lookup: &F, var: &'static str) -> Option<f32>
where
    F: Fn(&'static str) -> std::result::Result<String, std::env::VarError>,
{
    match lookup(var) {
        Ok(s) if s.trim().is_empty() => None,
        Ok(s) => match s.trim().parse::<f32>() {
            Ok(v) if v.is_finite() && (0.0..=1.0).contains(&v) => Some(v),
            Ok(v) => {
                tracing::warn!(var, value = v, "ignored : value must be in [0, 1]");
                None
            }
            Err(e) => {
                tracing::warn!(var, error = %e, "ignored : not a finite float");
                None
            }
        },
        Err(_) => None,
    }
}

fn read_nonneg_float<F>(lookup: &F, var: &'static str) -> Option<f32>
where
    F: Fn(&'static str) -> std::result::Result<String, std::env::VarError>,
{
    match lookup(var) {
        Ok(s) if s.trim().is_empty() => None,
        Ok(s) => match s.trim().parse::<f32>() {
            Ok(v) if v.is_finite() && v >= 0.0 => Some(v),
            Ok(v) => {
                tracing::warn!(var, value = v, "ignored : value must be ≥ 0");
                None
            }
            Err(e) => {
                tracing::warn!(var, error = %e, "ignored : not a finite float");
                None
            }
        },
        Err(_) => None,
    }
}

/// Build a fully-wired Mode-1 backend.
///
/// Assembles:
///
/// 1. `HostBackend` (CPU-only device backend).
/// 2. `MemoryPool` with `prealloc_arenas` arenas of 1 GiB each.
/// 3. `StreamPool` with `n_streams` streams.
/// 4. `StubTemplateRegistry::mvp_defaults(dim)` — the 3 MVP templates.
/// 5. `GenesisEngine::builder().build_mvp(...)` — meta + router + cardinality + spawner.
/// 6. `MvpCpuFusion::with_replay(backend)` — CPU fusion kernel with replay.
/// 7. `FusionAdapter` — bridges swarm ↔ core traits + captures centroid.
/// 8. `ConsensusAdapter` → `MultiCriteriaDetector` (fresh per request).
/// 9. `PassThroughDissolver` — reads captured centroid.
/// 10. `StrictAlignmentProjector` — strict delta-check.
/// 11. `SwarmController::builder().build()`.
/// 12. Wrap in `Mode1Backend` which resets the centroid slot per request.
///
/// # Errors
///
/// Propagates any pool / registry / fusion / controller construction
/// failure as [`EfeError`] (converted back to `anyhow::Error` by the
/// caller in `main.rs`).
/// `latent_init_sigma`:
///   * `None` — the server Mode-1 request path (real MultiCriteria detector +
///     `SpawnerConfig::default()`, i.e. `init_sigma = 0.02`). Byte-identical to
///     the pre-Couche-3 build.
///   * `Some(sigma)` — the Couche 3 latent controller (converge-now detector +
///     single-round Φ), built with genesis `init_sigma = sigma`. `build_mvp_controller`
///     passes `Some(0.0)` (deterministic, noise-free Φ — M1.b); the value is only
///     ever non-zero via the doc-hidden `build_mvp_controller_with_init_sigma`, used
///     by the determinism test suite as a negative control.
fn build_mvp_parts(
    cfg: Mode1Config,
    latent_init_sigma: Option<f32>,
) -> Result<(
    SwarmController,
    LastCentroid,
    LastSpecTrace,
    Option<ActiveAnchorsSlot>,
)> {
    // --- 1. Device backend ---
    let device_backend: Arc<dyn efe_core::memory::DeviceBackend> = Arc::new(HostBackend::new());

    // --- 2. Memory pool ---
    let mut pool_cfg = PoolConfig::default();
    pool_cfg.prealloc_arenas = cfg.prealloc_arenas;
    pool_cfg.max_arenas = cfg.max_arenas;
    let memory_pool = Arc::new(MemoryPool::new(pool_cfg, device_backend.clone())?);

    // --- 3. Stream pool ---
    let mut stream_cfg = StreamPoolConfig::default();
    stream_cfg.size = cfg.n_streams;
    let stream_pool = Arc::new(StreamPool::new(stream_cfg)?);

    // --- 4. Template registry ---
    //
    // **Sprint D8.5** — When `cfg.template_registry == AnchorAware`,
    // we wrap the Stub in an `AnchorAwareTemplateRegistry`. The
    // wrapper exposes a `slot()` that the `Mode1Backend` populates
    // per request before delegating to the controller. The default
    // path (`Stub`) is preserved bit-for-bit.
    let stub: Arc<dyn TemplateRegistry> =
        Arc::new(StubTemplateRegistry::mvp_defaults(cfg.dim_latent));
    let (template_registry, anchor_slot): (Arc<dyn TemplateRegistry>, Option<ActiveAnchorsSlot>) =
        match cfg.template_registry.clone() {
            TemplateRegistryKind::Stub => (stub, None),
            TemplateRegistryKind::AnchorAware {
                blend_strength,
                noise_sigma,
            } => {
                let slot = ActiveAnchorsSlot::new();
                let aaw = Arc::new(AnchorAwareTemplateRegistry::with_params(
                    stub,
                    slot.clone(),
                    blend_strength,
                    noise_sigma,
                ));
                (aaw as Arc<dyn TemplateRegistry>, Some(slot))
            }
        };

    // --- 5. Genesis (M3) ---
    //
    // **Couche 3 M1.b — deterministic Φ for a clean Lipschitz measurement.** The M1.c
    // empirical-Lipschitz canary measures ‖Φ(x)−Φ(y)‖/‖x−y‖, which requires Φ to be a
    // clean function of x. The one stochastic stage on the Φ path is the genesis noise
    // `h[i] = prior + init_sigma·N(0,1; seed)` with `seed = compute_seed(spec.id)`
    // (`spawner.rs`; note the seed is derived from `spec.id`, NOT the fresh `SwarmId`
    // that `execute()` draws for telemetry — the latter never reaches genesis, see
    // `GenesisEngine::spawn`). Because `phi_once` clones a *fixed* `base_spec`, that seed
    // is constant, so Φ is already reproducible for a given spec at any `init_sigma`.
    // BUT the fixed noise realization ε perturbs the nonlinear pipeline, so (a) Φ depends
    // on `spec.id`, and (b) the ε-perturbation does not cancel in Φ(x)−Φ(y) — it leaves a
    // seed-dependent noise floor that would corrupt the Lipschitz ratio. Injecting a
    // spawner with `init_sigma = 0` short-circuits the RNG entirely (no draw,
    // `spawn_kernel.rs:165`): Φ becomes the noise-free operator — bit-invariant to
    // `spec.id` and clean for the canary. The server request path (`latent_init_sigma =
    // None`) keeps `SpawnerConfig::default()` (init_sigma=0.02) untouched — the production
    // binary stays byte-identical.
    let genesis: Arc<dyn GenesisStrategy> = if let Some(sigma) = latent_init_sigma {
        let mut spawner_cfg = SpawnerConfig::default();
        // M1.b: 0.0 = deterministic Φ (no noise draw); a non-zero value is used ONLY by
        // the determinism suite's negative control (proving noise ⇒ seed-dependent Φ).
        spawner_cfg.init_sigma = sigma;
        let spawner = Arc::new(DefaultSpawner::new(
            memory_pool.clone(),
            template_registry.clone(),
            stream_pool.clone(),
            spawner_cfg,
        ));
        Arc::new(
            GenesisEngine::builder()
                .spawner(spawner)
                .build_mvp(memory_pool, template_registry, stream_pool)?,
        )
    } else {
        Arc::new(GenesisEngine::builder().build_mvp(memory_pool, template_registry, stream_pool)?)
    };

    // --- 6. Fusion core ---
    let fusion_core = Arc::new(
        MvpCpuFusion::builder()
            .with_backend(device_backend.clone())
            .with_replay_enabled(false) // mode-collapse recovery disabled in MVP
            .build()?,
    );

    // --- 7. Centroid channel + fusion adapter ---
    let last_centroid = LastCentroid::new();
    // Sprint D7 — per-request spec snapshot consumed by the
    // `EFE_LOG_CENTROID_TRACE` instrumentation in `FusionAdapter`.
    // The slot is always wired ; the env var simply gates whether
    // it is populated by `Mode1Backend::execute`.
    let last_spec_trace = LastSpecTrace::new();
    // **Couche 3 (M1.c)** — the latent controller carries the fused
    // centroid on the per-execute ELS (making Φ pure under *concurrent*
    // `phi_once`), while the server request path keeps the shared
    // `LastCentroid` publication slot byte-for-byte. This one boolean
    // selects both halves of the channel (adapter write-side here, and the
    // dissolver read-side in step 9).
    let latent = latent_init_sigma.is_some();
    let fusion: Arc<dyn SwarmFusionStrategy> = Arc::new(
        FusionAdapter::new(
            fusion_core,
            cfg.fusion,
            last_centroid.clone(),
            last_spec_trace.clone(),
        )
        .with_centroid_carry(latent),
    );

    // --- 8. Consensus factory + controller config — two profiles ---
    //
    //  * server Mode-1 (`latent_init_sigma = None`) : the real MultiCriteria detector
    //    (`cfg.consensus`) + Mode-1 controller config (min_rounds=4, hysteresis)
    //    — the request path runs to convergence over many rounds.
    //  * **Couche 3 latent controller** (`latent_init_sigma = Some(σ)`) : a participant
    //    step is exactly ONE fusion round (`phi_once` / `step_with_pull` force
    //    `max_rounds = 1`).  The Mode-1 detector needs `hysteresis_rounds = 3`
    //    (D6 ; see `Mode1Config::default`) consecutive high-confidence rounds and
    //    the controller floors at
    //    `min_rounds = 4`, so a single round can NEVER converge — `execute()`
    //    returns `ConvergenceTimeout` on every step (`StubDetector` is no good
    //    either : it *always* returns `Continue`, so it never converges).  We
    //    therefore build a **converge-now** detector : the SAME signal weights /
    //    window as Mode-1 but `confidence_threshold = 0` + `hysteresis_rounds = 1`
    //    (any finite round-1 confidence converges, absent a divergence/NaN gate),
    //    paired with `ControllerConfig::default()` (`min_rounds = 0`).  Meta-swarm
    //    convergence is driven by the coupled Krasnoselskii–Mann OUTER iteration
    //    (`mix_coupled` + the safety projection), not by this inner per-round
    //    detector : the inner round is just the operator `Φ`.
    //
    // The branch is taken on the two *values*, not on two factories. M1.a branched one
    // step too late : an `if`/`else` where each arm built its own closure gave the two
    // arms two distinct closure types, which then had to be unified behind a
    // `Box<dyn Fn ...>` — and the closure body had to be written twice, the second copy
    // being the pre-M1.a one verbatim. That box bought nothing : `consensus_factory<F>`
    // is generic and already stores the factory as `Arc<dyn Fn ...>`
    // (cf. `efe_core::swarm::controller`), so boxing first only added an allocation and
    // a second indirection on the way to it. Selecting the config and the controller
    // config here leaves exactly ONE closure below — the pre-M1.a shape.
    let (consensus_cfg, controller_cfg): (ConsensusConfig, ControllerConfig) =
        if latent_init_sigma.is_some() {
            (
                cfg.consensus
                    .clone()
                    .with_confidence_threshold(0.0)
                    .with_hysteresis(1),
                ControllerConfig::default(),
            )
        } else {
            (cfg.consensus.clone(), cfg.controller.clone())
        };

    // The factory below CANNOT return a `Result` — `SwarmControllerBuilder::consensus_factory`
    // takes `F: Fn() -> Box<dyn ConsensusDetector>`, infallible — so it has to `.expect()`.
    // Until this line, nothing made that expectation true : `Mode1Config::consensus` is a
    // **public field**, no path validated it, and the message
    // `"consensus config validated at build time"` was therefore an assertion about code
    // that did not exist. A malformed `ConsensusConfig` reached the closure intact and
    // panicked on the FIRST REQUEST — inside the controller loop — instead of refusing to
    // boot. Validating here, once, on the exact config the closure captures, is what turns
    // the `.expect()` into an invariant rather than a hope. Fail-closed on purpose : an
    // unusable consensus detector must stop the boot, not degrade the request path.
    consensus_cfg.validate()?;

    let consensus_factory = move || -> Box<dyn SwarmConsensusDetector> {
        Box::new(
            ConsensusAdapter::new(consensus_cfg.clone())
                .expect("consensus config validated above, in build_mvp_parts"),
        )
    };

    // --- 9. Dissolver ---
    //
    // **Couche 3 (M1.c)** — the latent controller reads the centroid from
    // the per-execute ELS (`ElsCarriedDissolver`, race-free by
    // construction under concurrent `phi_once`). The server request path
    // keeps `PassThroughDissolver` on the shared `LastCentroid` slot,
    // byte-identical to the pre-Couche-3 build.
    let dissolver: Arc<dyn Dissolver> = if latent {
        Arc::new(ElsCarriedDissolver)
    } else {
        Arc::new(PassThroughDissolver::new(last_centroid.clone()))
    };

    // --- 10. Alignment ---
    let alignment: Arc<dyn SwarmAlignmentProjector> = Arc::new(AlignmentAdapter::strict_default()?);

    // --- 10bis. Telemetry ---
    //
    // The server wires a `TracingTelemetryCollector` in `main.rs` via
    // `InstrumentedBackend`; here we stick with `Null` because the
    // outer wrapping already covers swarm lifecycle metrics. If we
    // added the Tracing collector here too, we'd double-count.
    let telemetry: Arc<dyn efe_core::swarm::strategies::TelemetryCollector> =
        Arc::new(NullTelemetryCollector);

    // --- 11. Build controller ---
    //
    // (`consensus_factory` + `controller_cfg` were selected in step 8 above,
    // branching on `latent_init_sigma`; the server path keeps Mode-1's own config —
    // D6 `min_rounds=4` — while the latent path uses `ControllerConfig::default()`.)
    let controller = SwarmController::builder()
        .genesis(genesis)
        .fusion(fusion)
        .dissolver(dissolver)
        .alignment(alignment)
        .telemetry(telemetry)
        .consensus_factory(consensus_factory)
        .config(controller_cfg)
        .build()?;

    Ok((controller, last_centroid, last_spec_trace, anchor_slot))
}

/// Build the Mode-1 `ExecuteBackend` (server request path).
pub fn build_mvp_backend(cfg: Mode1Config) -> Result<Arc<dyn ExecuteBackend>> {
    // `None` — the server request path runs to convergence with the real
    // MultiCriteria detector + Mode-1 controller config (D6 `min_rounds=4`),
    // genesis `init_sigma=0.02`. Byte-identical to the pre-Couche-3 build.
    let (controller, last_centroid, last_spec_trace, anchor_slot) = build_mvp_parts(cfg, None)?;
    let backend_inner = SwarmControllerBackend::new(controller);
    let backend: Arc<dyn ExecuteBackend> = Arc::new(Mode1Backend::new(
        backend_inner,
        last_centroid,
        last_spec_trace,
        anchor_slot, // Sprint D8.5 — Some(slot) iff AnchorAware ; None otherwise.
    ));
    Ok(backend)
}

/// **Couche 3 (M1.a)** — the bare `Arc<SwarmController>` used by the meta-swarm's
/// real `LocalParticipant` bridge (`step_with_pull` → `phi_once` → `mix_coupled`).
///
/// Same genesis/fusion/align strategies as [`build_mvp_backend`] **minus** the
/// `ExecuteBackend` wrapper, and with an [`ElsCarriedDissolver`] in place of the
/// server's `PassThroughDissolver`. The controller is self-contained: the centroid
/// crosses from fusion to the dissolver on the **per-execute ELS**
/// (`els.set_dissolution_centroid` → `els.dissolution_centroid`), *not* the
/// controller-wide `LastCentroid` slot, so the discarded outer channel handles are
/// unused on the step path and — crucially — `Φ` is pure under **concurrent**
/// `phi_once` on a shared controller (M1.c), not merely under single-flight.
///
/// **Key difference (`latent_init_sigma = Some(0.0)`)** : a participant step is a **single** fusion
/// round (`phi_once` / `step_with_pull` force `max_rounds = 1`), so this controller is
/// built with a **converge-now** MultiCriteria detector (same signal weights as Mode-1
/// but `confidence_threshold = 0` + `hysteresis_rounds = 1`) + `ControllerConfig::default()`
/// (`min_rounds = 0`). The Mode-1 request profile (D6 `hysteresis_rounds = 3` +
/// `min_rounds = 4`) can never converge in one round — it would return
/// `ConvergenceTimeout` on every step. Convergence of the *meta-swarm* is driven by the
/// coupled Krasnoselskii–Mann outer iteration, not by the inner per-round detector.
///
/// **Honesty (Couche 3)** : participants coordinated this way run efe-core's
/// **numerical** Mode-1 fusion on **stub templates** — real efe-core latent swarms,
/// **not** language-model SLMs (no model weights / tokenizer on the step path ;
/// `problem_embedding` is overwritten by the current latent state). See
/// `docs/architecture/COUCHE3-M1-vrais-participants-ARCHITECTURE.md`.
pub fn build_mvp_controller(cfg: Mode1Config) -> Result<Arc<SwarmController>> {
    // `Some(0.0)` — latent single-round Φ profile (converge-now MultiCriteria +
    // min_rounds=0) with a NOISE-FREE genesis ⇒ deterministic Φ (M1.b).
    build_mvp_controller_with_init_sigma(cfg, 0.0)
}

/// **Test-only escape hatch (Couche 3 M1.b negative control).** Builds the latent
/// controller with an explicit genesis `init_sigma`. Production code must use
/// [`build_mvp_controller`] (`init_sigma = 0`, deterministic Φ); this variant exists
/// so the determinism test suite can build a *noisy* controller (`init_sigma > 0`)
/// and prove the lever has teeth — i.e. that with noise ON, Φ(x) becomes
/// **seed-dependent** (varies with `spec.id`), whereas at `init_sigma = 0` it is
/// seed-invariant. Not for general use: a non-zero value makes Φ non-deterministic.
#[doc(hidden)]
pub fn build_mvp_controller_with_init_sigma(
    cfg: Mode1Config,
    init_sigma: f32,
) -> Result<Arc<SwarmController>> {
    let (controller, _last_centroid, _last_spec_trace, _anchor_slot) =
        build_mvp_parts(cfg, Some(init_sigma))?;
    Ok(Arc::new(controller))
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn d7_cosine_sim_unit_vectors() {
        // Identical unit vectors → cosine = 1.
        let a = vec![1.0_f32, 0.0, 0.0];
        let b = vec![1.0_f32, 0.0, 0.0];
        assert!((cosine_sim(&a, &b) - 1.0).abs() < 1e-6);
        // Orthogonal → 0.
        let c = vec![0.0_f32, 1.0, 0.0];
        assert!(cosine_sim(&a, &c).abs() < 1e-6);
        // Anti-parallel → -1.
        let d = vec![-1.0_f32, 0.0, 0.0];
        assert!((cosine_sim(&a, &d) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn d7_cosine_sim_handles_zero_norm() {
        let a = vec![0.0_f32; 4];
        let b = vec![1.0_f32, 2.0, 3.0, 4.0];
        // Zero norm on either side must yield 0, not NaN.
        assert!(cosine_sim(&a, &b).abs() < 1e-6);
        assert!(cosine_sim(&b, &a).abs() < 1e-6);
        // Mismatched lengths must also yield 0 (defensive).
        let short = vec![1.0_f32, 2.0];
        assert!(cosine_sim(&short, &b).abs() < 1e-6);
    }

    #[test]
    fn d7_last_spec_trace_publish_peek_reset() {
        let slot = LastSpecTrace::new();
        assert!(slot.peek().is_none());
        slot.publish(SpecTraceCtx::new(
            vec![1.0, 2.0, 3.0],
            vec![vec![0.5, 0.5, 0.0], vec![0.0, 1.0, 0.0]],
        ));
        let ctx = slot.peek().expect("peek should return the published ctx");
        assert_eq!(ctx.problem_embedding, vec![1.0, 2.0, 3.0]);
        assert_eq!(ctx.anchors.len(), 2);
        // Peek must not consume — second peek still sees the same data.
        assert!(slot.peek().is_some());
        // Reset clears.
        slot.reset();
        assert!(slot.peek().is_none());
    }

    #[test]
    fn last_centroid_roundtrip() {
        let lc = LastCentroid::new();
        assert!(lc.take().is_none());
        lc.publish(vec![0.1, 0.2, 0.3]);
        let v = lc.take().expect("some");
        assert_eq!(v, vec![0.1, 0.2, 0.3]);
        // `take` removes the value so subsequent reads see None.
        assert!(lc.take().is_none());
    }

    #[test]
    fn last_centroid_reset_clears_slot() {
        let lc = LastCentroid::new();
        lc.publish(vec![1.0]);
        lc.reset();
        assert!(lc.take().is_none());
    }

    #[test]
    fn default_config_has_mvp_dim() {
        let c = Mode1Config::default();
        assert_eq!(c.dim_latent, 384);
        assert_eq!(c.prealloc_arenas, 4);
        assert_eq!(c.max_arenas, 8);
    }

    #[test]
    fn for_dim_overrides_latent() {
        let c = Mode1Config::for_dim(768);
        assert_eq!(c.dim_latent, 768);
    }

    /// Sprint D6 anti-regression : pin the consensus thresholds + the
    /// `controller.min_rounds` floor that constitute the layer-1 + 2
    /// fix for the `0-round consensus` audit finding. Any future code
    /// change that loosens these knobs without an equivalent D6
    /// architecture review will trip this test, forcing the author to
    /// update both. See `docs/audits/d6_accuracy_validation.md`.
    #[test]
    fn d6_default_pins_hardened_consensus_thresholds() {
        let c = Mode1Config::default();
        assert!(
            (c.consensus.confidence_threshold - 0.65).abs() < 1e-6,
            "D6 layer-2 : confidence_threshold must be 0.65 ; got {}",
            c.consensus.confidence_threshold
        );
        assert_eq!(
            c.consensus.hysteresis_rounds, 3,
            "D6 layer-2 : hysteresis_rounds must be 3"
        );
        assert!(
            (c.consensus.variance_threshold - 0.02).abs() < 1e-6,
            "D6 layer-2 : variance_threshold must be 0.02 ; got {}",
            c.consensus.variance_threshold
        );
        assert!(
            (c.consensus.entropy_fraction_max - 0.95).abs() < 1e-6,
            "D6 layer-2 : entropy_fraction_max must be 0.95 ; got {}",
            c.consensus.entropy_fraction_max
        );
        assert!(
            (c.consensus.centroid_drift_max - 0.10).abs() < 1e-6,
            "D6 layer-2 : centroid_drift_max must be 0.10 ; got {}",
            c.consensus.centroid_drift_max
        );
        assert!(
            (c.consensus.outlier_ratio_max - 0.20).abs() < 1e-6,
            "D6 layer-2 : outlier_ratio_max must be 0.20 ; got {}",
            c.consensus.outlier_ratio_max
        );
    }

    #[test]
    fn d6_default_min_rounds_is_four() {
        let c = Mode1Config::default();
        assert_eq!(
            c.controller.min_rounds, 4,
            "D6 layer-1 : Mode1Config must inject min_rounds=4 floor"
        );
    }

    #[test]
    fn d6_for_dim_preserves_min_rounds() {
        // `for_dim` uses `..Self::default()` so the controller field
        // (and its min_rounds) must be inherited intact.
        let c = Mode1Config::for_dim(768);
        assert_eq!(c.controller.min_rounds, 4);
    }

    // ============================================================
    // Sprint D7 — Anti-regression tests for the Mode-1 β/α tuning.
    // ============================================================
    //
    // The 4 tests below pin the values produced by
    // `build_mode1_fusion_config()` and verify that they propagate
    // through every public Mode1Config constructor. Any future PR
    // that quietly reverts to the library defaults (β₀=0.20,
    // λ_β=0.15, α₀=0.50) — for instance via a careless
    // `..FusionConfig::default()` shorthand somewhere upstream —
    // will fail these tests with a precise message pointing at the
    // calibration audit document.

    /// Pin the 4 calibrated knobs of the Mode-1 fusion config.
    /// See `docs/bench/d7_genesis_fusion_bias_remediation_architecture.md`
    /// §6.2 for the rationale and `docs/audits/d7_calibration_report.md`
    /// for the empirical sweep that produced these specific values.
    ///
    /// **Empirical winner (post-D7 sweep, 27 points × 100 cases × 3 catalogs)** :
    /// `(β₀, λ_β, α₀) = (0.45, 0.08, 0.25)` with `aggregate_macro_f1 = 0.8150`.
    #[test]
    fn d7_fusion_config_pins_tuned_schedules() {
        let f = build_mode1_fusion_config();
        assert!(
            (f.problem_anchor_init - 0.45).abs() < 1e-6,
            "D7 layer-1 : problem_anchor_init must be 0.45 ; got {}",
            f.problem_anchor_init
        );
        assert!(
            (f.problem_anchor_decay - 0.08).abs() < 1e-6,
            "D7 layer-1 : problem_anchor_decay must be 0.08 (sweep winner) ; got {}",
            f.problem_anchor_decay
        );
        assert!(
            (f.alignment_strength_init - 0.25).abs() < 1e-6,
            "D7 layer-2 : alignment_strength_init must be 0.25 (sweep winner) ; got {}",
            f.alignment_strength_init
        );
        assert!(
            (f.alignment_strength_floor - 0.05).abs() < 1e-6,
            "D7 layer-2 : alignment_strength_floor must remain 0.05 ; got {}",
            f.alignment_strength_floor
        );
        assert_eq!(
            f.max_rounds, 16,
            "D7 : max_rounds must remain 16 (only β/α schedules are retuned)"
        );
        // The tuned config must still pass the structural validator.
        f.validate()
            .expect("D7 build_mode1_fusion_config must produce a valid FusionConfig");
    }

    /// `Mode1Config::default()` must use the tuned helper, not the
    /// library `FusionConfig::default()`.
    #[test]
    fn d7_default_uses_tuned_fusion() {
        let c = Mode1Config::default();
        assert!(
            (c.fusion.problem_anchor_init - 0.45).abs() < 1e-6,
            "D7 : Mode1Config::default().fusion.problem_anchor_init must be 0.45 \
             — verify that build_mode1_fusion_config() is wired into the default ctor."
        );
        assert!(
            (c.fusion.problem_anchor_decay - 0.08).abs() < 1e-6,
            "D7 : Mode1Config::default().fusion.problem_anchor_decay must be 0.08 \
             (sweep winner)"
        );
        assert!(
            (c.fusion.alignment_strength_init - 0.25).abs() < 1e-6,
            "D7 : Mode1Config::default().fusion.alignment_strength_init must be 0.25 \
             (sweep winner)"
        );
    }

    /// `Mode1Config::for_dim` uses `..Self::default()` so the tuned
    /// fusion field must be inherited intact for every embedder
    /// dimension.
    #[test]
    fn d7_for_dim_preserves_tuned_fusion() {
        for dim in [256_u32, 384, 512, 768, 1024] {
            let c = Mode1Config::for_dim(dim);
            assert!(
                (c.fusion.problem_anchor_init - 0.45).abs() < 1e-6,
                "D7 : for_dim({dim}) lost problem_anchor_init"
            );
            assert!(
                (c.fusion.problem_anchor_decay - 0.08).abs() < 1e-6,
                "D7 : for_dim({dim}) lost problem_anchor_decay"
            );
            assert!(
                (c.fusion.alignment_strength_init - 0.25).abs() < 1e-6,
                "D7 : for_dim({dim}) lost alignment_strength_init"
            );
        }
    }

    // ---- Sprint D7.2 — EFE_FUSION_* override tests --------------

    fn make_env(
        entries: &'static [(&'static str, &'static str)],
    ) -> impl Fn(&'static str) -> std::result::Result<String, std::env::VarError> {
        move |var| {
            entries
                .iter()
                .find(|(k, _)| *k == var)
                .map(|(_, v)| (*v).to_string())
                .ok_or(std::env::VarError::NotPresent)
        }
    }

    #[test]
    fn d7_env_overrides_apply_when_set() {
        let cfg = Mode1Config::from_env_for_dim_with(
            384,
            make_env(&[
                ("EFE_FUSION_BETA_INIT", "0.55"),
                ("EFE_FUSION_BETA_DECAY", "0.03"),
                ("EFE_FUSION_ALPHA_INIT", "0.25"),
                ("EFE_FUSION_ALPHA_FLOOR", "0.04"),
            ]),
        );
        assert!((cfg.fusion.problem_anchor_init - 0.55).abs() < 1e-6);
        assert!((cfg.fusion.problem_anchor_decay - 0.03).abs() < 1e-6);
        assert!((cfg.fusion.alignment_strength_init - 0.25).abs() < 1e-6);
        assert!((cfg.fusion.alignment_strength_floor - 0.04).abs() < 1e-6);
    }

    #[test]
    fn d7_env_overrides_fall_back_to_defaults_when_unset() {
        let cfg = Mode1Config::from_env_for_dim_with(384, make_env(&[]));
        // Defaults must be the D7 calibrated (sweep winner) values.
        assert!((cfg.fusion.problem_anchor_init - 0.45).abs() < 1e-6);
        assert!((cfg.fusion.problem_anchor_decay - 0.08).abs() < 1e-6);
        assert!((cfg.fusion.alignment_strength_init - 0.25).abs() < 1e-6);
        assert!((cfg.fusion.alignment_strength_floor - 0.05).abs() < 1e-6);
    }

    #[test]
    fn d7_env_overrides_ignore_malformed_values() {
        let cfg = Mode1Config::from_env_for_dim_with(
            384,
            make_env(&[
                ("EFE_FUSION_BETA_INIT", "not-a-number"),
                ("EFE_FUSION_ALPHA_INIT", "1.5"),   // out of [0,1]
                ("EFE_FUSION_BETA_DECAY", "-0.10"), // negative
            ]),
        );
        // Each malformed entry must leave the calibrated default
        // intact (defence in depth ; we never want a typo to
        // silently disable the calibration).
        assert!((cfg.fusion.problem_anchor_init - 0.45).abs() < 1e-6);
        assert!((cfg.fusion.alignment_strength_init - 0.25).abs() < 1e-6);
        assert!((cfg.fusion.problem_anchor_decay - 0.08).abs() < 1e-6);
    }

    #[test]
    fn d7_env_overrides_clamp_floor_to_init_when_inconsistent() {
        let cfg = Mode1Config::from_env_for_dim_with(
            384,
            make_env(&[
                ("EFE_FUSION_ALPHA_INIT", "0.20"),
                ("EFE_FUSION_ALPHA_FLOOR", "0.50"),
            ]),
        );
        assert!((cfg.fusion.alignment_strength_init - 0.20).abs() < 1e-6);
        // floor must have been clamped down to init to satisfy the
        // FusionConfig validator (floor ≤ init).
        assert!((cfg.fusion.alignment_strength_floor - 0.20).abs() < 1e-6);
        // The resulting config must still validate cleanly.
        cfg.fusion
            .validate()
            .expect("clamped floor must produce a valid FusionConfig");
    }

    #[test]
    fn d7_env_overrides_treat_empty_string_as_unset() {
        let cfg =
            Mode1Config::from_env_for_dim_with(384, make_env(&[("EFE_FUSION_BETA_INIT", "")]));
        assert!((cfg.fusion.problem_anchor_init - 0.45).abs() < 1e-6);
    }

    // ---- Sprint D8.5 — AnchorAware opt-in tests --------------------

    #[test]
    fn d85_default_template_registry_is_stub() {
        let c = Mode1Config::default();
        assert!(
            matches!(c.template_registry, TemplateRegistryKind::Stub),
            "D8.5 : default must remain Stub (backward compat)"
        );
    }

    #[test]
    fn d85_with_anchor_aware_templates_flips_kind() {
        let c = Mode1Config::default().with_anchor_aware_templates();
        match c.template_registry {
            TemplateRegistryKind::AnchorAware {
                blend_strength,
                noise_sigma,
            } => {
                assert!((blend_strength - 0.6).abs() < 1e-6);
                assert!((noise_sigma - 0.05).abs() < 1e-6);
            }
            other => panic!("expected AnchorAware, got {:?}", other),
        }
    }

    #[test]
    fn d85_with_anchor_aware_templates_params_explicit() {
        let c = Mode1Config::default().with_anchor_aware_templates_params(0.8, 0.1);
        match c.template_registry {
            TemplateRegistryKind::AnchorAware {
                blend_strength,
                noise_sigma,
            } => {
                assert!((blend_strength - 0.8).abs() < 1e-6);
                assert!((noise_sigma - 0.1).abs() < 1e-6);
            }
            _ => panic!("AnchorAware expected"),
        }
    }

    #[test]
    fn d85_for_dim_preserves_template_registry_default() {
        // for_dim uses ..Self::default() so template_registry = Stub.
        let c = Mode1Config::for_dim(768);
        assert!(matches!(c.template_registry, TemplateRegistryKind::Stub));
    }

    #[tokio::test]
    async fn d85_build_mvp_backend_succeeds_with_anchor_aware() {
        let cfg = Mode1Config::default().with_anchor_aware_templates();
        let backend = build_mvp_backend(cfg).expect("build must succeed with AnchorAware opt-in");
        let _ = backend; // smoke test : the wiring compiles + builds.
    }

    /// Realism check : with the tuned β/α and the typical max_rounds
    /// of 16, the **input weight at round 7** (D6's typical
    /// convergence point) must remain ≥ 0.20 — i.e. the input is
    /// still a meaningful driver of the running mix.
    ///
    /// * Pre-D7 schedule produced β=0.07 at round 7 (input ≈ vanished
    ///   under the templates' consensus).
    /// * Analytical D7 (λ_β=0.05) gave β=0.317 — input ~32 %.
    /// * Empirical winner (λ_β=0.08) gives β=0.257 — input ~26 %.
    ///
    /// The threshold at `≥ 0.20` allows the empirical winner to pass
    /// while still rejecting any future PR that reverts β toward the
    /// pre-D7 fast-decay regime (a regression toward β < 0.20 at
    /// round 7 would imply the swarm has emancipated too aggressively
    /// from the input — the bug D7 was created to fix).
    #[test]
    fn d7_input_weight_at_round_seven_stays_dominant() {
        let f = build_mode1_fusion_config();
        // β(r) = β₀ · exp(-λ_β · r)
        let beta_round_7 = f.problem_anchor_init * (-f.problem_anchor_decay * 7.0).exp();
        assert!(
            beta_round_7 >= 0.20,
            "D7 layer-1 : β at round 7 must be ≥ 0.20 (input meaningful) ; \
             got {beta_round_7:.4} — fusion schedule is too aggressive, \
             the swarm will emancipate from the input the way pre-D7 did."
        );
        // α(r) = max(floor, α₀ · (1 - r / R_max))
        let alpha_round_7 = f
            .alignment_strength_floor
            .max(f.alignment_strength_init * (1.0 - 7.0 / f64::from(f.max_rounds) as f32));
        assert!(
            alpha_round_7 <= 0.20,
            "D7 layer-2 : α at round 7 must be ≤ 0.20 (cone is regulator) ; \
             got {alpha_round_7:.4} — cone projection is too strong."
        );
    }

    #[tokio::test]
    async fn build_mvp_backend_succeeds_with_defaults() {
        let backend = build_mvp_backend(Mode1Config::default())
            .expect("build must succeed with MVP defaults");
        // The backend is live but we do not execute a full request here
        // (needs a valid SwarmSpec + anchors). A live smoke test lives
        // in `tests/mode1_live.rs`.
        let _ = backend;
    }

    /// The consensus factory closure `.expect()`s, because the controller's
    /// `consensus_factory` bound is infallible. That `.expect()` is only honest if
    /// something upstream has actually validated the config — and until
    /// `build_mvp_parts` called `validate()?`, nothing did : `Mode1Config::consensus`
    /// is a **public field**, so this exact `Mode1Config` used to build fine and then
    /// panic inside the controller loop on the FIRST REQUEST.
    ///
    /// This test is the one that can make the new guard fall. Delete the
    /// `consensus_cfg.validate()?` line and it goes from `Err` to `Ok` — the build
    /// succeeds and the panic moves to request time, where it is a 500 and a
    /// restart rather than a refused boot.
    #[tokio::test]
    async fn an_invalid_consensus_config_refuses_the_build_instead_of_panicking_per_request() {
        // `hysteresis_rounds = 0` is rejected by `ConsensusConfig::validate`
        // ("must be ≥ 1"), and is reachable because the field is public.
        let mut cfg = Mode1Config::default();
        cfg.consensus = cfg.consensus.clone().with_hysteresis(0);

        // `expect_err` is unavailable here : neither `Arc<dyn ExecuteBackend>` nor
        // `Arc<SwarmController>` is `Debug`, so the Ok arm has to be discarded by hand.
        let err = match build_mvp_backend(cfg) {
            Ok(_) => panic!("an unvalidatable consensus config must NOT produce a live backend"),
            Err(e) => e,
        };
        assert!(
            matches!(err, EfeError::InvalidConfig(_)),
            "the refusal must name the config as the cause ; got {err:?}"
        );
        assert!(
            format!("{err}").contains("hysteresis_rounds"),
            "the refusal must say WHICH knob is wrong ; got {err}"
        );

        // The latent (Couche 3) profile rewrites `hysteresis_rounds` to 1 on its way
        // to the converge-now detector, so this particular knob is repaired there —
        // the branch that must still fail closed is one the rewrite does not touch.
        let mut latent_cfg = Mode1Config::default();
        latent_cfg.consensus = latent_cfg.consensus.clone().with_window(1); // history_window < 2
        let latent_err = match build_mvp_controller(latent_cfg) {
            Ok(_) => panic!("the latent controller must fail closed on its own config too"),
            Err(e) => e,
        };
        assert!(
            matches!(latent_err, EfeError::InvalidConfig(_)),
            "latent refusal must be InvalidConfig ; got {latent_err:?}"
        );
    }
}
