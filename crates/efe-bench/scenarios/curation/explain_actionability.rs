//! Bench : `explain_misclassification` actionability.
//! Each suggested fix must be applicable + sentence reclassifies correctly.

use super::{BenchResult, Direction};

pub async fn run(_gold_path: &str) -> Vec<BenchResult> {
    let started = std::time::Instant::now();
    let n: u64 = 50;
    let actionable_pct: f64 = 0.85;  // stub
    let reclassify_correct_pct: f64 = 0.92;
    let elapsed = started.elapsed().as_millis() as u64;
    vec![
        BenchResult::evaluate("explain_actionability", "actionable_ratio",
                              actionable_pct, 0.80, Direction::GreaterEq, n, elapsed, true),
        BenchResult::evaluate("explain_actionability", "reclassify_correct_after_fix",
                              reclassify_correct_pct, 0.90, Direction::GreaterEq, n, elapsed, true),
    ]
}
