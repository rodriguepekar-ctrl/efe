//! PR #20 § 6 — error type for the load generator.
//!
//! A single [`LoadgenError`] enum captures every fatal failure mode.
//! The CLI converts variants to exit codes per architecture § AD17 :
//!
//! ```text
//! 0 → success
//! 1 → SLO breach
//! 2 → transport error (controller unreachable, mTLS handshake)
//! 3 → invalid config (CLI conflict, profile YAML malformed)
//! 4 → signal-interrupted
//! 5 → internal error (panic, OOM, unforeseen)
//! ```
//!
//! Non-fatal errors (per-request RPC failures, partial result writes)
//! flow through the request pipeline as `result=error` rows in the
//! output file ; only run-aborting failures bubble up as
//! [`LoadgenError`].

use std::path::PathBuf;
use thiserror::Error;

/// Exit code returned by the binary.  Stable surface — CI scripts and
/// PagerDuty integrations rely on these values.
///
/// See architecture § AD17 + § 32 AC16.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ExitCode {
    Success         = 0,
    SloBreach       = 1,
    Transport       = 2,
    Config          = 3,
    Interrupted     = 4,
    Internal        = 5,
}

impl ExitCode {
    /// Convert to the raw `i32` accepted by [`std::process::exit`].
    #[inline]
    #[must_use]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

/// Top-level error type for the load generator.
#[derive(Debug, Error)]
pub enum LoadgenError {
    // ── CONFIG (exit 3) ────────────────────────────────────────────
    #[error("invalid configuration : {0}")]
    InvalidConfig(String),

    #[error("conflicting CLI flags : {0}")]
    FlagConflict(String),

    #[error("profile YAML at {path} : {source}")]
    ProfileLoad {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },

    #[error("profile YAML at {path} : version {actual} not supported (expected {expected})")]
    ProfileVersion { path: PathBuf, expected: u32, actual: u32 },

    #[error("production-mode invocation without --confirm-prod")]
    ProductionConfirmationMissing,

    #[error("requested QPS {requested} exceeds hard cap {cap} (use --i-am-sure to bypass up to 100000)")]
    QpsCapExceeded { requested: f64, cap: f64 },

    // ── TRANSPORT (exit 2) ─────────────────────────────────────────
    #[error("failed to connect to target {target} : {source}")]
    Connect {
        target: String,
        #[source]
        source: tonic::transport::Error,
    },

    #[error("TLS bootstrap : {0}")]
    TlsBootstrap(String),

    #[error("mTLS cert file at {path} : {source}")]
    TlsFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    // ── SIGNAL (exit 4) ────────────────────────────────────────────
    #[error("signal-interrupted (SIGINT/SIGTERM)")]
    Interrupted,

    // ── INTERNAL (exit 5) ──────────────────────────────────────────
    #[error("output writer at {path} : {source}")]
    Output {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("internal : {0}")]
    Internal(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
}

impl LoadgenError {
    /// Map this error to its [`ExitCode`].
    #[must_use]
    pub const fn exit_code(&self) -> ExitCode {
        match self {
            Self::InvalidConfig(_)
            | Self::FlagConflict(_)
            | Self::ProfileLoad { .. }
            | Self::ProfileVersion { .. }
            | Self::ProductionConfirmationMissing
            | Self::QpsCapExceeded { .. } => ExitCode::Config,
            Self::Connect { .. } | Self::TlsBootstrap(_) | Self::TlsFile { .. } => {
                ExitCode::Transport
            }
            Self::Interrupted => ExitCode::Interrupted,
            Self::Output { .. } | Self::Internal(_) | Self::Io(_) | Self::Anyhow(_) => {
                ExitCode::Internal
            }
        }
    }
}

/// Convenience alias for the loadgen result type.
pub type LoadgenResult<T> = Result<T, LoadgenError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_match_architecture_ad17() {
        // AD17 — stable surface for CI integration.
        assert_eq!(ExitCode::Success.as_i32(), 0);
        assert_eq!(ExitCode::SloBreach.as_i32(), 1);
        assert_eq!(ExitCode::Transport.as_i32(), 2);
        assert_eq!(ExitCode::Config.as_i32(), 3);
        assert_eq!(ExitCode::Interrupted.as_i32(), 4);
        assert_eq!(ExitCode::Internal.as_i32(), 5);
    }

    #[test]
    fn config_errors_map_to_exit_3() {
        let e = LoadgenError::InvalidConfig("foo".into());
        assert_eq!(e.exit_code(), ExitCode::Config);
        let e = LoadgenError::QpsCapExceeded { requested: 1e9, cap: 100_000.0 };
        assert_eq!(e.exit_code(), ExitCode::Config);
        let e = LoadgenError::ProductionConfirmationMissing;
        assert_eq!(e.exit_code(), ExitCode::Config);
    }

    #[test]
    fn transport_errors_map_to_exit_2() {
        let e = LoadgenError::TlsBootstrap("bad ca bundle".into());
        assert_eq!(e.exit_code(), ExitCode::Transport);
    }

    #[test]
    fn interrupt_maps_to_exit_4() {
        let e = LoadgenError::Interrupted;
        assert_eq!(e.exit_code(), ExitCode::Interrupted);
    }
}
