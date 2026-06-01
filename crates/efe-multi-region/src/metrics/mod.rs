// SPDX-License-Identifier: UNLICENSED
//! **Sprint 9 — N16** : Prometheus surface for multi-region replay.
//!
//! Architecture § 7.8.  Ten metrics in the `n16_*` namespace.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

use efe_nano_sdk::{MatchOutcome, RegionCode, ReplayMode};

/// **Sprint 9 — N16** : ten Prometheus surfaces.
pub struct N16Metrics {
    /// (1) `n16_replay_requests_total{target_region, outcome}`
    requests_total: [[AtomicU64; 2]; 4],
    /// (2) `n16_replay_jobs_inflight{target_region}`
    jobs_inflight: [AtomicI64; 4],
    /// (3) `n16_replay_duration_seconds_sum{mode}`
    duration_sum_us: [AtomicU64; 2],
    duration_count: [AtomicU64; 2],
    /// (4) `n16_chain_shipment_bytes_total{source_region, target_region}`
    shipment_bytes: [[AtomicU64; 4]; 4],
    /// (5) `n16_chain_shipment_duration_seconds_sum{src,tgt}`
    shipment_dur_sum_us: [[AtomicU64; 4]; 4],
    shipment_dur_count: [[AtomicU64; 4]; 4],
    /// (6) `n16_residency_violations_total{target_region}`
    residency_violations: [AtomicU64; 4],
    /// (7) `n16_match_outcome_total{outcome}` — 4 outcomes.
    match_outcomes: [AtomicU64; 4],
    /// (8) `n16_cost_cents_total` — cumulative microcents.
    cost_cents_total_micro: AtomicU64,
    /// (9) `n16_concurrent_replay_collisions_total`
    concurrent_collisions: AtomicU64,
    /// (10) `n16_kms_access_denied_total{target_region}`
    kms_denials: [AtomicU64; 4],
}

fn region_index(r: RegionCode) -> usize {
    match r {
        RegionCode::UsEast1 => 0,
        RegionCode::UsWest2 => 1,
        RegionCode::EuWest1 => 2,
        RegionCode::ApSoutheast1 => 3,
    }
}

fn region_from_index(i: usize) -> RegionCode {
    match i {
        0 => RegionCode::UsEast1,
        1 => RegionCode::UsWest2,
        2 => RegionCode::EuWest1,
        _ => RegionCode::ApSoutheast1,
    }
}

fn outcome_index(o: MatchOutcome) -> usize {
    match o {
        MatchOutcome::Exact => 0,
        MatchOutcome::Tolerated(_) => 1,
        MatchOutcome::Mismatch => 2,
        MatchOutcome::Skipped => 3,
    }
}

fn outcome_label(i: usize) -> &'static str {
    match i {
        0 => "exact",
        1 => "tolerated",
        2 => "mismatch",
        _ => "skipped",
    }
}

fn mode_index(m: ReplayMode) -> usize {
    match m {
        ReplayMode::Full => 0,
        ReplayMode::DispatchOnly => 1,
    }
}

fn make_u64_array<const N: usize>() -> [AtomicU64; N] {
    std::array::from_fn(|_| AtomicU64::new(0))
}

fn make_i64_array<const N: usize>() -> [AtomicI64; N] {
    std::array::from_fn(|_| AtomicI64::new(0))
}

impl std::fmt::Debug for N16Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("N16Metrics")
            .field("cost_cents_total", &self.cost_cents_total())
            .field("concurrent_collisions", &self.concurrent_collisions_total())
            .finish()
    }
}

