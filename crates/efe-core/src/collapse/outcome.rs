//! `CollapseOutcome` — the result of one collapse call.
//!
//! Returned by `CollapseEngine::handle` and by every individual
//! `CollapseStrategy::apply`. Carries enough information for M1 to
//! decide next steps and for M11 to serialize telemetry.

use serde::{Deserialize, Serialize};

use crate::collapse::trigger::CollapseTrigger;

/// Discriminated action actually performed by a collapse.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CollapseAction {
    /// Latent rolled back to a previous snapshot via M5's replay buffer.
    SoftRollback {
        /// Round restored.
        to_round: u32,
    },
    /// Gaussian noise added to every active row.
    NoiseInjection {
        /// Sigma used.
        sigma: f32,
        /// PRNG seed for reproducibility.
        seed: u64,
    },
    /// Every active row overwritten with `centroid + jitter`.
    CentroidProjection {
        /// Jitter sigma.
        jitter: f32,
    },
    /// Outlier agents deactivated via the mask.
    PartialRespawn {
        /// Number of agents deactivated.
        deactivated: u32,
        /// Number of agents still active after the respawn.
        remaining: u32,
    },
    /// Swarm declared unrecoverable; controller must respawn via M3
    /// or abort the request.
    FullAbort,
    /// No strategy was applicable, or engine did not run any strategy.
    NoOp,
}

impl std::fmt::Display for CollapseAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SoftRollback { to_round } => write!(f, "soft_rollback(to_round={to_round})"),
            Self::NoiseInjection { sigma, seed } => {
                write!(f, "noise_injection(sigma={sigma:.4}, seed={seed})")
            }
            Self::CentroidProjection { jitter } => {
                write!(f, "centroid_projection(jitter={jitter:.4})")
            }
            Self::PartialRespawn {
                deactivated,
                remaining,
            } => write!(
                f,
                "partial_respawn(deactivated={deactivated}, remaining={remaining})"
            ),
            Self::FullAbort => f.write_str("full_abort"),
            Self::NoOp => f.write_str("no_op"),
        }
    }
}

impl CollapseAction {
    /// Stable short label for metrics.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::SoftRollback { .. } => "soft_rollback",
            Self::NoiseInjection { .. } => "noise_injection",
            Self::CentroidProjection { .. } => "centroid_projection",
            Self::PartialRespawn { .. } => "partial_respawn",
            Self::FullAbort => "full_abort",
            Self::NoOp => "no_op",
        }
    }
}

/// Non-fatal issues encountered during a collapse.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CollapseWarning {
    /// Replay buffer had no valid snapshot.
    ReplayBufferEmpty,
    /// All replay snapshots were themselves corrupt / too divergent.
    ReplayAllCorrupt,
    /// Noise injection produced a variance still above the threshold.
    NoiseIneffective {
        /// Variance observed after the injection.
        variance_after: f32,
    },
    /// Partial respawn would leave fewer active agents than
    /// `min_active_fraction * n_initial`.
    PartialRespawnBelowMinimum {
        /// Count that would have been deactivated.
        would_deactivate: u32,
        /// Minimum required to remain active.
        min_required: u32,
    },
    /// Collapse wallclock deadline exceeded.
    DeadlineExceeded {
        /// Budget (μs).
        budget_us: u32,
        /// Actual duration (μs).
        actual_us: u32,
    },
    /// Circuit breaker fired — max collapses reached for this swarm.
    CircuitBreaker {
        /// Current collapse count for the swarm.
        swarm_collapse_count: u32,
    },
    /// Swarm abandoned after all strategies were tried.
    Abandoned,
    /// Post-condition check failed: ELS still has NaN/Inf after the
    /// strategy.
    PostConditionFailed,
}

