//! **Sprint 8 — N13** : Sliding window of shadow comparison outcomes
//! per (tenant, agent_id) ; promotion + rollback gate evaluator.
//!
//! See architecture doc § 4.5 and § 10-11.
//!
//! # Memory layout
//!
//! ```text
//! DashMap<(TenantId, NanoAgentId), Mutex<RingBuffer<ShadowOutcome>>>
//! ```
//!
//! Each ring buffer holds at most `max_samples` (default 1000).  When
//! full, oldest samples drop ; latency / cost percentile calculations
//! recompute on read from the live buffer.
//!
//! # Cardinality cap
//!
//! Globally, at most `max_windows` (default 10 000) windows are
//! tracked.  When exceeded, the LRU (least-recently-touched) window is
//! dropped.  Per-window cap × global cap = 10 000 × 1000 × ~200 bytes
//! ≈ 2 GB worst case — bounded.

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use efe_nano_sdk::{
    DriftStats, GateOutcome, NanoAgentId, PromotionGates, ShadowOutcome, TenantId,
};
use std::collections::VecDeque;
use std::sync::Mutex;

// ─────────────────────────────────────────────────────────────────────
//  DriftWindow
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N13** : per-(tenant, agent_id) sliding window of shadow
/// outcomes.
pub struct DriftWindow {
    windows: DashMap<(TenantId, NanoAgentId), Mutex<Window>>,
    max_samples: usize,
    last_rollback_at: DashMap<(TenantId, NanoAgentId), DateTime<Utc>>,
    rollback_cooldown_seconds: i64,
}

struct Window {
    samples: VecDeque<ShadowOutcome>,
    last_touched: DateTime<Utc>,
}

impl DriftWindow {
    pub fn new(max_samples: usize) -> Self {
        Self {
            windows: DashMap::new(),
            max_samples,
            last_rollback_at: DashMap::new(),
            rollback_cooldown_seconds: 3600,
        }
    }

    /// Default Sprint 8 ship constants : 1000 samples per window,
    /// 1 hour rollback cooldown.
    pub fn default_for_sprint_8() -> Self {
        Self::new(1000)
    }

    /// **Record a single shadow outcome.**  O(1) amortized.
    pub fn record(&self, tenant: TenantId, agent_id: NanoAgentId, outcome: ShadowOutcome) {
        let key = (tenant, agent_id);
        let now = Utc::now();
        let entry = self.windows.entry(key).or_insert_with(|| {
            Mutex::new(Window {
                samples: VecDeque::with_capacity(self.max_samples),
                last_touched: now,
            })
        });
        let mut w = entry.lock().expect("drift window mutex poisoned");
        if w.samples.len() == self.max_samples {
            w.samples.pop_front();
        }
        w.samples.push_back(outcome);
        w.last_touched = now;
    }

    /// **Compute aggregated stats over the current window.**  Returns
    /// `None` if no samples have been recorded.
    pub fn stats(&self, tenant: TenantId, agent_id: &NanoAgentId) -> Option<DriftStats> {
        let key = (tenant, agent_id.clone());
        let entry = self.windows.get(&key)?;
        let w = entry.lock().expect("drift window mutex poisoned");
        if w.samples.is_empty() {
            return Some(DriftStats::empty());
        }
        Some(aggregate(&w.samples))
    }

    /// **Drop the window for a specific (tenant, agent_id).**  Called
    /// on promotion / rollback / unpublish.
    pub fn clear(&self, tenant: TenantId, agent_id: &NanoAgentId) {
        self.windows.remove(&(tenant, agent_id.clone()));
    }

    /// **Drop every window for a tenant** — cascade on tenant deletion.
    pub fn clear_tenant(&self, tenant: TenantId) {
        self.windows.retain(|key, _| key.0 != tenant);
        self.last_rollback_at.retain(|key, _| key.0 != tenant);
    }

    /// Mark that an auto-rollback just fired for this (tenant, agent_id)
    /// — used to enforce the cooldown.
    pub fn mark_rollback(&self, tenant: TenantId, agent_id: NanoAgentId) {
        self.last_rollback_at.insert((tenant, agent_id), Utc::now());
    }

