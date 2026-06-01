//! M23 curation bench scenarios.
//!
//! Each scenario evaluates one aspect of the curation system against a
//! known gold dataset and produces metrics that gate GA promotion
//! (cf. § 24.4 of the M23 architecture doc).

pub mod propose_split_quality;
pub mod explain_actionability;
pub mod synth_diversity;
pub mod merge_correctness;
pub mod audit_metric_reproducibility;
pub mod drift_detection_precision;
pub mod canary_safety;
pub mod e2e_propose_review_promote;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchResult {
    pub scenario: String,
    pub status: BenchStatus,
    pub metric_name: String,
    pub measured: f64,
    pub threshold: f64,
    pub direction: Direction,  // measured must be >= or <= threshold
    pub n_samples: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum BenchStatus { Pass, Warn, Fail }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum Direction { GreaterEq, LessEq }

impl BenchResult {
    pub fn evaluate(scenario: &str, metric: &str, measured: f64, threshold: f64,
                    dir: Direction, n: u64, elapsed_ms: u64, warn_only: bool) -> Self {
        let pass = match dir {
            Direction::GreaterEq => measured >= threshold,
            Direction::LessEq => measured <= threshold,
        };
        let status = if pass { BenchStatus::Pass }
                     else if warn_only { BenchStatus::Warn }
                     else { BenchStatus::Fail };
        Self {
            scenario: scenario.into(), status, metric_name: metric.into(),
            measured, threshold, direction: dir, n_samples: n, elapsed_ms,
        }
    }
}
