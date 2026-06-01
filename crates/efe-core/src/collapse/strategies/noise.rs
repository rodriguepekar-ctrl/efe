//! `NoiseInjectionStrategy` — level-2 recovery.
//!
//! Adds zero-mean Gaussian noise to every active row of the latent.
//! Useful for escaping local minima (stall) or mild trajectory issues
//! (confidence decay) where the latent is still finite.

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
use crate::consensus::verdict::DivergenceReason;
use crate::els::compute_stats;
use crate::errors::{EfeError, Result};

/// Level-2 strategy: add Gaussian noise to the latent to escape a
/// plateau or low-confidence loop.
#[derive(Debug, Default, Clone)]
pub struct NoiseInjectionStrategy;

#[async_trait]
impl CollapseStrategy for NoiseInjectionStrategy {
    async fn apply(&self, ctx: &mut CollapseContext<'_>) -> Result<CollapseOutcome> {
        let start = Instant::now();
        let name = self.name();
        let sigma = ctx.config.noise_sigma;
        let clip_max = ctx.config.clip_max;

        // Derive a fresh reproducible seed from the outer RNG.
        let seed = ctx.rng.next_u64();
        let mut local = Pcg64::seed_from_u64(seed);
        let normal = Normal::new(0.0f32, sigma).map_err(|_| {
            EfeError::Internal("noise_injection: failed to build normal distribution")
        })?;

        {
            let mut split = ctx.els.split_mut(ctx.arena)?;
            let n = split.latent.n_agents();
            for i in 0..n {
                if !split.mask.is_active(i) {
                    continue;
                }
                let row = split.latent.row_mut(i).expect("i < n");
                for r in row.iter_mut() {
                    let v = r.to_f32() + normal.sample(&mut local);
                    let clipped = v.clamp(-clip_max, clip_max);
                    *r = bf16::from_f32(clipped);
                }
            }
        }

        let stats = compute_stats(ctx.els, ctx.arena)?;
        let finite = stats.all_finite();
        let variance_ok = stats.variance <= ctx.config.post_variance_max;
        let success = finite && variance_ok;

        let mut warnings = Vec::new();
        if finite && !variance_ok {
            warnings.push(CollapseWarning::NoiseIneffective {
                variance_after: stats.variance,
            });
        }
        if !finite {
            warnings.push(CollapseWarning::PostConditionFailed);
        }

        Ok(CollapseOutcome {
            action_taken: CollapseAction::NoiseInjection { sigma, seed },
            success,
            trigger: ctx.trigger.clone(),
            new_round: if success { Some(ctx.round) } else { None },
            variance_after: if finite { Some(stats.variance) } else { None },
            recovery_confidence: estimate_recovery_confidence(&stats, ctx.config),
            warnings,
            duration_us: duration_us(start),
            strategy_name: name.into(),
        })
    }

    fn applicable_to(&self, trigger: &CollapseTrigger) -> bool {
        matches!(
            trigger,
            CollapseTrigger::FromStall { .. }
                | CollapseTrigger::FromDivergence {
                    reason: DivergenceReason::ConfidenceDecay { .. },
                    ..
                }
        )
    }

    fn name(&self) -> &'static str {
        "noise-injection"
    }
}

fn duration_us(start: Instant) -> u32 {
    u32::try_from(start.elapsed().as_micros()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::verdict::{AnomalyCode, DivergenceReason};

    #[test]
    fn applicable_to_stall() {
        let s = NoiseInjectionStrategy;
        let t = CollapseTrigger::FromStall {
            rounds_flat: 6,
            confidence: 0.3,
            round: 5,
        };
        assert!(s.applicable_to(&t));
    }

    #[test]
    fn applicable_to_confidence_decay() {
        let s = NoiseInjectionStrategy;
        let t = CollapseTrigger::FromDivergence {
            reason: DivergenceReason::ConfidenceDecay { from: 0.9, to: 0.3 },
            round: 5,
        };
        assert!(s.applicable_to(&t));
    }

    #[test]
    fn not_applicable_to_variance_explosion() {
        let s = NoiseInjectionStrategy;
        let t = CollapseTrigger::FromDivergence {
            reason: DivergenceReason::VarianceExplosion { variance: 1e5 },
            round: 0,
        };
        assert!(!s.applicable_to(&t));
    }

    #[test]
    fn not_applicable_to_anomaly() {
        let s = NoiseInjectionStrategy;
        let t = CollapseTrigger::FromAnomaly {
            code: AnomalyCode::NanInf,
            round: 0,
        };
        assert!(!s.applicable_to(&t));
    }

    #[test]
    fn name_is_stable() {
        assert_eq!(NoiseInjectionStrategy.name(), "noise-injection");
    }
}
