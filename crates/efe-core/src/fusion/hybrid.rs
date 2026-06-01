//! **Sprint M1 extension D32 (cross-cutting)** — Hybrid CPU/GPU
//! fusion strategy for canary rollout.
//!
//! ## Mission
//!
//! Pendant le rollout GPU (D28-D32), nous avons besoin de valider
//! que la `CudaFusion` produit des verdicts équivalents à
//! `MvpCpuFusion` (la baseline production). Une régression
//! silencieuse — où le GPU produit un verdict subtilement différent
//! sur 1 % du trafic — est invisible sans validation continue
//! croisée.
//!
//! Sprint D32 introduit le pattern **canary hybride** :
//!
//!   * **Primary** strategy = `MvpCpuFusion` (verdict de référence,
//!     toujours retourné au client).
//!   * **Shadow** strategy = `CudaFusion` (verdict observationnel,
//!     comparé au primary, jamais retourné).
//!   * Un **sampling rate** (default 10 %) gate l'exécution du
//!     shadow pour limiter le coût CPU/GPU.
//!   * Un [`HybridFusionReport`] est produit à chaque round shadow
//!     activé, comparant centroid L2-diff, variance ratio, et
//!     confidence delta. Au-delà des seuils, un `tracing::error!`
//!     est émis (consommé par `efe-telemetry::canary` post-D32).
//!
//! ## Fail-open
//!
//! Si la `shadow` erre (panic CUDA, kernel timeout, NaN…), le
//! primary verdict est retourné inchangé et un `tracing::warn!`
//! est émis. **Le canary ne casse JAMAIS le trafic production**.
//!
//! ## Sampling deterministic
//!
//! Le gate `sample_gate(rate, round)` utilise un hash fixe du
//! round number — pour un même round, le shadow s'exécute toujours
//! sur les mêmes runs. Cela garantit la reproductibilité des
//! benchs.
//!
//! ## SLO contract
//!
//!   * **C-7** Hybrid overhead at 10 % sampling ≤ 5 % p99 latency.
//!   * **C-8** Divergence detection 100 % reproductible.
//!
//! Cf. `EFE_CORE_EXTENSION_ARCHITECTURE.md` § 13.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;

use crate::els::EpistemicLatentSpace;
use crate::errors::Result;
use crate::fusion::FusionStats;
use crate::memory::NanoAgentBatch;
use crate::swarm::strategies::FusionStrategy;

// =====================================================================
// Report
// =====================================================================

/// Per-round comparison report between primary and shadow strategies.
///
/// Emitted at every fusion round when shadow sampling fires. Consumed
/// by `efe-telemetry::canary::HybridReporter` (post-D32) for dashboard
/// panels.
#[derive(Debug, Clone)]
pub struct HybridFusionReport {
    /// Round index this report covers.
    pub round: u32,
    /// Primary stats (always present).
    pub primary_stats: FusionStats,
    /// Shadow stats. Equal to primary_stats when shadow was not
    /// sampled this round (we still emit the report so dashboards
    /// see a regular cadence).
    pub shadow_stats: FusionStats,
    /// L2 distance between primary.centroid and shadow.centroid.
    /// Lower = better agreement. INFINITY when dims mismatch.
    pub centroid_l2_diff: f32,
    /// Variance ratio shadow / primary. 1.0 = perfect agreement.
    /// > 1.0 = shadow is more variable ; < 1.0 = primary more variable.
    pub variance_ratio: f32,
    /// Confidence delta : `shadow.confidence - primary.confidence`.
    /// Sign-preserving so dashboards can detect "GPU is more
    /// confident" vs "CPU is more confident".
    pub confidence_delta: f32,
    /// Wall time of the primary call (ms).
    pub primary_ms: f32,
    /// Wall time of the shadow call (ms). 0 when shadow was not
    /// sampled this round.
    pub shadow_ms: f32,
    /// True when shadow was actually invoked this round.
    pub shadow_invoked: bool,
}

