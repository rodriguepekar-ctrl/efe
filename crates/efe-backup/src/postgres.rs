//! Sprint D17 — `Snapshotter` impl shelling out to `pgBackRest`.
//!
//! Cf. architecture § 5.1 + § 6.1.
//!
//! This module wraps `pgbackrest backup --type=… --label=…`,
//! parses the JSON `info` output to extract LSN start/stop +
//! repo path, then signs the manifest via [`crate::manifest`].

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;

use crate::command_runner::CommandRunner;
use crate::error::BackupError;
use crate::metadata::{BackupKind, SnapshotMetadata, SnapshotSource};
use crate::snapshot::Snapshotter;

/// `pgBackRest`-backed snapshotter. Holds a [`CommandRunner`]
/// (`Arc<TokioCommandRunner>` in production, `Arc<MockCommandRunner>`
/// in tests).
#[derive(Debug)]
pub struct PgBackRestSnapshotter {
    runner: Arc<dyn CommandRunner>,
    /// pgBackRest stanza name (typically `"main"`).
    pub stanza: String,
    /// Path to `pgbackrest.conf`.
    pub config_path: String,
    /// Number of parallel I/O workers (`--process-max`).
    pub process_max: u32,
    /// Symbolic env tag to embed in the manifest.
    pub env: String,
    /// Cluster identifier embedded in the manifest.
    pub cluster_id: String,
}

impl PgBackRestSnapshotter {
    /// Construct with sensible defaults.
    pub fn new(
        runner: Arc<dyn CommandRunner>,
        stanza: impl Into<String>,
        config_path: impl Into<String>,
        env: impl Into<String>,
        cluster_id: impl Into<String>,
    ) -> Self {
        Self {
            runner,
            stanza: stanza.into(),
            config_path: config_path.into(),
            process_max: 4,
            env: env.into(),
            cluster_id: cluster_id.into(),
        }
    }

    /// Resolve the canonical pgbackrest CLI label from a [`BackupKind`].
    fn type_arg(kind: BackupKind) -> Result<&'static str, BackupError> {
        match kind {
            BackupKind::Full => Ok("full"),
            BackupKind::Diff => Ok("diff"),
            BackupKind::Incr => Ok("incr"),
            other => Err(BackupError::config(format!(
                "PgBackRestSnapshotter cannot run kind={:?} (only full/diff/incr)",
                other
            ))),
        }
    }
}

#[async_trait]
impl Snapshotter for PgBackRestSnapshotter {
    fn source(&self) -> SnapshotSource {
        SnapshotSource::Postgres
    }

    async fn snapshot(
        &self,
        label: &str,
        kind: BackupKind,
    ) -> Result<SnapshotMetadata, BackupError> {
        let started_at = Utc::now();
        let type_str = Self::type_arg(kind)?;
        let process_max = self.process_max.to_string();
        let args = vec![
            "--config",
            &self.config_path,
            "--stanza",
            &self.stanza,
            "--type",
            type_str,
            "--label",
            label,
            "--process-max",
            &process_max,
            "--output=json",
            "backup",
        ];

        // **Audit fix § D17-AH5** — pass the pgBackRest cipher
        // passphrase through the env. Pre-audit, the passphrase
        // env var was set on the calling pod (Helm Secret mount)
        // but the spawned child process didn't inherit it because
        // we passed `&[]` for env. Encrypted backups would fail
        // to write OR (worse) decrypt at restore.
        let env_owned = build_pgbackrest_env();
        let env: Vec<(&str, &str)> = env_owned
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        let out = self
            .runner
            .run("pgbackrest", &args, &env)
            .await
            .map_err(|e| BackupError::PgBackRest(format!("spawn failed: {e}")))?;
        if !out.success() {
            return Err(BackupError::PgBackRest(format!(
                "exit {} : {}",
                out.status,
                out.stderr_string()
            )));
        }

        let info = parse_backup_output(&out.stdout)?;

        let finished_at = Utc::now();
        Ok(SnapshotMetadata {
            label: label.to_string(),
            source: SnapshotSource::Postgres,
            kind,
            repo_path: info.repo_path,
            size_bytes: info.size_bytes,
            files_count: info.files_count,
            // The manifest sha + signature are computed by the
            // orchestrator after this call returns ; this struct
            // carries placeholders that the orchestrator overwrites.
            sha256_manifest: "0".repeat(64),
            ed25519_sig: String::new(),
            started_at,
            finished_at,
            pg_lsn_start: Some(info.lsn_start),
            pg_lsn_stop: Some(info.lsn_stop),
            ok: true,
            error_message: None,
        })
    }

    async fn verify(&self, _label: &str) -> Result<(), BackupError> {
        let args = vec![
            "--config",
            &self.config_path,
            "--stanza",
            &self.stanza,
            "check",
        ];
        let env_owned = build_pgbackrest_env();
        let env: Vec<(&str, &str)> = env_owned
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let out = self
            .runner
            .run("pgbackrest", &args, &env)
            .await
            .map_err(|e| BackupError::PgBackRest(format!("spawn failed: {e}")))?;
        if !out.success() {
            return Err(BackupError::PgBackRest(format!(
                "verify failed exit {} : {}",
                out.status,
                out.stderr_string()
            )));
        }
        Ok(())
    }

