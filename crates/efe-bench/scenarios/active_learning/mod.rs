//! M24 active learning bench scenarios.
//!
//! 6 scenarios covering : e2e full loop, holdout invariants, centroid
//! recalibration, promotion safety, GDPR Art. 17 cascade, adversarial
//! poisoning detection. See § 29 of the architecture doc.

pub mod adversarial_poisoning;
pub mod centroid_recalibration;
pub mod e2e_full_loop;
pub mod gdpr_art17_cascade;
pub mod holdout_invariants;
pub mod promotion_safety;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchResult {
    pub scenario: String,
    pub status: BenchStatus,
    pub metric_name: String,
    pub measured: f64,
    pub threshold: f64,
    pub direction: Direction,
    pub n_samples: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum BenchStatus { Pass, Warn, Fail }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum Direction { GreaterEq, LessEq, Equal }

impl BenchResult {
    pub fn evaluate(
        scenario: &str, metric: &str, measured: f64, threshold: f64,
        dir: Direction, n: u64, elapsed_ms: u64, warn_only: bool,
    ) -> Self {
        let pass = match dir {
            Direction::GreaterEq => measured >= threshold,
            Direction::LessEq    => measured <= threshold,
            Direction::Equal     => (measured - threshold).abs() < 1e-9,
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