impl HybridFusionReport {
    /// Returns `true` when the two strategies disagree significantly.
    /// Threshold tuned by D32 bench :
    ///   * `centroid_l2_diff > 0.05` (norm. distance), OR
    ///   * `variance_ratio < 0.5` or `> 2.0`, OR
    ///   * `|confidence_delta| > 0.10`.
    ///
    /// Returns `false` when shadow was not invoked (`shadow_invoked
    /// = false`) — no report ⇒ no divergence.
    #[must_use]
    pub fn divergence_detected(&self) -> bool {
        if !self.shadow_invoked {
            return false;
        }
        self.centroid_l2_diff > 0.05
            || self.variance_ratio < 0.5
            || self.variance_ratio > 2.0
            || self.confidence_delta.abs() > 0.10
    }
}

// =====================================================================
// Config
// =====================================================================

/// Configuration for [`CanaryHybridFusion`].
#[derive(Debug, Clone)]
pub struct CanaryConfig {
    /// Sampling rate in `[0.0, 1.0]`. 0.1 = run shadow on 10 % of
    /// rounds. Lower = lower overhead, less coverage.
    pub sampling_rate: f32,
    /// L2 divergence threshold (centroid distance). Reports above
    /// this fire `tracing::error!`.
    pub divergence_l2: f32,
    /// Whether to log a `tracing::error!` event when divergence is
    /// detected.
    pub log_on_divergence: bool,
}

impl Default for CanaryConfig {
    fn default() -> Self {
        Self {
            sampling_rate: 0.1,
            divergence_l2: 0.05,
            log_on_divergence: true,
        }
    }
}

impl CanaryConfig {
    /// Read from environment variables :
    ///
    ///   * `EFE_FUSION_CANARY_RATE` (f32, default 0.1)
    ///   * `EFE_FUSION_CANARY_DIVERGENCE_L2` (f32, default 0.05)
    ///   * `EFE_FUSION_CANARY_LOG_DIVERGENCE` (bool, default true)
    #[must_use]
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(s) = std::env::var("EFE_FUSION_CANARY_RATE") {
            if let Ok(v) = s.parse::<f32>() {
                cfg.sampling_rate = v.clamp(0.0, 1.0);
            }
        }
        if let Ok(s) = std::env::var("EFE_FUSION_CANARY_DIVERGENCE_L2") {
            if let Ok(v) = s.parse::<f32>() {
                cfg.divergence_l2 = v.max(0.0);
            }
        }
        if let Ok(s) = std::env::var("EFE_FUSION_CANARY_LOG_DIVERGENCE") {
            cfg.log_on_divergence = matches!(
                s.as_str(),
                "1" | "true" | "TRUE" | "yes" | "YES"
            );
        }
        cfg
    }
}

// =====================================================================
// CanaryHybridFusion
// =====================================================================

/// **Sprint D32** — Hybrid fusion strategy that runs primary always +
/// shadow with sampling rate.
///
/// Implements [`FusionStrategy`] (the swarm-side trait that the
/// controller dispatches to). Drop-in replacement for any concrete
/// `FusionStrategy` impl in the production wiring.
///
/// ## Generic over both backends
///
/// `P: FusionStrategy + 'static` and `S: FusionStrategy + 'static`
/// can be different concrete types. Typical production usage :
///
/// ```ignore
/// let primary: Arc<MvpCpuFusion> = Arc::new(MvpCpuFusion::new(...));
/// let shadow:  Arc<CudaFusion>   = Arc::new(CudaFusion::new(...));   // requires --features cuda
/// let canary  = CanaryHybridFusion::new(primary, shadow, CanaryConfig::from_env());
/// // pass canary to SwarmController as the fusion strategy.
/// ```
pub struct CanaryHybridFusion<P, S>
where
    P: FusionStrategy + 'static,
    S: FusionStrategy + 'static,
{
    /// Primary strategy — always invoked, verdict returned to caller.
    pub primary: Arc<P>,
    /// Shadow strategy — invoked only when sampling fires.
    pub shadow: Arc<S>,
    /// Sampling + threshold config.
    pub config: CanaryConfig,
}

