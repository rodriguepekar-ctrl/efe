//! MVP CPU implementation of `FusionStrategy`.
//!
//! Orchestrates one fusion round:
//!
//! 1. Validate config + pre-round ELS.
//! 2. (Optional) snapshot current latent into replay buffer.
//! 3. Acquire `split_mut` over the ELS.
//! 4. Run kernels: scores → softmax → aggregate (into scratch) → swap
//!    → β-mix → project → clip.
//! 5. Drop `split`, compute post-round stats.
//! 6. Validate post-round invariants.
//! 7. Return `FusionRound` with timing + warnings.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;

use crate::els::{compute_stats, EpistemicLatentSpace};
use crate::errors::{EfeError, Result};
use crate::fusion::config::FusionConfig;
use crate::fusion::kernels::{
    aggregate::aggregate, attention::compute_scores, clip::clip_and_check,
    entropy::attention_entropy_stats, mix::beta_mix, project::project_rows,
    softmax::softmax_in_place_rows, KernelParams,
};
use crate::fusion::replay::ReplayBuffer;
use crate::fusion::round::{FusionRound, FusionWarning, RoundTiming};
use crate::fusion::schedule::{alpha_for_round, beta_for_round};
use crate::fusion::stats::FusionStats;
use crate::fusion::strategy::{FusionStrategy, ReplayableFusion};
use crate::fusion::validator::{validate_post_round, validate_pre_round};
use crate::memory::{Arena, DeviceBackend};

/// MVP CPU implementation of `FusionStrategy`.
pub struct MvpCpuFusion {
    replay: Option<Mutex<ReplayBuffer>>,
    #[allow(dead_code)]
    backend: Arc<dyn DeviceBackend>,
}

impl std::fmt::Debug for MvpCpuFusion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MvpCpuFusion")
            .field("has_replay", &self.replay.is_some())
            .finish_non_exhaustive()
    }
}

impl MvpCpuFusion {
    /// Create a new builder.
    #[must_use]
    pub fn builder() -> MvpCpuFusionBuilder {
        MvpCpuFusionBuilder::default()
    }

    /// Convenience: construct with default config, backed by a
    /// freshly-allocated replay buffer on the given backend.
    ///
    /// # Errors
    ///
    /// Propagates replay buffer allocation failures.
    pub fn with_replay(backend: Arc<dyn DeviceBackend>) -> Result<Self> {
        let replay = ReplayBuffer::new(4, 1 << 22, backend.clone())?;
        Ok(Self {
            replay: Some(Mutex::new(replay)),
            backend,
        })
    }

    /// Create without a replay buffer. M7 rollback will not be available.
    #[must_use]
    pub fn without_replay(backend: Arc<dyn DeviceBackend>) -> Self {
        Self {
            replay: None,
            backend,
        }
    }

    /// Build the per-round `KernelParams`.
    fn params_for_round(cfg: &FusionConfig, round: u32) -> KernelParams {
        KernelParams {
            temperature: cfg.temperature,
            mask_self_attention: cfg.mask_self_attention,
            beta: beta_for_round(cfg, round),
            alpha: alpha_for_round(cfg, round),
            clip_max: cfg.clip_max,
        }
    }

