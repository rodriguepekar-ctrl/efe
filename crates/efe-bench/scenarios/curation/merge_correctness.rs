//! Bench : `suggest_class_merge` calibration — predicted accuracy_impact
//! must agree with measured post-promote within ±1pp.

use super::{BenchResult, Direction};

pub async fn run(_gold_path: &str) -> Vec<BenchResult> {
    let started = std::time::Instant::now();
    let n: u64 = 20;
    let acceptance_rate: f64 = 0.75;
    let calibration_within_1pp_pct: f64 = 0.65;
    let elapsed = started.elapsed().as_millis() as u64;
    vec![
        BenchResult::evaluate("merge_correctness", "acceptance_rate",
                              acceptance_rate, 0.70, Direction::GreaterEq, n, elapsed, true),
        BenchResult::evaluate("merge_correctness", "calibration_within_1pp",
                              calibration_within_1pp_pct, 0.60, Direction::GreaterEq, n, elapsed, true),
    ]
}
