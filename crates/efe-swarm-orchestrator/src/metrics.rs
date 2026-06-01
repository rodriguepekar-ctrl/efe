// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21** : Prometheus surface — § 7.5.
//!
//! 12 surfaces in the `n21_*` namespace.  Cardinality bounded per
//! I18 (≤ 200 distinct agent_id labels per region).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;

/// **Sprint 13 — N21** : the 12 Prometheus surfaces.
pub struct N21Metrics {
    // (1) n21_swarm_runs_total{outcome}
    swarm_runs: DashMap<String, AtomicU64>,
    // (2) n21_swarm_run_duration_ms{dag_layer_count} — summary
    swarm_dur_sum: DashMap<String, AtomicU64>,
    swarm_dur_count: DashMap<String, AtomicU64>,
    // (3) n21_swarm_run_cost_microcents_total{tenant_tier}
    swarm_cost: DashMap<String, AtomicU64>,
    // (4) n21_agent_invocations_total{agent_id, outcome}
    agent_invocations: DashMap<(String, String), AtomicU64>,
    // (5) n21_agent_duration_ms{agent_id} — summary
    agent_dur_sum: DashMap<String, AtomicU64>,
    agent_dur_count: DashMap<String, AtomicU64>,
    // (6) n21_agent_timeouts_total{agent_id}
    agent_timeouts: DashMap<String, AtomicU64>,
    // (7) n21_blackboard_writes_total{agent_id}
    blackboard_writes: DashMap<String, AtomicU64>,
    // (8) n21_blackboard_conflicts_total{pointer_class}
    blackboard_conflicts: DashMap<String, AtomicU64>,
    // (9) n21_eventbus_published_total{topic}
    eventbus_published: DashMap<String, AtomicU64>,
    // (10) n21_eventbus_dropped_total{reason}
    eventbus_dropped: DashMap<String, AtomicU64>,
    // (11) n21_claude_calls_total{outcome}
    claude_calls: DashMap<String, AtomicU64>,
    // (12) n21_context_bundle_size_bytes{dag_id} — summary
    bundle_size_sum: DashMap<String, AtomicU64>,
    bundle_size_count: DashMap<String, AtomicU64>,
}

impl std::fmt::Debug for N21Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("N21Metrics")
            .field("swarm_runs_card", &self.swarm_runs.len())
            .field("agent_invocations_card", &self.agent_invocations.len())
            .finish()
    }
}

impl Default for N21Metrics {
    fn default() -> Self {
        Self::zeroed()
    }
}

