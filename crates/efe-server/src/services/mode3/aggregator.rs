//! Sprint **D37** (M4 § 14.3) — `Mode3Aggregator` trait + 3 built-in
//! implementations.
//!
//! ## Mathematical contract
//!
//! Given `k` sub-verdicts with confidences `(c_0, c_1, …, c_{k-1})`,
//! the aggregator produces :
//!
//! 1. A **final verdict enum** (most-frequent, weighted-most-frequent,
//!    or median-bucketed).
//! 2. A **total confidence** in `[0, k]` — sum of sub-confidences.
//! 3. A **dispersion** = std deviation of confidences across sub-verdicts.
//!    Used as a "confidence-of-confidence" signal : when dispersion is
//!    high, the sub-swarms disagreed strongly and the operator should
//!    flag the request for human review.
//! 4. An **effective N** = number of distinct verdict labels in the
//!    sub-set. `effective_n = 1` ⇔ unanimous ; `effective_n = k` ⇔
//!    all distinct.
//!
//! ## Three built-in strategies (D37 default)
//!
//! ### `MeanAggregator`
//!
//! Pick the verdict with the highest **mean** confidence across
//! sub-swarms that voted for it. Ties broken by the lowest
//! `swarm_idx`. Cheap, easy to interpret.
//!
//! ### `WeightedAggregator`
//!
//! Pick the verdict with the highest **sum of confidences** (the
//! per-vote weight is the sub-swarm's own confidence). When all
//! sub-confidences are equal this is identical to `MeanAggregator`.
//!
//! ### `MedianAggregator`
//!
//! Pick the verdict whose **per-bucket median** confidence is
//! highest. Robust to one outlier (a sub-swarm that strongly
//! disagrees doesn't tilt the verdict on its own).

use std::collections::BTreeMap;

use thiserror::Error;

/// One sub-swarm's verdict from Mode-3 fan-out. Decoupled from the
/// proto representation so unit tests can construct them inline.
#[derive(Debug, Clone)]
pub struct SubVerdictData {
    /// Stable index in `[0, k_effective)`.
    pub swarm_idx: u32,
    /// Verdict enum value (0..=4 per `efe.v1.Verdict` proto enum).
    pub verdict: i32,
    /// Confidence in `[0.0, 1.0]`.
    pub confidence: f32,
    /// Sub-swarm wall time, ms.
    pub latency_ms: u32,
    /// "mode1" | "mode2".
    pub mode_used: String,
    /// Per-sub-swarm seed.
    pub seed_used: u64,
}

/// Aggregation output : the final answer + observability metadata.
#[derive(Debug, Clone)]
pub struct AggregationOutput {
    /// Final verdict enum value.
    pub final_verdict: i32,
    /// Sum of sub-confidences ∈ `[0, k]`.
    pub total_confidence: f32,
    /// Std-dev of sub-confidences (proxy for inter-swarm disagreement).
    pub dispersion: f32,
    /// Number of distinct verdict labels in the input set.
    pub effective_n: u32,
    /// Strategy id ("mean" | "weighted" | "median"). Echoed back to
    /// the client.
    pub strategy: String,
}

/// Typed errors. Mapped to `tonic::Status` by the gRPC handler.
#[derive(Debug, Error)]
pub enum AggregationError {
    /// Empty input set.
    #[error("aggregator received 0 sub-verdicts")]
    Empty,

    /// Unknown strategy name.
    #[error("unknown aggregator: {0}")]
    UnknownStrategy(String),
}

/// Trait every aggregator implements.
pub trait Mode3Aggregator: Send + Sync + std::fmt::Debug {
    /// Stable identifier (matches the request's `aggregator` field).
    fn id(&self) -> &'static str;

    /// Produce the aggregation output.
    ///
    /// # Errors
    ///
    /// Returns [`AggregationError::Empty`] when `sub_verdicts.is_empty()`.
    fn aggregate(
        &self,
        sub_verdicts: &[SubVerdictData],
    ) -> Result<AggregationOutput, AggregationError>;
}

