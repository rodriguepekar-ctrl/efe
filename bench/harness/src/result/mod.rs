//! PR #21 § 11 — result format.
//!
//! Three artefacts per scenario :
//!   * `result.parquet` (or ndjson stub in Phase-2 — matches PR #20 G.7)
//!   * `summary.json` (the canonical metric table)
//!   * `prom-snapshot.tar.gz` (optional)

pub mod summary;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub use summary::ScenarioResult;

use crate::scenario::ToleranceMetric;

/// Stable metric set committed in STABILITY_CONTRACT v1.3 § 21.3.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ResultMetrics {
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
    /// Fraction in [0, 1].
    pub error_rate: Option<f64>,
    pub throughput_qps: Option<f64>,
    /// Time-to-recover after a chaos fault (only meaningful for chaos scenarios).
    pub recovery_time_s: Option<f64>,
}

impl ResultMetrics {
    /// Lookup by [`ToleranceMetric`].
    #[must_use]
    pub fn get(&self, metric: ToleranceMetric) -> Option<f64> {
        match metric {
            ToleranceMetric::P50Ms => self.p50_ms,
            ToleranceMetric::P95Ms => self.p95_ms,
            ToleranceMetric::P99Ms => self.p99_ms,
            ToleranceMetric::ErrorRate => self.error_rate,
            ToleranceMetric::ThroughputQps => self.throughput_qps,
            ToleranceMetric::RecoveryTimeS => self.recovery_time_s,
        }
    }

    /// Convert to a flat map for serialisation in compact contexts.
    #[must_use]
    pub fn as_map(&self) -> HashMap<&'static str, f64> {
        let mut out = HashMap::new();
        if let Some(v) = self.p50_ms { out.insert("p50_ms", v); }
        if let Some(v) = self.p95_ms { out.insert("p95_ms", v); }
        if let Some(v) = self.p99_ms { out.insert("p99_ms", v); }
        if let Some(v) = self.error_rate { out.insert("error_rate", v); }
        if let Some(v) = self.throughput_qps { out.insert("throughput_qps", v); }
        if let Some(v) = self.recovery_time_s { out.insert("recovery_time_s", v); }
        out
    }
}

/// Chaos event timeline entry — recorded in `summary.json.chaos_events`
/// per AD42.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChaosEvent {
    pub at_offset_s: f64,
    pub fault_id: String,
    pub event_type: String, // "apply" | "delete" | "completed"
    pub crd_kind: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_metrics_lookup_via_taxonomy() {
        let m = ResultMetrics {
            p50_ms: Some(8.0),
            p95_ms: Some(100.0),
            p99_ms: None,
            error_rate: Some(0.001),
            throughput_qps: Some(100.0),
            recovery_time_s: None,
        };
        assert_eq!(m.get(ToleranceMetric::P50Ms), Some(8.0));
        assert_eq!(m.get(ToleranceMetric::P99Ms), None);
        assert_eq!(m.get(ToleranceMetric::ErrorRate), Some(0.001));
    }

    #[test]
    fn result_metrics_as_map_skips_none() {
        let m = ResultMetrics {
            p50_ms: Some(8.0),
            ..Default::default()
        };
        let map = m.as_map();
        assert_eq!(map.len(), 1);
        assert!(map.contains_key("p50_ms"));
    }
}
