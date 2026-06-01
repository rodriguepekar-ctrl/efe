//! PR #20 § 14 + § 20 — request stats aggregation + SLO evaluation.
//!
//! Holds the per-verb hdrhistogram + counters that drive :
//!   * the terminal TUI live panel,
//!   * the Prometheus self-instrumentation,
//!   * the SLO termination policy,
//!   * the end-of-run summary (also used for parquet output).
//!
//! Designed to be lock-friendly : the histogram is wrapped in
//! `parking_lot::Mutex` ; the engine emits one stat per request via
//! a tokio mpsc channel and a dedicated aggregator task that owns
//! the histogram exclusively (so the lock is uncontended).

use std::collections::HashMap;
use std::time::Duration;

use hdrhistogram::Histogram;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::config::{SloDirection, SloThreshold};

/// Verb taxonomy — mirrored from CLI for serialization clarity.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verb {
    Iterate,
    Step,
    Reset,
    State,
}

impl Verb {
    /// Stable wire-format string used by Prometheus labels + result file.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Iterate => "iterate",
            Self::Step => "step",
            Self::Reset => "reset",
            Self::State => "state",
        }
    }
}

/// Result classification — closed taxonomy from STABILITY_CONTRACT v1.0 § 3.1.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestResult {
    Ok,
    Error,
    Deadline,
    Canceled,
    Throttled,
}

impl RequestResult {
    /// Stable label value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Deadline => "deadline",
            Self::Canceled => "canceled",
            Self::Throttled => "throttled",
        }
    }

    /// True only for successful responses.
    #[must_use]
    pub const fn is_ok(self) -> bool {
        matches!(self, Self::Ok)
    }
}

/// Single request observation emitted by the engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestRecord {
    pub ts_ns: u64,
    pub verb: Verb,
    pub result: RequestResult,
    pub duration_ns: u64,
    pub target: String,
    pub trace_id: String,
    pub payload_bytes: u32,
    pub error_msg: Option<String>,
}

/// Per-verb stats container.
#[derive(Debug)]
pub struct VerbStats {
    /// Counts by [`RequestResult`].
    counts: HashMap<RequestResult, u64>,
    /// Latency histogram (microsecond resolution, 1µs..60s).
    histogram: Histogram<u64>,
}

impl Default for VerbStats {
    fn default() -> Self {
        Self {
            counts: HashMap::new(),
            // hdrhistogram : record values in microseconds from 1 µs to
            // 60 s with 3 significant figures.  ~ 30 KiB per histogram —
            // negligible vs the request future overhead.
            histogram: Histogram::<u64>::new_with_bounds(1, 60_000_000, 3)
                .expect("hdrhistogram bounds"),
        }
    }
}

impl VerbStats {
    /// Record a single observation.
    pub fn record(&mut self, result: RequestResult, dur: Duration) {
        *self.counts.entry(result).or_insert(0) += 1;
        // Saturate to the histogram's upper bound.
        let micros = dur.as_micros().min(60_000_000_u128) as u64;
        let _ = self.histogram.record(micros.max(1));
    }

    /// Total request count across all results.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.counts.values().sum()
    }

    /// Count for a specific result.
    #[must_use]
    pub fn count(&self, r: RequestResult) -> u64 {
        self.counts.get(&r).copied().unwrap_or(0)
    }

    /// Availability = ok / total (returns 1.0 when total = 0).
    #[must_use]
    pub fn availability(&self) -> f64 {
        let total = self.total();
        if total == 0 {
            return 1.0;
        }
        self.count(RequestResult::Ok) as f64 / total as f64
    }

    /// Quantile in seconds.
    #[must_use]
    pub fn quantile_secs(&self, q: f64) -> f64 {
        let micros = self.histogram.value_at_quantile(q.clamp(0.0, 1.0));
        micros as f64 / 1_000_000.0
    }

    /// p50 in seconds.
    #[must_use]
    pub fn p50(&self) -> f64 { self.quantile_secs(0.50) }
    /// p95 in seconds.
    #[must_use]
    pub fn p95(&self) -> f64 { self.quantile_secs(0.95) }
    /// p99 in seconds.
    #[must_use]
    pub fn p99(&self) -> f64 { self.quantile_secs(0.99) }
}

/// Aggregated per-run stats.  Thread-safe via interior `parking_lot::Mutex`
/// so the engine can write from any tokio task.
#[derive(Debug, Default)]
pub struct RunStats {
    inner: Mutex<RunStatsInner>,
}

