//! Per-round outcome types: `FusionRound`, `RoundTiming`, `FusionWarning`.
//!
//! A `FusionRound` is the authoritative record of a single application
//! of the fusion operator. It is serializable (for M11 telemetry) and
//! carries every scalar needed by M6 (consensus) and M1 (controller).

use serde::{Deserialize, Serialize};

use crate::fusion::stats::FusionStats;

/// Outcome of a single fusion round.
///
/// Returned by [`crate::fusion::FusionStrategy::run_round`]. Consumed by:
///
/// - M6 (consensus detector): reads `stats` and `warnings`.
/// - M1 (controller): reads `on_time`, decides whether to continue.
/// - M11 (telemetry): serializes the whole struct.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FusionRound {
    /// Round index (0-based).
    pub round: u32,
    /// Statistics emitted to M6 and M11.
    pub stats: FusionStats,
    /// Per-kernel timing (μs).
    pub timing: RoundTiming,
    /// Backend that executed this round (e.g. `"mvp-cpu"`).
    pub backend: String,
    /// Was the round completed within the configured deadline?
    pub on_time: bool,
    /// Non-fatal issues encountered during the round.
    pub warnings: Vec<FusionWarning>,
}

impl FusionRound {
    /// True iff this round is clean: on time, no warnings, no anomaly.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.on_time && self.warnings.is_empty() && !self.stats.has_numerical_issues()
    }

    /// Count of warnings of a given discriminant. Handy in tests.
    #[must_use]
    pub fn warning_count(&self) -> usize {
        self.warnings.len()
    }

    /// True iff any warning is a `DeadlineExceeded`.
    #[must_use]
    pub fn has_deadline_warning(&self) -> bool {
        self.warnings
            .iter()
            .any(|w| matches!(w, FusionWarning::DeadlineExceeded { .. }))
    }

    /// True iff any warning is a `ClippedCoordinates`.
    #[must_use]
    pub fn has_clipping(&self) -> bool {
        self.warnings
            .iter()
            .any(|w| matches!(w, FusionWarning::ClippedCoordinates { .. }))
    }
}

/// Per-stage timing (μs) for one fusion round.
///
/// All fields are 0 when a stage was not run (e.g. projection stage may
/// skip when α = 0). Sum of stages is typically close to `total_us`; the
/// delta accounts for orchestration overhead.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct RoundTiming {
    /// Time spent computing attention scores.
    pub scores_us: u32,
    /// Time spent on softmax.
    pub softmax_us: u32,
    /// Time spent on weighted aggregation.
    pub aggregate_us: u32,
    /// Time spent mixing the problem anchor.
    pub beta_mix_us: u32,
    /// Time spent projecting onto the alignment cone.
    pub project_us: u32,
    /// Time spent clipping coordinates.
    pub clip_us: u32,
    /// Time spent computing pre- and post- stats.
    pub stats_us: u32,
    /// Total wallclock of the round.
    pub total_us: u32,
}

impl RoundTiming {
    /// Sum of per-stage timings. Should approximate `total_us`.
    #[must_use]
    pub fn stages_us(&self) -> u32 {
        self.scores_us
            .saturating_add(self.softmax_us)
            .saturating_add(self.aggregate_us)
            .saturating_add(self.beta_mix_us)
            .saturating_add(self.project_us)
            .saturating_add(self.clip_us)
            .saturating_add(self.stats_us)
    }
}

/// Non-fatal issues reported by a fusion round.
///
/// Warnings do not abort the round — they are logged and surface to M11
/// for observability. The controller (M1) may use them to tighten future
/// rounds' configuration (e.g. lower temperature on repeated clipping).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum FusionWarning {
    /// Some coordinates were clipped to `±clip_max`.
    ///
    /// Repeated across rounds → the swarm is drifting; M1 may want to
    /// increase alignment strength.
    ClippedCoordinates {
        /// Number of rows with at least one clipped coordinate.
        count: u32,
        /// Worst absolute value observed before clipping.
        max_abs: f32,
    },

    /// All anchors received zero positive weight — projection became identity.
    ///
    /// Means the swarm latent is orthogonal to the alignment cone; this
    /// is bad alignment signal and may indicate a mis-specified cone.
    AnchorAllZeroWeight,

    /// The round exceeded its configured deadline.
    DeadlineExceeded {
        /// Configured budget (μs).
        budget_us: u32,
        /// Actual duration (μs).
        actual_us: u32,
    },

    /// Variance grew this round (expected to shrink on average).
    ///
    /// One-off spikes are normal near initialization; persistent growth
    /// suggests divergence.
    HighVarianceDelta {
        /// Variance before fusion.
        pre: f32,
        /// Variance after fusion.
        post: f32,
    },

    /// Softmax produced at least one row whose entries were all `-inf`
    /// (no valid attention targets). Row was treated as identity.
    ZeroAttentionRow {
        /// Count of rows affected.
        count: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_round(round: u32) -> FusionRound {
        FusionRound {
            round,
            stats: FusionStats::synthetic_converging(round, 4),
            timing: RoundTiming::default(),
            backend: "mvp-cpu".into(),
            on_time: true,
            warnings: vec![],
        }
    }

    #[test]
    fn is_clean_on_default_synthetic() {
        let r = mk_round(0);
        assert!(r.is_clean());
    }

    #[test]
    fn is_clean_false_on_warning() {
        let mut r = mk_round(0);
        r.warnings.push(FusionWarning::AnchorAllZeroWeight);
        assert!(!r.is_clean());
    }

    #[test]
    fn is_clean_false_on_nan() {
        let mut r = mk_round(0);
        r.stats.has_nan = true;
        assert!(!r.is_clean());
    }

    #[test]
    fn warning_count_matches_vec_len() {
        let mut r = mk_round(0);
        r.warnings.push(FusionWarning::AnchorAllZeroWeight);
        r.warnings.push(FusionWarning::ClippedCoordinates {
            count: 2,
            max_abs: 33.0,
        });
        assert_eq!(r.warning_count(), 2);
    }

    #[test]
    fn has_deadline_warning_detects() {
        let mut r = mk_round(0);
        assert!(!r.has_deadline_warning());
        r.warnings.push(FusionWarning::DeadlineExceeded {
            budget_us: 30_000,
            actual_us: 45_000,
        });
        assert!(r.has_deadline_warning());
    }

    #[test]
    fn has_clipping_detects() {
        let mut r = mk_round(0);
        assert!(!r.has_clipping());
        r.warnings.push(FusionWarning::ClippedCoordinates {
            count: 1,
            max_abs: 33.0,
        });
        assert!(r.has_clipping());
    }

    #[test]
    fn round_timing_stages_us_sums_correctly() {
        let t = RoundTiming {
            scores_us: 1,
            softmax_us: 2,
            aggregate_us: 3,
            beta_mix_us: 4,
            project_us: 5,
            clip_us: 6,
            stats_us: 7,
            total_us: 30,
        };
        assert_eq!(t.stages_us(), 28);
    }

    #[test]
    fn round_serializes_to_json() {
        let r = mk_round(2);
        let s = serde_json::to_string(&r).unwrap();
        let back: FusionRound = serde_json::from_str(&s).unwrap();
        assert_eq!(back.round, 2);
        assert_eq!(back.backend, "mvp-cpu");
    }
}