    /// Core round body. Factored out of the async trait impl so we can
    /// unit-test it without tokio ceremony.
    ///
    /// # Safety hint
    /// We acquire `split_mut` on the ELS, build a separate mutable view
    /// over the scratch allocation, run kernels, then swap the ELS
    /// latent ↔ scratch pointers. Between acquiring split and dropping
    /// it, no other code can touch `els` (&mut) — Rust enforces this.
    #[allow(clippy::too_many_lines)]
    fn run_round_sync(
        &self,
        els: &mut EpistemicLatentSpace,
        arena: &Arena,
        cfg: &FusionConfig,
        round: u32,
    ) -> Result<FusionRound> {
        let started = Instant::now();
        cfg.validate()?;

        // ---- Pre-round stats & validation ----
        let t_stats_start = Instant::now();
        let pre_stats = compute_stats(els, arena)?;
        let stats_us_pre = t_stats_start.elapsed().as_micros() as u32;
        validate_pre_round(&pre_stats, round)?;

        // ---- Replay snapshot (before mutation) ----
        if cfg.enable_replay {
            if let Some(rb) = &self.replay {
                let mut rb = rb
                    .lock()
                    .map_err(|_| EfeError::Internal("fusion: replay buffer mutex poisoned"))?;
                rb.push(round, els, arena)?;
            }
        }

        let params = Self::params_for_round(cfg, round);

        // Scratch allocations (pre-allocation in the round; Sprint 5+
        // can cache these in a per-strategy pool).
        let n = els.agent_count();
        let d = els.dim().get();
        let n_us = n as usize;
        let d_us = d as usize;
        let scores_len = n_us
            .checked_mul(n_us)
            .ok_or(EfeError::Internal("fusion: N² overflow"))?;
        let mut scores = vec![0.0f32; scores_len];
        let mut acc_scratch = vec![0.0f32; d_us];
        let mut row_scratch = vec![0.0f32; d_us];

        // Obtain a persistent scratch latent from the ELS. Lazy-allocated
        // on the first round per swarm; reused every subsequent round
        // via `swap_latent_with_scratch`. This is the fix for the
        // per-round arena leak that used to saturate the bump allocator
        // after ~8 rounds.
        let scratch_latent = els.ensure_scratch_latent(arena)?;

        // ---- Phase 1: scores -------------------------------------------
        let t_scores = Instant::now();
        let split = els.split_mut(arena)?;
        compute_scores(&split.latent.as_immut(), &split.mask, &mut scores, &params);
        let scores_us = t_scores.elapsed().as_micros() as u32;

        // ---- Phase 2: softmax ------------------------------------------
        let t_softmax = Instant::now();
        let softmax_metrics = softmax_in_place_rows(&mut scores, n_us).map_err(|e| match e {
            // Re-stamp the anomaly with the real round index.
            EfeError::NumericalExplosion { .. } => EfeError::NumericalExplosion { round },
            other => other,
        })?;
        let zero_attn_rows = softmax_metrics.zero_rows;
        let softmax_us = t_softmax.elapsed().as_micros() as u32;

        // ---- Phase 2.5: entropy stats (read-only) ----------------------
        let (h_mean, h_min, h_max) = attention_entropy_stats(&scores, n_us, &split.mask);

        // ---- Phase 3: aggregate into scratch ----------------------------
        //
        // Borrow a mutable view over the scratch (persistent, owned by
        // the ELS). The aggregation overwrites every active row; inactive
        // rows are copy-from-old (see `aggregate` semantics).
        let t_aggregate = Instant::now();
        {
            let mut new_latent_view =
                crate::els::views::LatentViewMut::from_raw(scratch_latent, n, d, arena)?;
            aggregate(
                &split.latent.as_immut(),
                &mut new_latent_view,
                &split.mask,
                &scores,
                &mut acc_scratch,
            );
        }
        // Scope ends here — both `new_latent_view` and the immutable
        // borrow derived from `split.latent.as_immut()` are gone, so
        // `split` itself is fully released (NLL), letting us take a
        // fresh `&mut els` for the swap below.
        let aggregate_us = t_aggregate.elapsed().as_micros() as u32;

        // ---- Pointer swap: ELS latent ↔ scratch ------------------------
        //
        // After the swap: ELS.latent points at the freshly-aggregated
        // bytes (previously the scratch); ELS.scratch points at the
        // previous latent bytes (which will be overwritten next round).
        // Zero-copy, O(1).
        els.swap_latent_with_scratch()?;

        // Phases 4-6 all mutate the (now swapped) ELS latent. We hold a
        // single `split_mut` borrow across them to avoid re-validating
        // the allocation's generation three times. The scope ends right
        // before the post-round stats call, releasing the `&mut els`
        // borrow cleanly via NLL.
        let (anchor_zero, clip_metrics, beta_mix_us, project_us, clip_us) = {
            let mut split = els.split_mut(arena)?;

            // ---- Phase 4: β-mix ---------------------------------------
            let t_beta = Instant::now();
            beta_mix(
                &mut split.latent,
                &split.problem_embedding,
                &split.mask,
                params.beta,
            );
            let beta_mix_us = t_beta.elapsed().as_micros() as u32;

            // ---- Phase 5: projection ----------------------------------
            let t_project = Instant::now();
            let anchor_zero = project_rows(
                &mut split.latent,
                &split.anchors,
                &split.mask,
                params.alpha,
                &mut row_scratch,
            )?;
            let project_us = t_project.elapsed().as_micros() as u32;

            // ---- Phase 6: clip ----------------------------------------
            let t_clip = Instant::now();
            let clip_metrics = clip_and_check(&mut split.latent, &split.mask, params.clip_max)
                .map_err(|e| match e {
                    EfeError::NumericalExplosion { .. } => EfeError::NumericalExplosion { round },
                    other => other,
                })?;
            let clip_us = t_clip.elapsed().as_micros() as u32;

            (anchor_zero, clip_metrics, beta_mix_us, project_us, clip_us)
        };

        // ---- Post-round stats & validation ------------------------------
        let t_stats_post = Instant::now();
        let post_stats = compute_stats(els, arena)?;
        let stats_us_post = t_stats_post.elapsed().as_micros() as u32;
        if cfg.validate_post {
            validate_post_round(&post_stats, round, cfg)?;
        }

        // ---- Build FusionStats & warnings ------------------------------
        let mut fusion_stats = FusionStats::synthetic_with(
            round,
            post_stats.centroid.len(),
            post_stats.variance,
            post_stats.gradient_norm,
        );
        fusion_stats.centroid = post_stats.centroid;
        fusion_stats.outlier_ratio = post_stats.outlier_ratio;
        fusion_stats.has_nan = post_stats.has_nan;
        fusion_stats.has_inf = post_stats.has_inf;
        fusion_stats.n_active = post_stats.n_active;
        fusion_stats.set_attention_entropy(h_mean, h_min, h_max);
        fusion_stats.set_schedule(params.alpha, params.beta);

        let total_us = started.elapsed().as_micros() as u32;
        let budget_us = cfg.round_deadline_ms.saturating_mul(1000);
        let on_time = total_us <= budget_us;

        let mut warnings = Vec::new();
        if clip_metrics.clipped_rows > 0 {
            warnings.push(FusionWarning::ClippedCoordinates {
                count: clip_metrics.clipped_rows,
                max_abs: clip_metrics.max_abs,
            });
        }
        if anchor_zero {
            warnings.push(FusionWarning::AnchorAllZeroWeight);
        }
        if !on_time {
            warnings.push(FusionWarning::DeadlineExceeded {
                budget_us,
                actual_us: total_us,
            });
        }
        if cfg.variance_warn_ratio.is_finite()
            && post_stats.variance.is_finite()
            && pre_stats.variance.is_finite()
            && post_stats.variance > pre_stats.variance * cfg.variance_warn_ratio + 1e-6
        {
            warnings.push(FusionWarning::HighVarianceDelta {
                pre: pre_stats.variance,
                post: post_stats.variance,
            });
        }
        if zero_attn_rows > 0 {
            warnings.push(FusionWarning::ZeroAttentionRow {
                count: zero_attn_rows,
            });
        }

        if cfg.strict_deadline && !on_time {
            return Err(EfeError::BudgetExhausted {
                kind: "fusion_deadline",
            });
        }

        let timing = RoundTiming {
            scores_us,
            softmax_us,
            aggregate_us,
            beta_mix_us,
            project_us,
            clip_us,
            stats_us: stats_us_pre.saturating_add(stats_us_post),
            total_us,
        };

        Ok(FusionRound {
            round,
            stats: fusion_stats,
            timing,
            backend: self.backend_name().to_string(),
            on_time,
            warnings,
        })
    }
}

