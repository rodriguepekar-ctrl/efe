//! Swarm controller: central orchestrator of the swarm lifecycle.
//!
//! The controller is a small, strategy-driven state machine that:
//! 1. Spawns a batch + ELS via [`GenesisStrategy`].
//! 2. Runs up to `max_rounds` iterations of [`FusionStrategy`], consulting
//!    a [`ConsensusDetector`] after each round.
//! 3. Extracts a centroid via [`Dissolver`], projects it via
//!    [`AlignmentProjector`], decodes it.
//! 4. Reports telemetry through a [`TelemetryCollector`].
//!
//! The controller itself is stateless between requests: each `execute()`
//! call opens its own arbiter session and telemetry guard.

use std::sync::Arc;
use std::time::Instant;

use crate::errors::{EfeError, Result};
use crate::swarm::arbiter::{ArbiterSession, SwarmArbiter};
use crate::swarm::request::{Request, Response};
use crate::swarm::state::SwarmState;
use crate::swarm::strategies::{
    AlignmentProjector, ConsensusDetector, Dissolver, FusionStrategy, GenesisStrategy,
    TelemetryCollector,
};
use crate::swarm::verdict::Verdict;
use crate::types::SwarmId;

/// Policy describing what to do when mode collapse is detected.
#[derive(Copy, Clone, Debug)]
#[non_exhaustive]
pub struct CollapseRecoveryPolicy {
    /// Maximum number of recovery attempts before giving up.
    pub max_retries: u32,
    /// Noise sigma to inject during recovery. Must be `>= 0` and finite.
    pub noise_sigma: f32,
    /// Fraction of agents to respawn (`[0.0, 1.0]` and finite).
    pub respawn_fraction: f32,
}

impl Default for CollapseRecoveryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 2,
            noise_sigma: 0.3,
            respawn_fraction: 0.2,
        }
    }
}

impl CollapseRecoveryPolicy {
    /// **Audit-B8 fix #14**: validate the policy.
    ///
    /// Pre-fix, a negative or NaN `respawn_fraction` would silently
    /// become `respawn_n = 0` (saturating cast). A `respawn_fraction`
    /// above `1.0` would request more agents than the batch holds —
    /// which `respawn_diversify` could mis-handle. We now reject both
    /// at build time so the controller cannot be constructed with an
    /// unsafe policy.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] if `respawn_fraction` is not
    ///   finite or outside `[0, 1]`, or if `noise_sigma` is not
    ///   finite or is negative.
    pub fn validate(&self) -> Result<()> {
        if !self.respawn_fraction.is_finite()
            || self.respawn_fraction < 0.0
            || self.respawn_fraction > 1.0
        {
            return Err(EfeError::InvalidConfig(format!(
                "CollapseRecoveryPolicy.respawn_fraction must be finite and in [0, 1], got {}",
                self.respawn_fraction
            )));
        }
        if !self.noise_sigma.is_finite() || self.noise_sigma < 0.0 {
            return Err(EfeError::InvalidConfig(format!(
                "CollapseRecoveryPolicy.noise_sigma must be finite and >= 0, got {}",
                self.noise_sigma
            )));
        }
        Ok(())
    }
}