impl N21Metrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::zeroed())
    }

    fn zeroed() -> Self {
        Self {
            swarm_runs: DashMap::new(),
            swarm_dur_sum: DashMap::new(),
            swarm_dur_count: DashMap::new(),
            swarm_cost: DashMap::new(),
            agent_invocations: DashMap::new(),
            agent_dur_sum: DashMap::new(),
            agent_dur_count: DashMap::new(),
            agent_timeouts: DashMap::new(),
            blackboard_writes: DashMap::new(),
            blackboard_conflicts: DashMap::new(),
            eventbus_published: DashMap::new(),
            eventbus_dropped: DashMap::new(),
            claude_calls: DashMap::new(),
            bundle_size_sum: DashMap::new(),
            bundle_size_count: DashMap::new(),
        }
    }

    fn bump_str(map: &DashMap<String, AtomicU64>, key: &str, delta: u64) {
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
        map.entry((k1.to_string(), k2.to_string()))
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(delta, Ordering::Relaxed);
    }

    fn load_str(map: &DashMap<String, AtomicU64>, key: &str) -> u64 {
        map.get(key).map_or(0, |r| r.value().load(Ordering::Relaxed))
    }

    fn load_pair(
        map: &DashMap<(String, String), AtomicU64>,
        k1: &str,
        k2: &str,
    ) -> u64 {
        map.get(&(k1.to_string(), k2.to_string()))
            .map_or(0, |r| r.value().load(Ordering::Relaxed))
    }

    // ─── 1 swarm runs ──────────────────────────────────────────────

    pub fn tick_swarm_run(&self, outcome: &str) {
        Self::bump_str(&self.swarm_runs, outcome, 1);
    }

    pub fn swarm_runs_total(&self, outcome: &str) -> u64 {
        Self::load_str(&self.swarm_runs, outcome)
    }

    // ─── 2 swarm duration ──────────────────────────────────────────

    pub fn observe_swarm_duration_ms(&self, dag_layer_count: usize, ms: u64) {
        let key = dag_layer_count.to_string();
        Self::bump_str(&self.swarm_dur_sum, &key, ms);
        Self::bump_str(&self.swarm_dur_count, &key, 1);
    }

    pub fn swarm_duration_count(&self, dag_layer_count: usize) -> u64 {
        Self::load_str(&self.swarm_dur_count, &dag_layer_count.to_string())
    }

    // ─── 3 swarm cost ──────────────────────────────────────────────

    pub fn add_swarm_cost(&self, tenant_tier: &str, microcents: i64) {
        if microcents > 0 {
            Self::bump_str(&self.swarm_cost, tenant_tier, microcents as u64);
        }
    }

    pub fn swarm_cost_total(&self, tenant_tier: &str) -> u64 {
        Self::load_str(&self.swarm_cost, tenant_tier)
    }

    // ─── 4 agent invocations ────────────────────────────────────

    pub fn tick_agent_invocation(&self, agent_id: &str, outcome: &str) {
        Self::bump_pair(&self.agent_invocations, agent_id, outcome, 1);
    }

    pub fn agent_invocations_total(&self, agent_id: &str, outcome: &str) -> u64 {
        Self::load_pair(&self.agent_invocations, agent_id, outcome)
    }

    // ─── 5 agent duration ──────────────────────────────────────

    pub fn observe_agent_duration_ms(&self, agent_id: &str, ms: u64) {
        Self::bump_str(&self.agent_dur_sum, agent_id, ms);
        Self::bump_str(&self.agent_dur_count, agent_id, 1);
    }

    pub fn agent_duration_count(&self, agent_id: &str) -> u64 {
        Self::load_str(&self.agent_dur_count, agent_id)
    }

    // ─── 6 agent timeouts ──────────────────────────────────────

    pub fn tick_agent_timeout(&self, agent_id: &str) {
        Self::bump_str(&self.agent_timeouts, agent_id, 1);
    }

    pub fn agent_timeouts_total(&self, agent_id: &str) -> u64 {
        Self::load_str(&self.agent_timeouts, agent_id)
    }

    // ─── 7 blackboard writes ───────────────────────────────────

    pub fn tick_blackboard_write(&self, agent_id: &str) {
        Self::bump_str(&self.blackboard_writes, agent_id, 1);
    }

    pub fn blackboard_writes_total(&self, agent_id: &str) -> u64 {
        Self::load_str(&self.blackboard_writes, agent_id)
    }

    // ─── 8 blackboard conflicts ────────────────────────────────

    pub fn tick_blackboard_conflict(&self, pointer_class: &str) {
        Self::bump_str(&self.blackboard_conflicts, pointer_class, 1);
    }

    pub fn blackboard_conflicts_total(&self, pointer_class: &str) -> u64 {
        Self::load_str(&self.blackboard_conflicts, pointer_class)
    }

    // ─── 9 eventbus published ──────────────────────────────────

    pub fn tick_eventbus_published(&self, topic: &str) {
        Self::bump_str(&self.eventbus_published, topic, 1);
    }

    pub fn eventbus_published_total(&self, topic: &str) -> u64 {
        Self::load_str(&self.eventbus_published, topic)
    }

    // ─── 10 eventbus dropped ───────────────────────────────────

    pub fn tick_eventbus_dropped(&self, reason: &str) {
        Self::bump_str(&self.eventbus_dropped, reason, 1);
    }

    pub fn eventbus_dropped_total(&self, reason: &str) -> u64 {
        Self::load_str(&self.eventbus_dropped, reason)
    }

    // ─── 11 claude calls ───────────────────────────────────────

    pub fn tick_claude_call(&self, outcome: &str) {
        Self::bump_str(&self.claude_calls, outcome, 1);
    }

    pub fn claude_calls_total(&self, outcome: &str) -> u64 {
        Self::load_str(&self.claude_calls, outcome)
    }

    // ─── 12 bundle size ────────────────────────────────────────

    pub fn observe_bundle_size(&self, dag_id: &str, bytes: u64) {
        Self::bump_str(&self.bundle_size_sum, dag_id, bytes);
        Self::bump_str(&self.bundle_size_count, dag_id, 1);
    }

    pub fn bundle_size_count(&self, dag_id: &str) -> u64 {
        Self::load_str(&self.bundle_size_count, dag_id)
    }

    // ─── Render ────────────────────────────────────────────────

    pub fn render_prometheus(&self) -> String {
        let mut out = String::new();
        out.push_str("# HELP n21_swarm_runs_total Swarm runs by outcome.\n");
        out.push_str("# TYPE n21_swarm_runs_total counter\n");
        for r in self.swarm_runs.iter() {
            out.push_str(&format!(
                "n21_swarm_runs_total{{outcome=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }
        out.push_str("# HELP n21_swarm_run_duration_ms Swarm run duration by layer count.\n");
        out.push_str("# TYPE n21_swarm_run_duration_ms summary\n");
        for r in self.swarm_dur_sum.iter() {
            let layers = r.key();
            let sum = r.value().load(Ordering::Relaxed);
            let count = self
                .swarm_dur_count
                .get(layers)
                .map_or(0, |c| c.value().load(Ordering::Relaxed));
            out.push_str(&format!(
                "n21_swarm_run_duration_ms_sum{{dag_layer_count=\"{layers}\"}} {sum}\n",
            ));
            out.push_str(&format!(
                "n21_swarm_run_duration_ms_count{{dag_layer_count=\"{layers}\"}} {count}\n",
            ));
        }
        out.push_str(
            "# HELP n21_swarm_run_cost_microcents_total Swarm cost microcents per tenant tier.\n",
        );
        out.push_str("# TYPE n21_swarm_run_cost_microcents_total counter\n");
        for r in self.swarm_cost.iter() {
            out.push_str(&format!(
                "n21_swarm_run_cost_microcents_total{{tenant_tier=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }
        out.push_str("# HELP n21_agent_invocations_total Agent invocations per outcome.\n");
        out.push_str("# TYPE n21_agent_invocations_total counter\n");
        for r in self.agent_invocations.iter() {
            let (a, o) = r.key();
            out.push_str(&format!(
                "n21_agent_invocations_total{{agent_id=\"{a}\",outcome=\"{o}\"}} {}\n",
                r.value().load(Ordering::Relaxed),
            ));
        }
        out.push_str("# HELP n21_agent_duration_ms Agent duration ms.\n");
        out.push_str("# TYPE n21_agent_duration_ms summary\n");
        for r in self.agent_dur_sum.iter() {
            let agent = r.key();
            let sum = r.value().load(Ordering::Relaxed);
            let count = self
                .agent_dur_count
                .get(agent)
                .map_or(0, |c| c.value().load(Ordering::Relaxed));
            out.push_str(&format!(
                "n21_agent_duration_ms_sum{{agent_id=\"{agent}\"}} {sum}\n",
            ));
            out.push_str(&format!(
                "n21_agent_duration_ms_count{{agent_id=\"{agent}\"}} {count}\n",
            ));
        }
        out.push_str("# HELP n21_agent_timeouts_total Agent timeouts.\n");
        out.push_str("# TYPE n21_agent_timeouts_total counter\n");
        for r in self.agent_timeouts.iter() {
            out.push_str(&format!(
                "n21_agent_timeouts_total{{agent_id=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }
        out.push_str("# HELP n21_blackboard_writes_total Blackboard writes per agent.\n");
        out.push_str("# TYPE n21_blackboard_writes_total counter\n");
        for r in self.blackboard_writes.iter() {
            out.push_str(&format!(
                "n21_blackboard_writes_total{{agent_id=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }
        out.push_str("# HELP n21_blackboard_conflicts_total Blackboard CAS conflicts.\n");
        out.push_str("# TYPE n21_blackboard_conflicts_total counter\n");
        for r in self.blackboard_conflicts.iter() {
            out.push_str(&format!(
                "n21_blackboard_conflicts_total{{pointer_class=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }
        out.push_str("# HELP n21_eventbus_published_total Events published per topic.\n");
        out.push_str("# TYPE n21_eventbus_published_total counter\n");
        for r in self.eventbus_published.iter() {
            out.push_str(&format!(
                "n21_eventbus_published_total{{topic=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }
        out.push_str("# HELP n21_eventbus_dropped_total Eventbus drops by reason.\n");
        out.push_str("# TYPE n21_eventbus_dropped_total counter\n");
        for r in self.eventbus_dropped.iter() {
            out.push_str(&format!(
                "n21_eventbus_dropped_total{{reason=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }
        out.push_str("# HELP n21_claude_calls_total Claude calls by outcome.\n");
        out.push_str("# TYPE n21_claude_calls_total counter\n");
        for r in self.claude_calls.iter() {
            out.push_str(&format!(
                "n21_claude_calls_total{{outcome=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }
        out.push_str("# HELP n21_context_bundle_size_bytes Context bundle size in bytes.\n");
        out.push_str("# TYPE n21_context_bundle_size_bytes summary\n");
        for r in self.bundle_size_sum.iter() {
            let dag = r.key();
            let sum = r.value().load(Ordering::Relaxed);
            let count = self
                .bundle_size_count
                .get(dag)
                .map_or(0, |c| c.value().load(Ordering::Relaxed));
            out.push_str(&format!(
                "n21_context_bundle_size_bytes_sum{{dag_id=\"{dag}\"}} {sum}\n",
            ));
            out.push_str(&format!(
                "n21_context_bundle_size_bytes_count{{dag_id=\"{dag}\"}} {count}\n",
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_metrics_zero() {
        let m = N21Metrics::new();
        assert_eq!(m.swarm_runs_total("passed"), 0);
        assert_eq!(m.swarm_cost_total("tier_1"), 0);
    }

    #[test]
    fn all_12_surfaces_tick() {
        let m = N21Metrics::new();
        m.tick_swarm_run("passed");
        m.observe_swarm_duration_ms(3, 500);
        m.add_swarm_cost("tier_1", 1_000);
        m.tick_agent_invocation("ner_extractor", "completed");
        m.observe_agent_duration_ms("ner_extractor", 12);
        m.tick_agent_timeout("slow_agent");
        m.tick_blackboard_write("ner_extractor");
        m.tick_blackboard_conflict("entities");
        m.tick_eventbus_published("pii_found");
        m.tick_eventbus_dropped("capacity");
        m.tick_claude_call("ok");
        m.observe_bundle_size("dag_1", 2048);

        assert_eq!(m.swarm_runs_total("passed"), 1);
        assert_eq!(m.swarm_duration_count(3), 1);
        assert_eq!(m.swarm_cost_total("tier_1"), 1_000);
        assert_eq!(m.agent_invocations_total("ner_extractor", "completed"), 1);
        assert_eq!(m.agent_duration_count("ner_extractor"), 1);
        assert_eq!(m.agent_timeouts_total("slow_agent"), 1);
        assert_eq!(m.blackboard_writes_total("ner_extractor"), 1);
        assert_eq!(m.blackboard_conflicts_total("entities"), 1);
        assert_eq!(m.eventbus_published_total("pii_found"), 1);
        assert_eq!(m.eventbus_dropped_total("capacity"), 1);
        assert_eq!(m.claude_calls_total("ok"), 1);
        assert_eq!(m.bundle_size_count("dag_1"), 1);
    }

    #[test]
    fn render_contains_all_12_metric_names() {
        let m = N21Metrics::new();
        m.tick_swarm_run("passed");
        m.observe_swarm_duration_ms(3, 100);
        m.add_swarm_cost("tier_1", 100);
        m.tick_agent_invocation("a", "completed");
        m.observe_agent_duration_ms("a", 10);
        m.tick_agent_timeout("a");
        m.tick_blackboard_write("a");
        m.tick_blackboard_conflict("x");
        m.tick_eventbus_published("t");
        m.tick_eventbus_dropped("c");
        m.tick_claude_call("ok");
        m.observe_bundle_size("dag", 100);
        let rendered = m.render_prometheus();
        for name in [
            "n21_swarm_runs_total",
            "n21_swarm_run_duration_ms",
            "n21_swarm_run_cost_microcents_total",
            "n21_agent_invocations_total",
            "n21_agent_duration_ms",
            "n21_agent_timeouts_total",
            "n21_blackboard_writes_total",
            "n21_blackboard_conflicts_total",
            "n21_eventbus_published_total",
            "n21_eventbus_dropped_total",
            "n21_claude_calls_total",
            "n21_context_bundle_size_bytes",
        ] {
            assert!(rendered.contains(name), "missing : {name}");
        }
    }

    #[test]
    fn add_swarm_cost_ignores_negative() {
        let m = N21Metrics::new();
        m.add_swarm_cost("tier_1", -100);
        assert_eq!(m.swarm_cost_total("tier_1"), 0);
    }

    #[test]
    fn debug_format_no_panic() {
        let m = N21Metrics::new();
        let _ = format!("{m:?}");
    }
}