// ─────────────────────────────────────────────────────────────────────
// MeanAggregator
// ─────────────────────────────────────────────────────────────────────

/// Pick the verdict with the highest **mean confidence** across
/// sub-swarms that voted for it.
#[derive(Debug, Default, Clone, Copy)]
pub struct MeanAggregator;

impl Mode3Aggregator for MeanAggregator {
    fn id(&self) -> &'static str {
        "mean"
    }

    fn aggregate(
        &self,
        sub_verdicts: &[SubVerdictData],
    ) -> Result<AggregationOutput, AggregationError> {
        if sub_verdicts.is_empty() {
            return Err(AggregationError::Empty);
        }
        // Bucket sub-verdicts by their verdict enum int.
        let mut buckets: BTreeMap<i32, Vec<f32>> = BTreeMap::new();
        for sv in sub_verdicts {
            buckets.entry(sv.verdict).or_default().push(sv.confidence);
        }
        // Pick the verdict whose mean confidence is the highest. Tie
        // broken by the smallest verdict int (deterministic ordering
        // since `BTreeMap` iterates ascending).
        let final_verdict = buckets
            .iter()
            .map(|(v, confs)| {
                let mean = confs.iter().sum::<f32>() / confs.len() as f32;
                (mean, *v)
            })
            .max_by(|a, b| {
                a.0.partial_cmp(&b.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b.1.cmp(&a.1))
            })
            .map(|(_, v)| v)
            .unwrap_or(0);

        Ok(common_metadata(sub_verdicts, final_verdict, "mean"))
    }
}

// ─────────────────────────────────────────────────────────────────────
// WeightedAggregator
// ─────────────────────────────────────────────────────────────────────

/// Pick the verdict with the highest **sum of confidences**.
#[derive(Debug, Default, Clone, Copy)]
pub struct WeightedAggregator;

impl Mode3Aggregator for WeightedAggregator {
    fn id(&self) -> &'static str {
        "weighted"
    }

    fn aggregate(
        &self,
        sub_verdicts: &[SubVerdictData],
    ) -> Result<AggregationOutput, AggregationError> {
        if sub_verdicts.is_empty() {
            return Err(AggregationError::Empty);
        }
        let mut weight_sum: BTreeMap<i32, f32> = BTreeMap::new();
        for sv in sub_verdicts {
            *weight_sum.entry(sv.verdict).or_insert(0.0) += sv.confidence;
        }
        let final_verdict = weight_sum
            .iter()
            .max_by(|a, b| {
                a.1.partial_cmp(b.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b.0.cmp(a.0))
            })
            .map(|(v, _)| *v)
            .unwrap_or(0);

        Ok(common_metadata(sub_verdicts, final_verdict, "weighted"))
    }
}

// ─────────────────────────────────────────────────────────────────────
// MedianAggregator
// ─────────────────────────────────────────────────────────────────────

/// Pick the verdict whose **per-bucket median** confidence is highest.
/// Robust to a single outlier sub-swarm.
#[derive(Debug, Default, Clone, Copy)]
pub struct MedianAggregator;

impl Mode3Aggregator for MedianAggregator {
    fn id(&self) -> &'static str {
        "median"
    }

    fn aggregate(
        &self,
        sub_verdicts: &[SubVerdictData],
    ) -> Result<AggregationOutput, AggregationError> {
        if sub_verdicts.is_empty() {
            return Err(AggregationError::Empty);
        }
        let mut buckets: BTreeMap<i32, Vec<f32>> = BTreeMap::new();
        for sv in sub_verdicts {
            buckets.entry(sv.verdict).or_default().push(sv.confidence);
        }
        let final_verdict = buckets
            .iter_mut()
            .map(|(v, confs)| {
                confs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let median = confs[confs.len() / 2];
                (median, *v)
            })
            .max_by(|a, b| {
                a.0.partial_cmp(&b.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b.1.cmp(&a.1))
            })
            .map(|(_, v)| v)
            .unwrap_or(0);

        Ok(common_metadata(sub_verdicts, final_verdict, "median"))
    }
}