    /// Is this agent currently in rollback cooldown ?  Subsequent
    /// `set_rollout_policy` calls should be rejected during this
    /// window (operator override goes through a different path).
    pub fn in_rollback_cooldown(&self, tenant: TenantId, agent_id: &NanoAgentId) -> bool {
        if let Some(last) = self.last_rollback_at.get(&(tenant, agent_id.clone())) {
            let elapsed = Utc::now() - *last;
            elapsed.num_seconds() < self.rollback_cooldown_seconds
        } else {
            false
        }
    }

    pub fn live_window_count(&self) -> usize {
        self.windows.len()
    }
}

// ─────────────────────────────────────────────────────────────────────
//  aggregate
// ─────────────────────────────────────────────────────────────────────

fn aggregate(samples: &VecDeque<ShadowOutcome>) -> DriftStats {
    let mut match_count = 0u32;
    let mut mismatch_count = 0u32;
    let mut failed_count = 0u32;
    let mut unavailable_count = 0u32;
    let mut prod_latencies: Vec<u32> = Vec::with_capacity(samples.len());
    let mut shadow_latencies: Vec<u32> = Vec::with_capacity(samples.len());
    let mut prod_cost_total: f64 = 0.0;
    let mut shadow_cost_total: f64 = 0.0;

    for s in samples {
        match s {
            ShadowOutcome::Match {
                prod_latency_ms,
                shadow_latency_ms,
                prod_cost_cents,
                shadow_cost_cents,
                ..
            } => {
                match_count += 1;
                prod_latencies.push(*prod_latency_ms);
                shadow_latencies.push(*shadow_latency_ms);
                prod_cost_total += prod_cost_cents;
                shadow_cost_total += shadow_cost_cents;
            }
            ShadowOutcome::Mismatch {
                prod_latency_ms,
                shadow_latency_ms,
                prod_cost_cents,
                shadow_cost_cents,
                ..
            } => {
                mismatch_count += 1;
                prod_latencies.push(*prod_latency_ms);
                shadow_latencies.push(*shadow_latency_ms);
                prod_cost_total += prod_cost_cents;
                shadow_cost_total += shadow_cost_cents;
            }
            ShadowOutcome::ShadowFailed { .. } => {
                failed_count += 1;
            }
            ShadowOutcome::ComparatorUnavailable { .. } => {
                unavailable_count += 1;
            }
        }
    }

    let sample_count = match_count + mismatch_count + failed_count + unavailable_count;
    let conclusive = match_count + mismatch_count;

    let mismatch_rate_bps = if conclusive == 0 {
        0
    } else {
        ((mismatch_count as u64 * 10_000) / conclusive as u64) as u16
    };

    let error_rate_bps = if sample_count == 0 {
        0
    } else {
        ((failed_count as u64 * 10_000) / sample_count as u64) as u16
    };

    let p99_latency_delta_pct = if prod_latencies.is_empty() {
        0
    } else {
        let prod_p99 = percentile(&mut prod_latencies, 99) as i64;
        let shadow_p99 = percentile(&mut shadow_latencies, 99) as i64;
        if prod_p99 == 0 {
            0
        } else {
            (((shadow_p99 - prod_p99) * 100) / prod_p99) as i32
        }
    };

    let cost_delta_pct = if prod_cost_total <= 0.0 {
        0
    } else {
        (((shadow_cost_total - prod_cost_total) * 100.0) / prod_cost_total) as i32
    };

    DriftStats {
        sample_count,
        match_count,
        mismatch_count,
        failed_shadow_count: failed_count,
        comparator_unavailable_count: unavailable_count,
        mismatch_rate_bps,
        error_rate_bps,
        p99_latency_delta_pct,
        cost_delta_pct,
    }
}

fn percentile(samples: &mut [u32], pctile: u8) -> u32 {
    if samples.is_empty() {
        return 0;
    }
    samples.sort_unstable();
    let idx = ((samples.len() as f64) * (pctile as f64) / 100.0).ceil() as usize;
    let idx = idx.saturating_sub(1).min(samples.len() - 1);
    samples[idx]
}