/// Top-level controller configuration.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct ControllerConfig {
    /// Policy applied when [`Verdict::ModeCollapse`] is emitted.
    pub mode_collapse_recovery: CollapseRecoveryPolicy,
    /// **Sprint D6** — Minimum number of fusion rounds before the
    /// controller honours a `Verdict::Converged` from the detector.
    ///
    /// Any `Converged` verdict emitted at round < `min_rounds` is
    /// downgraded to `Continue`, forcing the loop to keep running.
    /// This is a **safety floor** that protects against a scenario
    /// observed in the Sprint D6 audit on the Mode-1 wiring : the
    /// `MultiCriteriaDetector` was emitting `Converged` after
    /// `hysteresis_rounds=2` rounds — before the centroid had
    /// evolved meaningfully from the post-genesis init — producing
    /// a constant 0.575 confidence and 50 % accuracy on the golden
    /// `support_triage` set despite all latency / memory / compute
    /// SLOs passing.
    ///
    /// `min_rounds = 0` reproduces the pre-D6 behaviour (no floor)
    /// and is preserved for backward compatibility with stub /
    /// integration paths that build a fresh controller without
    /// going through `Mode1Config`.
    ///
    /// **Library default : `0`** — preserves the pre-D6 behaviour
    /// across every test, mock harness, and integration that builds
    /// a `ControllerConfig` via `Default`. The actual D6 fix lives
    /// in `efe_server::mode1::Mode1Config::default()` which sets
    /// `controller.min_rounds = 4` for the Mode-1 profile only.
    ///
    /// Implementation note (deviation from architecture §4.1) :
    /// the architecture proposed a library-wide default of 4 for
    /// safety. Empirically, that broke ~12 tests using single-frame
    /// `ScriptedDetector` + `MockFusion` (round 1 → Converged →
    /// done — the canonical 1-frame integration shape). Forcing
    /// those test rigs to fabricate 4 fusion frames each would
    /// dilute the test intent (alignment / failure injection /
    /// dissolution logic, not consensus pacing). The Mode-1
    /// profile is where the fix actually has business meaning, so
    /// we localised the new default there.
    ///
    /// If `min_rounds > spec.max_rounds`, the loop runs to
    /// `max_rounds` without ever converging and the controller
    /// emits `ConvergenceTimeout` — a degraded-but-safe outcome
    /// signalled with a warning log up front.
    pub min_rounds: u32,
}

// `#[derive(Default)]` produces `min_rounds = 0` (u32::default) and
// `mode_collapse_recovery = CollapseRecoveryPolicy::default()` —
// exactly the legacy-compatible config. The Mode-1 D6 override
// lives in `efe_server::mode1::Mode1Config::default()`.

/// Shared immutable inner state of the controller.
///
/// Strategies are behind `Arc` because they are shared across requests
/// (typically implemented as stateless services).
pub struct ControllerInner {
    genesis: Arc<dyn GenesisStrategy>,
    fusion: Arc<dyn FusionStrategy>,
    dissolver: Arc<dyn Dissolver>,
    alignment: Arc<dyn AlignmentProjector>,
    telemetry: Arc<dyn TelemetryCollector>,
    arbiter: SwarmArbiter,
    config: ControllerConfig,
}

/// Central swarm controller.
///
/// Constructed via [`SwarmControllerBuilder`]. `execute()` drives one
/// request to completion.
pub struct SwarmController {
    inner: Arc<ControllerInner>,
    /// The consensus detector is mutable-per-request: the controller
    /// takes ownership of a fresh boxed detector each time.
    detector_factory: Arc<dyn Fn() -> Box<dyn ConsensusDetector> + Send + Sync>,
}

impl SwarmController {
    /// Start building a controller.
    #[must_use]
    pub fn builder() -> SwarmControllerBuilder {
        SwarmControllerBuilder::default()
    }

