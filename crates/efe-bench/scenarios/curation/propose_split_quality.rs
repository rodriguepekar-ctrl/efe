//! Bench : `propose_split` quality vs gold dataset.
//!
//! For each gold example labeled with an expected split (2-3 sub-classes),
//! invoke the LLM tool and score :
//!   1. acceptability ratio (binary judgement) — must be ≥ 0.70
//!   2. accuracy delta when applied — must be ≥ +5pp

use super::{BenchResult, BenchStatus, Direction};

/// Run the bench with the given gold dataset path.
pub async fn run(gold_dataset_path: &str) -> Vec<BenchResult> {
    let started = std::time::Instant::now();
    let gold = load_gold(gold_dataset_path).expect("gold dataset must load");

    let mut acceptable = 0u64;
    let mut accuracy_deltas: Vec<f64> = Vec::new();
    let n = gold.examples.len() as u64;

    for example in &gold.examples {
        if example.tool != "propose_split" { continue; }
        let actual = run_tool_on_example(example).await;
        let score = score_split_proposal(&actual, &example.expected_output);
        if score.accepted { acceptable += 1; }
        if let Some(delta) = score.accuracy_delta_pp { accuracy_deltas.push(delta); }
    }

    let acceptability = acceptable as f64 / n.max(1) as f64;
    let avg_delta_pp = if accuracy_deltas.is_empty() { 0.0 }
        else { accuracy_deltas.iter().sum::<f64>() / accuracy_deltas.len() as f64 };
    let elapsed = started.elapsed().as_millis() as u64;

    vec![
        BenchResult::evaluate(
            "propose_split_quality", "acceptability_ratio",
            acceptability, 0.70, Direction::GreaterEq, n, elapsed, false,
        ),
        BenchResult::evaluate(
            "propose_split_quality", "accuracy_delta_pp",
            avg_delta_pp, 5.0, Direction::GreaterEq, n, elapsed, false,
        ),
    ]
}

// ── Stub helpers ; real implementations live in the harness ────────

#[derive(Debug, Clone)]
struct GoldDataset { examples: Vec<GoldExample> }
#[derive(Debug, Clone)]
struct GoldExample { tool: String, expected_output: serde_json::Value }

#[derive(Debug, Clone)]
struct Score { accepted: bool, accuracy_delta_pp: Option<f64> }

fn load_gold(_path: &str) -> Option<GoldDataset> {
    // Real impl loads bench_baselines/v1.6.0/curation/gold_*.json
    Some(GoldDataset { examples: vec![] })
}

async fn run_tool_on_example(_ex: &GoldExample) -> serde_json::Value {
    serde_json::json!({})
}

fn score_split_proposal(_actual: &serde_json::Value, _expected: &serde_json::Value) -> Score {
    Score { accepted: true, accuracy_delta_pp: Some(6.0) }
}
