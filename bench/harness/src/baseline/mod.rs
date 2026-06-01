//! PR #21 § 13 — versioned baseline store.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::compare::Verdict;
use crate::error::{BenchError, BenchResult};
use crate::result::{ChaosEvent, ResultMetrics, ScenarioResult};

/// Same shape as [`ScenarioResult`] but stored under
/// `bench_baselines/v{chart_version}/L2/<scenario_id>.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Baseline {
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
    pub verdict: Option<String>,
}

impl Baseline {
    /// Path layout : `<dir>/v<chart_version>/L2/<scenario_id>.json`.
    #[must_use]
    pub fn path(dir: &Path, chart_version: &str, scenario_id: &str) -> PathBuf {
        dir.join(format!("v{chart_version}")).join("L2").join(format!("{scenario_id}.json"))
    }

    pub fn load(path: &Path) -> BenchResult<Self> {
        let raw = std::fs::read(path).map_err(|source| BenchError::Baseline {
            path: path.to_path_buf(),
            source,
        })?;
        let b: Self = serde_json::from_slice(&raw)?;
        if b.schema_version != crate::BASELINE_SCHEMA_VERSION {
            return Err(BenchError::Internal(format!(
                "baseline schema_version {} not supported",
                b.schema_version
            )));
        }
        Ok(b)
    }

    /// Best-effort load — returns `None` when the file doesn't exist.
    pub fn load_optional(path: &Path) -> BenchResult<Option<Self>> {
        if path.exists() {
            Ok(Some(Self::load(path)?))
        } else {
            Ok(None)
        }
    }

    pub fn write(&self, path: &Path) -> BenchResult<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| BenchError::Baseline {
                path: path.to_path_buf(),
                source,
            })?;
        }
        let body = serde_json::to_vec_pretty(self)?;
        std::fs::write(path, body).map_err(|source| BenchError::Baseline {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Build a baseline from a successful run.
    #[must_use]
    pub fn from_result(result: &ScenarioResult, verdict: Verdict) -> Self {
        Self {
            schema_version: crate::BASELINE_SCHEMA_VERSION,
            scenario_id: result.scenario_id.clone(),
            harness_version: result.harness_version.clone(),
            loadgen_version: result.loadgen_version.clone(),
            chart_version: result.chart_version.clone(),
            image_tag: result.image_tag.clone(),
            git_commit: result.git_commit.clone(),
            run_id: result.run_id.clone(),
            started_at: result.started_at.clone(),
            duration_s: result.duration_s,
            total_requests: result.total_requests,
            metrics: result.metrics.clone(),
            chaos_events: result.chaos_events.clone(),
            verdict: Some(verdict.as_str().into()),
        }
    }
}

/// Manifest committed alongside the baselines (AD30).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineManifest {
    pub chart_version: String,
    pub harness_version: String,
    pub loadgen_version: String,
    pub git_commit: String,
    pub created_at: String,
    pub cluster: ClusterDescription,
    pub scenarios: Vec<ScenarioManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterDescription {
    pub provisioner: String,
    pub k8s_version: Option<String>,
    pub node_class: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioManifestEntry {
    pub id: String,
    pub duration_s: f64,
    pub verdict: String,
}

impl BaselineManifest {
    #[must_use]
    pub fn path(dir: &Path, chart_version: &str) -> PathBuf {
        dir.join(format!("v{chart_version}")).join("manifest.json")
    }

    pub fn write(&self, path: &Path) -> BenchResult<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| BenchError::Baseline {
                path: path.to_path_buf(),
                source,
            })?;
        }
        let body = serde_json::to_vec_pretty(self)?;
        std::fs::write(path, body).map_err(|source| BenchError::Baseline {
            path: path.to_path_buf(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn path_layout_matches_ad10() {
        let d = Path::new("bench_baselines");
        let p = Baseline::path(d, "0.6.0", "S01_baseline_100qps_5min");
        assert_eq!(
            p.to_string_lossy(),
            "bench_baselines/v0.6.0/L2/S01_baseline_100qps_5min.json"
        );
    }

    #[test]
    fn write_then_load_roundtrip() {
        let dir = TempDir::new().unwrap();
        let b = Baseline {
            schema_version: 1,
            scenario_id: "S01_test".into(),
            harness_version: "0.1.0".into(),
            loadgen_version: "0.1.0".into(),
            chart_version: "0.6.0".into(),
            image_tag: "v0.6.1".into(),
            git_commit: "abc123".into(),
            run_id: "r".into(),
            started_at: "2026-05-16T00:00:00Z".into(),
            duration_s: 300.0,
            total_requests: 30000,
            metrics: ResultMetrics::default(),
            chaos_events: vec![],
            verdict: Some("pass".into()),
        };
        let path = Baseline::path(dir.path(), "0.6.0", "S01_test");
        b.write(&path).unwrap();
        let b2 = Baseline::load(&path).unwrap();
        assert_eq!(b2.scenario_id, "S01_test");
    }

    #[test]
    fn load_optional_missing_returns_none() {
        let r = Baseline::load_optional(Path::new("/nonexistent/baseline.json")).unwrap();
        assert!(r.is_none());
    }
}