impl CollapseWarning {
    /// True if this warning means the collapse itself failed fatally
    /// (no recovery possible).
    #[must_use]
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            Self::CircuitBreaker { .. } | Self::Abandoned | Self::PostConditionFailed
        )
    }

    /// Stable short label for metrics.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::ReplayBufferEmpty => "replay_buffer_empty",
            Self::ReplayAllCorrupt => "replay_all_corrupt",
            Self::NoiseIneffective { .. } => "noise_ineffective",
            Self::PartialRespawnBelowMinimum { .. } => "partial_respawn_below_minimum",
            Self::DeadlineExceeded { .. } => "deadline_exceeded",
            Self::CircuitBreaker { .. } => "circuit_breaker",
            Self::Abandoned => "abandoned",
            Self::PostConditionFailed => "post_condition_failed",
        }
    }
}

impl std::fmt::Display for CollapseWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Outcome of one call to `CollapseStrategy::apply` (or the full engine
/// escalation).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CollapseOutcome {
    /// What was actually done.
    pub action_taken: CollapseAction,
    /// True iff the ELS is in a recoverable state after the call.
    pub success: bool,
    /// The trigger that invoked this collapse.
    pub trigger: CollapseTrigger,
    /// Round index to resume fusion from. `None` on fatal abort.
    pub new_round: Option<u32>,
    /// Variance of the ELS after the collapse. `None` if ELS is still
    /// in a corrupt state and cannot be measured.
    pub variance_after: Option<f32>,
    /// Heuristic estimate ∈ `[0, 1]` that subsequent fusion will
    /// converge. MVP uses a simple variance/outlier formula.
    pub recovery_confidence: f32,
    /// Non-fatal issues.
    pub warnings: Vec<CollapseWarning>,
    /// Total duration (μs).
    pub duration_us: u32,
    /// Name of the strategy that produced this outcome.
    pub strategy_name: String,
}

impl CollapseOutcome {
    /// True iff the swarm is in a good-enough state to resume fusion.
    ///
    /// Equivalent to `success && action_taken != FullAbort`.
    #[must_use]
    pub fn is_recoverable(&self) -> bool {
        self.success && !matches!(self.action_taken, CollapseAction::FullAbort)
    }

    /// True iff the swarm is in a terminal state (must respawn or abort).
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self.action_taken, CollapseAction::FullAbort)
            || (!self.success && self.warnings.iter().any(CollapseWarning::is_fatal))
    }

    /// True iff any warning is present.
    #[must_use]
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }
}

