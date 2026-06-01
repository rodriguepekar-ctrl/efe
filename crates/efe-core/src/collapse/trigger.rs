//! `CollapseTrigger` — the cause of a collapse, derived from M6's
//! verdict or issued manually by the controller.

use serde::{Deserialize, Serialize};

use crate::consensus::verdict::{AnomalyCode, ConsensusVerdict, DivergenceReason};

/// What caused the collapse to be triggered.
///
/// Every field is owned (no lifetimes) so the trigger can be stored in
/// `CollapseEvent` history for M11 without lifetime pressure.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CollapseTrigger {
    /// M6 emitted `ConsensusVerdict::Diverged`.
    FromDivergence {
        /// Precise divergence reason (M6).
        reason: DivergenceReason,
        /// Round index at which divergence was declared.
        round: u32,
    },
    /// M6 emitted `ConsensusVerdict::Anomaly`.
    FromAnomaly {
        /// Specific anomaly code (M6).
        code: AnomalyCode,
        /// Round index at which anomaly was declared.
        round: u32,
    },
    /// M6 emitted `ConsensusVerdict::Stalled` and the controller
    /// decided to treat it as non-recoverable.
    FromStall {
        /// Number of rounds without progress.
        rounds_flat: u32,
        /// Confidence at stall detection.
        confidence: f32,
        /// Round index at which stall was declared.
        round: u32,
    },
    /// Manual intervention — operator or watchdog invoked collapse
    /// without a consensus verdict.
    Manual {
        /// Human-readable reason for audit logs.
        reason: String,
        /// Round index at the time of manual trigger.
        round: u32,
    },
}

impl CollapseTrigger {
    /// Derive a trigger from a consensus verdict.
    ///
    /// Returns `None` for non-collapse verdicts (`Continue`, `Converged`),
    /// `Some(trigger)` otherwise.
    #[must_use]
    pub fn from_verdict(verdict: &ConsensusVerdict, round: u32) -> Option<Self> {
        match verdict {
            ConsensusVerdict::Continue | ConsensusVerdict::Converged { .. } => None,
            ConsensusVerdict::Diverged { reason } => Some(Self::FromDivergence {
                reason: reason.clone(),
                round,
            }),
            ConsensusVerdict::Anomaly { code } => Some(Self::FromAnomaly {
                code: code.clone(),
                round,
            }),
            ConsensusVerdict::Stalled {
                rounds_flat,
                confidence,
            } => Some(Self::FromStall {
                rounds_flat: *rounds_flat,
                confidence: *confidence,
                round,
            }),
        }
    }

    /// Round index carried by the trigger.
    #[must_use]
    pub fn round(&self) -> u32 {
        match self {
            Self::FromDivergence { round, .. }
            | Self::FromAnomaly { round, .. }
            | Self::FromStall { round, .. }
            | Self::Manual { round, .. } => *round,
        }
    }

    /// Short stable label for metrics / logs.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::FromDivergence { .. } => "divergence",
            Self::FromAnomaly { .. } => "anomaly",
            Self::FromStall { .. } => "stall",
            Self::Manual { .. } => "manual",
        }
    }
}

impl std::fmt::Display for CollapseTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FromDivergence { reason, round } => {
                write!(f, "divergence@{round}({reason})")
            }
            Self::FromAnomaly { code, round } => {
                write!(f, "anomaly@{round}({code})")
            }
            Self::FromStall {
                rounds_flat,
                confidence,
                round,
            } => {
                write!(f, "stall@{round}(flat={rounds_flat}, conf={confidence:.4})")
            }
            Self::Manual { reason, round } => write!(f, "manual@{round}({reason})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_continue_returns_none() {
        assert!(CollapseTrigger::from_verdict(&ConsensusVerdict::Continue, 0).is_none());
    }

    #[test]
    fn from_converged_returns_none() {
        assert!(
            CollapseTrigger::from_verdict(&ConsensusVerdict::Converged { confidence: 0.9 }, 5)
                .is_none()
        );
    }

    #[test]
    fn from_diverged_captures_reason_and_round() {
        let v = ConsensusVerdict::Diverged {
            reason: DivergenceReason::VarianceExplosion { variance: 1e5 },
        };
        let t = CollapseTrigger::from_verdict(&v, 7).unwrap();
        assert!(matches!(
            t,
            CollapseTrigger::FromDivergence {
                reason: DivergenceReason::VarianceExplosion { .. },
                round: 7
            }
        ));
    }

    #[test]
    fn from_anomaly_captures_code() {
        let v = ConsensusVerdict::Anomaly {
            code: AnomalyCode::NanInf,
        };
        let t = CollapseTrigger::from_verdict(&v, 3).unwrap();
        assert!(matches!(
            t,
            CollapseTrigger::FromAnomaly {
                code: AnomalyCode::NanInf,
                round: 3,
            }
        ));
    }

    #[test]
    fn from_stalled_captures_fields() {
        let v = ConsensusVerdict::Stalled {
            rounds_flat: 6,
            confidence: 0.42,
        };
        let t = CollapseTrigger::from_verdict(&v, 10).unwrap();
        match t {
            CollapseTrigger::FromStall {
                rounds_flat,
                confidence,
                round,
            } => {
                assert_eq!(rounds_flat, 6);
                assert!((confidence - 0.42).abs() < 1e-6);
                assert_eq!(round, 10);
            }
            _ => panic!("expected FromStall"),
        }
    }

    #[test]
    fn round_accessor_returns_inner_round() {
        let t = CollapseTrigger::Manual {
            reason: "watchdog".into(),
            round: 42,
        };
        assert_eq!(t.round(), 42);
    }

    #[test]
    fn label_snake_case_stable() {
        for t in [
            CollapseTrigger::FromDivergence {
                reason: DivergenceReason::VarianceExplosion { variance: 1.0 },
                round: 0,
            },
            CollapseTrigger::FromAnomaly {
                code: AnomalyCode::NanInf,
                round: 0,
            },
            CollapseTrigger::FromStall {
                rounds_flat: 3,
                confidence: 0.2,
                round: 0,
            },
            CollapseTrigger::Manual {
                reason: String::new(),
                round: 0,
            },
        ] {
            let l = t.label();
            assert!(l.chars().all(|c| c.is_ascii_lowercase() || c == '_'));
        }
    }

    #[test]
    fn display_includes_trigger_info() {
        let t = CollapseTrigger::FromAnomaly {
            code: AnomalyCode::NanInf,
            round: 9,
        };
        let s = format!("{t}");
        assert!(s.contains("anomaly"));
        assert!(s.contains('9'));
    }

    #[test]
    fn serde_round_trip() {
        let t = CollapseTrigger::FromStall {
            rounds_flat: 5,
            confidence: 0.3,
            round: 1,
        };
        let s = serde_json::to_string(&t).unwrap();
        let back: CollapseTrigger = serde_json::from_str(&s).unwrap();
        assert_eq!(t, back);
    }
}
