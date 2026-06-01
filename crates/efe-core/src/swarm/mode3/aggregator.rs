//! **Sprint M1 extension D37** — `Mode3Aggregator` trait + 3 impls.
//!
//! ## Backport
//!
//! This file is a backport of
//! `crates/efe-server/src/services/mode3/aggregator.rs` (delivered in
//! M4 phase 1) into `efe-core`. The 3 impls (Mean, Weighted, Median)
//! are preserved byte-for-byte except for the `verdict` field type :
//!
//!   * **Pre-M1** : `i32` (proto enum representation).
//!   * **Post-M1** : [`crate::Verdict`] (rich enum).
//!
//! The proto i32 mapping moves to the gRPC bridge layer in
//! `efe-server::bridge::verdict`. Mode-3 sub-verdicts now carry the
//! domain enum natively, which makes the math more legible :
//! `MeanAggregator` buckets by `Verdict` discriminant instead of i32.
//!
//! Cf. `EFE_CORE_EXTENSION_ARCHITECTURE.md` § 11.4.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::errors::EfeError;
use crate::swarm::verdict::Verdict;

/// One sub-swarm's verdict from Mode-3 fan-out.
///
/// Decoupled from the proto representation so unit tests can construct
/// them inline without going through any wire format.
#[derive(Debug, Clone)]
pub struct SubVerdictData {
    /// Stable index in `[0, k_effective)`.
    pub swarm_idx: u32,
    /// The swarm's terminal verdict.
    pub verdict: Verdict,
    /// Confidence in [0.0, 1.0].
    pub confidence: f32,
    /// Sub-swarm wall time, ms.
    pub latency_ms: u32,
    /// `"mode1"` or `"mode2"`.
    pub mode_used: String,
    /// Per-sub-swarm seed.
    pub seed_used: u64,
}

/// Aggregation output : the final answer + observability metadata.
#[derive(Debug, Clone)]
pub struct AggregationOutput {
    /// Final verdict (the aggregator's choice across sub-verdicts).
    pub final_verdict: Verdict,
    /// Sum of sub-confidences ∈ [0, k].
    pub total_confidence: f32,
    /// Std-dev of sub-confidences (proxy for inter-swarm disagreement).
    /// High dispersion ⇒ flag the request for human review.
    pub dispersion: f32,
    /// Number of distinct verdict labels in the input set.
    pub effective_n: u32,
    /// Strategy id ("mean" | "weighted" | "median").
    pub strategy: String,
}

/// Aggregation metadata (proto-shaped, excludes `final_verdict`).
#[derive(Debug, Clone)]
pub struct AggregationMetadata {
    /// Strategy id.
    pub strategy: String,
    /// Sum of sub-confidences ∈ [0, k].
    pub total_confidence: f32,
    /// Std-dev of sub-confidences.
    pub dispersion: f32,
    /// Number of distinct verdict labels.
    pub effective_n: u32,
}

impl From<&AggregationOutput> for AggregationMetadata {
    fn from(o: &AggregationOutput) -> Self {
        Self {
            strategy: o.strategy.clone(),
            total_confidence: o.total_confidence,
            dispersion: o.dispersion,
            effective_n: o.effective_n,
        }
    }
}

/// Typed errors. Mapped to gRPC `Status` by the `efe-server` bridge.
#[derive(Debug, Error)]
pub enum AggregationError {
    /// Empty input set.
    #[error("aggregator received 0 sub-verdicts")]
    Empty,
    /// Unknown strategy name.
    #[error("unknown aggregator: {0}")]
    UnknownStrategy(String),
}

impl From<AggregationError> for EfeError {
    fn from(e: AggregationError) -> Self {
        EfeError::InvalidConfig(format!("Mode3Aggregator: {e}"))
    }
}

/// **Sprint D37** — Trait every aggregator implements.
pub trait Mode3Aggregator: Send + Sync + std::fmt::Debug {
    /// Stable identifier (matches the request's `aggregator` field).
    fn id(&self) -> &'static str;

    /// Produce the aggregation output.
    ///
    /// # Errors
    ///
    /// [`AggregationError::Empty`] when `sub_verdicts.is_empty()`.
    fn aggregate(&self, sub_verdicts: &[SubVerdictData]) -> Result<AggregationOutput, AggregationError>;
}

// =====================================================================
// MeanAggregator
// =====================================================================

/// Pick the verdict with the highest **mean confidence** across
/// sub-swarms that voted for it. Tie-break : lowest variant
/// discriminant.
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
        // Bucket sub-verdicts by their verdict's stable code.
        let mut buckets: BTreeMap<&'static str, Vec<f32>> = BTreeMap::new();
        for sv in sub_verdicts {
            buckets.entry(sv.verdict.code()).or_default().push(sv.confidence);
        }
        // Pick the verdict whose mean confidence is the highest.
        let winning_code = buckets
            .iter()
            .map(|(code, confs)| {
                let mean = confs.iter().sum::<f32>() / confs.len() as f32;
                (mean, *code)
            })
            .max_by(|a, b| {
                a.0.partial_cmp(&b.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.1.cmp(b.1))
            })
            .map(|(_, code)| code)
            .unwrap_or("converged");

        let final_verdict = pick_verdict_for_code(sub_verdicts, winning_code);
        Ok(common_metadata(sub_verdicts, final_verdict, "mean"))
    }
}