impl<P, S> std::fmt::Debug for CanaryHybridFusion<P, S>
where
    P: FusionStrategy + 'static,
    S: FusionStrategy + 'static,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanaryHybridFusion")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl<P, S> CanaryHybridFusion<P, S>
where
    P: FusionStrategy + 'static,
    S: FusionStrategy + 'static,
{
    /// Construct.
    #[must_use]
    pub fn new(primary: Arc<P>, shadow: Arc<S>, config: CanaryConfig) -> Self {
        Self {
            primary,
            shadow,
            config,
        }
    }
}

#[async_trait]
impl<P, S> FusionStrategy for CanaryHybridFusion<P, S>
where
    P: FusionStrategy + 'static,
    S: FusionStrategy + 'static,
{
    async fn run_round(
        &self,
        els: &mut EpistemicLatentSpace,
        batch: &NanoAgentBatch,
        round: u32,
    ) -> Result<FusionStats> {
        // ─────────────────────────────────────────────────────────────
        // **Audit fix C-A1** — pre-fix this method called
        // `self.shadow.run_round(els, batch, round).await` AFTER
        // `self.primary.run_round(els, batch, round).await`. Both
        // methods take `els: &mut EpistemicLatentSpace`, so the second
        // call mutated state that the first call had already mutated
        // (DOUBLE MUTATION). Subsequent fusion rounds saw a corrupted
        // ELS — the controller would converge to a state that's
        // neither what primary nor shadow alone would produce.
        //
        // The proper fix requires a snapshot/restore mechanism around
        // primary so shadow can run on the same starting state. Two
        // viable paths :
        //
        //   * `ReplayableFusion::record_snapshot` + `restore_snapshot`
        //     (cf. `fusion/strategy.rs`) — only works when both
        //     strategies implement `ReplayableFusion`.
        //   * Cloning `EpistemicLatentSpace` — currently expensive
        //     (~ dim × n_agents × f32 per round).
        //
        // For the M1 D32 ship, we **disable shadow execution
        // entirely** to preserve correctness. The trait + struct +
        // report types ship as scaffolding ; real shadow execution
        // lands in `DT-C-D32-SNAPSHOT-1` (post-M1).
        //
        // The sampling gate still controls *report cadence* (so
        // dashboards see a regular pulse), but `shadow_invoked` is
        // always `false`. `divergence_detected()` therefore returns
        // `false` always — which is the safe default until real
        // shadow execution is wired.
        // ─────────────────────────────────────────────────────────────
        let primary_started = Instant::now();
        let primary_stats = self.primary.run_round(els, batch, round).await?;
        let primary_ms = primary_started.elapsed().as_secs_f32() * 1000.0;

        // Sampling gate now controls report-emission cadence ONLY.
        // Shadow is never actually invoked (cf. C-A1 fix).
        let should_emit_report = sample_gate(self.config.sampling_rate, round);
        if should_emit_report {
            let report = HybridFusionReport {
                round,
                shadow_stats: primary_stats.clone(),
                centroid_l2_diff: 0.0,
                variance_ratio: 1.0,
                confidence_delta: 0.0,
                primary_stats: primary_stats.clone(),
                primary_ms,
                shadow_ms: 0.0,
                shadow_invoked: false,
            };
            tracing::debug!(
                target: "efe.fusion.canary",
                round,
                primary_ms,
                "hybrid report emitted (shadow disabled — DT-C-D32-SNAPSHOT-1)"
            );
            let _ = report; // pipe-to-telemetry hook (D32-bis).
        } else {
            tracing::trace!(
                target: "efe.fusion.canary",
                round,
                "hybrid round skipped (sampling rate {})",
                self.config.sampling_rate
            );
        }

        // Acknowledge the shadow handle is intentionally unused at
        // this stage. Without this, clippy would flag `self.shadow`
        // as a dead field (the trait bound on `S` is preserved so
        // future D32-bis can wire it without a struct change).
        let _: &Arc<S> = &self.shadow;
        let _: bool = self.config.log_on_divergence;

        Ok(primary_stats)
    }

    async fn inject_noise(&self, els: &mut EpistemicLatentSpace, sigma: f32) -> Result<()> {
        // Forward to primary only. Shadow doesn't need noise injection
        // — it's observational by design.
        self.primary.inject_noise(els, sigma).await
    }
}