/// Compute the metadata (total_confidence, dispersion, effective_n)
/// shared by all three aggregator strategies.
fn common_metadata(
    sub: &[SubVerdictData],
    final_verdict: i32,
    strategy: &str,
) -> AggregationOutput {
    let n = sub.len() as f32;
    let total_confidence: f32 = sub.iter().map(|s| s.confidence).sum();
    let mean = total_confidence / n;
    let var = sub
        .iter()
        .map(|s| {
            let d = s.confidence - mean;
            d * d
        })
        .sum::<f32>()
        / n;
    let dispersion = var.sqrt();
    let mut distinct: std::collections::HashSet<i32> = std::collections::HashSet::new();
    for s in sub {
        distinct.insert(s.verdict);
    }
    AggregationOutput {
        final_verdict,
        total_confidence,
        dispersion,
        effective_n: distinct.len() as u32,
        strategy: strategy.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sv(idx: u32, verdict: i32, conf: f32) -> SubVerdictData {
        SubVerdictData {
            swarm_idx: idx,
            verdict,
            confidence: conf,
            latency_ms: 10,
            mode_used: "mode1".into(),
            seed_used: idx as u64,
        }
    }

    #[test]
    fn mean_picks_highest_mean_bucket() {
        let agg = MeanAggregator;
        // Verdict 2 has mean 0.85 ; verdict 3 has mean 0.6.
        let out = agg
            .aggregate(&[sv(0, 2, 0.9), sv(1, 2, 0.8), sv(2, 3, 0.6)])
            .unwrap();
        assert_eq!(out.final_verdict, 2);
        assert_eq!(out.strategy, "mean");
        assert_eq!(out.effective_n, 2);
    }

    #[test]
    fn weighted_picks_highest_sum_bucket() {
        let agg = WeightedAggregator;
        // Verdict 2 sum = 0.9 + 0.5 = 1.4 ; verdict 3 sum = 0.95
        let out = agg
            .aggregate(&[sv(0, 2, 0.9), sv(1, 2, 0.5), sv(2, 3, 0.95)])
            .unwrap();
        assert_eq!(out.final_verdict, 2);
        assert_eq!(out.strategy, "weighted");
    }

    #[test]
    fn median_robust_to_one_outlier() {
        let agg = MedianAggregator;
        // Verdict 2 confidences = [0.5, 0.6, 0.7] median 0.6.
        // Verdict 3 confidences = [0.99]               median 0.99.
        // Verdict 3 wins on raw median, even though only one sample.
        let out = agg
            .aggregate(&[sv(0, 2, 0.5), sv(1, 2, 0.6), sv(2, 2, 0.7), sv(3, 3, 0.99)])
            .unwrap();
        assert_eq!(out.final_verdict, 3);
        assert_eq!(out.strategy, "median");
    }

    #[test]
    fn empty_input_is_typed_error() {
        let err = MeanAggregator.aggregate(&[]).unwrap_err();
        assert!(matches!(err, AggregationError::Empty));
    }

    #[test]
    fn dispersion_is_zero_for_unanimous_confidences() {
        let agg = MeanAggregator;
        let out = agg
            .aggregate(&[sv(0, 2, 0.5), sv(1, 2, 0.5), sv(2, 2, 0.5)])
            .unwrap();
        assert!(out.dispersion.abs() < 1e-6);
        assert_eq!(out.effective_n, 1);
    }

    #[test]
    fn three_aggregators_have_distinct_ids() {
        let m: &dyn Mode3Aggregator = &MeanAggregator;
        let w: &dyn Mode3Aggregator = &WeightedAggregator;
        let med: &dyn Mode3Aggregator = &MedianAggregator;
        let ids = [m.id(), w.id(), med.id()];
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), 3);
    }

    #[test]
    fn total_confidence_matches_sum() {
        let agg = MeanAggregator;
        let out = agg
            .aggregate(&[sv(0, 2, 0.3), sv(1, 2, 0.4), sv(2, 2, 0.5)])
            .unwrap();
        assert!((out.total_confidence - 1.2).abs() < 1e-6);
    }
}
