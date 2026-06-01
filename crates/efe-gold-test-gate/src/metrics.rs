// SPDX-License-Identifier: UNLICENSED
//! **Sprint 11 — N19** : Prometheus surface — § 7.8.
//!
//! 12 surfaces in the `n19_*` namespace.  Cardinality is bounded
//! per I20 (≤ ~5 000 lines across the cluster).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;

/// **Sprint 11 — N19** : the 12 Prometheus surfaces.
pub struct N19Metrics {
    /// (1) `n19_gold_runs_total{outcome, agent_tier}`
    gold_runs: DashMap<(String, String), AtomicU64>,
    /// (2) `n19_gold_cases_executed_total{strategy, outcome}`
    gold_cases: DashMap<(String, String), AtomicU64>,
    /// (3) `n19_gold_run_cost_cents_total{agent_tier}` — fractional
    /// cents stored as microcents to avoid f64 atomics.
    cost_microcents: DashMap<String, AtomicU64>,
    /// (4) `n19_gold_run_duration_seconds{agent_tier}` — summary.
    run_duration_sum_us: DashMap<String, AtomicU64>,
    run_duration_count: DashMap<String, AtomicU64>,
    /// (5) `n19_gold_case_duration_seconds{strategy}` — summary.
    case_duration_sum_us: DashMap<String, AtomicU64>,
    case_duration_count: DashMap<String, AtomicU64>,
    /// (6) `n19_gold_set_approvals_total{outcome}`
    approvals: DashMap<String, AtomicU64>,
    /// (7) `n19_bypass_usage_total{tier, ticket_class}`
    bypass_usage: DashMap<(String, String), AtomicU64>,
    /// (8) `n19_drift_sweep_demotions_total{reason}`
    drift_demotions: DashMap<String, AtomicU64>,
    /// (9) `n19_hermeticity_violations_total{kind}`
    hermeticity: DashMap<String, AtomicU64>,
    /// (10) `n19_replay_anchor_lookups_total{outcome}`
    replay_lookups: DashMap<String, AtomicU64>,
    /// (11) `n19_assertion_engine_evaluations_total{strategy, verdict}`
    assertions: DashMap<(String, String), AtomicU64>,
    /// (12) `n19_gold_run_age_days{agent_id, tier}` — gauge.
    age_days: DashMap<(String, String), AtomicU64>,
}

impl std::fmt::Debug for N19Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("N19Metrics")
            .field("runs_cardinality", &self.gold_runs.len())
            .field("cases_cardinality", &self.gold_cases.len())
            .field("approvals_cardinality", &self.approvals.len())
            .finish()
    }
}

impl Default for N19Metrics {
    fn default() -> Self {
        Self::zeroed()
    }
}