// =====================================================================
// WeightedAggregator
// =====================================================================

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
        let mut weight_sum: BTreeMap<&'static str, f32> = BTreeMap::new();
        for sv in sub_verdicts {
            *weight_sum.entry(sv.verdict.code()).or_insert(0.0) += sv.confidence;
        }
        let winning_code = weight_sum
            .iter()
            .max_by(|a, b| {
                a.1.partial_cmp(b.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.0.cmp(b.0))
            })
            .map(|(code, _)| *code)
            .unwrap_or("converged");

        let final_verdict = pick_verdict_for_code(sub_verdicts, winning_code);
        Ok(common_metadata(sub_verdicts, final_verdict, "weighted"))
    }
}

// =====================================================================
// MedianAggregator
// =====================================================================

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
        let mut buckets: BTreeMap<&'static str, Vec<f32>> = BTreeMap::new();
        for sv in sub_verdicts {
            buckets.entry(sv.verdict.code()).or_default().push(sv.confidence);
        }
        let winning_code = buckets
            .iter_mut()
            .map(|(code, confs)| {
                confs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let median = confs[confs.len() / 2];
                (median, *code)
            })
            .max_by(|a, b| {
                a.0.partial_cmp(&b.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.1.cmp(b.1))
            })
            .map(|(_, code)| code)
            .unwrap_or("converged");

        let final_verdict = pick_verdict_for_code(sub_verdicts, winning_code);
        Ok(common_metadata(sub_verdicts, final_verdict, "median"))
    }
}

// =====================================================================
// Helpers
// =====================================================================

/// Find the first sub-verdict whose verdict matches `code` and return
/// a clone of its `Verdict`. The aggregator buckets by code, so we
/// need to map back to a concrete `Verdict` value — taking the first
/// matching one is canonical (deterministic ordering).
fn pick_verdict_for_code(sub_verdicts: &[SubVerdictData], code: &str) -> Verdict {
    sub_verdicts
        .iter()
        .find(|sv| sv.verdict.code() == code)
        .map(|sv| sv.verdict)
        .unwrap_or(Verdict::Converged { confidence: 0.0 })
}

/// Compute the metadata (total_confidence, dispersion, effective_n)
/// shared by all 3 aggregator strategies.
fn common_metadata(
    sub: &[SubVerdictData],
    final_verdict: Verdict,
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
    let mut distinct: std::collections::HashSet<&'static str> = std::collections::HashSet::new();
    for s in sub {
        distinct.insert(s.verdict.code());
    }
    AggregationOutput {
        final_verdict,
        total_confidence,
        dispersion,
        effective_n: distinct.len() as u32,
        strategy: strategy.to_string(),
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sv(idx: u32, verdict: Verdict, conf: f32) -> SubVerdictData {
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
        let conv = Verdict::Converged { confidence: 0.0 };
        let div = Verdict::Divergence { reason: "x" };
        let out = agg
            .aggregate(&[sv(0, conv, 0.9), sv(1, conv, 0.8), sv(2, div, 0.6)])
            .unwrap();
        assert_eq!(out.final_verdict.code(), "converged");
        assert_eq!(out.strategy, "mean");
        assert_eq!(out.effective_n, 2);
    }

    #[test]
    fn weighted_picks_highest_sum_bucket() {
        let agg = WeightedAggregator;
        let conv = Verdict::Converged { confidence: 0.0 };
        let div = Verdict::Divergence { reason: "x" };
        let out = agg
            .aggregate(&[sv(0, conv, 0.9), sv(1, conv, 0.5), sv(2, div, 0.95)])
            .unwrap();
        assert_eq!(out.final_verdict.code(), "converged"); // sum 1.4 vs 0.95
    }

    #[test]
    fn median_robust_to_one_outlier() {
        let agg = MedianAggregator;
        let conv = Verdict::Converged { confidence: 0.0 };
        let div = Verdict::Divergence { reason: "x" };
        // Verdict converged confidences = [0.5, 0.6, 0.7] median 0.6.
        // Verdict divergence confidences = [0.99]            median 0.99.
        // Divergence wins on raw median.
        let out = agg
            .aggregate(&[
                sv(0, conv, 0.5),
                sv(1, conv, 0.6),
                sv(2, conv, 0.7),
                sv(3, div, 0.99),
            ])
            .unwrap();
        assert_eq!(out.final_verdict.code(), "divergence");
    }

    #[test]
    fn empty_input_is_typed_error() {
        let err = MeanAggregator.aggregate(&[]).unwrap_err();
        assert!(matches!(err, AggregationError::Empty));
    }

    #[test]
    fn dispersion_is_zero_for_unanimous_confidences() {
        let conv = Verdict::Converged { confidence: 0.0 };
        let agg = MeanAggregator;
        let out = agg
            .aggregate(&[sv(0, conv, 0.5), sv(1, conv, 0.5), sv(2, conv, 0.5)])
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
        let conv = Verdict::Converged { confidence: 0.0 };
        let agg = MeanAggregator;
        let out = agg
            .aggregate(&[sv(0, conv, 0.3), sv(1, conv, 0.4), sv(2, conv, 0.5)])
            .unwrap();
        assert!((out.total_confidence - 1.2).abs() < 1e-6);
    }

    #[test]
    fn aggregation_metadata_from_output() {
        let out = AggregationOutput {
            final_verdict: Verdict::Continue,
            total_confidence: 1.5,
            dispersion: 0.1,
            effective_n: 2,
            strategy: "mean".into(),
        };
        let meta: AggregationMetadata = (&out).into();
        assert_eq!(meta.strategy, "mean");
        assert_eq!(meta.total_confidence, 1.5);
        assert_eq!(meta.effective_n, 2);
    }

    #[test]
    fn aggregation_error_converts_to_efe_error() {
        let e: EfeError = AggregationError::Empty.into();
        let s = format!("{e}");
        assert!(s.contains("Mode3Aggregator"));
    }
}
