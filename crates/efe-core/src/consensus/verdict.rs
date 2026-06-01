//! `ConsensusVerdict` — the decision emitted by M6 on every ingest.
//!
//! The verdict is a tagged union, not a float: M1 (the controller) needs
//! to branch **precisely** on what to do next. A `Converged` carries its
//! confidence; a `Diverged` carries the precise reason (for M11 and for
//! operators); a `Stalled` carries the flat-window length.
//!
//! All variants are `#[non_exhaustive]` so adding new kinds later (e.g.
//! `Exploring` for meta-learning experiments) is not a breaking change.

use serde::{Deserialize, Serialize};

/// The decision emitted by M6 after ingesting one `FusionStats`.
///
/// See the type-level docs for the semantics of each variant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ConsensusVerdict {
    /// Not converged yet — controller continues fusion.
    Continue,

    /// Converged with the given confidence ∈ [threshold, 1].
    Converged {
        /// Confidence at the moment of verdict emission, ∈ `[threshold, 1]`.
        confidence: f32,
    },

    /// Diverged — controller triggers M7 (collapse + rollback).
    Diverged {
        /// Specific reason, for telemetry and operator-facing logs.
        reason: DivergenceReason,
    },

    /// Stalled on a plateau — low-confidence flat region.
    ///
    /// Controller may extend the campaign, accept as weak consensus,
    /// or fail — M6 does not prescribe.
    Stalled {
        /// Number of rounds that stayed flat.
        rounds_flat: u32,
        /// Confidence at the moment the stall was detected.
        confidence: f32,
    },

    /// Numerical anomaly — controller triggers M7 (collapse).
    Anomaly {
        /// Specific anomaly code.
        code: AnomalyCode,
    },
}

impl ConsensusVerdict {
    /// True iff this verdict is terminal (the controller should stop
    /// the fusion loop). Covers `Converged`, `Diverged`, `Anomaly`.
    ///
    /// `Stalled` is **not** terminal by default — the controller's
    /// policy decides.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Converged { .. } | Self::Diverged { .. } | Self::Anomaly { .. }
        )
    }

    /// True iff the controller should roll back the latent state via M7.
    ///
    /// `Diverged` and `Anomaly` both imply rollback; `Converged` does
    /// not; `Stalled` does not (the latent is still coherent).
    #[must_use]
    pub fn should_rollback(&self) -> bool {
        matches!(self, Self::Diverged { .. } | Self::Anomaly { .. })
    }

    /// Stable label string for telemetry / metric cardinality.
    ///
    /// These values are part of the observability contract and must
    /// not change across MINOR bumps.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::Converged { .. } => "converged",
            Self::Diverged { .. } => "diverged",
            Self::Stalled { .. } => "stalled",
            Self::Anomaly { .. } => "anomaly",
        }
    }
}

impl Default for ConsensusVerdict {
    fn default() -> Self {
        Self::Continue
    }
}

impl std::fmt::Display for ConsensusVerdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Continue => f.write_str("continue"),
            Self::Converged { confidence } => {
                write!(f, "converged(confidence={confidence:.4})")
            }
            Self::Diverged { reason } => write!(f, "diverged({reason})"),
            Self::Stalled {
                rounds_flat,
                confidence,
            } => write!(
                f,
                "stalled(rounds={rounds_flat}, confidence={confidence:.4})"
            ),
            Self::Anomaly { code } => write!(f, "anomaly({code})"),
        }
    }
}

impl std::fmt::Display for DivergenceReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::VarianceExplosion { variance } => write!(f, "variance_explosion={variance:.4}"),
            Self::VarianceGrowing { slope, window } => {
                write!(f, "variance_growing(slope={slope:.6}, window={window})")
            }
            Self::ConfidenceDecay { from, to } => {
                write!(f, "confidence_decay({from:.4}→{to:.4})")
            }
        }
    }
}

impl std::fmt::Display for AnomalyCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NanInf => f.write_str("nan_inf"),
            Self::CentroidDimMismatch { prev, current } => {
                write!(f, "centroid_dim_mismatch(prev={prev}, current={current})")
            }
            Self::NonMonotonicRound { last, current } => {
                write!(f, "non_monotonic_round(last={last}, current={current})")
            }
        }
    }
}

/// Precise reason for a `Diverged` verdict.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DivergenceReason {
    /// Variance exceeded the `variance_explosion_threshold` config.
    VarianceExplosion {
        /// Observed variance.
        variance: f32,
    },
    /// Variance slope has been strictly positive over the full window
    /// and every round variance increased.
    VarianceGrowing {
        /// Median Theil-Sen slope over the window.
        slope: f32,
        /// Window size used for the trend estimate.
        window: u32,
    },
    /// Confidence dropped ≥ 50% from its peak.
    ConfidenceDecay {
        /// Peak confidence observed earlier.
        from: f32,
        /// Current confidence.
        to: f32,
    },
}