#[async_trait]
impl FusionStrategy for MvpCpuFusion {
    async fn run_round(
        &self,
        els: &mut EpistemicLatentSpace,
        arena: &Arena,
        cfg: &FusionConfig,
        round: u32,
    ) -> Result<FusionRound> {
        self.run_round_sync(els, arena, cfg, round)
    }

    fn backend_name(&self) -> &'static str {
        "mvp-cpu"
    }

    fn supports_replay(&self) -> bool {
        self.replay.is_some()
    }
}

#[async_trait]
impl ReplayableFusion for MvpCpuFusion {
    async fn record_snapshot(
        &self,
        round: u32,
        els: &EpistemicLatentSpace,
        arena: &Arena,
    ) -> Result<()> {
        let rb = self
            .replay
            .as_ref()
            .ok_or(EfeError::Internal("record_snapshot: replay disabled"))?;
        let mut rb = rb
            .lock()
            .map_err(|_| EfeError::Internal("fusion: replay buffer mutex poisoned"))?;
        rb.push(round, els, arena)
    }

    async fn restore_snapshot(
        &self,
        round: u32,
        els: &mut EpistemicLatentSpace,
        arena: &Arena,
    ) -> Result<()> {
        let rb = self
            .replay
            .as_ref()
            .ok_or(EfeError::Internal("restore_snapshot: replay disabled"))?;
        let rb = rb
            .lock()
            .map_err(|_| EfeError::Internal("fusion: replay buffer mutex poisoned"))?;
        rb.restore(round, els, arena)
    }