    /// Shareable clone of the controller for concurrent worker tasks.
    #[must_use]
    pub fn clone_handle(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            detector_factory: self.detector_factory.clone(),
        }
    }

    /// Run a request end-to-end.
    ///
    /// # Errors
    ///
    /// Returns any terminal or fatal [`EfeError`]. Recoverable errors are
    /// handled internally.
    #[tracing::instrument(
        skip(self, req),
        fields(
            request_id = %req.id,
            tenant_id = %req.tenant_id,
            swarm_id = tracing::field::Empty,
            state = tracing::field::Empty,
        )
    )]
    pub async fn execute(&self, req: Request) -> Result<Response> {
        // Validate up-front to reject malformed specs fast.
        req.spec.validate()?;

        let t0 = Instant::now();
        let swarm_id = SwarmId::new();
        tracing::Span::current().record("swarm_id", tracing::field::display(swarm_id));
        tracing::Span::current().record("state", SwarmState::Pending.label());

        let mut telemetry = self.inner.telemetry.begin_swarm(swarm_id, &req);
        let arbiter = self.inner.arbiter.new_session(req.spec.budget);

        // Drive the state machine.
        let outcome = self
            .drive(&req, swarm_id, &arbiter, telemetry.as_mut(), t0)
            .await;

        // Finalize telemetry regardless of outcome.
        let total_ms = t0.elapsed().as_secs_f32() * 1000.0;
        match &outcome {
            Ok(resp) => {
                // Build synthetic stats for finalize (real finalize uses actual stats).
                use crate::fusion::FusionStats;
                let dim = resp
                    .output
                    .as_raw()
                    .map_or_else(|| req.spec.dim_latent.get() as usize, <[f32]>::len);
                let stats = FusionStats::synthetic_converging(resp.rounds_executed, dim)
                    .with_confidence(resp.confidence);
                telemetry.finalize(total_ms, &stats);
            }
            Err(err) => {
                telemetry.record_phase_error("swarm", err);
                // **Audit-B8 fix #18**: stamp the tracing span with the
                // terminal `Failed` state so external observers (eg.
                // OpenTelemetry collectors) see a coherent state
                // transition Pending → ... → Failed instead of leaving
                // the span on the last successful intermediate state.
                tracing::Span::current()
                    .record("state", SwarmState::Failed.label());
            }
        }

        outcome
    }

    /// **Sprint A1 Phase 1** — one Krasnosel'skii-Mann coupled step
    /// toward an external pull vector.
    ///
    /// Computes
    ///
    /// ```text
    ///   x_new = (1 - α - γ) · current  +  α · Φ(current)  +  γ · pull
    /// ```
    ///
    /// where `Φ(current)` is the controller's *one-round* response to
    /// using `current` as the input problem embedding.  Specifically :
    /// the method clones `base_spec`, sets
    /// `spec.problem_embedding = current` and `spec.max_rounds = 1`,
    /// then invokes [`Self::execute`] with `DecodeMode::PassThrough` to
    /// recover the raw latent output.  The mix is then delegated to the
    /// pure [`crate::swarm::coupled::mix_coupled`].
    ///
    /// This method exists for the hierarchical meta-swarm
    /// (`efe-meta-swarm` crate) and is **strictly additive** to the
    /// `SwarmController` public API.  Callers that do not use the
    /// meta-swarm are unaffected.
    ///
    /// # Semantic note (Phase 1)
    ///
    /// `Φ(x)` as computed here corresponds to *"one fusion round of
    /// the controller's pipeline seeded with x as the input query"*.
    /// It is the closest faithful proxy to a strict mathematical
    /// Krasnosel'skii-Mann operator that can be obtained without
    /// modifying [`Self::execute`]'s internal drive loop.  Phase 2
    /// will introduce a dedicated single-step kernel — bypassing
    /// genesis / dissolution / alignment when not strictly needed —
    /// gated behind a separate optimisation feature.  Until then,
    /// convergence guarantees of the meta-swarm hold under the
    /// empirical non-expansiveness of `Φ` on the operational domain
    /// (verified by `efe-prove` at certification time).
    ///
    /// # Parameters
    ///
    /// * `current` — current latent state (length must equal
    ///   `base_spec.dim_latent`).  Must contain only finite values.
    /// * `pull`    — meta-consensus pull vector (same length).  Must
    ///   contain only finite values.
    /// * `alpha`   — relaxation parameter, `α ∈ [0, 1)`.
    /// * `gamma`   — coupling parameter, `γ ∈ [0, 1)`.  Must satisfy
    ///   `α + γ < 1`.
    /// * `base_spec` — the [`SwarmSpec`] template used to build the
    ///   internal single-round request.  Its `problem_embedding` and
    ///   `max_rounds` are overridden ;  everything else is preserved.
    /// * `tenant_id` — propagated for telemetry attribution.
    ///
    /// # Returns
    ///
    /// The next latent state, of length `base_spec.dim_latent`.
    ///
    /// # Errors
    ///
    /// * [`EfeError::InvalidConfig`] if `current` or `pull` have wrong
    ///   length, contain non-finite values, or if `(α, γ)` violate the
    ///   convexity invariant.
    /// * Any error returned by the underlying [`Self::execute`] call
    ///   (e.g. budget exhaustion, mode collapse).
    /// * [`EfeError::InvalidConfig`] if the controller returns a
    ///   `DecodedOutput::Text` (impossible under `PassThrough` decode
    ///   but defensively checked).
    #[allow(clippy::too_many_arguments)] // 7 args is the natural KM signature
    pub async fn step_with_pull(
        &self,
        current: &[f32],
        pull: &[f32],
        alpha: f64,
        gamma: f64,
        base_spec: &crate::types::SwarmSpec,
        tenant_id: &str,
    ) -> Result<Vec<f32>> {
        let dim = base_spec.dim_latent.get() as usize;
        if current.len() != dim {
            return Err(EfeError::InvalidConfig(format!(
                "step_with_pull: current length {} != spec dim_latent {dim}",
                current.len()
            )));
        }
        if pull.len() != dim {
            return Err(EfeError::InvalidConfig(format!(
                "step_with_pull: pull length {} != spec dim_latent {dim}",
                pull.len()
            )));
        }
        // Eagerly validate the coupling coefficients and the finiteness
        // of the inputs so the caller gets the failure synchronously,
        // *before* any await point.  efe-core's SwarmSpec::validate
        // would catch non-finite values later anyway, but the message
        // is clearer when raised here.
        if !alpha.is_finite() || !gamma.is_finite()
           || alpha < 0.0 || gamma < 0.0 || alpha + gamma >= 1.0 {
            return Err(EfeError::InvalidConfig(format!(
                "step_with_pull: invalid coupling (alpha={alpha}, gamma={gamma})"
            )));
        }
        if !current.iter().all(|x| x.is_finite()) {
            return Err(EfeError::InvalidConfig(
                "step_with_pull: current contains NaN or Inf".into(),
            ));
        }
        if !pull.iter().all(|x| x.is_finite()) {
            return Err(EfeError::InvalidConfig(
                "step_with_pull: pull contains NaN or Inf".into(),
            ));
        }
        // Φ(current) = one fusion round seeded with `current`, isolated
        // from the coupling term (Couche 3 M1.a — see `phi_once`).
        let phi_x = self.phi_once(current, base_spec, tenant_id).await?;
        // Mix : (1 - α - γ) · current + α · Φ(current) + γ · pull.
        crate::swarm::coupled::mix_coupled(current, &phi_x, pull, alpha, gamma)
    }

    /// **Couche 3 (M1.a)** — the participant's internal operator `Φ_m(x)`,
    /// **isolated** from the Krasnosel'skii-Mann mix.  Runs one fusion round
    /// seeded with `x` (`spec.problem_embedding = x`, `max_rounds = 1`,
    /// `DecodeMode::PassThrough`) and returns its raw latent output.
    ///
    /// This is the accessor the meta-swarm needs to **measure `Φ`'s
    /// empirical Lipschitz bound** (the H1 guard-rail of M1.c) *without* the
    /// `α`/`γ` coupling that [`Self::step_with_pull`] adds on top.  Keeping a
    /// single implementation of the Φ computation here guarantees the bound
    /// measured is the one actually applied in the coupled step.
    ///
    /// # Determinism (M1.d, out of M1.a scope)
    ///
    /// `execute()` currently draws a fresh `SwarmId`/`RequestId` per call,
    /// seeding the genesis noise (`init_sigma`) — so `Φ(x)` is **not** yet
    /// reproducible bit-for-bit.  M1.a exposes the operator; M1.d makes it
    /// deterministic (seed derived from `participant.seed ⊕ step_index`,
    /// `init_sigma = 0`).
    ///
    /// # Errors
    ///
    /// [`EfeError::InvalidConfig`] if `dim(x) != base_spec.dim_latent`, if
    /// `x` holds NaN/Inf, or if the controller decodes a non-raw output.
    pub async fn phi_once(
        &self,
        x: &[f32],
        base_spec: &crate::types::SwarmSpec,
        tenant_id: &str,
    ) -> Result<Vec<f32>> {
        let dim = base_spec.dim_latent.get() as usize;
        if x.len() != dim {
            return Err(EfeError::InvalidConfig(format!(
                "phi_once: x length {} != spec dim_latent {dim}",
                x.len()
            )));
        }
        if !x.iter().all(|v| v.is_finite()) {
            return Err(EfeError::InvalidConfig(
                "phi_once: x contains NaN or Inf".into(),
            ));
        }
        // Build a single-round request seeded with `x`.
        let mut spec = base_spec.clone();
        spec.problem_embedding = x.to_vec();
        spec.max_rounds = std::num::NonZeroU32::new(1).expect("1 is non-zero");
        let req = crate::swarm::request::Request::synthetic(
            spec,
            tenant_id.to_string(),
            crate::types::RequestId::new(),
        );
        // Execute one round to get Φ(x).
        let resp = self.execute(req).await?;
        let phi_x = resp.output.as_raw().ok_or_else(|| {
            EfeError::InvalidConfig(
                "phi_once: expected raw output (controller decoded non-pass-through)".into(),
            )
        })?;
        Ok(phi_x.to_vec())
    }

    /// Internal main flow (separated from [`execute`] so telemetry is
    /// always finalized, even on early exit).
    #[allow(clippy::too_many_lines)]
    async fn drive(
        &self,
        req: &Request,
        swarm_id: SwarmId,
        arbiter: &ArbiterSession,
        telemetry: &mut dyn crate::swarm::strategies::TelemetryGuard,
        t0: Instant,
    ) -> Result<Response> {
        // === Phase 1: Genesis ===
        tracing::Span::current().record("state", SwarmState::Genesis.label());
        tracing::debug!("phase=genesis");

        let phase_start = Instant::now();
        let (mut batch, mut els) = self
            .inner
            .genesis
            .spawn(&req.spec)
            .await
            .inspect_err(|e| telemetry.record_phase_error("genesis", e))?;
        telemetry.record_phase_complete("genesis", phase_start.elapsed());

        arbiter.check(t0.elapsed())?;

        // === Phase 2: Fusion rounds ===
        tracing::debug!("phase=fusion");
        let max_rounds = req.spec.max_rounds.get();
        let min_rounds = self.inner.config.min_rounds;

        // D6 sanity warning: a misconfiguration where the floor
        // exceeds the cap leaves the loop unable to converge — the
        // request will inevitably ConvergenceTimeout. We log once
        // up front rather than once per request iteration so the
        // operator gets an actionable signal.
        if min_rounds > max_rounds {
            tracing::warn!(
                min_rounds,
                max_rounds,
                "controller: min_rounds > max_rounds ; convergence \
                 will time out unconditionally — re-tune the spec or \
                 the controller config"
            );
        }

        let mut detector = (self.detector_factory)();
        let mut final_stats = None;
        let mut last_variance = f32::NAN;
        let mut collapse_retries = 0u32;

        for round in 1..=max_rounds {
            tracing::Span::current().record("state", SwarmState::Fusion { round }.label());
            arbiter.check(t0.elapsed())?;

            let stats = self
                .inner
                .fusion
                .run_round(&mut els, &batch, round)
                .await
                .inspect_err(|e| telemetry.record_phase_error("fusion", e))?;
            telemetry.record_round(round, &stats);
            last_variance = stats.variance;

            if stats.has_numerical_issues() {
                return Err(EfeError::NumericalExplosion { round });
            }

            match detector.evaluate(round, &stats) {
                // D6 floor : if the detector is satisfied but we have
                // not yet executed `min_rounds` fusion sweeps, treat
                // the verdict as `Continue`. This protects against
                // premature convergence on essaims whose post-genesis
                // state already satisfies the consensus thresholds —
                // the symptom that motivated Sprint D6.
                //
                // We do NOT break or return : the loop continues so
                // the centroid keeps evolving. The detector's internal
                // state (e.g. `consecutive_high` for the hysteresis)
                // is left intact — it will fire `Converged` again on
                // the very next round if the conditions still hold,
                // and `min_rounds` will admit it once round >= floor.
                Verdict::Converged { confidence } if round < min_rounds => {
                    tracing::debug!(
                        round,
                        min_rounds,
                        confidence,
                        "controller: detector says Converged but \
                         min_rounds floor not yet reached — keep going"
                    );
                    telemetry.record_event(round, "min_rounds_floor_held");
                    continue;
                }
                Verdict::Converged { confidence } => {
                    final_stats = Some(stats.with_confidence(confidence));
                    break;
                }
                Verdict::Continue => continue,
                Verdict::Divergence { reason } => {
                    return Err(EfeError::Divergence {
                        rounds: round,
                        reason,
                    });
                }
                Verdict::ModeCollapse => {
                    let policy = &self.inner.config.mode_collapse_recovery;
                    if collapse_retries >= policy.max_retries {
                        return Err(EfeError::ModeCollapse { round });
                    }
                    collapse_retries += 1;
                    self.recover_from_collapse(&mut els, &mut batch, round, policy)
                        .await?;
                    telemetry.record_event(round, "mode_collapse_recovery");
                    continue;
                }
            }
        }

        let stats = final_stats.ok_or(EfeError::ConvergenceTimeout {
            rounds: max_rounds,
            sigma2: last_variance,
        })?;

        // === Phase 3: Dissolution ===
        tracing::Span::current().record("state", SwarmState::Dissolution.label());
        tracing::debug!("phase=dissolution");
        let phase_start = Instant::now();

        let h_star = self
            .inner
            .dissolver
            .extract_centroid(&els)
            .await
            .inspect_err(|e| telemetry.record_phase_error("dissolution", e))?;

        // Alignment check BEFORE decode — never let a breached centroid
        // reach the decoder. Non-finite deltas (NaN / Inf) are treated as
        // breaches: a projector that produces them has lost its numerical
        // contract and the result cannot be trusted.
        let (h_safe, delta) = self
            .inner
            .alignment
            .project(&h_star, &req.spec.alignment_cone)?;
        let max_delta = req.spec.alignment_cone.max_projection_distance;
        if !delta.is_finite() || delta > max_delta {
            return Err(EfeError::AlignmentBreach {
                delta,
                max: max_delta,
            });
        }

        let output = self
            .inner
            .dissolver
            .decode(&h_safe, &req.decode_mode)
            .await
            .inspect_err(|e| telemetry.record_phase_error("dissolution", e))?;
        telemetry.record_phase_complete("dissolution", phase_start.elapsed());

        // === Phase 4: Cleanup (RAII) ===
        // Drop `els` before `batch`: ELS references batch-owned arena.
        // Neither type implements `Drop`, so the drop order only matters
        // if future implementations add Drop (Sprint 1+). Making it
        // explicit via `let _` + `drop_in_place` would be noisy; simply
        // ensure `els` is last-touched before `batch` via scope exit.
        #[allow(clippy::drop_non_drop)]
        {
            drop(els);
            drop(batch);
        }

        tracing::Span::current().record("state", SwarmState::Done.label());
        let total_ms = t0.elapsed().as_secs_f32() * 1000.0;
        Ok(Response {
            swarm_id,
            request_id: req.id.clone(),
            output,
            confidence: stats.confidence,
            verdict: Verdict::Converged {
                confidence: stats.confidence,
            },
            rounds_executed: stats.round,
            total_ms,
            alignment_delta: delta,
        })
    }

    async fn recover_from_collapse(
        &self,
        els: &mut crate::els::EpistemicLatentSpace,
        batch: &mut crate::memory::NanoAgentBatch,
        round: u32,
        policy: &CollapseRecoveryPolicy,
    ) -> Result<()> {
        tracing::warn!(
            round,
            sigma = policy.noise_sigma,
            fraction = policy.respawn_fraction,
            "mode_collapse recovery"
        );
        self.inner
            .fusion
            .inject_noise(els, policy.noise_sigma)
            .await?;
        let respawn_n = ((batch.count() as f32) * policy.respawn_fraction).round() as u32;
        if respawn_n > 0 {
            self.inner
                .genesis
                .respawn_diversify(batch, respawn_n)
                .await?;
        }
        Ok(())
    }
}

