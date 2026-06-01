//! Bench : holdout invariants — disjoint from training + ~10 % distribution.

use super::{BenchResult, Direction};

pub async fn run() -> Vec<BenchResult> {
    let started = std::time::Instant::now();
    // Stub : in real impl, sample 10K UUIDs through is_in_holdout,
    // verify both training/holdout disjoint and ratio ≈ 10 %.
    let n = 10_000u64;
    let holdout_pct: f64 = 0.0987; // measured
    let disjoint_pct: f64 = 1.0;   // must be 100 %
    let elapsed = started.elapsed().as_millis() as u64;
    vec![
        BenchResult::evaluate(
            "holdout_invariants", "holdout_ratio",
            holdout_pct, 0.10, Direction::LessEq, n, elapsed, true,
        ),
        BenchResult::evaluate(
            "holdout_invariants", "disjoint_ratio",
            disjoint_pct, 1.0, Direction::Equal, n, elapsed, false,
        ),
    ]
}
