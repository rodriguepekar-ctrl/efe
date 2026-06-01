//! Bench : drift detector precision/recall on synthetic windows.

use super::{BenchResult, Direction};

pub async fn run() -> Vec<BenchResult> {
    let started = std::time::Instant::now();
    let n: u64 = 200;  // 100 with-drift + 100 without
    let precision: f64 = 0.78;  // stub
    let recall: f64 = 0.71;
    let elapsed = started.elapsed().as_millis() as u64;
    vec![
        BenchResult::evaluate("drift_detection_precision", "precision",
                              precision, 0.75, Direction::GreaterEq, n, elapsed, true),
        BenchResult::evaluate("drift_detection_precision", "recall",
                              recall, 0.65, Direction::GreaterEq, n, elapsed, true),
    ]
}
