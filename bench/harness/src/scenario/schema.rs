//! PR #21 § 10 + Appendix B — serde types for the scenario YAML.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::cli::ProvisionerKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    pub schema_version: u32,
    pub id: String,
    pub description: String,
    /// humantime string (e.g. "5m", "2h").
    pub duration: String,

    #[serde(default)]
    pub cluster: ClusterSpec,

    /// Verbatim PR #20 profile-v1 (subset).  Deserialised as an opaque
    /// YAML node so we can re-emit it to a temp file for the loadgen
    /// binary or pass to the loadgen library.
    pub loadgen: ScenarioLoadgen,

    #[serde(default)]
    pub chaos: Vec<ChaosFaultRef>,

    #[serde(default)]
    pub results: ResultsConfig,

    #[serde(default)]
    pub tolerance: ToleranceSpec,
}

/// Cluster topology + provisioner.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClusterSpec {
    #[serde(default)]
    pub provisioner: ProvisionerYaml,
    #[serde(default)]
    pub kind_image: Option<String>,
    #[serde(default)]
    pub k3s_version: Option<String>,
    #[serde(default)]
    pub kubeconfig: Option<PathBuf>,
    #[serde(default)]
    pub topology: ClusterTopology,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProvisionerYaml {
    #[default]
    Kind,
    K3s,
    External,
}

impl From<ProvisionerYaml> for ProvisionerKind {
    fn from(p: ProvisionerYaml) -> Self {
        match p {
            ProvisionerYaml::Kind => Self::Kind,
            ProvisionerYaml::K3s => Self::K3s,
            ProvisionerYaml::External => Self::External,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClusterTopology {
    #[serde(default = "default_controller_replicas")]
    pub controller_replicas: u32,
    #[serde(default = "default_participant_replicas")]
    pub participant_replicas: u32,
    #[serde(default)]
    pub namespace: Option<String>,
}

fn default_controller_replicas() -> u32 { 2 }
fn default_participant_replicas() -> u32 { 3 }

/// Reference to a chaos fault.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChaosFaultRef {
    pub id: String,
    /// humantime offset from scenario start (e.g. "3m").
    pub at_offset: String,
    /// Path relative to `deploy/chaos/` (e.g. `pr21_meta_swarm/participant-loss.yaml`).
    #[serde(default)]
    pub crd_file: Option<String>,
    /// Inline action (e.g. "delete-fault" — references a previously-applied
    /// fault by id).
    #[serde(default)]
    pub action: Option<ChaosFaultAction>,
    /// humantime or "permanent".
    #[serde(default)]
    pub duration: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ChaosFaultAction {
    DeleteFault,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResultsConfig {
    #[serde(default)]
    pub collect_prom_snapshot: bool,
    #[serde(default)]
    pub collect_chaos_events: bool,
    #[serde(default)]
    pub output_dir: Option<PathBuf>,
}

/// Per-metric tolerance overrides.  Missing keys fall back to the
/// defaults in [`crate::tolerance_defaults`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToleranceSpec {
    #[serde(default)]
    pub p50_ms_max_delta_pct: Option<f64>,
    #[serde(default)]
    pub p95_ms_max_delta_pct: Option<f64>,
    #[serde(default)]
    pub p99_ms_max_delta_pct: Option<f64>,
    #[serde(default)]
    pub error_rate_max_delta_pp: Option<f64>,
    #[serde(default)]
    pub throughput_qps_min_delta_pct: Option<f64>,
    #[serde(default)]
    pub recovery_time_s_max_delta_s: Option<f64>,
}

/// Stable identifiers for the closed metric set (STABILITY_CONTRACT v1.3 § 21.3).
///
/// **`Ord` is intentionally derived** : `compare()` stores rows in a
/// `BTreeMap<ToleranceMetric, ComparisonRow>` so PR comment output is
/// deterministic (variant declaration order = p50 → recovery_time).
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToleranceMetric {
    P50Ms,
    P95Ms,
    P99Ms,
    ErrorRate,
    ThroughputQps,
    RecoveryTimeS,
}

impl ToleranceMetric {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::P50Ms => "p50_ms",
            Self::P95Ms => "p95_ms",
            Self::P99Ms => "p99_ms",
            Self::ErrorRate => "error_rate",
            Self::ThroughputQps => "throughput_qps",
            Self::RecoveryTimeS => "recovery_time_s",
        }
    }

    /// Whether lower-is-better (latency, error_rate) or higher-is-better (throughput).
    #[must_use]
    pub const fn lower_is_better(self) -> bool {
        !matches!(self, Self::ThroughputQps)
    }

    /// All metrics, stable order.
    pub const fn all() -> [Self; 6] {
        [
            Self::P50Ms,
            Self::P95Ms,
            Self::P99Ms,
            Self::ErrorRate,
            Self::ThroughputQps,
            Self::RecoveryTimeS,
        ]
    }
}

/// Verbatim PR #20 loadgen profile.  We deliberately keep it as an
/// opaque map so loadgen schema bumps don't break the bench harness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioLoadgen {
    pub version: u32,
    pub name: String,
    pub target: serde_yaml::Value,
    pub pattern: serde_yaml::Value,
    #[serde(default)]
    pub workload: Option<serde_yaml::Value>,
    #[serde(default)]
    pub rate: Option<serde_yaml::Value>,
    #[serde(default)]
    pub termination: Option<serde_yaml::Value>,
    #[serde(default)]
    pub output: Option<serde_yaml::Value>,
    #[serde(default)]
    pub observability: Option<serde_yaml::Value>,
    #[serde(default)]
    pub run: Option<ScenarioLoadgenRun>,
    /// Any other fields (forward-compat) — captured via flattened map.
    #[serde(flatten, default)]
    pub extra: HashMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScenarioLoadgenRun {
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub allow_mutation: bool,
    #[serde(default)]
    pub chaos_marker: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_taxonomy_stable() {
        let m = ToleranceMetric::all();
        assert_eq!(m.len(), 6);
        assert_eq!(m[0].as_str(), "p50_ms");
        assert_eq!(m[5].as_str(), "recovery_time_s");
        assert!(m[3].lower_is_better());
        assert!(!ToleranceMetric::ThroughputQps.lower_is_better());
    }

    #[test]
    fn provisioner_yaml_to_cli() {
        assert_eq!(ProvisionerKind::from(ProvisionerYaml::Kind), ProvisionerKind::Kind);
        assert_eq!(ProvisionerKind::from(ProvisionerYaml::External), ProvisionerKind::External);
    }
}