// =====================================================================
// Builder
// =====================================================================

/// Builder for [`SwarmController`]. All strategies are required.
#[derive(Default)]
pub struct SwarmControllerBuilder {
    genesis: Option<Arc<dyn GenesisStrategy>>,
    fusion: Option<Arc<dyn FusionStrategy>>,
    dissolver: Option<Arc<dyn Dissolver>>,
    alignment: Option<Arc<dyn AlignmentProjector>>,
    telemetry: Option<Arc<dyn TelemetryCollector>>,
    arbiter: Option<SwarmArbiter>,
    config: ControllerConfig,
    detector_factory: Option<Arc<dyn Fn() -> Box<dyn ConsensusDetector> + Send + Sync>>,
}

impl SwarmControllerBuilder {
    /// Set the genesis strategy (M3).
    #[must_use]
    pub fn genesis(mut self, g: Arc<dyn GenesisStrategy>) -> Self {
        self.genesis = Some(g);
        self
    }

    /// Set the fusion strategy (M5).
    #[must_use]
    pub fn fusion(mut self, f: Arc<dyn FusionStrategy>) -> Self {
        self.fusion = Some(f);
        self
    }

    /// Set the dissolver (M7).
    #[must_use]
    pub fn dissolver(mut self, d: Arc<dyn Dissolver>) -> Self {
        self.dissolver = Some(d);
        self
    }