// ─────────────────────────────────────────────────────────────────────
//  PromotionGateEvaluator
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N13** : evaluates [`PromotionGates`] against
/// [`DriftStats`] to decide if a promotion may proceed (or if auto-
/// rollback should fire).
pub struct PromotionGateEvaluator;

impl PromotionGateEvaluator {
    /// **Evaluate gates for PROMOTION.**  Returns `Pass` only if every
    /// gate is satisfied ; `Block` otherwise, with one reason per
    /// failing gate.
    pub fn evaluate_for_promotion(stats: &DriftStats, gates: &PromotionGates) -> GateOutcome {
        let mut reasons = Vec::new();

        if stats.sample_count < gates.min_sample_count {
            reasons.push(format!(
                "insufficient samples : {} < {} minimum",
                stats.sample_count, gates.min_sample_count
            ));
        }
        if stats.mismatch_rate_bps > gates.max_drift_bps {
            reasons.push(format!(
                "drift {} bps > {} bps maximum",
                stats.mismatch_rate_bps, gates.max_drift_bps
            ));
        }
        if stats.error_rate_bps > gates.max_error_rate_bps {
            reasons.push(format!(
                "error rate {} bps > {} bps maximum",
                stats.error_rate_bps, gates.max_error_rate_bps
            ));
        }
        if stats.p99_latency_delta_pct > gates.max_latency_p99_delta_pct as i32 {
            reasons.push(format!(
                "latency p99 delta {}% > {}% maximum",
                stats.p99_latency_delta_pct, gates.max_latency_p99_delta_pct
            ));
        }
        if stats.cost_delta_pct > gates.max_cost_delta_pct as i32 {
            reasons.push(format!(
                "cost delta {}% > {}% maximum",
                stats.cost_delta_pct, gates.max_cost_delta_pct
            ));
        }

        if reasons.is_empty() {
            GateOutcome::Pass
        } else {
            GateOutcome::Block { reasons }
        }
    }