    fn available_replay_rounds(&self) -> Vec<u32> {
        let Some(rb) = self.replay.as_ref() else {
            return Vec::new();
        };
        let Ok(rb) = rb.lock() else {
            return Vec::new();
        };
        // Ring buffer stores oldest-first; M7 wants newest-first.
        let mut rounds = rb.rounds();
        rounds.reverse();
        rounds
    }
}

/// Builder for `MvpCpuFusion`.
#[derive(Default)]
pub struct MvpCpuFusionBuilder {
    backend: Option<Arc<dyn DeviceBackend>>,
    replay_capacity: Option<u32>,
    replay_arena_bytes: Option<u64>,
    enable_replay: bool,
}

impl MvpCpuFusionBuilder {
    /// Backend for replay arena allocation (required if `enable_replay`).
    #[must_use]
    pub fn with_backend(mut self, backend: Arc<dyn DeviceBackend>) -> Self {
        self.backend = Some(backend);
        self
    }

    /// Enable or disable the replay buffer.
    ///
    /// Defaults to disabled in the builder. When enabled, a
    /// [`ReplayBuffer`] is allocated on `build()` using the supplied
    /// backend. Required for M7 rollback.
    ///
    /// Renamed from `with_replay(bool)` to avoid shadowing
    /// [`MvpCpuFusion::with_replay`] (which takes a backend).
    #[must_use]
    pub fn with_replay_enabled(mut self, enable: bool) -> Self {
        self.enable_replay = enable;
        self
    }

    /// Override the replay buffer capacity (default: 4).
    #[must_use]
    pub fn with_replay_capacity(mut self, c: u32) -> Self {
        self.replay_capacity = Some(c);
        self
    }

    /// Override the replay arena bytes (default: 4 MB).
    #[must_use]
    pub fn with_replay_arena_bytes(mut self, b: u64) -> Self {
        self.replay_arena_bytes = Some(b);
        self
    }

    /// Finalize the build.
    ///
    /// # Errors
    /// - [`EfeError::InvalidConfig`] if `with_replay(true)` was set but
    ///   no backend supplied.
    /// - Propagates replay buffer allocation failures.
    pub fn build(self) -> Result<MvpCpuFusion> {
        let backend = self
            .backend
            .ok_or_else(|| EfeError::InvalidConfig("MvpCpuFusion: backend is required".into()))?;
        let replay = if self.enable_replay {
            let cap = self.replay_capacity.unwrap_or(4);
            let bytes = self.replay_arena_bytes.unwrap_or(1 << 22);
            Some(Mutex::new(ReplayBuffer::new(cap, bytes, backend.clone())?))
        } else {
            None
        };
        Ok(MvpCpuFusion { replay, backend })
    }
}