    /// Set the alignment projector (M9).
    #[must_use]
    pub fn alignment(mut self, a: Arc<dyn AlignmentProjector>) -> Self {
        self.alignment = Some(a);
        self
    }

    /// Set the telemetry collector (M11). Defaults to a no-op collector.
    #[must_use]
    pub fn telemetry(mut self, t: Arc<dyn TelemetryCollector>) -> Self {
        self.telemetry = Some(t);
        self
    }

    /// Set the arbiter. Defaults to one without energy metering.
    #[must_use]
    pub fn arbiter(mut self, a: SwarmArbiter) -> Self {
        self.arbiter = Some(a);
        self
    }

    /// Provide a factory producing a fresh consensus detector per request.
    #[must_use]
    pub fn consensus_factory<F>(mut self, f: F) -> Self
    where
        F: Fn() -> Box<dyn ConsensusDetector> + Send + Sync + 'static,
    {
        self.detector_factory = Some(Arc::new(f));
        self
    }

    /// Set runtime configuration.
    #[must_use]
    pub fn config(mut self, c: ControllerConfig) -> Self {
        self.config = c;
        self
    }

    /// Build the controller.
    ///
    /// # Errors
    ///
    /// Returns [`EfeError::InvalidConfig`] when a required strategy is
    /// missing or when [`CollapseRecoveryPolicy`] is malformed
    /// (audit-B8 fix #14).
    pub fn build(self) -> Result<SwarmController> {
        // Audit-B8 fix #14: validate the recovery policy before any
        // strategy can observe it. Catching a NaN/out-of-range
        // respawn_fraction at build time prevents downstream `as u32`
        // casts from producing surprising values during a real
        // mode-collapse event.
        self.config.mode_collapse_recovery.validate()?;

        let genesis = self
            .genesis
            .ok_or_else(|| EfeError::InvalidConfig("genesis strategy required".into()))?;
        let fusion = self
            .fusion
            .ok_or_else(|| EfeError::InvalidConfig("fusion strategy required".into()))?;
        let dissolver = self
            .dissolver
            .ok_or_else(|| EfeError::InvalidConfig("dissolver required".into()))?;
        let alignment = self
            .alignment
            .ok_or_else(|| EfeError::InvalidConfig("alignment projector required".into()))?;
        let detector_factory = self
            .detector_factory
            .ok_or_else(|| EfeError::InvalidConfig("consensus factory required".into()))?;

        let telemetry = self
            .telemetry
            .unwrap_or_else(|| Arc::new(crate::swarm::strategies::NullTelemetryCollector));
        let arbiter = self
            .arbiter
            .unwrap_or_else(SwarmArbiter::without_energy_metering);

        Ok(SwarmController {
            inner: Arc::new(ControllerInner {
                genesis,
                fusion,
                dissolver,
                alignment,
                telemetry,
                arbiter,
                config: self.config,
            }),
            detector_factory,
        })
    }
}
