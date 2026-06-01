//! Sprint D17 — error type for backup/restore orchestration.
//!
//! `BackupError` covers the full set of failure modes documented
//! in `docs/bench/d17_backup_dr_architecture.md` § 7.2. It is
//! `non_exhaustive` so future sprints can add variants without
//! breaking external matchers.

use thiserror::Error;

/// Top-level error for backup + restore operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum BackupError {
    /// `pgBackRest` returned a non-zero exit code, or its output
    /// could not be parsed as the expected JSON.
    #[error("pgBackRest failed: {0}")]
    PgBackRest(String),

    /// Vault snapshot via `vault operator raft snapshot save` failed
    /// or the subsequent S3 upload was rejected.
    #[error("Vault snapshot failed: {0}")]
    Vault(String),

    /// `rclone sync source dest` failed (source unreachable, dest
    /// permission denied, mid-flight integrity check mismatch).
    #[error("rclone sync failed: {0}")]
    Rclone(String),

    /// Manifest signature creation failed — typically a missing or
    /// unparseable Ed25519 signing key.
    #[error("manifest signing failed: {0}")]
    SignManifest(String),

    /// Manifest signature verification failed — typically a
    /// detached signature pointing to a key not in the trust
    /// store, or a tampered manifest body.
    #[error("manifest verification failed: {0}")]
    VerifyManifest(String),

    /// Failure to emit a backup-related event into the D14-bis
    /// audit chain.
    #[error("audit log emission failed: {0}")]
    AuditLog(String),

    /// I/O error during local manifest read/write or temp-file
    /// management.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialisation / deserialisation failure on a manifest
    /// or `efe_backup_history` row.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// One of the post-restore [`crate::Verifier`] checks reported
    /// a fatal anomaly (HMAC chain broken, bundle signature
    /// invalid, OIDC issuer unreachable).
    #[error("post-restore verification failed: {0}")]
    Verify(String),

    /// The 4-eyes flow (CLI `delete` subcommand) was attempted with
    /// only one approver token, two tokens with the same `sub`
    /// claim, or insufficient permission scopes.
    #[error("4-eyes authorization required: {0}")]
    FourEyesRequired(String),

    /// A restore was requested into an environment whose label does
    /// not match the manifest's `env` field (e.g. restoring a prod
    /// backup into staging without `--force-cross-env`).
    /// Mitigation T-DR-6.
    #[error("cross-environment restore refused: {0}")]
    CrossEnvRefused(String),

    /// The most recent successful backup is older than the RPO
    /// target ; the operator should investigate before relying on
    /// this backup for restore.
    #[error("RPO violation: last successful backup {age_secs}s ago, RPO target {target_secs}s")]
    RpoViolation {
        /// Age in seconds of the most recent successful backup.
        age_secs: u64,
        /// RPO target in seconds (typically `cfg.rpo_target_seconds`).
        target_secs: u64,
    },

    /// The DR drill completed but exceeded the configured RTO
    /// target. Returned by [`crate::DrDrill::run`].
    #[error("RTO violation: drill took {actual_secs}s, RTO target {target_secs}s")]
    RtoViolation {
        /// Actual end-to-end drill duration in seconds.
        actual_secs: u64,
        /// RTO target in seconds (typically `cfg.rto_target_seconds`).
        target_secs: u64,
    },

    /// The manifest's `version` field is set to a value this
    /// release doesn't know how to read. Pinned by architecture
    /// § 27. Strict reader : refuse unknown.
    #[error("unknown manifest version: {got:?} (this release supports {supported:?})")]
    UnknownManifestVersion {
        /// Version literal read from the manifest JSON.
        got: String,
        /// Version this binary supports (`crate::MANIFEST_VERSION`).
        supported: &'static str,
    },

    /// An invariant on the manifest payload itself failed (env
    /// mismatch with header, components list empty, etc.).
    #[error("manifest invariant violated: {0}")]
    ManifestInvariant(String),

    /// A snapshot or restore operation timed out (typically vs the
    /// configured `flush_timeout` or `restore_timeout`).
    #[error("operation timed out: {op} after {elapsed_secs}s")]
    Timeout {
        /// Operation symbolic name (`snapshot`, `restore`, …).
        op: &'static str,
        /// Wall-clock seconds before the timeout fired.
        elapsed_secs: u64,
    },

    /// Generic configuration error (env var missing, value out of
    /// range, etc.).
    #[error("configuration error: {0}")]
    Config(String),

    /// A D14-bis-style chain anomaly detected during
    /// post-restore verification.
    #[error("audit chain integrity check failed: {0}")]
    AuditChainBroken(String),
}