    async fn restore(
        &self,
        label: &str,
        _target_env: &str,
    ) -> Result<(), BackupError> {
        self.restore_at(label, None).await
    }
}

impl PgBackRestSnapshotter {
    /// **Audit fix § D17-AH4** — extended restore that honors a
    /// PITR target time. Pre-audit, [`Snapshotter::restore`]
    /// silently dropped the `target_time` argument and used
    /// `--type=immediate`, breaking the documented PITR feature.
    /// Post-audit, callers that want PITR call this method with
    /// `target_time = Some("2026-04-28 14:30:00 UTC")` ; the
    /// canonical [`Snapshotter::restore`] still uses immediate
    /// mode for backward compatibility.
    pub async fn restore_at(
        &self,
        label: &str,
        target_time: Option<&str>,
    ) -> Result<(), BackupError> {
        let mut args: Vec<&str> = vec![
            "--config",
            &self.config_path,
            "--stanza",
            &self.stanza,
            "--set",
            label,
        ];
        let target_time_arg;
        if let Some(t) = target_time {
            args.push("--type=time");
            target_time_arg = format!("--target={}", t);
            args.push(&target_time_arg);
        } else {
            args.push("--type=immediate");
        }
        args.push("restore");
        let env_owned = build_pgbackrest_env();
        let env: Vec<(&str, &str)> = env_owned
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let out = self
            .runner
            .run("pgbackrest", &args, &env)
            .await
            .map_err(|e| BackupError::PgBackRest(format!("spawn failed: {e}")))?;
        if !out.success() {
            return Err(BackupError::PgBackRest(format!(
                "restore failed exit {} : {}",
                out.status,
                out.stderr_string()
            )));
        }
        Ok(())
    }
}

/// **Audit fix § D17-AH5** — collect pgBackRest-related env vars
/// from the parent process and forward them to the spawned child.
/// pgBackRest reads `PGBACKREST_REPO1_CIPHER_PASS`,
/// `PGBACKREST_REPO1_S3_KEY*`, `AWS_ACCESS_KEY_ID`,
/// `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`. Tokio's
/// `Command::env_clear` semantics + `MockCommandRunner` test path
/// require us to pass these explicitly rather than rely on
/// `Command::env_remove` defaults.
fn build_pgbackrest_env() -> Vec<(String, String)> {
    let candidates = [
        "PGBACKREST_REPO1_CIPHER_PASS",
        "PGBACKREST_REPO1_S3_KEY",
        "PGBACKREST_REPO1_S3_KEY_SECRET",
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_SESSION_TOKEN",
        "AWS_REGION",
    ];
    let mut out = Vec::new();
    for k in candidates {
        if let Ok(v) = std::env::var(k) {
            if !v.is_empty() {
                out.push((k.to_string(), v));
            }
        }
    }
    out
}

/// Parsed shape of `pgbackrest --output=json backup`.
#[derive(Debug)]
struct BackupOutput {
    repo_path: String,
    size_bytes: u64,
    files_count: u32,
    lsn_start: String,
    lsn_stop: String,
}

#[derive(Debug, Deserialize)]
struct PgBackRestJson {
    #[serde(default)]
    repo_path: Option<String>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    files: Option<u32>,
    #[serde(default)]
    lsn_start: Option<String>,
    #[serde(default)]
    lsn_stop: Option<String>,
    /// Real pgBackRest output is an array ; we tolerate both an
    /// object and an array (taking the last element).
    #[serde(default)]
    backup: Vec<PgBackRestJson>,
}

