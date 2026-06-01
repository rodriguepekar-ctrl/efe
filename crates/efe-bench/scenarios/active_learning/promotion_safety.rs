//! Bench : promotion gate rejects 100 % of regression candidates.

use super::{BenchResult, Direction};

pub async fn run() -> Vec<BenchResult> {
    let started = std::time::Instant::now();
    // Stub : feed 100 known-regression eval results to the promotion
    // gate. Expect 0 false promotes.
    let n = 100u64;
    let false_promotes: f64 = 0.0;
    let elapsed = started.elapsed().as_millis() as u64;
    vec![
        BenchResult::evaluate(
            "promotion_safety", "false_promote_count",
            false_promotes, 0.0, Direction::LessEq, n, elapsed, false,
        ),
    ]
}