impl N19Metrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::zeroed())
    }

    fn zeroed() -> Self {
        Self {
            gold_runs: DashMap::new(),
            gold_cases: DashMap::new(),
            cost_microcents: DashMap::new(),
            run_duration_sum_us: DashMap::new(),
            run_duration_count: DashMap::new(),
            case_duration_sum_us: DashMap::new(),
            case_duration_count: DashMap::new(),
            approvals: DashMap::new(),
            bypass_usage: DashMap::new(),
            drift_demotions: DashMap::new(),
            hermeticity: DashMap::new(),
            replay_lookups: DashMap::new(),
            assertions: DashMap::new(),
            age_days: DashMap::new(),
        }
    }

    fn bump_dash(map: &DashMap<String, AtomicU64>, key: &str, delta: u64) {
        map.entry(key.to_string())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(delta, Ordering::Relaxed);
    }

    fn bump_pair(
        map: &DashMap<(String, String), AtomicU64>,
        k1: &str,
        k2: &str,
        delta: u64,
    ) {
        let key = (k1.to_string(), k2.to_string());
        map.entry(key)
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(delta, Ordering::Relaxed);
    }

    fn load_dash(map: &DashMap<String, AtomicU64>, key: &str) -> u64 {
        map.get(key).map_or(0, |r| r.value().load(Ordering::Relaxed))
    }

    fn load_pair(
        map: &DashMap<(String, String), AtomicU64>,
        k1: &str,
        k2: &str,
    ) -> u64 {
        let key = (k1.to_string(), k2.to_string());
        map.get(&key).map_or(0, |r| r.value().load(Ordering::Relaxed))
    }

    // ─── (1) gold runs ────────────────────────────────────────────

    pub fn tick_gold_run(&self, outcome: &str, agent_tier: &str) {
        Self::bump_pair(&self.gold_runs, outcome, agent_tier, 1);
    }

    pub fn gold_runs_total(&self, outcome: &str, agent_tier: &str) -> u64 {
        Self::load_pair(&self.gold_runs, outcome, agent_tier)
    }

    // ─── (2) gold cases ───────────────────────────────────────────

    pub fn tick_gold_case(&self, strategy: &str, outcome: &str) {
        Self::bump_pair(&self.gold_cases, strategy, outcome, 1);
    }

    pub fn gold_cases_total(&self, strategy: &str, outcome: &str) -> u64 {
        Self::load_pair(&self.gold_cases, strategy, outcome)
    }

    // ─── (3) cost ─────────────────────────────────────────────────

    pub fn add_cost(&self, agent_tier: &str, cents: f64) {
        if cents < 0.0 || !cents.is_finite() {
            return;
        }
        let micro = (cents * 1_000_000.0).round() as u64;
        Self::bump_dash(&self.cost_microcents, agent_tier, micro);
    }

    pub fn cost_cents_total(&self, agent_tier: &str) -> f64 {
        Self::load_dash(&self.cost_microcents, agent_tier) as f64 / 1_000_000.0
    }

    // ─── (4) run duration ────────────────────────────────────────

    pub fn observe_run_duration(&self, agent_tier: &str, seconds: f64) {
        if seconds < 0.0 || !seconds.is_finite() {
            return;
        }
        let us = (seconds * 1_000_000.0).round() as u64;
        Self::bump_dash(&self.run_duration_sum_us, agent_tier, us);
        Self::bump_dash(&self.run_duration_count, agent_tier, 1);
    }

    pub fn run_duration_count(&self, agent_tier: &str) -> u64 {
        Self::load_dash(&self.run_duration_count, agent_tier)
    }

    // ─── (5) case duration ───────────────────────────────────────

    pub fn observe_case_duration(&self, strategy: &str, seconds: f64) {
        if seconds < 0.0 || !seconds.is_finite() {
            return;
        }
        let us = (seconds * 1_000_000.0).round() as u64;
        Self::bump_dash(&self.case_duration_sum_us, strategy, us);
        Self::bump_dash(&self.case_duration_count, strategy, 1);
    }

    pub fn case_duration_count(&self, strategy: &str) -> u64 {
        Self::load_dash(&self.case_duration_count, strategy)
    }

    // ─── (6) approvals ───────────────────────────────────────────

    pub fn tick_approval(&self, outcome: &str) {
        Self::bump_dash(&self.approvals, outcome, 1);
    }

    pub fn approvals_total(&self, outcome: &str) -> u64 {
        Self::load_dash(&self.approvals, outcome)
    }

    // ─── (7) bypass usage ────────────────────────────────────────

    pub fn tick_bypass(&self, tier: &str, ticket_class: &str) {
        Self::bump_pair(&self.bypass_usage, tier, ticket_class, 1);
    }

    pub fn bypass_total(&self, tier: &str, ticket_class: &str) -> u64 {
        Self::load_pair(&self.bypass_usage, tier, ticket_class)
    }

    // ─── (8) drift demotions ─────────────────────────────────────

    pub fn tick_drift_demotion(&self, reason: &str) {
        Self::bump_dash(&self.drift_demotions, reason, 1);
    }

    pub fn drift_demotions_total(&self, reason: &str) -> u64 {
        Self::load_dash(&self.drift_demotions, reason)
    }

    // ─── (9) hermeticity violations ──────────────────────────────

    pub fn tick_hermeticity(&self, kind: &str) {
        Self::bump_dash(&self.hermeticity, kind, 1);
    }

    pub fn hermeticity_total(&self, kind: &str) -> u64 {
        Self::load_dash(&self.hermeticity, kind)
    }

    // ─── (10) replay anchor lookups ──────────────────────────────

    pub fn tick_replay_lookup(&self, outcome: &str) {
        Self::bump_dash(&self.replay_lookups, outcome, 1);
    }

    pub fn replay_lookups_total(&self, outcome: &str) -> u64 {
        Self::load_dash(&self.replay_lookups, outcome)
    }

    // ─── (11) assertion evaluations ──────────────────────────────

    pub fn tick_assertion(&self, strategy: &str, verdict: &str) {
        Self::bump_pair(&self.assertions, strategy, verdict, 1);
    }

    pub fn assertions_total(&self, strategy: &str, verdict: &str) -> u64 {
        Self::load_pair(&self.assertions, strategy, verdict)
    }

    // ─── (12) age days ───────────────────────────────────────────

    pub fn set_age_days(&self, agent_id: &str, tier: &str, days: u64) {
        let key = (agent_id.to_string(), tier.to_string());
        self.age_days
            .entry(key)
            .or_insert_with(|| AtomicU64::new(0))
            .store(days, Ordering::Relaxed);
    }

    pub fn age_days(&self, agent_id: &str, tier: &str) -> u64 {
        Self::load_pair(&self.age_days, agent_id, tier)
    }

    // ─── Render ──────────────────────────────────────────────────

    /// **N19** : Render in OpenMetrics 1.0.0 text format.
    pub fn render_prometheus(&self) -> String {
        let mut out = String::with_capacity(4_096);

        // 1
        out.push_str("# HELP n19_gold_runs_total Gold runs by outcome + agent tier.\n");
        out.push_str("# TYPE n19_gold_runs_total counter\n");
        for r in self.gold_runs.iter() {
            let (o, t) = r.key();
            out.push_str(&format!(
                "n19_gold_runs_total{{outcome=\"{o}\",agent_tier=\"{t}\"}} {}\n",
                r.value().load(Ordering::Relaxed),
            ));
        }
        // 2
        out.push_str("# HELP n19_gold_cases_executed_total Gold cases by strategy + outcome.\n");
        out.push_str("# TYPE n19_gold_cases_executed_total counter\n");
        for r in self.gold_cases.iter() {
            let (s, o) = r.key();
            out.push_str(&format!(
                "n19_gold_cases_executed_total{{strategy=\"{s}\",outcome=\"{o}\"}} {}\n",
                r.value().load(Ordering::Relaxed),
            ));
        }
        // 3
        out.push_str("# HELP n19_gold_run_cost_cents_total Gold-run cost in cents.\n");
        out.push_str("# TYPE n19_gold_run_cost_cents_total counter\n");
        for r in self.cost_microcents.iter() {
            out.push_str(&format!(
                "n19_gold_run_cost_cents_total{{agent_tier=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed) as f64 / 1_000_000.0,
            ));
        }
        // 4
        out.push_str("# HELP n19_gold_run_duration_seconds Gold-run wall duration.\n");
        out.push_str("# TYPE n19_gold_run_duration_seconds summary\n");
        for r in self.run_duration_sum_us.iter() {
            let t = r.key();
            let sum_us = r.value().load(Ordering::Relaxed);
            let count = self
                .run_duration_count
                .get(t)
                .map_or(0, |c| c.value().load(Ordering::Relaxed));
            out.push_str(&format!(
                "n19_gold_run_duration_seconds_sum{{agent_tier=\"{t}\"}} {}\n",
                sum_us as f64 / 1_000_000.0,
            ));
            out.push_str(&format!(
                "n19_gold_run_duration_seconds_count{{agent_tier=\"{t}\"}} {count}\n",
            ));
        }
        // 5
        out.push_str("# HELP n19_gold_case_duration_seconds Per-case wall duration.\n");
        out.push_str("# TYPE n19_gold_case_duration_seconds summary\n");
        for r in self.case_duration_sum_us.iter() {
            let s = r.key();
            let sum_us = r.value().load(Ordering::Relaxed);
            let count = self
                .case_duration_count
                .get(s)
                .map_or(0, |c| c.value().load(Ordering::Relaxed));
            out.push_str(&format!(
                "n19_gold_case_duration_seconds_sum{{strategy=\"{s}\"}} {}\n",
                sum_us as f64 / 1_000_000.0,
            ));
            out.push_str(&format!(
                "n19_gold_case_duration_seconds_count{{strategy=\"{s}\"}} {count}\n",
            ));
        }
        // 6
        out.push_str("# HELP n19_gold_set_approvals_total Approval ceremonies by outcome.\n");
        out.push_str("# TYPE n19_gold_set_approvals_total counter\n");
        for r in self.approvals.iter() {
            out.push_str(&format!(
                "n19_gold_set_approvals_total{{outcome=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }
        // 7
        out.push_str("# HELP n19_bypass_usage_total Emergency-bypass invocations.\n");
        out.push_str("# TYPE n19_bypass_usage_total counter\n");
        for r in self.bypass_usage.iter() {
            let (t, c) = r.key();
            out.push_str(&format!(
                "n19_bypass_usage_total{{tier=\"{t}\",ticket_class=\"{c}\"}} {}\n",
                r.value().load(Ordering::Relaxed),
            ));
        }
        // 8
        out.push_str("# HELP n19_drift_sweep_demotions_total Drift-sweep demotions.\n");
        out.push_str("# TYPE n19_drift_sweep_demotions_total counter\n");
        for r in self.drift_demotions.iter() {
            out.push_str(&format!(
                "n19_drift_sweep_demotions_total{{reason=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }
        // 9
        out.push_str("# HELP n19_hermeticity_violations_total Hermeticity violations.\n");
        out.push_str("# TYPE n19_hermeticity_violations_total counter\n");
        for r in self.hermeticity.iter() {
            out.push_str(&format!(
                "n19_hermeticity_violations_total{{kind=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }
        // 10
        out.push_str("# HELP n19_replay_anchor_lookups_total Replay-anchor lookups.\n");
        out.push_str("# TYPE n19_replay_anchor_lookups_total counter\n");
        for r in self.replay_lookups.iter() {
            out.push_str(&format!(
                "n19_replay_anchor_lookups_total{{outcome=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }
        // 11
        out.push_str("# HELP n19_assertion_engine_evaluations_total Assertion evaluations.\n");
        out.push_str("# TYPE n19_assertion_engine_evaluations_total counter\n");
        for r in self.assertions.iter() {
            let (s, v) = r.key();
            out.push_str(&format!(
                "n19_assertion_engine_evaluations_total{{strategy=\"{s}\",verdict=\"{v}\"}} {}\n",
                r.value().load(Ordering::Relaxed),
            ));
        }
        // 12
        out.push_str("# HELP n19_gold_run_age_days Age of the last passing gold run in days.\n");
        out.push_str("# TYPE n19_gold_run_age_days gauge\n");
        for r in self.age_days.iter() {
            let (a, t) = r.key();
            out.push_str(&format!(
                "n19_gold_run_age_days{{agent_id=\"{a}\",tier=\"{t}\"}} {}\n",
                r.value().load(Ordering::Relaxed),
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_metrics_all_zero() {
        let m = N19Metrics::new();
        assert_eq!(m.gold_runs_total("passed", "tier_1"), 0);
        assert_eq!(m.cost_cents_total("tier_1"), 0.0);
    }

    #[test]
    fn tick_gold_run_per_outcome_and_tier() {
        let m = N19Metrics::new();
        m.tick_gold_run("passed", "tier_1");
        m.tick_gold_run("passed", "tier_1");
        m.tick_gold_run("failed", "tier_1");
        assert_eq!(m.gold_runs_total("passed", "tier_1"), 2);
        assert_eq!(m.gold_runs_total("failed", "tier_1"), 1);
    }

    #[test]
    fn tick_gold_case_per_strategy_outcome() {
        let m = N19Metrics::new();
        m.tick_gold_case("strict_equality", "passed");
        m.tick_gold_case("semantic_similarity", "failed");
        assert_eq!(m.gold_cases_total("strict_equality", "passed"), 1);
        assert_eq!(m.gold_cases_total("semantic_similarity", "failed"), 1);
    }

    #[test]
    fn add_cost_round_trips_cents() {
        let m = N19Metrics::new();
        m.add_cost("tier_1", 1.5);
        m.add_cost("tier_1", 0.75);
        assert!((m.cost_cents_total("tier_1") - 2.25).abs() < 1e-6);
    }

    #[test]
    fn add_cost_ignores_nan_or_negative() {
        let m = N19Metrics::new();
        m.add_cost("tier_1", -1.0);
        m.add_cost("tier_1", f64::NAN);
        assert_eq!(m.cost_cents_total("tier_1"), 0.0);
    }

    #[test]
    fn observe_durations_accumulate() {
        let m = N19Metrics::new();
        m.observe_run_duration("tier_1", 0.5);
        m.observe_run_duration("tier_1", 0.7);
        assert_eq!(m.run_duration_count("tier_1"), 2);
        m.observe_case_duration("strict_equality", 0.01);
        m.observe_case_duration("strict_equality", 0.02);
        assert_eq!(m.case_duration_count("strict_equality"), 2);
    }

    #[test]
    fn observe_durations_ignore_invalid() {
        let m = N19Metrics::new();
        m.observe_run_duration("tier_1", -1.0);
        m.observe_run_duration("tier_1", f64::NAN);
        assert_eq!(m.run_duration_count("tier_1"), 0);
    }

    #[test]
    fn approvals_per_outcome() {
        let m = N19Metrics::new();
        m.tick_approval("approved");
        m.tick_approval("rejected");
        m.tick_approval("approved");
        assert_eq!(m.approvals_total("approved"), 2);
        assert_eq!(m.approvals_total("rejected"), 1);
    }

    #[test]
    fn bypass_per_tier_class() {
        let m = N19Metrics::new();
        m.tick_bypass("tier_2", "INC");
        m.tick_bypass("tier_3", "SEC");
        assert_eq!(m.bypass_total("tier_2", "INC"), 1);
        assert_eq!(m.bypass_total("tier_3", "SEC"), 1);
    }

    #[test]
    fn drift_demotions_per_reason() {
        let m = N19Metrics::new();
        m.tick_drift_demotion("stale_gold_set");
        assert_eq!(m.drift_demotions_total("stale_gold_set"), 1);
    }

    #[test]
    fn hermeticity_per_kind() {
        let m = N19Metrics::new();
        m.tick_hermeticity("network_leak");
        assert_eq!(m.hermeticity_total("network_leak"), 1);
    }

    #[test]
    fn replay_lookups_per_outcome() {
        let m = N19Metrics::new();
        m.tick_replay_lookup("hit");
        m.tick_replay_lookup("miss");
        assert_eq!(m.replay_lookups_total("hit"), 1);
        assert_eq!(m.replay_lookups_total("miss"), 1);
    }

    #[test]
    fn assertions_per_strategy_verdict() {
        let m = N19Metrics::new();
        m.tick_assertion("strict_equality", "passed");
        m.tick_assertion("numeric_tolerance", "failed");
        assert_eq!(m.assertions_total("strict_equality", "passed"), 1);
        assert_eq!(m.assertions_total("numeric_tolerance", "failed"), 1);
    }

    #[test]
    fn age_days_gauge_overrides() {
        let m = N19Metrics::new();
        m.set_age_days("agent_1", "tier_1", 5);
        m.set_age_days("agent_1", "tier_1", 12);
        assert_eq!(m.age_days("agent_1", "tier_1"), 12);
    }

    #[test]
    fn render_contains_all_12_metric_names() {
        let m = N19Metrics::new();
        m.tick_gold_run("passed", "tier_1");
        m.tick_gold_case("strict_equality", "passed");
        m.add_cost("tier_1", 1.0);
        m.observe_run_duration("tier_1", 0.5);
        m.observe_case_duration("strict_equality", 0.01);
        m.tick_approval("approved");
        m.tick_bypass("tier_2", "INC");
        m.tick_drift_demotion("stale");
        m.tick_hermeticity("net");
        m.tick_replay_lookup("hit");
        m.tick_assertion("strict_equality", "passed");
        m.set_age_days("a", "tier_1", 3);

        let out = m.render_prometheus();
        for name in [
            "n19_gold_runs_total",
            "n19_gold_cases_executed_total",
            "n19_gold_run_cost_cents_total",
            "n19_gold_run_duration_seconds",
            "n19_gold_case_duration_seconds",
            "n19_gold_set_approvals_total",
            "n19_bypass_usage_total",
            "n19_drift_sweep_demotions_total",
            "n19_hermeticity_violations_total",
            "n19_replay_anchor_lookups_total",
            "n19_assertion_engine_evaluations_total",
            "n19_gold_run_age_days",
        ] {
            assert!(out.contains(name), "missing : {name}");
        }
        assert_eq!(out.matches("# HELP").count(), 12);
        assert_eq!(out.matches("# TYPE").count(), 12);
    }

    #[test]
    fn debug_format_no_panic() {
        let m = N19Metrics::new();
        let _ = format!("{m:?}");
    }

    #[test]
    fn arc_sharing_allows_concurrent_writers() {
        let m = N19Metrics::new();
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let m = Arc::clone(&m);
                std::thread::spawn(move || {
                    for _ in 0..100 {
                        m.tick_gold_run("passed", "tier_1");
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(m.gold_runs_total("passed", "tier_1"), 800);
    }
}