/// Anomaly kind for the `Anomaly` verdict.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AnomalyCode {
    /// NaN or Inf observed in the stats.
    NanInf,
    /// The centroid dimension changed between rounds.
    CentroidDimMismatch {
        /// Previous dim.
        prev: u32,
        /// Current dim.
        current: u32,
    },
    /// The round index went backward.
    NonMonotonicRound {
        /// Last round seen by the detector.
        last: u32,
        /// Current round being ingested.
        current: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continue_is_default() {
        let v = ConsensusVerdict::default();
        assert_eq!(v, ConsensusVerdict::Continue);
    }

    #[test]
    fn is_terminal_covers_expected_variants() {
        assert!(!ConsensusVerdict::Continue.is_terminal());
        assert!(ConsensusVerdict::Converged { confidence: 0.9 }.is_terminal());
        assert!(ConsensusVerdict::Diverged {
            reason: DivergenceReason::VarianceExplosion { variance: 1e6 }
        }
        .is_terminal());
        assert!(ConsensusVerdict::Anomaly {
            code: AnomalyCode::NanInf
        }
        .is_terminal());
        assert!(!ConsensusVerdict::Stalled {
            rounds_flat: 5,
            confidence: 0.3
        }
        .is_terminal());
    }

    #[test]
    fn should_rollback_covers_expected_variants() {
        assert!(!ConsensusVerdict::Continue.should_rollback());
        assert!(!ConsensusVerdict::Converged { confidence: 0.9 }.should_rollback());
        assert!(ConsensusVerdict::Diverged {
            reason: DivergenceReason::VarianceExplosion { variance: 1.0 }
        }
        .should_rollback());
        assert!(ConsensusVerdict::Anomaly {
            code: AnomalyCode::NanInf
        }
        .should_rollback());
        assert!(!ConsensusVerdict::Stalled {
            rounds_flat: 3,
            confidence: 0.5
        }
        .should_rollback());
    }

    #[test]
    fn labels_are_stable_snake_case() {
        let cases = [
            (ConsensusVerdict::Continue, "continue"),
            (ConsensusVerdict::Converged { confidence: 0.9 }, "converged"),
            (
                ConsensusVerdict::Diverged {
                    reason: DivergenceReason::VarianceExplosion { variance: 1.0 },
                },
                "diverged",
            ),
            (
                ConsensusVerdict::Stalled {
                    rounds_flat: 3,
                    confidence: 0.5,
                },
                "stalled",
            ),
            (
                ConsensusVerdict::Anomaly {
                    code: AnomalyCode::NanInf,
                },
                "anomaly",
            ),
        ];
        for (v, expected) in cases {
            assert_eq!(v.label(), expected);
            assert!(v.label().chars().all(|c| c.is_ascii_lowercase()));
        }
    }

    #[test]
    fn serde_round_trip_for_all_variants() {
        let verdicts = vec![
            ConsensusVerdict::Continue,
            ConsensusVerdict::Converged { confidence: 0.85 },
            ConsensusVerdict::Diverged {
                reason: DivergenceReason::VarianceGrowing {
                    slope: 0.02,
                    window: 6,
                },
            },
            ConsensusVerdict::Stalled {
                rounds_flat: 8,
                confidence: 0.42,
            },
            ConsensusVerdict::Anomaly {
                code: AnomalyCode::NonMonotonicRound {
                    last: 5,
                    current: 3,
                },
            },
        ];
        for v in verdicts {
            let s = serde_json::to_string(&v).unwrap();
            let back: ConsensusVerdict = serde_json::from_str(&s).unwrap();
            assert_eq!(v, back);
        }
    }

    #[test]
    fn anomaly_code_variants_serialize() {
        let codes = vec![
            AnomalyCode::NanInf,
            AnomalyCode::CentroidDimMismatch {
                prev: 1024,
                current: 2048,
            },
            AnomalyCode::NonMonotonicRound {
                last: 10,
                current: 9,
            },
        ];
        for c in codes {
            let s = serde_json::to_string(&c).unwrap();
            let back: AnomalyCode = serde_json::from_str(&s).unwrap();
            assert_eq!(c, back);
        }
    }

    #[test]
    fn display_includes_all_variants() {
        let cases = vec![
            ConsensusVerdict::Continue,
            ConsensusVerdict::Converged { confidence: 0.93 },
            ConsensusVerdict::Diverged {
                reason: DivergenceReason::VarianceExplosion { variance: 1e3 },
            },
            ConsensusVerdict::Stalled {
                rounds_flat: 7,
                confidence: 0.42,
            },
            ConsensusVerdict::Anomaly {
                code: AnomalyCode::NanInf,
            },
        ];
        for v in cases {
            let s = format!("{v}");
            assert!(!s.is_empty());
            assert!(s.contains(v.label()));
        }
    }

    #[test]
    fn display_formats_floats_compactly() {
        let v = ConsensusVerdict::Converged {
            confidence: 0.876_543,
        };
        let s = format!("{v}");
        // 4 decimal places → 0.8765
        assert!(s.contains("0.8765"), "got: {s}");
    }

    #[test]
    fn divergence_reason_serialization() {
        let reasons = vec![
            DivergenceReason::VarianceExplosion { variance: 1e6 },
            DivergenceReason::VarianceGrowing {
                slope: 0.15,
                window: 6,
            },
            DivergenceReason::ConfidenceDecay { from: 0.9, to: 0.4 },
        ];
        for r in reasons {
            let s = serde_json::to_string(&r).unwrap();
            let back: DivergenceReason = serde_json::from_str(&s).unwrap();
            assert_eq!(r, back);
        }
    }
}