impl N16Metrics {
    /// **N16** : Construct an all-zero metrics surface.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::zeroed())
    }

    fn zeroed() -> Self {
        Self {
            requests_total: std::array::from_fn(|_| make_u64_array()),
            jobs_inflight: make_i64_array(),
            duration_sum_us: make_u64_array(),
            duration_count: make_u64_array(),
            shipment_bytes: std::array::from_fn(|_| make_u64_array()),
            shipment_dur_sum_us: std::array::from_fn(|_| make_u64_array()),
            shipment_dur_count: std::array::from_fn(|_| make_u64_array()),
            residency_violations: make_u64_array(),
            match_outcomes: make_u64_array(),
            cost_cents_total_micro: AtomicU64::new(0),
            concurrent_collisions: AtomicU64::new(0),
            kms_denials: make_u64_array(),
        }
    }

    /// **N16** : Tick a replay request.
    pub fn tick_request(&self, target_region: RegionCode, accepted: bool) {
        let r = region_index(target_region);
        let o = usize::from(!accepted);
        self.requests_total[r][o].fetch_add(1, Ordering::Relaxed);
    }

    /// **N16** : Adjust the inflight gauge.
    pub fn adjust_inflight(&self, target_region: RegionCode, delta: i64) {
        let r = region_index(target_region);
        self.jobs_inflight[r].fetch_add(delta, Ordering::Relaxed);
    }

    /// **N16** : Record a replay duration in seconds.
    pub fn observe_replay_duration(&self, mode: ReplayMode, seconds: f64) {
        if seconds < 0.0 || !seconds.is_finite() {
            return;
        }
        let i = mode_index(mode);
        let us = (seconds * 1_000_000.0).round() as u64;
        self.duration_sum_us[i].fetch_add(us, Ordering::Relaxed);
        self.duration_count[i].fetch_add(1, Ordering::Relaxed);
    }

    /// **N16** : Record a chain shipment in bytes.
    pub fn record_shipment_bytes(
        &self,
        source: RegionCode,
        target: RegionCode,
        bytes: u64,
    ) {
        let s = region_index(source);
        let t = region_index(target);
        self.shipment_bytes[s][t].fetch_add(bytes, Ordering::Relaxed);
    }

    /// **N16** : Record a chain shipment duration (seconds).
    pub fn observe_shipment_duration(
        &self,
        source: RegionCode,
        target: RegionCode,
        seconds: f64,
    ) {
        if seconds < 0.0 || !seconds.is_finite() {
            return;
        }
        let s = region_index(source);
        let t = region_index(target);
        let us = (seconds * 1_000_000.0).round() as u64;
        self.shipment_dur_sum_us[s][t].fetch_add(us, Ordering::Relaxed);
        self.shipment_dur_count[s][t].fetch_add(1, Ordering::Relaxed);
    }

    /// **N16** : Tick a residency violation.
    pub fn tick_residency_violation(&self, target_region: RegionCode) {
        let r = region_index(target_region);
        self.residency_violations[r].fetch_add(1, Ordering::Relaxed);
    }

    /// **N16** : Tick a per-op match outcome.
    pub fn tick_match_outcome(&self, outcome: MatchOutcome) {
        let o = outcome_index(outcome);
        self.match_outcomes[o].fetch_add(1, Ordering::Relaxed);
    }

    /// **N16** : Add `cents` to the cumulative operator-attributed
    /// cost.
    pub fn record_operator_cost(&self, cents: f64) {
        if cents < 0.0 || !cents.is_finite() {
            return;
        }
        let micro = (cents * 1_000_000.0).round() as u64;
        self.cost_cents_total_micro.fetch_add(micro, Ordering::Relaxed);
    }

    /// **N16** : Tick a concurrent-replay collision.
    pub fn tick_concurrent_collision(&self) {
        self.concurrent_collisions.fetch_add(1, Ordering::Relaxed);
    }

    /// **N16** : Tick a KMS access denial.
    pub fn tick_kms_denial(&self, target_region: RegionCode) {
        let r = region_index(target_region);
        self.kms_denials[r].fetch_add(1, Ordering::Relaxed);
    }

    // ─── Snapshot helpers ────────────────────────────────────────────

    pub fn cost_cents_total(&self) -> f64 {
        self.cost_cents_total_micro.load(Ordering::Relaxed) as f64 / 1_000_000.0
    }

    pub fn concurrent_collisions_total(&self) -> u64 {
        self.concurrent_collisions.load(Ordering::Relaxed)
    }

    pub fn residency_violations_for(&self, target_region: RegionCode) -> u64 {
        self.residency_violations[region_index(target_region)].load(Ordering::Relaxed)
    }

    pub fn inflight_for(&self, target_region: RegionCode) -> i64 {
        self.jobs_inflight[region_index(target_region)].load(Ordering::Relaxed)
    }

    pub fn kms_denials_for(&self, target_region: RegionCode) -> u64 {
        self.kms_denials[region_index(target_region)].load(Ordering::Relaxed)
    }

    pub fn match_outcome_total(&self, outcome: MatchOutcome) -> u64 {
        self.match_outcomes[outcome_index(outcome)].load(Ordering::Relaxed)
    }

    pub fn requests_total_for(&self, target_region: RegionCode, accepted: bool) -> u64 {
        let r = region_index(target_region);
        let o = usize::from(!accepted);
        self.requests_total[r][o].load(Ordering::Relaxed)
    }

    /// **N16** : Render in Prometheus 0.0.4 / OpenMetrics 1.0.0 text.
    pub fn render_prometheus(&self) -> String {
        let mut out = String::with_capacity(4_096);

        out.push_str("# HELP n16_replay_requests_total Replay requests submitted.\n");
        out.push_str("# TYPE n16_replay_requests_total counter\n");
        for r in 0..4 {
            let region = region_from_index(r);
            for o in 0..2 {
                let outcome = if o == 0 { "accepted" } else { "rejected" };
                out.push_str(&format!(
                    "n16_replay_requests_total{{target_region=\"{}\",outcome=\"{}\"}} {}\n",
                    region.as_str(),
                    outcome,
                    self.requests_total[r][o].load(Ordering::Relaxed),
                ));
            }
        }

        out.push_str("# HELP n16_replay_jobs_inflight Replay jobs in flight.\n");
        out.push_str("# TYPE n16_replay_jobs_inflight gauge\n");
        for r in 0..4 {
            let region = region_from_index(r);
            out.push_str(&format!(
                "n16_replay_jobs_inflight{{target_region=\"{}\"}} {}\n",
                region.as_str(),
                self.jobs_inflight[r].load(Ordering::Relaxed),
            ));
        }

        out.push_str("# HELP n16_replay_duration_seconds Replay duration summary.\n");
        out.push_str("# TYPE n16_replay_duration_seconds summary\n");
        for m in 0..2 {
            let mode_label = if m == 0 { "full" } else { "dispatch_only" };
            let sum_us = self.duration_sum_us[m].load(Ordering::Relaxed);
            out.push_str(&format!(
                "n16_replay_duration_seconds_sum{{mode=\"{}\"}} {}\n",
                mode_label,
                sum_us as f64 / 1_000_000.0,
            ));
            out.push_str(&format!(
                "n16_replay_duration_seconds_count{{mode=\"{}\"}} {}\n",
                mode_label,
                self.duration_count[m].load(Ordering::Relaxed),
            ));
        }

        out.push_str("# HELP n16_chain_shipment_bytes_total Bytes shipped cross-region.\n");
        out.push_str("# TYPE n16_chain_shipment_bytes_total counter\n");
        for s in 0..4 {
            for t in 0..4 {
                let src = region_from_index(s);
                let tgt = region_from_index(t);
                out.push_str(&format!(
                    "n16_chain_shipment_bytes_total{{source_region=\"{}\",target_region=\"{}\"}} {}\n",
                    src.as_str(),
                    tgt.as_str(),
                    self.shipment_bytes[s][t].load(Ordering::Relaxed),
                ));
            }
        }

        out.push_str("# HELP n16_chain_shipment_duration_seconds Shipment duration.\n");
        out.push_str("# TYPE n16_chain_shipment_duration_seconds summary\n");
        for s in 0..4 {
            for t in 0..4 {
                let src = region_from_index(s);
                let tgt = region_from_index(t);
                let sum_us = self.shipment_dur_sum_us[s][t].load(Ordering::Relaxed);
                out.push_str(&format!(
                    "n16_chain_shipment_duration_seconds_sum{{source_region=\"{}\",target_region=\"{}\"}} {}\n",
                    src.as_str(),
                    tgt.as_str(),
                    sum_us as f64 / 1_000_000.0,
                ));
                out.push_str(&format!(
                    "n16_chain_shipment_duration_seconds_count{{source_region=\"{}\",target_region=\"{}\"}} {}\n",
                    src.as_str(),
                    tgt.as_str(),
                    self.shipment_dur_count[s][t].load(Ordering::Relaxed),
                ));
            }
        }

        out.push_str("# HELP n16_residency_violations_total Residency-policy denials.\n");
        out.push_str("# TYPE n16_residency_violations_total counter\n");
        for r in 0..4 {
            let region = region_from_index(r);
            out.push_str(&format!(
                "n16_residency_violations_total{{target_region=\"{}\"}} {}\n",
                region.as_str(),
                self.residency_violations[r].load(Ordering::Relaxed),
            ));
        }

        out.push_str("# HELP n16_match_outcome_total Per-op replay match outcome.\n");
        out.push_str("# TYPE n16_match_outcome_total counter\n");
        for o in 0..4 {
            out.push_str(&format!(
                "n16_match_outcome_total{{outcome=\"{}\"}} {}\n",
                outcome_label(o),
                self.match_outcomes[o].load(Ordering::Relaxed),
            ));
        }

        out.push_str(
            "# HELP n16_cost_cents_total Cumulative operator-attributed cost.\n",
        );
        out.push_str("# TYPE n16_cost_cents_total counter\n");
        out.push_str(&format!(
            "n16_cost_cents_total {}\n",
            self.cost_cents_total(),
        ));

        out.push_str(
            "# HELP n16_concurrent_replay_collisions_total Concurrent-replay 409 events.\n",
        );
        out.push_str("# TYPE n16_concurrent_replay_collisions_total counter\n");
        out.push_str(&format!(
            "n16_concurrent_replay_collisions_total {}\n",
            self.concurrent_collisions.load(Ordering::Relaxed),
        ));

        out.push_str("# HELP n16_kms_access_denied_total KMS access denials.\n");
        out.push_str("# TYPE n16_kms_access_denied_total counter\n");
        for r in 0..4 {
            let region = region_from_index(r);
            out.push_str(&format!(
                "n16_kms_access_denied_total{{target_region=\"{}\"}} {}\n",
                region.as_str(),
                self.kms_denials[r].load(Ordering::Relaxed),
            ));
        }

        out
    }
}