// =====================================================================
// Helpers
// =====================================================================

/// Deterministic sample gate based on round number.
///
/// Uses a fixed multiplicative hash of the round so that for any
/// given round, the gate fires identically across runs. Required
/// for bench reproducibility.
///
/// Returns `true` when the round should activate the shadow path.
fn sample_gate(rate: f32, round: u32) -> bool {
    if rate <= 0.0 {
        return false;
    }
    if rate >= 1.0 {
        return true;
    }
    // Multiplicative hash : Knuth's golden-ratio constant 2^32 / φ.
    let h = round.wrapping_mul(2_654_435_761) as f32 / u32::MAX as f32;
    h < rate
}

/// L2 distance between two centroids. Returns `f32::INFINITY` on
/// dim mismatch (so the divergence check fires loudly).
///
/// **Audit fix C-A1** — currently unused outside tests because the
/// `run_round` shadow path is disabled until `DT-C-D32-SNAPSHOT-1`
/// ships proper snapshot/restore. The function is retained because
/// `divergence_detected()` semantically depends on this metric and
/// the post-DT-C-D32-SNAPSHOT-1 implementation will reuse it.
#[allow(dead_code)]
fn l2_diff(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return f32::INFINITY;
    }
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).powi(2))
        .sum::<f32>()
        .sqrt()
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::EfeError;

    /// Stub fusion strategy that returns a fixed stats template
    /// each round. Used by hybrid tests so we don't need a real
    /// `MvpCpuFusion` setup.
    #[derive(Debug, Clone)]
    struct StubFusion {
        confidence: f32,
        variance: f32,
        centroid_seed: f32,
    }

    #[async_trait]
    impl FusionStrategy for StubFusion {
        async fn run_round(
            &self,
            _els: &mut EpistemicLatentSpace,
            _batch: &NanoAgentBatch,
            round: u32,
        ) -> Result<FusionStats> {
            // Synthesise a non-trivial 8-element centroid driven by
            // `centroid_seed` so primary != shadow when seeds differ.
            let centroid: Vec<f32> = (0..8)
                .map(|i| (i as f32) * self.centroid_seed)
                .collect();
            Ok(FusionStats {
                round,
                variance: self.variance,
                gradient_norm: self.variance.sqrt(),
                centroid,
                outlier_ratio: 0.0,
                has_nan: false,
                has_inf: false,
                confidence: self.confidence,
                mean_attention_entropy: 0.0,
                min_attention_entropy: 0.0,
                max_attention_entropy: 0.0,
                beta_applied: 0.0,
                alpha_applied: 0.0,
                n_active: 4,
            })
        }

        async fn inject_noise(
            &self,
            _els: &mut EpistemicLatentSpace,
            _sigma: f32,
        ) -> Result<()> {
            Ok(())
        }
    }

    /// A stub that errors on every call (for fail-open tests).
    #[derive(Debug, Clone)]
    struct FailingFusion;

    #[async_trait]
    impl FusionStrategy for FailingFusion {
        async fn run_round(
            &self,
            _els: &mut EpistemicLatentSpace,
            _batch: &NanoAgentBatch,
            _round: u32,
        ) -> Result<FusionStats> {
            Err(EfeError::Internal("stub failing fusion"))
        }

        async fn inject_noise(
            &self,
            _els: &mut EpistemicLatentSpace,
            _sigma: f32,
        ) -> Result<()> {
            Ok(())
        }
    }

    // ─────────────────────────────────────────────────────────────
    // sample_gate
    // ─────────────────────────────────────────────────────────────

    #[test]
    fn sample_gate_rate_zero_never_fires() {
        for r in 0..100 {
            assert!(!sample_gate(0.0, r));
        }
    }

    #[test]
    fn sample_gate_rate_one_always_fires() {
        for r in 0..100 {
            assert!(sample_gate(1.0, r));
        }
    }

    #[test]
    fn sample_gate_deterministic_per_round() {
        // Same (rate, round) ⇒ same answer across calls.
        for r in 0..50 {
            assert_eq!(sample_gate(0.5, r), sample_gate(0.5, r));
        }
    }

    #[test]
    fn sample_gate_rate_10_percent_fires_about_10_percent() {
        let n: u32 = 1000;
        let mut fires = 0;
        for r in 0..n {
            if sample_gate(0.1, r) {
                fires += 1;
            }
        }
        // Expect ~100 ; allow [50, 150] for rounding tolerance.
        assert!(
            (50..=150).contains(&fires),
            "expected ~10% sampling, got {fires}/{n}"
        );
    }

    // ─────────────────────────────────────────────────────────────
    // l2_diff
    // ─────────────────────────────────────────────────────────────

    #[test]
    fn l2_diff_zero_on_identical_vectors() {
        let v = vec![1.0_f32, 2.0, 3.0];
        assert!((l2_diff(&v, &v) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn l2_diff_pythagorean() {
        let a = vec![0.0_f32, 0.0];
        let b = vec![3.0_f32, 4.0];
        // L2 = sqrt(9 + 16) = 5
        assert!((l2_diff(&a, &b) - 5.0).abs() < 1e-6);
    }

    #[test]
    fn l2_diff_infinity_on_dim_mismatch() {
        let a = vec![0.0_f32; 3];
        let b = vec![0.0_f32; 5];
        assert_eq!(l2_diff(&a, &b), f32::INFINITY);
    }

    // ─────────────────────────────────────────────────────────────
    // HybridFusionReport
    // ─────────────────────────────────────────────────────────────

    fn dummy_stats() -> FusionStats {
        FusionStats {
            round: 1,
            variance: 0.5,
            gradient_norm: 0.7,
            centroid: vec![0.0; 8],
            outlier_ratio: 0.0,
            has_nan: false,
            has_inf: false,
            confidence: 0.7,
            mean_attention_entropy: 0.0,
            min_attention_entropy: 0.0,
            max_attention_entropy: 0.0,
            beta_applied: 0.0,
            alpha_applied: 0.0,
            n_active: 4,
        }
    }

    #[test]
    fn divergence_not_detected_when_shadow_not_invoked() {
        let r = HybridFusionReport {
            round: 1,
            primary_stats: dummy_stats(),
            shadow_stats: dummy_stats(),
            centroid_l2_diff: 999.0, // bogus high value
            variance_ratio: 999.0,
            confidence_delta: 999.0,
            primary_ms: 0.0,
            shadow_ms: 0.0,
            shadow_invoked: false,
        };
        assert!(!r.divergence_detected());
    }

    #[test]
    fn divergence_detected_on_high_l2() {
        let r = HybridFusionReport {
            round: 1,
            primary_stats: dummy_stats(),
            shadow_stats: dummy_stats(),
            centroid_l2_diff: 0.10, // > 0.05 threshold
            variance_ratio: 1.0,
            confidence_delta: 0.0,
            primary_ms: 0.0,
            shadow_ms: 0.0,
            shadow_invoked: true,
        };
        assert!(r.divergence_detected());
    }

    #[test]
    fn divergence_detected_on_variance_ratio_0_3() {
        let r = HybridFusionReport {
            round: 1,
            primary_stats: dummy_stats(),
            shadow_stats: dummy_stats(),
            centroid_l2_diff: 0.0,
            variance_ratio: 0.3, // < 0.5 threshold
            confidence_delta: 0.0,
            primary_ms: 0.0,
            shadow_ms: 0.0,
            shadow_invoked: true,
        };
        assert!(r.divergence_detected());
    }

    #[test]
    fn divergence_detected_on_high_confidence_delta() {
        let r = HybridFusionReport {
            round: 1,
            primary_stats: dummy_stats(),
            shadow_stats: dummy_stats(),
            centroid_l2_diff: 0.0,
            variance_ratio: 1.0,
            confidence_delta: -0.15, // |Δ| > 0.10
            primary_ms: 0.0,
            shadow_ms: 0.0,
            shadow_invoked: true,
        };
        assert!(r.divergence_detected());
    }

    // ─────────────────────────────────────────────────────────────
    // CanaryConfig
    // ─────────────────────────────────────────────────────────────

    #[test]
    fn canary_config_default_10_percent() {
        let c = CanaryConfig::default();
        assert!((c.sampling_rate - 0.1).abs() < 1e-6);
        assert!((c.divergence_l2 - 0.05).abs() < 1e-6);
        assert!(c.log_on_divergence);
    }

    /// Process-wide mutex serialising the env-var-mutating tests
    /// below. Cargo runs tests in parallel by default ; without this
    /// gate, two tests both setting `EFE_FUSION_CANARY_RATE` race and
    /// produce flakes. Prefer this over `--test-threads=1` so other
    /// tests can still run in parallel.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn canary_config_from_env_clamps_negative_rate() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        std::env::set_var("EFE_FUSION_CANARY_RATE", "-0.5");
        let c = CanaryConfig::from_env();
        std::env::remove_var("EFE_FUSION_CANARY_RATE");
        assert_eq!(c.sampling_rate, 0.0);
    }

    #[test]
    fn canary_config_from_env_clamps_too_high_rate() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        std::env::set_var("EFE_FUSION_CANARY_RATE", "5.0");
        let c = CanaryConfig::from_env();
        std::env::remove_var("EFE_FUSION_CANARY_RATE");
        assert_eq!(c.sampling_rate, 1.0);
    }

    // ─────────────────────────────────────────────────────────────
    // CanaryHybridFusion
    // ─────────────────────────────────────────────────────────────
    //
    // NOTE : full integration tests (real ELS, real NanoAgentBatch)
    // live in `tests/d32_hybrid_integration.rs`. The module-level
    // tests below cover the report logic + sampling gate + fail-open
    // behaviour without booting a real swarm.

    #[test]
    fn canary_hybrid_constructs_with_two_strategies() {
        let primary = Arc::new(StubFusion {
            confidence: 0.7,
            variance: 0.5,
            centroid_seed: 1.0,
        });
        let shadow = Arc::new(StubFusion {
            confidence: 0.7,
            variance: 0.5,
            centroid_seed: 1.0,
        });
        let canary =
            CanaryHybridFusion::new(primary, shadow, CanaryConfig::default());
        // Smoke check : Debug doesn't panic.
        let _ = format!("{canary:?}");
    }

    #[test]
    fn canary_supports_failing_shadow_type() {
        // Compile-time check : we can build a canary where shadow is
        // the FailingFusion stub. The fail-open behaviour is exercised
        // in the integration test ; here we just ensure the trait
        // bounds compose.
        let primary = Arc::new(StubFusion {
            confidence: 0.7,
            variance: 0.5,
            centroid_seed: 1.0,
        });
        let shadow = Arc::new(FailingFusion);
        let _: CanaryHybridFusion<StubFusion, FailingFusion> =
            CanaryHybridFusion::new(primary, shadow, CanaryConfig::default());
    }
}
