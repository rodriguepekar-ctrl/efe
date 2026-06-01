//! `PartialRespawnStrategy` — level-4 recovery.
//!
//! Deactivates agents whose distance from the centroid exceeds
//! `outlier_z_threshold * gradient_norm`. The remaining population
//! continues fusion; outliers are masked out permanently for this
//! campaign.

use std::time::Instant;

use async_trait::async_trait;

use crate::collapse::context::CollapseContext;
use crate::collapse::helpers::estimate_recovery_confidence;
use crate::collapse::outcome::{CollapseAction, CollapseOutcome, CollapseWarning};
use crate::collapse::strategy::CollapseStrategy;
use crate::collapse::trigger::CollapseTrigger;
use crate::consensus::verdict::DivergenceReason;
use crate::els::compute_stats;
use crate::els::metrics::l2_distance_sq_mixed;
use crate::errors::{EfeError, Result};

/// Level-4 strategy: mask-out outlier agents.
#[derive(Debug, Default, Clone)]
pub struct PartialRespawnStrategy;

#[async_trait]
impl CollapseStrategy for PartialRespawnStrategy {
    async fn apply(&self, ctx: &mut CollapseContext<'_>) -> Result<CollapseOutcome> {
        let start = Instant::now();
        let name = self.name();

        let pre_stats = match compute_stats(ctx.els, ctx.arena) {
            Ok(s) if s.all_finite() => s,
            _ => {
                return Ok(post_condition_failed(ctx, start, name));
            }
        };
        let n_initial = pre_stats.n_active;
        let threshold_sq = (ctx.config.outlier_z_threshold * pre_stats.gradient_norm).powi(2);
        let centroid = pre_stats.centroid.clone();

        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let min_active = ((n_initial as f32) * ctx.config.min_active_fraction)
            .round()
            .max(1.0) as u32;

        // First pass: identify outliers (requires reading the latent).
        //
        // `LatentViewMut` exposes only `row_mut`; for read-only access we
        // demote it via `as_immut()` which returns a `LatentView` (same
        // bytes, read-only).
        let mut to_deactivate: Vec<u32> = Vec::new();
        {
            let split = ctx.els.split_mut(ctx.arena)?;
            let latent = split.latent.as_immut();
            let n = latent.n_agents();
            for i in 0..n {
                if !split.mask.is_active(i) {
                    continue;
                }
                let row = latent.row(i).expect("i < n");
                let dsq = l2_distance_sq_mixed(row, &centroid);
                if dsq > threshold_sq {
                    to_deactivate.push(i);
                }
            }
        }

        let to_deactivate_count = u32::try_from(to_deactivate.len()).unwrap_or(u32::MAX);
        let remaining = n_initial.saturating_sub(to_deactivate_count);
        if remaining < min_active {
            return Ok(CollapseOutcome {
                action_taken: CollapseAction::NoOp,
                success: false,
                trigger: ctx.trigger.clone(),
                new_round: None,
                variance_after: Some(pre_stats.variance),
                recovery_confidence: 0.0,
                warnings: vec![CollapseWarning::PartialRespawnBelowMinimum {
                    would_deactivate: to_deactivate_count,
                    min_required: min_active,
                }],
                duration_us: duration_us(start),
                strategy_name: name.into(),
            });
        }

        // Second pass: actually flip the mask bits.
        {
            let mut mask = ctx
                .els
                .mask_view_mut(ctx.arena)?
                .ok_or(EfeError::Internal("partial_respawn: ELS missing mask"))?;
            for i in to_deactivate {
                mask.set(i, false)?;
            }
        }

        let stats = compute_stats(ctx.els, ctx.arena)?;
        let success = stats.all_finite() && stats.variance <= ctx.config.post_variance_max;
        let mut warnings = Vec::new();
        if !stats.all_finite() {
            warnings.push(CollapseWarning::PostConditionFailed);
        }

        Ok(CollapseOutcome {
            action_taken: CollapseAction::PartialRespawn {
                deactivated: to_deactivate_count,
                remaining: stats.n_active,
            },
            success,
            trigger: ctx.trigger.clone(),
            new_round: if success { Some(ctx.round) } else { None },
            variance_after: if stats.all_finite() {
                Some(stats.variance)
            } else {
                None
            },
            recovery_confidence: estimate_recovery_confidence(&stats, ctx.config),
            warnings,
            duration_us: duration_us(start),
            strategy_name: name.into(),
        })
    }

    fn applicable_to(&self, trigger: &CollapseTrigger) -> bool {
        matches!(
            trigger,
            CollapseTrigger::FromDivergence {
                reason: DivergenceReason::VarianceGrowing { .. }
                    | DivergenceReason::VarianceExplosion { .. },
                ..
            }
        )
    }

    fn name(&self) -> &'static str {
        "partial-respawn"
    }
}

fn duration_us(start: Instant) -> u32 {
    u32::try_from(start.elapsed().as_micros()).unwrap_or(u32::MAX)
}

fn post_condition_failed(ctx: &CollapseContext<'_>, start: Instant, name: &str) -> CollapseOutcome {
    CollapseOutcome {
        action_taken: CollapseAction::NoOp,
        success: false,
        trigger: ctx.trigger.clone(),
        new_round: None,
        variance_after: None,
        recovery_confidence: 0.0,
        warnings: vec![CollapseWarning::PostConditionFailed],
        duration_us: duration_us(start),
        strategy_name: name.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::verdict::{AnomalyCode, DivergenceReason};

    #[test]
    fn applicable_to_variance_growing() {
        let s = PartialRespawnStrategy;
        let t = CollapseTrigger::FromDivergence {
            reason: DivergenceReason::VarianceGrowing {
                slope: 0.1,
                window: 6,
            },
            round: 0,
        };
        assert!(s.applicable_to(&t));
    }

    #[test]
    fn applicable_to_variance_explosion() {
        let s = PartialRespawnStrategy;
        let t = CollapseTrigger::FromDivergence {
            reason: DivergenceReason::VarianceExplosion { variance: 1e5 },
            round: 0,
        };
        assert!(s.applicable_to(&t));
    }

    #[test]
    fn not_applicable_to_stall() {
        let s = PartialRespawnStrategy;
        let t = CollapseTrigger::FromStall {
            rounds_flat: 6,
            confidence: 0.3,
            round: 0,
        };
        assert!(!s.applicable_to(&t));
    }

    #[test]
    fn not_applicable_to_nan_anomaly() {
        let s = PartialRespawnStrategy;
        let t = CollapseTrigger::FromAnomaly {
            code: AnomalyCode::NanInf,
            round: 0,
        };
        assert!(!s.applicable_to(&t));
    }

    #[test]
    fn name_is_stable() {
        assert_eq!(PartialRespawnStrategy.name(), "partial-respawn");
    }
}
