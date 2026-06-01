//! PR #21 § 11.2 — `summary.json` (the canonical metric table).

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{ChaosEvent, ResultMetrics};
use crate::compare::Verdict;
use crate::error::{BenchError, BenchResult};

/// Per-scenario result.  Serialised to `summary.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioResult {
    pub schema_version: u32,
    pub scenario_id: String,
    pub harness_version: String,
    pub loadgen_version: String,
    pub chart_version: String,
    pub image_tag: String,
    pub git_commit: String,
    pub run_id: String,
    pub started_at: String,
    pub duration_s: f64,
    pub total_requests: u64,
    pub metrics: ResultMetrics,
    pub chaos_events: Vec<ChaosEvent>,
    pub verdict: Option<Verdict>,
}

impl ScenarioResult {
    /// Build a minimal result for testing.
    #[must_use]
    pub fn with_metrics(scenario_id: String, metrics: ResultMetrics) -> Self {
        Self {
            schema_version: crate::BASELINE_SCHEMA_VERSION,
            scenario_id,
            harness_version: crate::build::VERSION.into(),
            loadgen_version: efe_meta_swarm_loadgen::build::VERSION.into(),
            chart_version: "unknown".into(),
            image_tag: "unknown".into(),
            git_commit: crate::build::GIT_SHA.into(),
            run_id: uuid::Uuid::new_v4().to_string(),
            started_at: chrono::Utc::now().to_rfc3339(),
            duration_s: 0.0,
            total_requests: 0,
            metrics,
            chaos_events: vec![],
            verdict: None,
        }
    }

    /// Write the summary as pretty-printed JSON.
    pub fn write(&self, path: &Path) -> BenchResult<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| BenchError::Result {
                path: path.to_path_buf(),
                source,
            })?;
        }
        let body = serde_json::to_vec_pretty(self)?;
        std::fs::write(path, body).map_err(|source| BenchError::Result {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Read a summary from disk.
    pub fn read(path: &Path) -> BenchResult<Self> {
        let raw = std::fs::read(path).map_err(|source| BenchError::Result {
            path: path.to_path_buf(),
            source,
        })?;
        let s: Self = serde_json::from_slice(&raw)?;
        if s.schema_version != crate::BASELINE_SCHEMA_VERSION {
            return Err(BenchError::Internal(format!(
                "schema_version {} not supported (expected {})",
                s.schema_version,
                crate::BASELINE_SCHEMA_VERSION
            )));
        }
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn write_then_read_roundtrip() {
        let m = ResultMetrics {
            p50_ms: Some(8.0),
            p95_ms: Some(100.0),
            p99_ms: Some(500.0),
            error_rate: Some(0.001),
            throughput_qps: Some(100.0),
            recovery_time_s: None,
        };
        let s = ScenarioResult::with_metrics("S01_test".into(), m.clone());
        let tmp = NamedTempFile::new().unwrap();
        s.write(tmp.path()).unwrap();
        let s2 = ScenarioResult::read(tmp.path()).unwrap();
        assert_eq!(s2.metrics, m);
        assert_eq!(s2.scenario_id, "S01_test");
    }
}
