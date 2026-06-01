// SPDX-License-Identifier: UNLICENSED
//! **Sprint 9 — N17** : Prometheus surface for LLM provenance.
//!
//! Architecture § 7.8.  12 metrics in the `n17_*` namespace.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;

/// **Sprint 9 — N17** : the 12 Prometheus surfaces.
pub struct N17Metrics {
    /// (1) `n17_llm_calls_total{model_id, outcome}`
    llm_calls: DashMap<(String, String), AtomicU64>,
    /// (2) + (3) `n17_llm_tokens_{input,output}_total{model_id}`
    tokens_input: DashMap<String, AtomicU64>,
    tokens_output: DashMap<String, AtomicU64>,
    /// (4) `n17_llm_cost_cents_total{model_id}` — fractional cents
    /// stored as integer microcents to avoid f64 atomics.
    cost_microcents: DashMap<String, AtomicU64>,
    /// (5) `n17_llm_response_time_seconds{model_id}` — sum + count.
    response_time_sum_us: DashMap<String, AtomicU64>,
    response_time_count: DashMap<String, AtomicU64>,
    /// (6) `n17_model_hash_mismatch_total{model_id}`
    model_hash_mismatch: DashMap<String, AtomicU64>,
    /// (7) `n17_provenance_writes_total{outcome}`
    provenance_writes_success: AtomicU64,
    provenance_writes_failure: AtomicU64,
    /// (8) `n17_cache_hits_total`
    cache_hits: AtomicU64,
    /// (9) `n17_replay_calls_total`
    replay_calls: AtomicU64,
    /// (10) `n17_safety_flags_total{flag}`
    safety_flags: DashMap<String, AtomicU64>,
    /// (11) `n17_token_count_drift_ratio{model_id}` — gauge ; stored
    /// as i64 microsigned-ratio (× 1e6).
    token_drift: DashMap<String, AtomicU64>,
    /// (12) `n17_provenance_query_latency_seconds{endpoint}` — sum + count.
    query_latency_sum_us: DashMap<String, AtomicU64>,
    query_latency_count: DashMap<String, AtomicU64>,
}

impl std::fmt::Debug for N17Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("N17Metrics")
            .field("llm_calls", &self.llm_calls.len())
            .field("cache_hits", &self.cache_hits.load(Ordering::Relaxed))
            .field("replay_calls", &self.replay_calls.load(Ordering::Relaxed))
            .finish()
    }
}

impl Default for N17Metrics {
    fn default() -> Self {
        Self::zeroed()
    }
}

