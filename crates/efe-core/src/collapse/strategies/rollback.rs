//! `SoftRollbackStrategy` — level-1 recovery.
//!
//! Restores the latent from the most recent sound snapshot in M5's
//! `ReplayBuffer`. Low cost, non-destructive. Cannot address
//! structural anomalies (`CentroidDimMismatch`, `NonMonotonicRound`)
//! because those indicate the consensus detector was fed inconsistent
//! data — rolling back doesn't fix that.

use std::time::Instant;

use async_trait::async_trait;

use crate::collapse::context::CollapseContext;
use crate::collapse::helpers::estimate_recovery_confidence;
use crate::collapse::outcome::{CollapseAction, CollapseOutcome, CollapseWarning};
use crate::collapse::strategy::CollapseStrategy;
use crate::collapse::trigger::CollapseTrigger;
use crate::consensus::verdict::AnomalyCode;
use crate::els::compute_stats;
use crate::errors::Result;

/// Level-1 strategy: restore the latent from M5's replay buffer.
#[derive(Debug, Default, Clone)]
pub struct SoftRollbackStrategy;

#[async_trait]
impl CollapseStrategy for SoftRollbackStrategy {
    async fn apply(&self, ctx: &mut CollapseContext<'_>) -> Result<CollapseOutcome> {
        let start = Instant::now();
        let name = self.name();

        let rounds = ctx.fusion.available_replay_rounds();
        if rounds.is_empty() {
            return Ok(CollapseOutcome {
                action_taken: CollapseAction::NoOp,
                success: false,
                trigger: ctx.trigger.clone(),
                new_round: None,
                variance_after: None,
                recovery_confidence: 0.0,
                warnings: vec![CollapseWarning::ReplayBufferEmpty],
                duration_us: duration_us(start),
                strategy_name: name.into(),
            });
        }

        let current_round = ctx.round;
        // Try each candidate newest-first.
        //
        // M5 takes each replay snapshot at the *start* of its round
        // (before any mutation — see `run_round_sync`). So
        // `snapshot(r) = state immediately before round r ran`. If the
        // collapse was triggered because round `r` produced bad stats,
        // restoring to `snapshot(r)` precisely rolls back that bad
        // round while preserving all earlier progress — so
        // `target == current_round` is the **preferred** target, not
        // an invalid one.
        //
        // We only skip `target > current_round` (future snapshots,
        // which can't exist in practice but we check defensively).
        for target in rounds {
            if target > current_round {
                continue;
            }
            if ctx.past_deadline() {
                return Ok(deadline_outcome(ctx, start, name));
            }

            ctx.fusion
                .restore_snapshot(target, ctx.els, ctx.arena)
                .await?;
            let stats = compute_stats(ctx.els, ctx.arena)?;
            if !stats.all_finite() {
                // Snapshot was itself corrupt — try the next one.
                continue;
            }
            if stats.variance > ctx.config.post_variance_max {
                // Snapshot was on the divergence trajectory already.
                continue;
            }
            return Ok(CollapseOutcome {
                action_taken: CollapseAction::SoftRollback { to_round: target },
                success: true,
                trigger: ctx.trigger.clone(),
                new_round: Some(target),
                variance_after: Some(stats.variance),
                recovery_confidence: estimate_recovery_confidence(&stats, ctx.config),
                warnings: vec![],
                duration_us: duration_us(start),
                strategy_name: name.into(),
            });
        }

        Ok(CollapseOutcome {
            action_taken: CollapseAction::NoOp,
            success: false,
            trigger: ctx.trigger.clone(),
            new_round: None,
            variance_after: None,
            recovery_confidence: 0.0,
            warnings: vec![CollapseWarning::ReplayAllCorrupt],
            duration_us: duration_us(start),
            strategy_name: name.into(),
        })
    }

    fn applicable_to(&self, trigger: &CollapseTrigger) -> bool {
        // Rollback cannot fix structural issues.
        !matches!(
            trigger,
            CollapseTrigger::FromAnomaly {
                code: AnomalyCode::CentroidDimMismatch { .. }
                    | AnomalyCode::NonMonotonicRound { .. },
                ..
            }
        )
    }

    fn name(&self) -> &'static str {
        "soft-rollback"
    }
}

fn duration_us(start: Instant) -> u32 {
    u32::try_from(start.elapsed().as_micros()).unwrap_or(u32::MAX)
}

fn deadline_outcome(ctx: &CollapseContext<'_>, start: Instant, name: &str) -> CollapseOutcome {
    let actual = duration_us(start);
    CollapseOutcome {
        action_taken: CollapseAction::NoOp,
        success: false,
        trigger: ctx.trigger.clone(),
        new_round: None,
        variance_after: None,
        recovery_confidence: 0.0,
        warnings: vec![CollapseWarning::DeadlineExceeded {
            budget_us: ctx.config.deadline_ms.saturating_mul(1000),
            actual_us: actual,
        }],
        duration_us: actual,
        strategy_name: name.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::verdict::{AnomalyCode, DivergenceReason};

    #[test]
    fn applicable_to_allows_nan_anomaly() {
        let s = SoftRollbackStrategy;
        let t = CollapseTrigger::FromAnomaly {
            code: AnomalyCode::NanInf,
            round: 0,
        };
        assert!(s.applicable_to(&t));
    }

    #[test]
    fn applicable_to_rejects_centroid_dim_mismatch() {
        let s = SoftRollbackStrategy;
        let t = CollapseTrigger::FromAnomaly {
            code: AnomalyCode::CentroidDimMismatch {
                prev: 4,
                current: 8,
            },
            round: 0,
        };
        assert!(!s.applicable_to(&t));
    }

    #[test]
    fn applicable_to_rejects_non_monotonic() {
        let s = SoftRollbackStrategy;
        let t = CollapseTrigger::FromAnomaly {
            code: AnomalyCode::NonMonotonicRound {
                last: 5,
                current: 3,
            },
            round: 0,
        };
        assert!(!s.applicable_to(&t));
    }

    #[test]
    fn applicable_to_handles_all_divergence_reasons() {
        let s = SoftRollbackStrategy;
        for reason in [
            DivergenceReason::VarianceExplosion { variance: 1.0 },
            DivergenceReason::VarianceGrowing {
                slope: 0.1,
                window: 6,
            },
            DivergenceReason::ConfidenceDecay { from: 0.9, to: 0.3 },
        ] {
            let t = CollapseTrigger::FromDivergence { reason, round: 0 };
            assert!(s.applicable_to(&t));
        }
    }

    #[test]
    fn name_is_stable() {
        assert_eq!(SoftRollbackStrategy.name(), "soft-rollback");
    }
}
