//! PR #22 § 19 — read PR #21 `summary.json` files as `source=bench`
//! SLI cells.
//!
//! NB : This is **read-only** (AD3).  The rollup never writes back
//! to `bench_baselines/`.

use std::path::Path;

use async_trait::async_trait;
use serde::Deserialize;

use crate::error::{SloError, SloResult};
use crate::source::{EventQuery, HistogramQuery, MetricQuery, RecoveryEvent, SourceProvider};

/// A loaded bench summary (matches the `bench/harness` schema v1).
#[derive(Debug, Deserialize)]
pub struct BenchSummary {
    pub schema_version: u32,
    pub scenario_id: String,
    pub metrics: BenchMetrics,
    #[serde(default)]
    pub duration_s: f64,
    #[serde(default)]
    pub total_requests: u64,
}

#[derive(Debug, Default, Deserialize)]
pub struct BenchMetrics {
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
    pub error_rate: Option<f64>,
    pub throughput_qps: Option<f64>,
    pub recovery_time_s: Option<f64>,
}

#[derive(Debug)]
pub struct BenchBaselineSource {
    summary: BenchSummary,
}

impl BenchBaselineSource {
    /// Load a `summary.json` from disk.
    pub fn load(path: &Path) -> SloResult<Self> {
        let raw = std::fs::read(path).map_err(|source| SloError::StorageIo {
            path: path.to_path_buf(),
            source,
        })?;
        let summary: BenchSummary = serde_json::from_slice(&raw)?;
        if summary.schema_version != 1 {
            return Err(SloError::SchemaVersion {
                expected: 1,
                actual: summary.schema_version,
            });
        }
        Ok(Self { summary })
    }
}

#[async_trait]
impl SourceProvider for BenchBaselineSource {
    async fn integer_counter(&self, q: MetricQuery) -> SloResult<u64> {
        // Bench summaries don't carry per-status request counts ; we
        // map total_requests + error_rate into success/error split.
        let total = self.summary.total_requests;
        let err_rate = self.summary.metrics.error_rate.unwrap_or(0.0);
        match q.label_matchers.first() {
            Some((k, v)) if k == "status" && v == "success" => {
                Ok(((total as f64) * (1.0 - err_rate)).round() as u64)
            }
            Some((k, v)) if k == "status" && (v == "server_error" || v == "transport_error") => {
                // We can't split server/transport ; attribute all errors to server.
                if v == "server_error" {
                    Ok(((total as f64) * err_rate).round() as u64)
                } else {
                    Ok(0)
                }
            }
            _ => Ok(total),
        }
    }

    async fn histogram_quantile(&self, q: HistogramQuery) -> SloResult<f64> {
        // Convert "ms" units in BenchMetrics to "s" expected by SLI math.
        let val_ms = match q.quantile {
            x if (x - 0.50).abs() < 1e-3 => self.summary.metrics.p50_ms,
            x if (x - 0.95).abs() < 1e-3 => self.summary.metrics.p95_ms,
            x if (x - 0.99).abs() < 1e-3 => self.summary.metrics.p99_ms,
            _ => return Ok(f64::NAN),
        };
        Ok(val_ms.map(|ms| ms / 1000.0).unwrap_or(f64::NAN))
    }

    async fn recovery_events(&self, _: EventQuery) -> SloResult<Vec<RecoveryEvent>> {
        // Bench summary carries one optional recovery time per scenario.
        if let Some(rt) = self.summary.metrics.recovery_time_s {
            Ok(vec![RecoveryEvent {
                duration_seconds: rt,
                at_unix: 0,
            }])
        } else {
            Ok(vec![])
        }
    }

    async fn scrape_coverage(&self, _: i64, _: u64) -> SloResult<f64> {
        // Bench is a controlled run ⇒ full coverage by construction.
        Ok(1.0)
    }

    async fn participants(&self, _: i64, _: u64) -> SloResult<Vec<String>> {
        // Bench doesn't expose per-participant lag ; SLO-4 is skipped
        // for source=bench in the rollup orchestrator.
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const FIXTURE: &str = r#"{
        "schema_version": 1,
        "scenario_id": "S01_x",
        "harness_version": "0.1.0",
        "loadgen_version": "0.1.0",
        "chart_version": "0.6.0",
        "image_tag": "t",
        "git_commit": "abc",
        "run_id": "r",
        "started_at": "0",
        "duration_s": 60.0,
        "total_requests": 10000,
        "metrics": {
            "p50_ms": 8.0,
            "p95_ms": 45.0,
            "p99_ms": 90.0,
            "error_rate": 0.001,
            "throughput_qps": 166.0,
            "recovery_time_s": null
        },
        "chaos_events": [],
        "verdict": "pass"
    }"#;

    #[tokio::test]
    async fn loads_bench_summary() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.as_file().write_all(FIXTURE.as_bytes()).unwrap();
        let src = BenchBaselineSource::load(tmp.path()).unwrap();
        assert_eq!(src.summary.scenario_id, "S01_x");
    }

    #[tokio::test]
    async fn maps_success_count_from_error_rate() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.as_file().write_all(FIXTURE.as_bytes()).unwrap();
        let src = BenchBaselineSource::load(tmp.path()).unwrap();
        let v = src
            .integer_counter(MetricQuery {
                metric: "efe_l2_request_total".into(),
                window_seconds: 60,
                window_start_unix: 0,
                label_matchers: vec![("status".into(), "success".into())],
            })
            .await
            .unwrap();
        // error_rate 0.1% ⇒ success ≈ 99.9% of 10_000 = 9_990
        assert_eq!(v, 9990);
    }

    #[tokio::test]
    async fn histogram_quantile_converts_ms_to_s() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.as_file().write_all(FIXTURE.as_bytes()).unwrap();
        let src = BenchBaselineSource::load(tmp.path()).unwrap();
        let v = src
            .histogram_quantile(HistogramQuery {
                metric_bucket: "x".into(),
                quantile: 0.95,
                window_seconds: 60,
                window_start_unix: 0,
                label_matchers: vec![],
            })
            .await
            .unwrap();
        // 45ms → 0.045s
        assert!((v - 0.045).abs() < 1e-9);
    }

    #[test]
    fn rejects_wrong_schema_version() {
        let bad = FIXTURE.replace("\"schema_version\": 1", "\"schema_version\": 99");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.as_file().write_all(bad.as_bytes()).unwrap();
        let e = BenchBaselineSource::load(tmp.path()).unwrap_err();
        assert!(matches!(e, SloError::SchemaVersion { .. }));
    }
}
