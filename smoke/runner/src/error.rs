//! PR #23 § 17 + AD41 — error + 6 ExitCodes (frozen STABILITY v1.5 § 23.7).

use std::path::PathBuf;
use thiserror::Error;

pub type SmokeResult<T> = Result<T, SmokeError>;

/// 6-level closed exit-code taxonomy (STABILITY_CONTRACT v1.5 § 23.7 — frozen).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ExitCode {
    /// Success.
    Success = 0,
    /// At least one of S1-S9 failed.
    SmokeFail = 1,
    /// Cluster provisioner failure.
    Cluster = 2,
    /// Configuration / flag / env error.
    Config = 3,
    /// SIGINT / SIGTERM.
    Interrupted = 4,
    /// Catch-all.
    Internal = 5,
}

impl ExitCode {
    #[must_use]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

#[derive(Debug, Error)]
pub enum SmokeError {
    /// Cluster provisioner failure (extracted crate).
    #[error(transparent)]
    Provisioner(#[from] efe_cluster_provisioner::ProvisionError),

    /// One of S1-S9 failed (AD31 — exit code 1).
    #[error("smoke check {check_id} failed : {detail}")]
    Check { check_id: String, detail: String },

    /// One or more checks failed (final aggregate).
    #[error("{failed_count} smoke check(s) failed")]
    SmokeVerdict { failed_count: usize },

    /// Configuration / flag error.
    #[error("config : {0}")]
    Config(String),

    /// Production cluster target rejected (AD9 — T108).
    #[error("smoke target is production cluster — REFUSED (AD9 / T108)")]
    ProductionRefused,

    /// Wall-clock budget exceeded (AD13).
    #[error("check {check_id} exceeded budget {budget_s}s (took {took_s}s)")]
    BudgetExceeded {
        check_id: String,
        budget_s: u64,
        took_s: u64,
    },

    /// Signal-interrupted.
    #[error("interrupted")]
    Interrupted,

    /// I/O failure.
    #[error("io at {path:?} : {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Source failure (Prom / OTel collector / loadgen).
    #[error("source : {0}")]
    Source(String),

    /// Catch-all.
    #[error("internal : {0}")]
    Internal(String),

    /// Serialisation passthrough.
    #[error(transparent)]
    Json(#[from] serde_json::Error),

    /// YAML passthrough.
    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),
}

impl SmokeError {
    #[must_use]
    pub const fn exit_code(&self) -> ExitCode {
        match self {
            Self::Provisioner(_) => ExitCode::Cluster,
            Self::Check { .. }
            | Self::SmokeVerdict { .. }
            | Self::BudgetExceeded { .. } => ExitCode::SmokeFail,
            Self::Config(_) | Self::ProductionRefused => ExitCode::Config,
            Self::Interrupted => ExitCode::Interrupted,
            Self::Io { .. } | Self::Source(_) | Self::Json(_) | Self::Yaml(_) => {
                ExitCode::Internal
            }
            Self::Internal(_) => ExitCode::Internal,
        }
    }
}

impl From<std::io::Error> for SmokeError {
    fn from(source: std::io::Error) -> Self {
        Self::Io {
            path: PathBuf::from("(unknown)"),
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_frozen() {
        // STABILITY_CONTRACT v1.5 § 23.7.
        assert_eq!(ExitCode::Success.as_i32(), 0);
        assert_eq!(ExitCode::SmokeFail.as_i32(), 1);
        assert_eq!(ExitCode::Cluster.as_i32(), 2);
        assert_eq!(ExitCode::Config.as_i32(), 3);
        assert_eq!(ExitCode::Interrupted.as_i32(), 4);
        assert_eq!(ExitCode::Internal.as_i32(), 5);
    }

    #[test]
    fn provisioner_maps_to_cluster_exit() {
        let e: SmokeError = efe_cluster_provisioner::ProvisionError::Cluster {
            provisioner: "kind".into(),
            reason: "boom".into(),
        }
        .into();
        assert_eq!(e.exit_code(), ExitCode::Cluster);
    }

    #[test]
    fn smoke_verdict_maps_to_smoke_fail() {
        let e = SmokeError::SmokeVerdict { failed_count: 1 };
        assert_eq!(e.exit_code(), ExitCode::SmokeFail);
    }

    #[test]
    fn production_refused_maps_to_config() {
        assert_eq!(SmokeError::ProductionRefused.exit_code(), ExitCode::Config);
    }

    #[test]
    fn budget_exceeded_maps_to_smoke_fail() {
        let e = SmokeError::BudgetExceeded {
            check_id: "S1".into(),
            budget_s: 30,
            took_s: 45,
        };
        assert_eq!(e.exit_code(), ExitCode::SmokeFail);
    }
}