impl std::fmt::Display for CollapseOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let success = if self.success { "ok" } else { "fail" };
        write!(
            f,
            "collapse[{strategy}/{action}]({trigger}) → {success} conf={conf:.4} in {dur}μs",
            strategy = self.strategy_name,
            action = self.action_taken.label(),
            trigger = self.trigger.label(),
            success = success,
            conf = self.recovery_confidence,
            dur = self.duration_us,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::verdict::AnomalyCode;

    fn mk_trigger() -> CollapseTrigger {
        CollapseTrigger::FromAnomaly {
            code: AnomalyCode::NanInf,
            round: 3,
        }
    }

    #[test]
    fn soft_rollback_is_recoverable() {
        let o = CollapseOutcome {
            action_taken: CollapseAction::SoftRollback { to_round: 2 },
            success: true,
            trigger: mk_trigger(),
            new_round: Some(2),
            variance_after: Some(0.1),
            recovery_confidence: 0.8,
            warnings: vec![],
            duration_us: 500,
            strategy_name: "soft-rollback".into(),
        };
        assert!(o.is_recoverable());
        assert!(!o.is_terminal());
    }

    #[test]
    fn full_abort_is_terminal() {
        let o = CollapseOutcome {
            action_taken: CollapseAction::FullAbort,
            success: false,
            trigger: mk_trigger(),
            new_round: None,
            variance_after: None,
            recovery_confidence: 0.0,
            warnings: vec![CollapseWarning::Abandoned],
            duration_us: 10,
            strategy_name: "full-abort".into(),
        };
        assert!(!o.is_recoverable());
        assert!(o.is_terminal());
    }

    #[test]
    fn failure_with_fatal_warning_is_terminal() {
        let o = CollapseOutcome {
            action_taken: CollapseAction::NoOp,
            success: false,
            trigger: mk_trigger(),
            new_round: None,
            variance_after: None,
            recovery_confidence: 0.0,
            warnings: vec![CollapseWarning::PostConditionFailed],
            duration_us: 100,
            strategy_name: "none".into(),
        };
        assert!(o.is_terminal());
    }

    #[test]
    fn failure_with_non_fatal_warning_is_not_terminal() {
        let o = CollapseOutcome {
            action_taken: CollapseAction::NoOp,
            success: false,
            trigger: mk_trigger(),
            new_round: None,
            variance_after: Some(1.0),
            recovery_confidence: 0.0,
            warnings: vec![CollapseWarning::ReplayBufferEmpty],
            duration_us: 50,
            strategy_name: "soft-rollback".into(),
        };
        // ReplayBufferEmpty is not fatal; controller should escalate.
        assert!(!o.is_terminal());
    }

    #[test]
    fn action_labels_are_snake_case() {
        for a in [
            CollapseAction::SoftRollback { to_round: 0 },
            CollapseAction::NoiseInjection {
                sigma: 0.1,
                seed: 0,
            },
            CollapseAction::CentroidProjection { jitter: 0.01 },
            CollapseAction::PartialRespawn {
                deactivated: 0,
                remaining: 0,
            },
            CollapseAction::FullAbort,
            CollapseAction::NoOp,
        ] {
            let l = a.label();
            assert!(
                l.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "label is not snake_case: {l}"
            );
        }
    }

    #[test]
    fn warning_labels_are_snake_case() {
        let warnings = [
            CollapseWarning::ReplayBufferEmpty,
            CollapseWarning::ReplayAllCorrupt,
            CollapseWarning::NoiseIneffective {
                variance_after: 1.0,
            },
            CollapseWarning::PartialRespawnBelowMinimum {
                would_deactivate: 0,
                min_required: 0,
            },
            CollapseWarning::DeadlineExceeded {
                budget_us: 0,
                actual_us: 0,
            },
            CollapseWarning::CircuitBreaker {
                swarm_collapse_count: 0,
            },
            CollapseWarning::Abandoned,
            CollapseWarning::PostConditionFailed,
        ];
        for w in warnings {
            let l = w.label();
            assert!(l.chars().all(|c| c.is_ascii_lowercase() || c == '_'));
        }
    }

    #[test]
    fn is_fatal_covers_expected_warnings() {
        assert!(!CollapseWarning::ReplayBufferEmpty.is_fatal());
        assert!(!CollapseWarning::NoiseIneffective {
            variance_after: 1.0
        }
        .is_fatal());
        assert!(CollapseWarning::CircuitBreaker {
            swarm_collapse_count: 5
        }
        .is_fatal());
        assert!(CollapseWarning::Abandoned.is_fatal());
        assert!(CollapseWarning::PostConditionFailed.is_fatal());
    }

    #[test]
    fn serde_round_trip_action() {
        let a = CollapseAction::NoiseInjection {
            sigma: 0.1,
            seed: 42,
        };
        let s = serde_json::to_string(&a).unwrap();
        let back: CollapseAction = serde_json::from_str(&s).unwrap();
        assert_eq!(a, back);
    }

    #[test]
    fn display_outcome_readable() {
        let o = CollapseOutcome {
            action_taken: CollapseAction::SoftRollback { to_round: 2 },
            success: true,
            trigger: mk_trigger(),
            new_round: Some(2),
            variance_after: Some(0.1),
            recovery_confidence: 0.82,
            warnings: vec![],
            duration_us: 500,
            strategy_name: "soft-rollback".into(),
        };
        let s = format!("{o}");
        assert!(s.contains("soft-rollback"));
        assert!(s.contains("ok"));
        assert!(s.contains("500"));
    }
}
