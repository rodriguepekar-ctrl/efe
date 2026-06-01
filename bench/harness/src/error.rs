//! PR #21 § 24 + AD11 — error + exit-code taxonomy.
//!
//! Closed taxonomy committed in STABILITY_CONTRACT v1.3 :
//!
//! ```text
//! 0 → success — all scenarios pass
//! 1 → bench fail — at least one scenario verdict = fail
//! 2 → cluster error — provisioner / kubectl failure
//! 3 → config error — scenario YAML invalid or production-mode refused
//! 4 → signal interrupted (SIGINT/SIGTERM)
//! 5 → internal error (panic, OOM)
//! ```

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ExitCode {
    Success      = 0,
    BenchFail    = 1,
    Cluster      = 2,
    Config       = 3,
    Interrupted  = 4,
    Internal     = 5,
}

impl ExitCode {
    #[inline]
    #[must_use]
    pub const fn as_i32(self) -> i32 { self as i32 }
}

#[derive(Debug, Error)]
pub enum BenchError {
    // ── CONFIG (exit 3) ──
    #[error("invalid scenario : {0}")]
    InvalidScenario(String),

    #[error("scenario YAML at {path} : {source}")]
    ScenarioLoad {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },

    #[error("scenario schema version {actual} not supported (expected {expected})")]
    SchemaVersion { expected: u32, actual: u32 },

    #[error("scenario {id} references chaos faults but chaos = []")]
    ChaosConfigMismatch { id: String },

    #[error("loadgen profile run.mode=production REFUSED by bench harness (AD11 — T86)")]
    ProductionRefused,

    #[error("scenario {id} not found in {dir}")]
    ScenarioNotFound { id: String, dir: PathBuf },

    // ── CLUSTER (exit 2) ──
    #[error("cluster provisioner '{provisioner}' failed : {reason}")]
    Cluster { provisioner: String, reason: String },

    #[error("kubectl '{command}' failed : {stderr}")]
    Kubectl { command: String, stderr: String },

    #[error("chaos-mesh CRD apply failed : {0}")]
    ChaosApply(String),

    // ── BENCH VERDICT (exit 1) ──
    #[error("bench verdict : {failed_count} scenario(s) failed")]
    BenchVerdict { failed_count: usize },

    // ── SIGNAL (exit 4) ──
    #[error("signal-interrupted")]
    Interrupted,

    // ── INTERNAL (exit 5) ──
    #[error("baseline at {path} : {source}")]
    Baseline {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("result writer at {path} : {source}")]
    Result {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("loadgen integration : {0}")]
    Loadgen(String),

    #[error("internal : {0}")]
    Internal(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
}

/// PR #23 § 10 — adapter from the shared cluster-provisioner crate's
/// error type into `BenchError`.  Maintains AC6 byte-identical exit
/// codes for PR #21 callers.
impl From<efe_cluster_provisioner::ProvisionError> for BenchError {
    fn from(e: efe_cluster_provisioner::ProvisionError) -> Self {
        match e {
            efe_cluster_provisioner::ProvisionError::Cluster { provisioner, reason } => {
                Self::Cluster { provisioner, reason }
            }
            efe_cluster_provisioner::ProvisionError::Kubectl { command, stderr } => {
                Self::Kubectl { command, stderr }
            }
            efe_cluster_provisioner::ProvisionError::Io(io) => Self::Io(io),
        }
    }
}

impl BenchError {
    #[must_use]
    pub const fn exit_code(&self) -> ExitCode {
        match self {
            Self::InvalidScenario(_)
            | Self::ScenarioLoad { .. }
            | Self::SchemaVersion { .. }
            | Self::ChaosConfigMismatch { .. }
            | Self::ProductionRefused
            | Self::ScenarioNotFound { .. } => ExitCode::Config,
            Self::Cluster { .. } | Self::Kubectl { .. } | Self::ChaosApply(_) => ExitCode::Cluster,
            Self::BenchVerdict { .. } => ExitCode::BenchFail,
            Self::Interrupted => ExitCode::Interrupted,
            Self::Baseline { .. }
            | Self::Result { .. }
            | Self::Loadgen(_)
            | Self::Internal(_)
            | Self::Io(_)
            | Self::Json(_)
            | Self::Anyhow(_) => ExitCode::Internal,
        }
    }
}

pub type BenchResult<T> = Result<T, BenchError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_stable() {
        // STABILITY_CONTRACT v1.3 § 22 commits these values.
        assert_eq!(ExitCode::Success.as_i32(), 0);
        assert_eq!(ExitCode::BenchFail.as_i32(), 1);
        assert_eq!(ExitCode::Cluster.as_i32(), 2);
        assert_eq!(ExitCode::Config.as_i32(), 3);
        assert_eq!(ExitCode::Interrupted.as_i32(), 4);
        assert_eq!(ExitCode::Internal.as_i32(), 5);
    }

    #[test]
    fn production_refused_maps_to_config() {
        assert_eq!(BenchError::ProductionRefused.exit_code(), ExitCode::Config);
    }

    #[test]
    fn bench_verdict_maps_to_exit_1() {
        let e = BenchError::BenchVerdict { failed_count: 3 };
        assert_eq!(e.exit_code(), ExitCode::BenchFail);
    }
}
