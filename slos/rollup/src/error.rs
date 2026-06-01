//! PR #22 § 14.1 + AD49 — error taxonomy + 7 exit codes.

use std::path::PathBuf;

use thiserror::Error;

/// Public result alias.
pub type SloResult<T> = Result<T, SloError>;

/// All errors emitted by the rollup binary + library.
#[derive(Debug, Error)]
pub enum SloError {
    /// Burn-rate threshold exceeded — distinct from a generic error
    /// because it maps to `ExitCode::BurnRateExceeded` (1), not the
    /// generic internal error.
    #[error("burn-rate exceeded for {slo} : {detail}")]
    BurnRate { slo: String, detail: String },

    /// Math kernel error (e.g. divide-by-zero, NaN, malformed PromQL
    /// response that can't be turned into numbers).  Maps to `ExitCode::MathError`.
    #[error("SLI math error : {0}")]
    Math(String),

    /// Configuration error (bad flag, missing required env, malformed
    /// scenario reference).  Maps to `ExitCode::Config`.
    #[error("config : {0}")]
    Config(String),

    /// User interrupt or signal.  Maps to `ExitCode::Interrupted`.
    #[error("interrupted")]
    Interrupted,

    /// I/O failure on a storage path (Parquet / JSON / PDF read or write).
    /// Maps to `ExitCode::StorageIO`.
    #[error("storage I/O at {path:?} : {source}")]
    StorageIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Source-provider failure (Prometheus HTTP, baseline read, etc.).
    /// Maps to `ExitCode::StorageIO` (we treat sources as I/O for exit
    /// code purposes — distinct error variant for log clarity).
    #[error("source provider : {0}")]
    Source(String),

    /// Cosign signature failure (sign or verify).  Maps to
    /// `ExitCode::EvidenceSign`.
    #[error("evidence signing : {0}")]
    EvidenceSign(String),

    /// Schema-version mismatch detected at file read.  Maps to
    /// `ExitCode::Config` (operator must point at the right version).
    #[error("schema version mismatch : expected {expected}, got {actual}")]
    SchemaVersion { expected: u32, actual: u32 },

    /// Catch-all internal failure.  Maps to `ExitCode::Internal`.
    #[error("internal : {0}")]
    Internal(String),

    /// Serialization passthrough.
    #[error(transparent)]
    Json(#[from] serde_json::Error),

    /// YAML passthrough (for recording-rule emit + alert YAML).
    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),
}

impl SloError {
    /// Map to the matching exit code (AD49).
    #[must_use]
    pub const fn exit_code(&self) -> ExitCode {
        match self {
            Self::BurnRate { .. } => ExitCode::BurnRateExceeded,
            Self::Math(_) => ExitCode::MathError,
            Self::Config(_) | Self::SchemaVersion { .. } => ExitCode::Config,
            Self::Interrupted => ExitCode::Interrupted,
            Self::StorageIo { .. } | Self::Source(_) | Self::Json(_) | Self::Yaml(_) => {
                ExitCode::StorageIo
            }
            Self::EvidenceSign(_) => ExitCode::EvidenceSign,
            Self::Internal(_) => ExitCode::Internal,
        }
    }
}

/// 7-level closed exit-code taxonomy (PR #22 AD49 — frozen).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ExitCode {
    /// Success.
    Success = 0,
    /// Burn-rate threshold exceeded.
    BurnRateExceeded = 1,
    /// Math kernel failure.
    MathError = 2,
    /// Configuration / flag / env / schema-version error.
    Config = 3,
    /// SIGINT / SIGTERM.
    Interrupted = 4,
    /// Storage / source I/O failure.
    StorageIo = 5,
    /// Cosign sign or verify failure.
    EvidenceSign = 6,
    /// Catch-all.
    Internal = 7,
}

impl ExitCode {
    #[must_use]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

impl From<std::io::Error> for SloError {
    fn from(source: std::io::Error) -> Self {
        Self::StorageIo {
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
        assert_eq!(ExitCode::Success.as_i32(), 0);
        assert_eq!(ExitCode::BurnRateExceeded.as_i32(), 1);
        assert_eq!(ExitCode::MathError.as_i32(), 2);
        assert_eq!(ExitCode::Config.as_i32(), 3);
        assert_eq!(ExitCode::Interrupted.as_i32(), 4);
        assert_eq!(ExitCode::StorageIo.as_i32(), 5);
        assert_eq!(ExitCode::EvidenceSign.as_i32(), 6);
        assert_eq!(ExitCode::Internal.as_i32(), 7);
    }

    #[test]
    fn burn_rate_maps_to_code_1() {
        let e = SloError::BurnRate { slo: "L2_AVAILABILITY".into(), detail: "x".into() };
        assert_eq!(e.exit_code(), ExitCode::BurnRateExceeded);
    }

    #[test]
    fn math_maps_to_code_2() {
        assert_eq!(SloError::Math("/0".into()).exit_code(), ExitCode::MathError);
    }

    #[test]
    fn schema_version_maps_to_config() {
        let e = SloError::SchemaVersion { expected: 1, actual: 2 };
        assert_eq!(e.exit_code(), ExitCode::Config);
    }
}