impl N17Metrics {
    /// **N17** : Construct.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::zeroed())
    }

    fn zeroed() -> Self {
        Self {
            llm_calls: DashMap::new(),
            tokens_input: DashMap::new(),
            tokens_output: DashMap::new(),
            cost_microcents: DashMap::new(),
            response_time_sum_us: DashMap::new(),
            response_time_count: DashMap::new(),
            model_hash_mismatch: DashMap::new(),
            provenance_writes_success: AtomicU64::new(0),
            provenance_writes_failure: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            replay_calls: AtomicU64::new(0),
            safety_flags: DashMap::new(),
            token_drift: DashMap::new(),
            query_latency_sum_us: DashMap::new(),
            query_latency_count: DashMap::new(),
        }
    }

    fn bump_dash(map: &DashMap<String, AtomicU64>, key: &str, delta: u64) {
        map.entry(key.to_string())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(delta, Ordering::Relaxed);
    }

    fn bump_dash_pair(
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

    fn dash_load(map: &DashMap<String, AtomicU64>, key: &str) -> u64 {
        map.get(key).map_or(0, |r| r.value().load(Ordering::Relaxed))
    }

    fn dash_pair_load(
        map: &DashMap<(String, String), AtomicU64>,
        k1: &str,
        k2: &str,
    ) -> u64 {
        let key = (k1.to_string(), k2.to_string());
        map.get(&key)
            .map_or(0, |r| r.value().load(Ordering::Relaxed))
    }

    /// (1) Tick an LLM call.
    pub fn tick_llm_call(&self, model_id: &str, outcome: &str) {
        Self::bump_dash_pair(&self.llm_calls, model_id, outcome, 1);
    }

    /// (2)+(3) Add token counts.
    pub fn add_tokens(&self, model_id: &str, input: u32, output: u32) {
        Self::bump_dash(&self.tokens_input, model_id, u64::from(input));
        Self::bump_dash(&self.tokens_output, model_id, u64::from(output));
    }

    /// (4) Add cost cents.
    pub fn add_cost(&self, model_id: &str, cents: f64) {
        if cents < 0.0 || !cents.is_finite() {
            return;
        }
        let micro = (cents * 1_000_000.0).round() as u64;
        Self::bump_dash(&self.cost_microcents, model_id, micro);
    }

    /// (5) Record a response-time observation.
    pub fn observe_response_time(&self, model_id: &str, seconds: f64) {
        if seconds < 0.0 || !seconds.is_finite() {
            return;
        }
        let us = (seconds * 1_000_000.0).round() as u64;
        Self::bump_dash(&self.response_time_sum_us, model_id, us);
        Self::bump_dash(&self.response_time_count, model_id, 1);
    }

    /// (6) Tick a hash-mismatch event.
    pub fn tick_model_hash_mismatch(&self, model_id: &str) {
        Self::bump_dash(&self.model_hash_mismatch, model_id, 1);
    }

    /// (7) Tick a provenance write.
    pub fn tick_provenance_write(&self, success: bool) {
        let target = if success {
            &self.provenance_writes_success
        } else {
            &self.provenance_writes_failure
        };
        target.fetch_add(1, Ordering::Relaxed);
    }

    /// (8) Tick a cache hit.
    pub fn tick_cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    /// (9) Tick a replay call.
    pub fn tick_replay_call(&self) {
        self.replay_calls.fetch_add(1, Ordering::Relaxed);
    }

    /// (10) Tick a safety flag.
    pub fn tick_safety_flag(&self, flag_label: &str) {
        Self::bump_dash(&self.safety_flags, flag_label, 1);
    }

    /// (11) Set token-count drift ratio for a model.
    ///
    /// Encoded as `(ratio + 2.0) × 1e6` so the unset sentinel `0`
    /// cannot collide with a legitimate ratio of `-1.0` (which would
    /// represent provider reporting 0 tokens for every call — rare
    /// but possible).  Range covered : `ratio ∈ [-2.0, ∞)`.
    ///
    /// `ratio` is clamped to a finite value before encoding ; NaN /
    /// ±∞ are silently dropped.
    pub fn set_token_drift(&self, model_id: &str, ratio: f64) {
        if !ratio.is_finite() {
            return;
        }
        // Shift by +2 so ratio == -1.0 → encoded == 1_000_000, never
        // 0 (which is reserved for "unset").
        let encoded = ((ratio + 2.0) * 1_000_000.0).round() as i64;
        let storage = encoded.max(1) as u64;
        self.token_drift
            .entry(model_id.to_string())
            .or_insert_with(|| AtomicU64::new(0))
            .store(storage, Ordering::Relaxed);
    }

    /// (12) Record a query-endpoint latency.
    pub fn observe_query_latency(&self, endpoint: &str, seconds: f64) {
        if seconds < 0.0 || !seconds.is_finite() {
            return;
        }
        let us = (seconds * 1_000_000.0).round() as u64;
        Self::bump_dash(&self.query_latency_sum_us, endpoint, us);
        Self::bump_dash(&self.query_latency_count, endpoint, 1);
    }

    // ─── Snapshot helpers ────────────────────────────────────────────

    pub fn llm_call_total(&self, model_id: &str, outcome: &str) -> u64 {
        Self::dash_pair_load(&self.llm_calls, model_id, outcome)
    }

    pub fn tokens_input_total(&self, model_id: &str) -> u64 {
        Self::dash_load(&self.tokens_input, model_id)
    }

    pub fn tokens_output_total(&self, model_id: &str) -> u64 {
        Self::dash_load(&self.tokens_output, model_id)
    }

    pub fn cost_cents_total(&self, model_id: &str) -> f64 {
        Self::dash_load(&self.cost_microcents, model_id) as f64 / 1_000_000.0
    }

    pub fn model_hash_mismatch_total(&self, model_id: &str) -> u64 {
        Self::dash_load(&self.model_hash_mismatch, model_id)
    }

    pub fn cache_hits_total(&self) -> u64 {
        self.cache_hits.load(Ordering::Relaxed)
    }

    pub fn replay_calls_total(&self) -> u64 {
        self.replay_calls.load(Ordering::Relaxed)
    }

    pub fn safety_flag_total(&self, label: &str) -> u64 {
        Self::dash_load(&self.safety_flags, label)
    }

    pub fn provenance_writes_success_total(&self) -> u64 {
        self.provenance_writes_success.load(Ordering::Relaxed)
    }

    pub fn provenance_writes_failure_total(&self) -> u64 {
        self.provenance_writes_failure.load(Ordering::Relaxed)
    }

    pub fn token_drift(&self, model_id: &str) -> Option<f64> {
        let raw = self
            .token_drift
            .get(model_id)
            .map(|r| r.value().load(Ordering::Relaxed))?;
        if raw == 0 {
            // Unset sentinel.
            None
        } else {
            // Inverse of the `(ratio + 2.0) * 1e6` encoding.
            Some((raw as f64 / 1_000_000.0) - 2.0)
        }
    }

    /// **N17** : Render in OpenMetrics 1.0.0 text format.
    pub fn render_prometheus(&self) -> String {
        let mut out = String::with_capacity(4_096);

        out.push_str("# HELP n17_llm_calls_total LLM call counter.\n");
        out.push_str("# TYPE n17_llm_calls_total counter\n");
        for r in self.llm_calls.iter() {
            let (m, o) = r.key();
            out.push_str(&format!(
                "n17_llm_calls_total{{model_id=\"{m}\",outcome=\"{o}\"}} {}\n",
                r.value().load(Ordering::Relaxed),
            ));
        }

        out.push_str("# HELP n17_llm_tokens_input_total LLM input tokens.\n");
        out.push_str("# TYPE n17_llm_tokens_input_total counter\n");
        for r in self.tokens_input.iter() {
            out.push_str(&format!(
                "n17_llm_tokens_input_total{{model_id=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }

        out.push_str("# HELP n17_llm_tokens_output_total LLM output tokens.\n");
        out.push_str("# TYPE n17_llm_tokens_output_total counter\n");
        for r in self.tokens_output.iter() {
            out.push_str(&format!(
                "n17_llm_tokens_output_total{{model_id=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }

        out.push_str("# HELP n17_llm_cost_cents_total Operator-attributable LLM cost.\n");
        out.push_str("# TYPE n17_llm_cost_cents_total counter\n");
        for r in self.cost_microcents.iter() {
            out.push_str(&format!(
                "n17_llm_cost_cents_total{{model_id=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed) as f64 / 1_000_000.0,
            ));
        }

        out.push_str("# HELP n17_llm_response_time_seconds LLM response time.\n");
        out.push_str("# TYPE n17_llm_response_time_seconds summary\n");
        for r in self.response_time_sum_us.iter() {
            let m = r.key();
            let sum_us = r.value().load(Ordering::Relaxed);
            let count =
                self.response_time_count.get(m).map_or(0, |c| c.value().load(Ordering::Relaxed));
            out.push_str(&format!(
                "n17_llm_response_time_seconds_sum{{model_id=\"{m}\"}} {}\n",
                sum_us as f64 / 1_000_000.0,
            ));
            out.push_str(&format!(
                "n17_llm_response_time_seconds_count{{model_id=\"{m}\"}} {count}\n",
            ));
        }

        out.push_str(
            "# HELP n17_model_hash_mismatch_total Model integrity violations.\n",
        );
        out.push_str("# TYPE n17_model_hash_mismatch_total counter\n");
        for r in self.model_hash_mismatch.iter() {
            out.push_str(&format!(
                "n17_model_hash_mismatch_total{{model_id=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }

        out.push_str("# HELP n17_provenance_writes_total Provenance writes.\n");
        out.push_str("# TYPE n17_provenance_writes_total counter\n");
        out.push_str(&format!(
            "n17_provenance_writes_total{{outcome=\"success\"}} {}\n",
            self.provenance_writes_success.load(Ordering::Relaxed),
        ));
        out.push_str(&format!(
            "n17_provenance_writes_total{{outcome=\"failure\"}} {}\n",
            self.provenance_writes_failure.load(Ordering::Relaxed),
        ));

        out.push_str("# HELP n17_cache_hits_total Cache hits served as Mode C.\n");
        out.push_str("# TYPE n17_cache_hits_total counter\n");
        out.push_str(&format!("n17_cache_hits_total {}\n", self.cache_hits.load(Ordering::Relaxed)));

        out.push_str("# HELP n17_replay_calls_total Replay-driven LLM calls.\n");
        out.push_str("# TYPE n17_replay_calls_total counter\n");
        out.push_str(&format!(
            "n17_replay_calls_total {}\n",
            self.replay_calls.load(Ordering::Relaxed),
        ));

        out.push_str("# HELP n17_safety_flags_total Per-flag safety events.\n");
        out.push_str("# TYPE n17_safety_flags_total counter\n");
        for r in self.safety_flags.iter() {
            out.push_str(&format!(
                "n17_safety_flags_total{{flag=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }

        out.push_str(
            "# HELP n17_token_count_drift_ratio Provider-vs-client token drift.\n",
        );
        out.push_str("# TYPE n17_token_count_drift_ratio gauge\n");
        for r in self.token_drift.iter() {
            let model_id = r.key();
            let raw = r.value().load(Ordering::Relaxed);
            if raw == 0 {
                continue;
            }
            // Inverse of `(ratio + 2.0) * 1e6` encoding (matches
            // `Self::token_drift`).
            let ratio = (raw as f64 / 1_000_000.0) - 2.0;
            out.push_str(&format!(
                "n17_token_count_drift_ratio{{model_id=\"{model_id}\"}} {ratio}\n",
            ));
        }

        out.push_str(
            "# HELP n17_provenance_query_latency_seconds Query endpoint latency.\n",
        );
        out.push_str("# TYPE n17_provenance_query_latency_seconds summary\n");
        for r in self.query_latency_sum_us.iter() {
            let e = r.key();
            let sum_us = r.value().load(Ordering::Relaxed);
            let count =
                self.query_latency_count.get(e).map_or(0, |c| c.value().load(Ordering::Relaxed));
            out.push_str(&format!(
                "n17_provenance_query_latency_seconds_sum{{endpoint=\"{e}\"}} {}\n",
                sum_us as f64 / 1_000_000.0,
            ));
            out.push_str(&format!(
                "n17_provenance_query_latency_seconds_count{{endpoint=\"{e}\"}} {count}\n",
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
        let m = N17Metrics::new();
        assert_eq!(m.cache_hits_total(), 0);
        assert_eq!(m.replay_calls_total(), 0);
        assert_eq!(m.llm_call_total("gpt-4", "complete"), 0);
    }

    #[test]
    fn tick_llm_call_per_model_outcome() {
        let m = N17Metrics::new();
        m.tick_llm_call("gpt-4", "complete");
        m.tick_llm_call("gpt-4", "complete");
        m.tick_llm_call("gpt-4", "refused");
        m.tick_llm_call("gpt-3.5", "complete");
        assert_eq!(m.llm_call_total("gpt-4", "complete"), 2);
        assert_eq!(m.llm_call_total("gpt-4", "refused"), 1);
        assert_eq!(m.llm_call_total("gpt-3.5", "complete"), 1);
    }

    #[test]
    fn add_tokens_accumulates() {
        let m = N17Metrics::new();
        m.add_tokens("gpt-4", 100, 200);
        m.add_tokens("gpt-4", 50, 75);
        assert_eq!(m.tokens_input_total("gpt-4"), 150);
        assert_eq!(m.tokens_output_total("gpt-4"), 275);
    }

    #[test]
    fn add_cost_accumulates_in_microcents() {
        let m = N17Metrics::new();
        m.add_cost("gpt-4", 1.5);
        m.add_cost("gpt-4", 0.75);
        assert!((m.cost_cents_total("gpt-4") - 2.25).abs() < 1e-6);
    }

    #[test]
    fn add_cost_ignores_negative_or_nan() {
        let m = N17Metrics::new();
        m.add_cost("gpt-4", -1.0);
        m.add_cost("gpt-4", f64::NAN);
        assert_eq!(m.cost_cents_total("gpt-4"), 0.0);
    }

    #[test]
    fn observe_response_time_records_sum_count() {
        let m = N17Metrics::new();
        m.observe_response_time("gpt-4", 0.5);
        m.observe_response_time("gpt-4", 0.7);
        let out = m.render_prometheus();
        assert!(out.contains("n17_llm_response_time_seconds_count{model_id=\"gpt-4\"} 2"));
    }

    #[test]
    fn model_hash_mismatch_total_per_model() {
        let m = N17Metrics::new();
        m.tick_model_hash_mismatch("gpt-4");
        m.tick_model_hash_mismatch("gpt-4");
        m.tick_model_hash_mismatch("gpt-3.5");
        assert_eq!(m.model_hash_mismatch_total("gpt-4"), 2);
        assert_eq!(m.model_hash_mismatch_total("gpt-3.5"), 1);
    }

    #[test]
    fn provenance_writes_success_failure_separate() {
        let m = N17Metrics::new();
        m.tick_provenance_write(true);
        m.tick_provenance_write(true);
        m.tick_provenance_write(false);
        assert_eq!(m.provenance_writes_success_total(), 2);
        assert_eq!(m.provenance_writes_failure_total(), 1);
    }

    #[test]
    fn cache_and_replay_counters() {
        let m = N17Metrics::new();
        m.tick_cache_hit();
        m.tick_cache_hit();
        m.tick_replay_call();
        assert_eq!(m.cache_hits_total(), 2);
        assert_eq!(m.replay_calls_total(), 1);
    }

    #[test]
    fn safety_flag_per_label() {
        let m = N17Metrics::new();
        m.tick_safety_flag("content_filter");
        m.tick_safety_flag("content_filter");
        m.tick_safety_flag("toxicity");
        assert_eq!(m.safety_flag_total("content_filter"), 2);
        assert_eq!(m.safety_flag_total("toxicity"), 1);
    }

    #[test]
    fn token_drift_round_trips() {
        let m = N17Metrics::new();
        m.set_token_drift("gpt-4", 0.033);
        let got = m.token_drift("gpt-4").unwrap();
        assert!((got - 0.033).abs() < 1e-5);
    }

    #[test]
    fn token_drift_handles_negative() {
        let m = N17Metrics::new();
        m.set_token_drift("gpt-4", -0.1);
        let got = m.token_drift("gpt-4").unwrap();
        assert!((got - -0.1).abs() < 1e-5);
    }

    /// **Post-audit fix — bug H** : a ratio of exactly `-1.0` (the
    /// extreme case where provider reports 0 tokens for every call)
    /// must NOT collide with the unset sentinel.
    #[test]
    fn token_drift_negative_one_does_not_collide_with_unset() {
        let m = N17Metrics::new();
        m.set_token_drift("gpt-4", -1.0);
        let got = m.token_drift("gpt-4").unwrap();
        assert!((got - -1.0).abs() < 1e-5);
    }

    #[test]
    fn token_drift_unset_returns_none() {
        let m = N17Metrics::new();
        assert!(m.token_drift("never-set").is_none());
    }

    #[test]
    fn token_drift_rejects_non_finite_values() {
        let m = N17Metrics::new();
        m.set_token_drift("a", f64::NAN);
        m.set_token_drift("b", f64::INFINITY);
        m.set_token_drift("c", f64::NEG_INFINITY);
        assert!(m.token_drift("a").is_none());
        assert!(m.token_drift("b").is_none());
        assert!(m.token_drift("c").is_none());
    }

    #[test]
    fn observe_query_latency_per_endpoint() {
        let m = N17Metrics::new();
        m.observe_query_latency("get_by_trace", 0.005);
        m.observe_query_latency("get_by_trace", 0.003);
        let out = m.render_prometheus();
        assert!(out
            .contains("n17_provenance_query_latency_seconds_count{endpoint=\"get_by_trace\"} 2"));
    }

    #[test]
    fn render_contains_all_12_metric_names() {
        let m = N17Metrics::new();
        // Seed at least one observation per metric so labels emit.
        m.tick_llm_call("x", "complete");
        m.add_tokens("x", 1, 1);
        m.add_cost("x", 0.1);
        m.observe_response_time("x", 0.01);
        m.tick_model_hash_mismatch("x");
        m.tick_provenance_write(true);
        m.tick_cache_hit();
        m.tick_replay_call();
        m.tick_safety_flag("toxicity");
        m.set_token_drift("x", 0.1);
        m.observe_query_latency("y", 0.001);
        let out = m.render_prometheus();
        for name in [
            "n17_llm_calls_total",
            "n17_llm_tokens_input_total",
            "n17_llm_tokens_output_total",
            "n17_llm_cost_cents_total",
            "n17_llm_response_time_seconds",
            "n17_model_hash_mismatch_total",
            "n17_provenance_writes_total",
            "n17_cache_hits_total",
            "n17_replay_calls_total",
            "n17_safety_flags_total",
            "n17_token_count_drift_ratio",
            "n17_provenance_query_latency_seconds",
        ] {
            assert!(out.contains(name), "missing : {name}");
        }
        assert_eq!(out.matches("# HELP").count(), 12);
        assert_eq!(out.matches("# TYPE").count(), 12);
    }

    #[test]
    fn debug_format_does_not_panic() {
        let m = N17Metrics::new();
        let _ = format!("{m:?}");
    }
}
