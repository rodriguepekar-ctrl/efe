//! `CentroidProjectionStrategy` — level-3 recovery.
//!
//! Forces every active row of the latent to `centroid + small_noise`,
//! which drives variance to ~`jitter² * D` — effectively a soft reset.
//! Loses all agent-specific diversity; use only when rollback is
//! unavailable and noise injection has failed.

use std::time::Instant;

use async_trait::async_trait;
use half::bf16;
use rand::{RngCore, SeedableRng};
use rand_distr::{Distribution, Normal};
use rand_pcg::Pcg64;

use crate::collapse::context::CollapseContext;
use crate::collapse::helpers::estimate_recovery_confidence;
use crate::collapse::outcome::{CollapseAction, CollapseOutcome, CollapseWarning};
use crate::collapse::strategy::CollapseStrategy;
use crate::collapse::trigger::CollapseTrigger;
use crate::els::compute_stats;
use crate::errors::{EfeError, Result};

/// Level-3 strategy: overwrite every active row with `centroid + jitter`.
#[derive(Debug, Default, Clone)]
pub struct CentroidProjectionStrategy;

#[async_trait]
impl CollapseStrategy for CentroidProjectionStrategy {
    async fn apply(&self, ctx: &mut CollapseContext<'_>) -> Result<CollapseOutcome> {
        let start = Instant::now();
        let name = self.name();
        let jitter = ctx.config.centroid_project_jitter;

        // 1. Compute the current centroid. If the centroid itself is
        //    corrupt, the projection cannot proceed.
        let Ok(pre_stats) = compute_stats(ctx.els, ctx.arena) else {
            return Ok(post_condition_failed(ctx, start, name));
        };
        if !pre_stats.all_finite() {
            return Ok(post_condition_failed(ctx, start, name));
        }

        let centroid = pre_stats.centroid.clone();
        let seed = ctx.rng.next_u64();
        let mut local = Pcg64::seed_from_u64(seed);
        let normal = Normal::new(0.0f32, jitter.max(0.0)).map_err(|_| {
            EfeError::Internal("centroid_projection: failed to build normal distribution")
        })?;
        let clip_max = ctx.config.clip_max;

        {
            let mut split = ctx.els.split_mut(ctx.arena)?;
            let n = split.latent.n_agents();
            for i in 0..n {
                if !split.mask.is_active(i) {
                    continue;
                }
                let row = split.latent.row_mut(i).expect("i < n");
                for (r, c) in row.iter_mut().zip(centroid.iter()) {
                    let v = c + normal.sample(&mut local);
                    *r = bf16::from_f32(v.clamp(-clip_max, clip_max));
                }
            }
        }

        let stats = compute_stats(ctx.els, ctx.arena)?;
        let success = stats.all_finite() && stats.variance <= ctx.config.post_variance_max;
        let mut warnings = Vec::new();
        if !stats.all_finite() {
            warnings.push(CollapseWarning::PostConditionFailed);
        }

        Ok(CollapseOutcome {
            action_taken: CollapseAction::CentroidProjection { jitter },
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
        // Applicable as a last-resort for any *numerical* trigger. Not
        // structural: a `CentroidDimMismatch` means the ELS has been
        // fundamentally mis-shaped and cannot be fixed by overwriting
        // rows. Similarly a `NonMonotonicRound` is a controller bug,
        // not a numerical issue.
        !matches!(
            trigger,
            CollapseTrigger::FromAnomaly {
                code: crate::consensus::verdict::AnomalyCode::CentroidDimMismatch { .. }
                    | crate::consensus::verdict::AnomalyCode::NonMonotonicRound { .. },
                ..
            }
        )
    }

    fn name(&self) -> &'static str {
        "centroid-projection"
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
    use crate::consensus::verdict::AnomalyCode;

    #[test]
    fn applicable_to_numerical_triggers() {
        let s = CentroidProjectionStrategy;
        let triggers = [
            CollapseTrigger::FromAnomaly {
                code: AnomalyCode::NanInf,
                round: 0,
            },
            CollapseTrigger::FromStall {
                rounds_flat: 3,
                confidence: 0.1,
                round: 0,
            },
            CollapseTrigger::Manual {
                reason: "x".into(),
                round: 0,
            },
        ];
        for t in triggers {
            assert!(s.applicable_to(&t));
        }
    }

    #[test]
    fn not_applicable_to_structural_anomalies() {
        let s = CentroidProjectionStrategy;
        for code in [
            AnomalyCode::CentroidDimMismatch {
                prev: 4,
                current: 8,
            },
            AnomalyCode::NonMonotonicRound {
                last: 5,
                current: 3,
            },
        ] {
            let t = CollapseTrigger::FromAnomaly { code, round: 0 };
            assert!(!s.applicable_to(&t));
        }
    }

    #[test]
    fn name_is_stable() {
        assert_eq!(CentroidProjectionStrategy.name(), "centroid-projection");
    }
}