impl BackupError {
    /// Convenience builder for the `Config` variant, used heavily
    /// in `BackupConfig::from_env` for shape-mismatch errors.
    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }

    /// Returns the canonical kebab-case label used by the
    /// `efe_backup_runs_total{result="..."}` metric.
    #[must_use]
    pub fn metric_label(&self) -> &'static str {
        match self {
            Self::PgBackRest(_) => "pgbackrest_failed",
            Self::Vault(_) => "vault_failed",
            Self::Rclone(_) => "rclone_failed",
            Self::SignManifest(_) => "sign_failed",
            Self::VerifyManifest(_) => "verify_manifest_failed",
            Self::AuditLog(_) => "audit_failed",
            Self::Io(_) => "io_error",
            Self::Json(_) => "json_error",
            Self::Verify(_) => "post_restore_failed",
            Self::FourEyesRequired(_) => "four_eyes_required",
            Self::CrossEnvRefused(_) => "cross_env_refused",
            Self::RpoViolation { .. } => "rpo_violation",
            Self::RtoViolation { .. } => "rto_violation",
            Self::UnknownManifestVersion { .. } => "unknown_manifest_version",
            Self::ManifestInvariant(_) => "manifest_invariant",
            Self::Timeout { .. } => "timeout",
            Self::Config(_) => "config_error",
            Self::AuditChainBroken(_) => "audit_chain_broken",
        }
    }

    /// Whether this error is **transient** (worth retrying after
    /// a short backoff) or **fatal** (requires operator action).
    /// Used by the CronJob retry-with-backoff policy.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::PgBackRest(_)
                | Self::Vault(_)
                | Self::Rclone(_)
                | Self::Timeout { .. }
                | Self::Io(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_label_covers_all_variants() {
        // Pin the shape of metric_label by sampling each variant.
        // A new variant added without metric_label coverage triggers
        // an exhaustive-match failure on this list — manual-but-cheap
        // gate.
        let cases: Vec<(BackupError, &'static str)> = vec![
            (BackupError::PgBackRest("x".into()), "pgbackrest_failed"),
            (BackupError::Vault("x".into()), "vault_failed"),
            (BackupError::Rclone("x".into()), "rclone_failed"),
            (BackupError::SignManifest("x".into()), "sign_failed"),
            (BackupError::VerifyManifest("x".into()), "verify_manifest_failed"),
            (BackupError::AuditLog("x".into()), "audit_failed"),
            (BackupError::Verify("x".into()), "post_restore_failed"),
            (BackupError::FourEyesRequired("x".into()), "four_eyes_required"),
            (BackupError::CrossEnvRefused("x".into()), "cross_env_refused"),
            (
                BackupError::RpoViolation { age_secs: 1, target_secs: 1 },
                "rpo_violation",
            ),
            (
                BackupError::RtoViolation { actual_secs: 1, target_secs: 1 },
                "rto_violation",
            ),
            (
                BackupError::UnknownManifestVersion {
                    got: "x".into(),
                    supported: "1.0.0",
                },
                "unknown_manifest_version",
            ),
            (BackupError::ManifestInvariant("x".into()), "manifest_invariant"),
            (
                BackupError::Timeout { op: "snapshot", elapsed_secs: 1 },
                "timeout",
            ),
            (BackupError::Config("x".into()), "config_error"),
            (BackupError::AuditChainBroken("x".into()), "audit_chain_broken"),
        ];
        for (err, expected) in cases {
            assert_eq!(err.metric_label(), expected);
        }
    }

    #[test]
    fn is_transient_classification() {
        assert!(BackupError::PgBackRest("x".into()).is_transient());
        assert!(BackupError::Timeout { op: "x", elapsed_secs: 1 }.is_transient());
        assert!(!BackupError::Config("x".into()).is_transient());
        assert!(!BackupError::FourEyesRequired("x".into()).is_transient());
        assert!(!BackupError::CrossEnvRefused("x".into()).is_transient());
    }

    #[test]
    fn config_helper_builds_variant() {
        let e = BackupError::config("bad value");
        assert!(matches!(e, BackupError::Config(_)));
        assert_eq!(e.to_string(), "configuration error: bad value");
    }
}