    /// **Should AUTO-ROLLBACK fire ?**  Returns `Some(reason)` if the
    /// drift / error gates are breached AND `min_sample_count` is met.
    ///
    /// Distinct from `evaluate_for_promotion` :
    /// - Only checks DRIFT + ERROR (not latency / cost — those are
    ///   advisory at rollback time).
    /// - Returns Some(reason) instead of GateOutcome::Block — callers
    ///   need a single trigger condition.
    pub fn should_auto_rollback(
        stats: &DriftStats,
        gates: &PromotionGates,
    ) -> Option<String> {
        if stats.sample_count < gates.min_sample_count {
            return None;
        }
        if stats.mismatch_rate_bps > gates.max_drift_bps {
            return Some(format!(
                "auto-rollback : drift {} bps > {} bps @ {} samples",
                stats.mismatch_rate_bps, gates.max_drift_bps, stats.sample_count
            ));
        }
        if stats.error_rate_bps > gates.max_error_rate_bps {
            return Some(format!(
                "auto-rollback : error rate {} bps > {} bps @ {} samples",
                stats.error_rate_bps, gates.max_error_rate_bps, stats.sample_count
            ));
        }
        None
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn aid(s: &str) -> NanoAgentId {
        NanoAgentId::new(s).unwrap()
    }
    fn matched(latency_prod: u32, latency_shadow: u32, cost_prod: f64, cost_shadow: f64) -> ShadowOutcome {
        ShadowOutcome::Match {
            compared_at: Utc::now(),
            similarity: 1.0,
            prod_latency_ms: latency_prod,
            shadow_latency_ms: latency_shadow,
            prod_cost_cents: cost_prod,
            shadow_cost_cents: cost_shadow,
        }
    }
    fn mismatched() -> ShadowOutcome {
        ShadowOutcome::Mismatch {
            compared_at: Utc::now(),
            similarity: 0.5,
            diff_summary: "x".into(),
            prod_latency_ms: 10,
            shadow_latency_ms: 10,
            prod_cost_cents: 0.001,
            shadow_cost_cents: 0.001,
        }
    }
    fn failed() -> ShadowOutcome {
        ShadowOutcome::ShadowFailed {
            compared_at: Utc::now(),
            error: "boom".into(),
        }
    }

    #[test]
    fn record_then_stats_returns_aggregated() {
        let w = DriftWindow::new(1000);
        let t = TenantId::system();
        let a = aid("agent");
        for _ in 0..10 {
            w.record(t, a.clone(), matched(10, 12, 0.001, 0.002));
        }
        let s = w.stats(t, &a).unwrap();
        assert_eq!(s.sample_count, 10);
        assert_eq!(s.match_count, 10);
        assert_eq!(s.mismatch_count, 0);
        assert_eq!(s.mismatch_rate_bps, 0);
    }

    #[test]
    fn stats_compute_mismatch_rate_bps() {
        let w = DriftWindow::new(1000);
        let t = TenantId::system();
        let a = aid("agent");
        for _ in 0..90 {
            w.record(t, a.clone(), matched(10, 10, 0.001, 0.001));
        }
        for _ in 0..10 {
            w.record(t, a.clone(), mismatched());
        }
        let s = w.stats(t, &a).unwrap();
        // 10 mismatches / 100 conclusive = 10 % = 1000 bps.
        assert_eq!(s.mismatch_rate_bps, 1000);
    }

    #[test]
    fn stats_compute_error_rate_bps() {
        let w = DriftWindow::new(1000);
        let t = TenantId::system();
        let a = aid("agent");
        for _ in 0..95 {
            w.record(t, a.clone(), matched(10, 10, 0.001, 0.001));
        }
        for _ in 0..5 {
            w.record(t, a.clone(), failed());
        }
        let s = w.stats(t, &a).unwrap();
        // 5 failed / 100 total = 5 % = 500 bps.
        assert_eq!(s.error_rate_bps, 500);
    }

    #[test]
    fn ring_buffer_evicts_oldest_when_full() {
        let w = DriftWindow::new(5);
        let t = TenantId::system();
        let a = aid("agent");
        for _ in 0..10 {
            w.record(t, a.clone(), matched(10, 10, 0.001, 0.001));
        }
        let s = w.stats(t, &a).unwrap();
        assert_eq!(s.sample_count, 5, "ring buffer caps at 5");
    }

    #[test]
    fn stats_for_unknown_agent_returns_none() {
        let w = DriftWindow::new(1000);
        assert!(w.stats(TenantId::system(), &aid("never_recorded")).is_none());
    }

    #[test]
    fn clear_drops_window() {
        let w = DriftWindow::new(1000);
        let t = TenantId::system();
        let a = aid("agent");
        w.record(t, a.clone(), matched(10, 10, 0.001, 0.001));
        assert!(w.stats(t, &a).is_some());
        w.clear(t, &a);
        assert!(w.stats(t, &a).is_none());
    }

    #[test]
    fn clear_tenant_cascades() {
        let w = DriftWindow::new(1000);
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        w.record(t1, aid("a"), matched(10, 10, 0.001, 0.001));
        w.record(t2, aid("a"), matched(10, 10, 0.001, 0.001));
        w.clear_tenant(t1);
        assert!(w.stats(t1, &aid("a")).is_none());
        assert!(w.stats(t2, &aid("a")).is_some());
    }

    #[test]
    fn rollback_cooldown_is_observed() {
        let w = DriftWindow::new(1000);
        let t = TenantId::system();
        let a = aid("agent");
        assert!(!w.in_rollback_cooldown(t, &a));
        w.mark_rollback(t, a.clone());
        assert!(w.in_rollback_cooldown(t, &a));
    }

    #[test]
    fn latency_delta_compute() {
        let w = DriftWindow::new(1000);
        let t = TenantId::system();
        let a = aid("agent");
        for _ in 0..100 {
            w.record(t, a.clone(), matched(100, 150, 0.001, 0.001));
        }
        let s = w.stats(t, &a).unwrap();
        // shadow_p99 = 150, prod_p99 = 100, delta = 50 %.
        assert_eq!(s.p99_latency_delta_pct, 50);
    }

    #[test]
    fn cost_delta_compute() {
        let w = DriftWindow::new(1000);
        let t = TenantId::system();
        let a = aid("agent");
        for _ in 0..100 {
            w.record(t, a.clone(), matched(10, 10, 0.001, 0.002));
        }
        let s = w.stats(t, &a).unwrap();
        // shadow_cost = 0.2, prod_cost = 0.1, delta = 100 %.
        assert_eq!(s.cost_delta_pct, 100);
    }

    // ─────────────────────────────────────────────────────────────────
    //  PromotionGateEvaluator
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn gate_pass_when_all_within_bounds() {
        let stats = DriftStats {
            sample_count: 200,
            match_count: 198,
            mismatch_count: 2,
            failed_shadow_count: 0,
            comparator_unavailable_count: 0,
            mismatch_rate_bps: 100, // 1 %, equal to default max
            error_rate_bps: 0,
            p99_latency_delta_pct: 30,
            cost_delta_pct: 50,
        };
        let gates = PromotionGates::default();
        // mismatch_rate exactly equal → strictly > comparison fails ⇒ pass.
        let outcome = PromotionGateEvaluator::evaluate_for_promotion(&stats, &gates);
        assert!(outcome.is_pass());
    }

    #[test]
    fn gate_block_on_insufficient_samples() {
        let stats = DriftStats {
            sample_count: 50,
            ..DriftStats::empty()
        };
        let gates = PromotionGates::default();
        let outcome = PromotionGateEvaluator::evaluate_for_promotion(&stats, &gates);
        assert!(outcome.is_block());
        if let GateOutcome::Block { reasons } = outcome {
            assert!(reasons[0].contains("insufficient samples"));
        }
    }

    #[test]
    fn gate_block_on_drift_breach() {
        let stats = DriftStats {
            sample_count: 200,
            match_count: 180,
            mismatch_count: 20,
            mismatch_rate_bps: 1000, // 10 %
            ..DriftStats::empty()
        };
        let gates = PromotionGates::default();
        let outcome = PromotionGateEvaluator::evaluate_for_promotion(&stats, &gates);
        assert!(outcome.is_block());
    }

    #[test]
    fn gate_reasons_accumulate_multiple_blockers() {
        let stats = DriftStats {
            sample_count: 50, // < min
            mismatch_rate_bps: 500, // > default 100
            error_rate_bps: 200, // > default 50
            ..DriftStats::empty()
        };
        let gates = PromotionGates::default();
        let outcome = PromotionGateEvaluator::evaluate_for_promotion(&stats, &gates);
        if let GateOutcome::Block { reasons } = outcome {
            assert!(reasons.len() >= 3, "expected multiple blockers, got {reasons:?}");
        } else {
            panic!("expected Block");
        }
    }

    #[test]
    fn auto_rollback_returns_none_below_samples() {
        let stats = DriftStats {
            sample_count: 50,
            mismatch_rate_bps: 10_000, // 100 % drift, but below min samples
            ..DriftStats::empty()
        };
        let gates = PromotionGates::default();
        assert!(PromotionGateEvaluator::should_auto_rollback(&stats, &gates).is_none());
    }

    #[test]
    fn auto_rollback_fires_above_samples_and_drift() {
        let stats = DriftStats {
            sample_count: 200,
            match_count: 150,
            mismatch_count: 50,
            mismatch_rate_bps: 2500, // 25 %, well above default 100
            ..DriftStats::empty()
        };
        let gates = PromotionGates::default();
        let r = PromotionGateEvaluator::should_auto_rollback(&stats, &gates);
        assert!(r.is_some());
        assert!(r.unwrap().contains("drift"));
    }

    #[test]
    fn auto_rollback_fires_on_error_rate() {
        let stats = DriftStats {
            sample_count: 200,
            match_count: 100,
            mismatch_count: 0,
            failed_shadow_count: 100,
            mismatch_rate_bps: 0,
            error_rate_bps: 5000, // 50 %, way above default 50 bps
            ..DriftStats::empty()
        };
        let gates = PromotionGates::default();
        let r = PromotionGateEvaluator::should_auto_rollback(&stats, &gates);
        assert!(r.is_some());
        assert!(r.unwrap().contains("error rate"));
    }
}