#[derive(Debug, Default)]
struct RunStatsInner {
    by_verb: HashMap<Verb, VerbStats>,
    inflight: HashMap<Verb, u64>,
    start_ts_ns: u64,
}

impl RunStats {
    /// New empty stats.  `start_ts_ns` is captured here for the SLO
    /// "actual" computation window.
    #[must_use]
    pub fn new(start_ts_ns: u64) -> Self {
        Self {
            inner: Mutex::new(RunStatsInner {
                by_verb: HashMap::new(),
                inflight: HashMap::new(),
                start_ts_ns,
            }),
        }
    }

    /// Record a finished request.
    pub fn record(&self, verb: Verb, result: RequestResult, dur: Duration) {
        let mut g = self.inner.lock();
        g.by_verb.entry(verb).or_default().record(result, dur);
    }

    /// Mark a request as in-flight (called at request send time).
    pub fn mark_inflight(&self, verb: Verb) {
        let mut g = self.inner.lock();
        *g.inflight.entry(verb).or_insert(0) += 1;
    }

    /// Mark a request as completed (called at response time).
    pub fn mark_finished(&self, verb: Verb) {
        let mut g = self.inner.lock();
        let v = g.inflight.entry(verb).or_insert(0);
        *v = v.saturating_sub(1);
    }

    /// In-flight count for a verb (atomic snapshot).
    #[must_use]
    pub fn inflight(&self, verb: Verb) -> u64 {
        self.inner.lock().inflight.get(&verb).copied().unwrap_or(0)
    }

    /// Snapshot the stats for serialisation.
    #[must_use]
    pub fn snapshot(&self) -> StatsSnapshot {
        let g = self.inner.lock();
        let mut by_verb = HashMap::new();
        for (v, s) in &g.by_verb {
            by_verb.insert(
                *v,
                VerbSnapshot {
                    total: s.total(),
                    ok: s.count(RequestResult::Ok),
                    error: s.count(RequestResult::Error),
                    deadline: s.count(RequestResult::Deadline),
                    canceled: s.count(RequestResult::Canceled),
                    throttled: s.count(RequestResult::Throttled),
                    p50_s: s.p50(),
                    p95_s: s.p95(),
                    p99_s: s.p99(),
                },
            );
        }
        StatsSnapshot { by_verb, start_ts_ns: g.start_ts_ns }
    }

    /// Evaluate every SLO threshold against the live state.
    /// Returns the first breach found, or `None` when all SLOs hold.
    #[must_use]
    pub fn first_slo_breach(&self, thresholds: &[SloThreshold]) -> Option<SloBreach> {
        let snap = self.snapshot();
        for t in thresholds {
            let actual = snap.compute_slo(&t.slo_id)?;
            let pass = match t.direction {
                SloDirection::AtLeast => actual >= t.target,
                SloDirection::AtMost => actual <= t.target,
            };
            if !pass {
                return Some(SloBreach {
                    slo_id: t.slo_id.clone(),
                    target: t.target,
                    actual,
                    direction: t.direction,
                });
            }
        }
        None
    }
}

/// Cheap immutable snapshot for serialisation / SLO check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsSnapshot {
    pub by_verb: HashMap<Verb, VerbSnapshot>,
    pub start_ts_ns: u64,
}

/// Per-verb numbers — JSON-serialised in the result summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerbSnapshot {
    pub total: u64,
    pub ok: u64,
    pub error: u64,
    pub deadline: u64,
    pub canceled: u64,
    pub throttled: u64,
    pub p50_s: f64,
    pub p95_s: f64,
    pub p99_s: f64,
}

impl StatsSnapshot {
    /// Compute the value of a given SLO id from the snapshot.  Returns
    /// `None` when the SLO id is unknown.
    ///
    /// Mapping (STABILITY_CONTRACT v1.1 § 9.5) :
    /// * `iterate-availability` → ok-ratio for verb=iterate
    /// * `iterate-latency-p95`  → p95 in seconds for verb=iterate
    /// * `manifest-reload-latency-p95` → no equivalent on loadgen side ; None
    /// * `otel-drop-rate`       → no equivalent on loadgen side ; None
    #[must_use]
    pub fn compute_slo(&self, slo_id: &str) -> Option<f64> {
        match slo_id {
            "iterate-availability" => {
                let v = self.by_verb.get(&Verb::Iterate)?;
                if v.total == 0 { Some(1.0) } else { Some(v.ok as f64 / v.total as f64) }
            }
            "iterate-latency-p95" => {
                let v = self.by_verb.get(&Verb::Iterate)?;
                Some(v.p95_s)
            }
            "iterate-latency-p99" => {
                let v = self.by_verb.get(&Verb::Iterate)?;
                Some(v.p99_s)
            }
            _ => None,
        }
    }
}

