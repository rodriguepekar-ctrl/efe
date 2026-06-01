//! Gold-gate evaluation for prompt templates.
//!
//! A template `vN+1` may only be promoted from `traffic_split: 0.5`
//! (canary) to `1.0` (replacing `vN`) if eval against gold passes
//! all gates. Used by the deploy pipeline at template upgrade time.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalMetrics {
    pub template_name: String,
    pub template_version: u32,
    pub gold_dataset_id: String,
    pub n_samples: u32,
    pub accuracy: f32,
    pub diversity: f32,
    pub fail_rate: f32,
    pub cost_avg_cents: f32,
    pub latency_avg_ms: u32,
    pub computed_at: chrono::DateTime<chrono::Utc>,
}

/// Eval gates applied to a candidate template version.
#[derive(Debug, Clone)]
pub struct EvalGates {
    pub min_accuracy: f32,
    pub min_diversity: f32,
    pub max_fail_rate: f32,
    pub max_cost_cents: f32,
    pub max_latency_ms: u32,
}

impl Default for EvalGates {
    fn default() -> Self {
        Self {
            min_accuracy: 0.70,
            min_diversity: 0.60,
            max_fail_rate: 0.05,
            max_cost_cents: 20.0,
            max_latency_ms: 8000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PromotionDecision {
    pub can_promote: bool,
    pub reasons: Vec<String>,
    pub delta_pp: f32,
}

/// Decide whether candidate can replace control. Promotion threshold
/// `delta_pp` is the minimum required accuracy improvement of candidate
/// over control (e.g. +2.0 pp) — below threshold = no promote even if
/// candidate is "better".
pub fn evaluate_promotion(
    control: &EvalMetrics, candidate: &EvalMetrics,
    gates: &EvalGates, delta_pp_threshold: f32,
) -> PromotionDecision {
    let mut reasons = Vec::new();
    let delta_pp = (candidate.accuracy - control.accuracy) * 100.0;

    // Hard gates : candidate must satisfy each absolute floor
    if candidate.accuracy < gates.min_accuracy {
        reasons.push(format!("accuracy {:.4} < min {:.4}", candidate.accuracy, gates.min_accuracy));
    }
    if candidate.diversity < gates.min_diversity {
        reasons.push(format!("diversity {:.4} < min {:.4}", candidate.diversity, gates.min_diversity));
    }
    if candidate.fail_rate > gates.max_fail_rate {
        reasons.push(format!("fail_rate {:.4} > max {:.4}", candidate.fail_rate, gates.max_fail_rate));
    }
    if candidate.cost_avg_cents > gates.max_cost_cents {
        reasons.push(format!("cost {:.2} cents > max {:.2}", candidate.cost_avg_cents, gates.max_cost_cents));
    }
    if candidate.latency_avg_ms > gates.max_latency_ms {
        reasons.push(format!("latency {} ms > max {} ms", candidate.latency_avg_ms, gates.max_latency_ms));
    }

    // Comparative : candidate must beat control by `delta_pp_threshold`
    if delta_pp < delta_pp_threshold {
        reasons.push(format!(
            "delta_pp {:.2} < threshold {:.2}", delta_pp, delta_pp_threshold,
        ));
    }
    // Diversity must not regress > 5%
    if candidate.diversity < control.diversity * 0.95 {
        reasons.push(format!(
            "diversity regression : {:.4} < 95% of control {:.4}",
            candidate.diversity, control.diversity,
        ));
    }
    // Cost must not increase > 20%
    if candidate.cost_avg_cents > control.cost_avg_cents * 1.20 {
        reasons.push(format!(
            "cost regression : {:.2} > 120% of control {:.2}",
            candidate.cost_avg_cents, control.cost_avg_cents,
        ));
    }
    // Fail-rate must not increase
    if candidate.fail_rate > control.fail_rate {
        reasons.push(format!(
            "fail_rate regression : {:.4} > control {:.4}",
            candidate.fail_rate, control.fail_rate,
        ));
    }

    PromotionDecision { can_promote: reasons.is_empty(), reasons, delta_pp }
}

/// Aggregate eval metrics across runs (e.g. for a 7-day observation window).
pub fn aggregate_metrics(runs: &[EvalMetrics]) -> Option<EvalMetrics> {
    if runs.is_empty() { return None; }
    let n = runs.len() as f32;
    let total_samples: u32 = runs.iter().map(|r| r.n_samples).sum();
    Some(EvalMetrics {
        template_name: runs[0].template_name.clone(),
        template_version: runs[0].template_version,
        gold_dataset_id: runs[0].gold_dataset_id.clone(),
        n_samples: total_samples,
        accuracy: runs.iter().map(|r| r.accuracy).sum::<f32>() / n,
        diversity: runs.iter().map(|r| r.diversity).sum::<f32>() / n,
        fail_rate: runs.iter().map(|r| r.fail_rate).sum::<f32>() / n,
        cost_avg_cents: runs.iter().map(|r| r.cost_avg_cents).sum::<f32>() / n,
        latency_avg_ms: (runs.iter().map(|r| r.latency_avg_ms as u64).sum::<u64>() / n as u64) as u32,
        computed_at: chrono::Utc::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(acc: f32, div: f32, fail: f32, cost: f32, lat: u32) -> EvalMetrics {
        EvalMetrics {
            template_name: "test".into(), template_version: 1,
            gold_dataset_id: "gold".into(), n_samples: 100,
            accuracy: acc, diversity: div, fail_rate: fail,
            cost_avg_cents: cost, latency_avg_ms: lat,
            computed_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn promote_happy_path() {
        let ctrl = metrics(0.80, 0.70, 0.01, 4.0, 1000);
        let cand = metrics(0.85, 0.71, 0.005, 4.5, 1100);
        let dec = evaluate_promotion(&ctrl, &cand, &EvalGates::default(), 2.0);
        assert!(dec.can_promote, "expected promote, got {:?}", dec.reasons);
        assert!((dec.delta_pp - 5.0).abs() < 0.01);
    }

    #[test]
    fn block_when_delta_too_small() {
        let ctrl = metrics(0.80, 0.70, 0.01, 4.0, 1000);
        let cand = metrics(0.81, 0.70, 0.01, 4.0, 1000);  // delta = 1pp < 2pp
        let dec = evaluate_promotion(&ctrl, &cand, &EvalGates::default(), 2.0);
        assert!(!dec.can_promote);
    }

    #[test]
    fn block_on_diversity_regression() {
        let ctrl = metrics(0.80, 0.80, 0.01, 4.0, 1000);
        let cand = metrics(0.85, 0.60, 0.01, 4.0, 1000);  // diversity dropped
        let dec = evaluate_promotion(&ctrl, &cand, &EvalGates::default(), 2.0);
        assert!(!dec.can_promote);
    }

    #[test]
    fn block_on_cost_regression() {
        let ctrl = metrics(0.80, 0.70, 0.01, 4.0, 1000);
        let cand = metrics(0.85, 0.71, 0.01, 5.0, 1000);  // 25% more expensive
        let dec = evaluate_promotion(&ctrl, &cand, &EvalGates::default(), 2.0);
        assert!(!dec.can_promote);
    }

    #[test]
    fn block_on_hard_floor() {
        let ctrl = metrics(0.80, 0.70, 0.01, 4.0, 1000);
        let cand = metrics(0.65, 0.70, 0.01, 4.0, 1000);  // accuracy below floor
        let dec = evaluate_promotion(&ctrl, &cand, &EvalGates::default(), 2.0);
        assert!(!dec.can_promote);
    }

    #[test]
    fn aggregate_averages() {
        let runs = vec![
            metrics(0.80, 0.70, 0.01, 4.0, 1000),
            metrics(0.82, 0.72, 0.01, 4.0, 1100),
        ];
        let agg = aggregate_metrics(&runs).unwrap();
        assert!((agg.accuracy - 0.81).abs() < 0.001);
        assert_eq!(agg.n_samples, 200);
    }
}