fn parse_backup_output(stdout: &[u8]) -> Result<BackupOutput, BackupError> {
    let parsed: serde_json::Value =
        serde_json::from_slice(stdout).map_err(|e| {
            BackupError::PgBackRest(format!("output is not JSON: {e}"))
        })?;

    // pgBackRest's real output format varies between versions ;
    // the common subset we need is :
    //   { "repo_path": "...", "size": 12345, "files": 7,
    //     "lsn_start": "0/1A000000", "lsn_stop": "0/1A111111" }
    // Some versions wrap it under `[{ "backup": [{...}] }]`.
    // Try direct object first ; fall back to array-of-objects.
    let pj: PgBackRestJson = match serde_json::from_value::<PgBackRestJson>(parsed.clone()) {
        Ok(v) => v,
        Err(_) => {
            let arr: Vec<PgBackRestJson> =
                serde_json::from_value(parsed).map_err(|e| {
                    BackupError::PgBackRest(format!("unexpected JSON shape: {e}"))
                })?;
            arr.into_iter().last().ok_or_else(|| {
                BackupError::PgBackRest("empty pgbackrest JSON output".into())
            })?
        }
    };

    // Look in `backup[]` first if present (pgBackRest >= 2.40).
    let chosen = if let Some(last) = pj.backup.into_iter().last() {
        last
    } else {
        PgBackRestJson {
            repo_path: pj.repo_path,
            size: pj.size,
            files: pj.files,
            lsn_start: pj.lsn_start,
            lsn_stop: pj.lsn_stop,
            backup: vec![],
        }
    };

    Ok(BackupOutput {
        repo_path: chosen.repo_path.unwrap_or_else(|| "(unknown)".into()),
        size_bytes: chosen.size.unwrap_or(0),
        files_count: chosen.files.unwrap_or(0),
        lsn_start: chosen.lsn_start.unwrap_or_else(|| "0/0".into()),
        lsn_stop: chosen.lsn_stop.unwrap_or_else(|| "0/0".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_runner::MockCommandRunner;

    fn mock_pgbackrest_output() -> &'static str {
        r#"{
          "repo_path": "/main/full/20260428T060000Z",
          "size": 12345678,
          "files": 42,
          "lsn_start": "0/1A000028",
          "lsn_stop": "0/1A111111"
        }"#
    }

    #[tokio::test]
    async fn snapshot_full_happy_path_returns_metadata() {
        let runner = Arc::new(
            MockCommandRunner::new()
                .expect_ok("pgbackrest --config", mock_pgbackrest_output()),
        );
        let s = PgBackRestSnapshotter::new(
            runner,
            "main",
            "/etc/pgbackrest/pgbackrest.conf",
            "prod",
            "efe-prod-eu-west-1",
        );
        let meta = s
            .snapshot("20260428T060000Z-full", BackupKind::Full)
            .await
            .unwrap();
        assert_eq!(meta.source, SnapshotSource::Postgres);
        assert_eq!(meta.kind, BackupKind::Full);
        assert_eq!(meta.label, "20260428T060000Z-full");
        assert_eq!(meta.size_bytes, 12345678);
        assert_eq!(meta.files_count, 42);
        assert_eq!(meta.pg_lsn_start.as_deref(), Some("0/1A000028"));
        assert_eq!(meta.pg_lsn_stop.as_deref(), Some("0/1A111111"));
        assert!(meta.ok);
    }

    #[tokio::test]
    async fn snapshot_propagates_pgbackrest_failure() {
        let runner = Arc::new(
            MockCommandRunner::new().expect_err("pgbackrest", 99, "stanza missing"),
        );
        let s = PgBackRestSnapshotter::new(
            runner,
            "main",
            "/etc/pgbackrest/pgbackrest.conf",
            "prod",
            "efe-prod",
        );
        let err = s
            .snapshot("x", BackupKind::Full)
            .await
            .unwrap_err();
        assert!(matches!(err, BackupError::PgBackRest(_)));
        assert!(err.to_string().contains("stanza missing"));
    }

    #[tokio::test]
    async fn snapshot_rejects_unsupported_kind() {
        let runner = Arc::new(MockCommandRunner::new().expect_ok("pgbackrest", "{}"));
        let s = PgBackRestSnapshotter::new(runner, "main", "/x", "prod", "c");
        let err = s.snapshot("x", BackupKind::Wal).await.unwrap_err();
        assert!(matches!(err, BackupError::Config(_)));
    }

    #[tokio::test]
    async fn verify_calls_pgbackrest_check() {
        let runner = Arc::new(
            MockCommandRunner::new().expect_ok("pgbackrest --config", ""),
        );
        let s = PgBackRestSnapshotter::new(
            runner.clone(),
            "main",
            "/etc/pgbackrest/pgbackrest.conf",
            "prod",
            "efe-prod",
        );
        s.verify("any-label").await.unwrap();
        let invs = runner.invocations();
        assert_eq!(invs.len(), 1);
        assert_eq!(invs[0].program, "pgbackrest");
        assert!(invs[0].args.iter().any(|a| a == "check"));
    }

    #[tokio::test]
    async fn restore_calls_pgbackrest_restore_with_label() {
        let runner = Arc::new(
            MockCommandRunner::new().expect_ok("pgbackrest --config", ""),
        );
        let s = PgBackRestSnapshotter::new(
            runner.clone(),
            "main",
            "/etc/pgbackrest/pgbackrest.conf",
            "prod",
            "efe-prod",
        );
        s.restore("20260428T060000Z-full", "drill").await.unwrap();
        let invs = runner.invocations();
        assert_eq!(invs.len(), 1);
        assert!(invs[0].args.iter().any(|a| a == "restore"));
        // --set <label> must be present
        let set_idx = invs[0].args.iter().position(|a| a == "--set").unwrap();
        assert_eq!(invs[0].args[set_idx + 1], "20260428T060000Z-full");
    }

    #[tokio::test]
    async fn parse_handles_array_wrapper() {
        let out = br#"[ { "repo_path": "x", "size": 1, "files": 2, "lsn_start": "0/0", "lsn_stop": "0/1" } ]"#;
        let parsed = parse_backup_output(out).unwrap();
        assert_eq!(parsed.size_bytes, 1);
    }

    #[tokio::test]
    async fn parse_rejects_garbage() {
        let err = parse_backup_output(b"not json").unwrap_err();
        assert!(matches!(err, BackupError::PgBackRest(_)));
    }
}