/// SLO breach record returned by [`RunStats::first_slo_breach`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SloBreach {
    pub slo_id: String,
    pub target: f64,
    pub actual: f64,
    pub direction: SloDirection,
}

// ────────────────────────────────────────────────────────────────
//                              Tests
// ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verb_strings_are_stable_taxonomy() {
        // STABILITY_CONTRACT v1.2 § 9.10 closed taxonomy
        assert_eq!(Verb::Iterate.as_str(), "iterate");
        assert_eq!(Verb::Step.as_str(), "step");
        assert_eq!(Verb::Reset.as_str(), "reset");
        assert_eq!(Verb::State.as_str(), "state");
    }

    #[test]
    fn result_strings_match_stability_contract() {
        assert_eq!(RequestResult::Ok.as_str(), "ok");
        assert_eq!(RequestResult::Error.as_str(), "error");
        assert_eq!(RequestResult::Deadline.as_str(), "deadline");
    }

    #[test]
    fn verb_stats_records_and_quantiles() {
        let mut v = VerbStats::default();
        for ms in [1u64, 2, 3, 4, 5, 10, 100, 200, 500, 1000] {
            v.record(RequestResult::Ok, Duration::from_millis(ms));
        }
        assert_eq!(v.total(), 10);
        assert_eq!(v.count(RequestResult::Ok), 10);
        // p50 should land at ~ 5 ms
        let p50 = v.p50();
        assert!((0.001..=0.012).contains(&p50), "got p50={p50}");
        // p99 should be roughly 1.0 s
        let p99 = v.p99();
        assert!(p99 >= 0.5, "got p99={p99}");
    }

    #[test]
    fn run_stats_thread_safe_record() {
        use std::sync::Arc;
        use std::thread;
        let stats = Arc::new(RunStats::new(0));
        let mut handles = vec![];
        for _ in 0..4 {
            let s = stats.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..1000 {
                    s.record(Verb::Iterate, RequestResult::Ok, Duration::from_micros(50));
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let snap = stats.snapshot();
        let v = snap.by_verb.get(&Verb::Iterate).unwrap();
        assert_eq!(v.total, 4000);
    }

    #[test]
    fn slo_breach_detected_when_threshold_violated() {
        let stats = RunStats::new(0);
        // 950 ok, 50 error → availability 95 % < 99.9 %
        for _ in 0..950 {
            stats.record(Verb::Iterate, RequestResult::Ok, Duration::from_millis(10));
        }
        for _ in 0..50 {
            stats.record(Verb::Iterate, RequestResult::Error, Duration::from_millis(10));
        }
        let thresholds = vec![SloThreshold {
            slo_id: "iterate-availability".into(),
            target: 0.999,
            direction: SloDirection::AtLeast,
        }];
        let breach = stats.first_slo_breach(&thresholds).expect("breach");
        assert_eq!(breach.slo_id, "iterate-availability");
        assert!(breach.actual < breach.target);
    }

    #[test]
    fn slo_no_breach_when_met() {
        let stats = RunStats::new(0);
        for _ in 0..10000 {
            stats.record(Verb::Iterate, RequestResult::Ok, Duration::from_millis(10));
        }
        let thresholds = vec![SloThreshold {
            slo_id: "iterate-availability".into(),
            target: 0.999,
            direction: SloDirection::AtLeast,
        }];
        assert!(stats.first_slo_breach(&thresholds).is_none());
    }

    #[test]
    fn inflight_bookkeeping_balanced() {
        let s = RunStats::new(0);
        s.mark_inflight(Verb::Iterate);
        s.mark_inflight(Verb::Iterate);
        assert_eq!(s.inflight(Verb::Iterate), 2);
        s.mark_finished(Verb::Iterate);
        assert_eq!(s.inflight(Verb::Iterate), 1);
        s.mark_finished(Verb::Iterate);
        assert_eq!(s.inflight(Verb::Iterate), 0);
        // Saturating on extra finish.
        s.mark_finished(Verb::Iterate);
        assert_eq!(s.inflight(Verb::Iterate), 0);
    }
}