impl Default for N16Metrics {
    fn default() -> Self {
        Self::zeroed()
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_metrics_all_zero() {
        let m = N16Metrics::new();
        assert_eq!(m.cost_cents_total(), 0.0);
        assert_eq!(m.concurrent_collisions_total(), 0);
        for r in RegionCode::all() {
            assert_eq!(m.inflight_for(r), 0);
        }
    }

    #[test]
    fn tick_request_increments_per_region_and_outcome() {
        let m = N16Metrics::new();
        m.tick_request(RegionCode::EuWest1, true);
        m.tick_request(RegionCode::EuWest1, true);
        m.tick_request(RegionCode::EuWest1, false);
        m.tick_request(RegionCode::UsEast1, false);
        assert_eq!(m.requests_total_for(RegionCode::EuWest1, true), 2);
        assert_eq!(m.requests_total_for(RegionCode::EuWest1, false), 1);
        assert_eq!(m.requests_total_for(RegionCode::UsEast1, false), 1);
    }

    #[test]
    fn inflight_adjusts_up_and_down() {
        let m = N16Metrics::new();
        m.adjust_inflight(RegionCode::EuWest1, 3);
        m.adjust_inflight(RegionCode::EuWest1, -1);
        assert_eq!(m.inflight_for(RegionCode::EuWest1), 2);
    }

    #[test]
    fn observe_replay_duration_sum_count() {
        let m = N16Metrics::new();
        m.observe_replay_duration(ReplayMode::Full, 1.5);
        m.observe_replay_duration(ReplayMode::Full, 2.5);
        let out = m.render_prometheus();
        assert!(out.contains("n16_replay_duration_seconds_count{mode=\"full\"} 2"));
        assert!(out.contains("n16_replay_duration_seconds_sum{mode=\"full\"} 4"));
    }

    #[test]
    fn shipment_bytes_per_source_target() {
        let m = N16Metrics::new();
        m.record_shipment_bytes(RegionCode::EuWest1, RegionCode::UsEast1, 1024);
        m.record_shipment_bytes(RegionCode::EuWest1, RegionCode::UsEast1, 512);
        let out = m.render_prometheus();
        assert!(out.contains(
            "n16_chain_shipment_bytes_total{source_region=\"eu-west-1\",target_region=\"us-east-1\"} 1536"
        ));
    }

    #[test]
    fn tick_match_outcome_records_per_outcome() {
        let m = N16Metrics::new();
        m.tick_match_outcome(MatchOutcome::Exact);
        m.tick_match_outcome(MatchOutcome::Mismatch);
        m.tick_match_outcome(MatchOutcome::Mismatch);
        m.tick_match_outcome(MatchOutcome::Tolerated(0.97));
        assert_eq!(m.match_outcome_total(MatchOutcome::Exact), 1);
        assert_eq!(m.match_outcome_total(MatchOutcome::Mismatch), 2);
        assert_eq!(m.match_outcome_total(MatchOutcome::Tolerated(0.5)), 1);
    }

    #[test]
    fn record_operator_cost_accumulates_in_microcents() {
        let m = N16Metrics::new();
        m.record_operator_cost(1.5);
        m.record_operator_cost(0.75);
        assert!((m.cost_cents_total() - 2.25).abs() < 1e-6);
    }

    #[test]
    fn record_negative_cost_ignored() {
        let m = N16Metrics::new();
        m.record_operator_cost(-3.0);
        m.record_operator_cost(f64::NAN);
        assert_eq!(m.cost_cents_total(), 0.0);
    }

    #[test]
    fn tick_concurrent_collision() {
        let m = N16Metrics::new();
        m.tick_concurrent_collision();
        m.tick_concurrent_collision();
        assert_eq!(m.concurrent_collisions_total(), 2);
    }

    #[test]
    fn tick_kms_denial_per_region() {
        let m = N16Metrics::new();
        m.tick_kms_denial(RegionCode::UsEast1);
        m.tick_kms_denial(RegionCode::UsEast1);
        m.tick_kms_denial(RegionCode::EuWest1);
        assert_eq!(m.kms_denials_for(RegionCode::UsEast1), 2);
        assert_eq!(m.kms_denials_for(RegionCode::EuWest1), 1);
    }

    #[test]
    fn render_contains_all_10_metric_names() {
        let m = N16Metrics::new();
        let out = m.render_prometheus();
        for name in [
            "n16_replay_requests_total",
            "n16_replay_jobs_inflight",
            "n16_replay_duration_seconds",
            "n16_chain_shipment_bytes_total",
            "n16_chain_shipment_duration_seconds",
            "n16_residency_violations_total",
            "n16_match_outcome_total",
            "n16_cost_cents_total",
            "n16_concurrent_replay_collisions_total",
            "n16_kms_access_denied_total",
        ] {
            assert!(out.contains(name), "missing metric : {name}");
        }
        assert_eq!(out.matches("# HELP").count(), 10);
        assert_eq!(out.matches("# TYPE").count(), 10);
    }

    #[test]
    fn debug_format_does_not_panic() {
        let m = N16Metrics::new();
        let _ = format!("{m:?}");
    }
}
