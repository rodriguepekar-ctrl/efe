//! PR #21 § 10 — scenario loader + schema v1.

pub mod schema;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{BenchError, BenchResult};

pub use schema::{
    ChaosFaultRef, ChaosFaultAction, ClusterSpec, ProvisionerYaml, Scenario,
    ScenarioLoadgen, ToleranceSpec, ToleranceMetric,
};

impl Scenario {
    /// Load + validate a scenario YAML file.
    pub fn load(path: impl AsRef<Path>) -> BenchResult<Self> {
        let path = path.as_ref();
        let raw = fs::read_to_string(path).map_err(|source| BenchError::Result {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_str(&raw, path)
    }

    /// Parse + validate from raw YAML.
    pub fn from_str(raw: &str, source_hint: &Path) -> BenchResult<Self> {
        let s: Self = serde_yaml::from_str(raw).map_err(|source| BenchError::ScenarioLoad {
            path: source_hint.to_path_buf(),
            source,
        })?;

        // Schema version check (AD37).
        if s.schema_version != crate::SCENARIO_SCHEMA_VERSION {
            return Err(BenchError::SchemaVersion {
                expected: crate::SCENARIO_SCHEMA_VERSION,
                actual: s.schema_version,
            });
        }

        // ID shape : ^S[0-9]{2}_[a-z0-9_]+$
        if !is_valid_id(&s.id) {
            return Err(BenchError::InvalidScenario(format!(
                "id '{}' must match ^S[0-9]{{2}}_[a-z0-9_]+$",
                s.id
            )));
        }

        // Production-mode refusal (AD11, T86).
        if let Some(run) = &s.loadgen.run {
            if run.mode.as_deref() == Some("production") {
                return Err(BenchError::ProductionRefused);
            }
        }

        // Chaos cross-check : if chaos[] is non-empty, all referenced
        // CRD files must be paths under deploy/chaos/pr21_meta_swarm/.
        for fault in &s.chaos {
            if let Some(file) = &fault.crd_file {
                if !file.starts_with("pr21_meta_swarm/") && !file.starts_with("deploy/chaos/") {
                    return Err(BenchError::InvalidScenario(format!(
                        "chaos fault '{}' crd_file must be under deploy/chaos/pr21_meta_swarm/ (got {})",
                        fault.id, file
                    )));
                }
            }
        }

        Ok(s)
    }

    /// Find every scenario YAML in a directory.
    pub fn discover(dir: &Path) -> BenchResult<Vec<Self>> {
        let pattern = dir.join("*.yaml");
        let pattern_str = pattern.to_string_lossy().to_string();
        let mut out = Vec::new();
        for entry in glob::glob(&pattern_str)
            .map_err(|e| BenchError::InvalidScenario(format!("glob : {e}")))?
            .filter_map(Result::ok)
        {
            out.push(Self::load(&entry)?);
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    /// Parse `duration` field into [`Duration`].
    pub fn duration(&self) -> BenchResult<Duration> {
        humantime::parse_duration(&self.duration)
            .map_err(|e| BenchError::InvalidScenario(format!("scenario {} duration : {e}", self.id)))
    }

    /// Convenience : path to default output dir.
    #[must_use]
    pub fn default_output_dir(&self, run_id: &str) -> PathBuf {
        PathBuf::from("bench/results").join(run_id).join(&self.id)
    }
}

/// Returns true if `id` matches `^S[0-9]{2}_[a-z0-9_]+$`.
fn is_valid_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    bytes.len() >= 5
        && bytes[0] == b'S'
        && bytes[1].is_ascii_digit()
        && bytes[2].is_ascii_digit()
        && bytes[3] == b'_'
        && bytes[4..].iter().all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
schema_version: 1
id: S01_baseline_100qps_5min
description: "test"
duration: "5m"
cluster:
  provisioner: kind
loadgen:
  version: 1
  name: t
  target:
    endpoints: [{ url: "grpc://x:50051" }]
  pattern: { type: constant, qps: 10.0 }
chaos: []
tolerance: {}
"#;

    #[test]
    fn valid_loads() {
        let s = Scenario::from_str(VALID, Path::new("inline")).unwrap();
        assert_eq!(s.id, "S01_baseline_100qps_5min");
        assert_eq!(s.duration().unwrap(), Duration::from_secs(300));
    }

    #[test]
    fn rejects_wrong_schema_version() {
        let raw = VALID.replace("schema_version: 1", "schema_version: 2");
        let e = Scenario::from_str(&raw, Path::new("inline")).unwrap_err();
        assert!(matches!(e, BenchError::SchemaVersion { .. }));
    }

    #[test]
    fn rejects_invalid_id() {
        let raw = VALID.replace("S01_baseline_100qps_5min", "bad-id");
        let e = Scenario::from_str(&raw, Path::new("inline")).unwrap_err();
        assert!(matches!(e, BenchError::InvalidScenario(_)));
    }

    #[test]
    fn rejects_production_mode() {
        let raw = VALID.replace(
            "chaos: []",
            "chaos: []\nrun:\n  seed: 42\n  mode: production",
        );
        // Move `run:` block to loadgen.
        let raw = raw.replace(
            "pattern: { type: constant, qps: 10.0 }",
            "pattern: { type: constant, qps: 10.0 }\n  run: { mode: production }",
        );
        let e = Scenario::from_str(&raw, Path::new("inline")).unwrap_err();
        assert!(matches!(e, BenchError::ProductionRefused));
    }

    #[test]
    fn rejects_chaos_outside_pr21_dir() {
        let raw = VALID.replace(
            "chaos: []",
            "chaos:\n  - id: bad\n    at_offset: 1m\n    crd_file: something/random.yaml",
        );
        let e = Scenario::from_str(&raw, Path::new("inline")).unwrap_err();
        assert!(matches!(e, BenchError::InvalidScenario(_)));
    }

    #[test]
    fn id_regex_strict() {
        assert!(is_valid_id("S01_x"));
        assert!(is_valid_id("S99_baseline_100qps_5min"));
        assert!(!is_valid_id("s01_x"));
        assert!(!is_valid_id("S1_x"));
        assert!(!is_valid_id("S01-x"));
        assert!(!is_valid_id("S01_X"));
    }
}
