//! Sprint D17 — `Snapshotter` impl for HashiCorp Vault Raft snapshots.
//!
//! Cf. architecture § 5.3.
//!
//! Wraps `vault operator raft snapshot save /tmp/<label>.snap` then
//! shells out to `aws s3 cp` to upload.
//!
//! Restore is the inverse : `aws s3 cp` from the backup bucket then
//! `vault operator raft snapshot restore`.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;

use crate::command_runner::CommandRunner;
use crate::error::BackupError;
use crate::metadata::{BackupKind, SnapshotMetadata, SnapshotSource};
use crate::snapshot::Snapshotter;

/// Vault Raft snapshotter.
#[derive(Debug)]
pub struct VaultSnapshotter {
    runner: Arc<dyn CommandRunner>,
    /// VAULT_ADDR the runner will see (env-injected).
    pub addr: String,
    /// Path to a file containing VAULT_TOKEN (loaded from K8s Secret).
    pub token_path: String,
    /// S3 destination bucket prefix, e.g. `s3://efe-backups-prod/vault-snapshots`.
    pub s3_dest_prefix: String,
    /// Local working directory (default `/tmp`).
    pub work_dir: String,
    /// Symbolic env tag.
    pub env: String,
}

impl VaultSnapshotter {
    /// Construct.
    pub fn new(
        runner: Arc<dyn CommandRunner>,
        addr: impl Into<String>,
        token_path: impl Into<String>,
        s3_dest_prefix: impl Into<String>,
        env: impl Into<String>,
    ) -> Self {
        Self {
            runner,
            addr: addr.into(),
            token_path: token_path.into(),
            s3_dest_prefix: s3_dest_prefix.into(),
            work_dir: "/tmp".into(),
            env: env.into(),
        }
    }

    /// Override the working directory (tests use `tempfile::tempdir()`).
    #[must_use]
    pub fn with_work_dir(mut self, dir: impl Into<String>) -> Self {
        self.work_dir = dir.into();
        self
    }

    fn read_token(&self) -> Result<String, BackupError> {
        let token = std::fs::read_to_string(&self.token_path)
            .map_err(|e| BackupError::Vault(format!(
                "cannot read vault token at {:?}: {e}",
                self.token_path
            )))?;
        Ok(token.trim().to_string())
    }
}

#[async_trait]
impl Snapshotter for VaultSnapshotter {
    fn source(&self) -> SnapshotSource {
        SnapshotSource::Vault
    }

    async fn snapshot(
        &self,
        label: &str,
        kind: BackupKind,
    ) -> Result<SnapshotMetadata, BackupError> {
        if kind != BackupKind::VaultSnapshot {
            return Err(BackupError::config(format!(
                "VaultSnapshotter requires kind=VaultSnapshot, got {kind:?}"
            )));
        }
        let started_at = Utc::now();
        let token = self.read_token()?;
        let local_path = format!("{}/vault-snapshot-{}.snap", self.work_dir, label);

        // 1. `vault operator raft snapshot save <local_path>`
        let env: &[(&str, &str)] = &[
            ("VAULT_ADDR", &self.addr),
            ("VAULT_TOKEN", &token),
        ];
        let out = self
            .runner
            .run(
                "vault",
                &["operator", "raft", "snapshot", "save", &local_path],
                env,
            )
            .await
            .map_err(|e| BackupError::Vault(format!("vault spawn failed: {e}")))?;
        if !out.success() {
            return Err(BackupError::Vault(format!(
                "vault snapshot save exit {} : {}",
                out.status,
                out.stderr_string()
            )));
        }

        // 2. SHA-256 + size of the local file (stat).
        let (size_bytes, sha256) = stat_file(&local_path)?;

        // 3. Upload to S3.
        let s3_url = format!("{}/{}.snap", self.s3_dest_prefix, label);
        let out = self
            .runner
            .run("aws", &["s3", "cp", &local_path, &s3_url], &[])
            .await
            .map_err(|e| BackupError::Vault(format!("aws spawn failed: {e}")))?;
        if !out.success() {
            return Err(BackupError::Vault(format!(
                "aws s3 cp exit {} : {}",
                out.status,
                out.stderr_string()
            )));
        }

        // 4. Best-effort cleanup of the local file (don't fail the
        // snapshot on cleanup failure).
        let _ = std::fs::remove_file(&local_path);

        let finished_at = Utc::now();
        Ok(SnapshotMetadata {
            label: label.to_string(),
            source: SnapshotSource::Vault,
            kind,
            repo_path: s3_url,
            size_bytes,
            files_count: 1,
            sha256_manifest: sha256,
            ed25519_sig: String::new(),
            started_at,
            finished_at,
            pg_lsn_start: None,
            pg_lsn_stop: None,
            ok: true,
            error_message: None,
        })
    }

    async fn verify(&self, label: &str) -> Result<(), BackupError> {
        let s3_url = format!("{}/{}.snap", self.s3_dest_prefix, label);
        // We use `aws s3 ls` as a cheap object-exists check.
        let out = self
            .runner
            .run("aws", &["s3", "ls", &s3_url], &[])
            .await
            .map_err(|e| BackupError::Vault(format!("aws spawn failed: {e}")))?;
        if !out.success() {
            return Err(BackupError::Vault(format!(
                "aws s3 ls exit {} : {} (snapshot may be missing)",
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
        let token = self.read_token()?;
        let s3_url = format!("{}/{}.snap", self.s3_dest_prefix, label);
        let local_path = format!("{}/vault-restore-{}.snap", self.work_dir, label);

        // 1. download from S3.
        let out = self
            .runner
            .run("aws", &["s3", "cp", &s3_url, &local_path], &[])
            .await
            .map_err(|e| BackupError::Vault(format!("aws spawn failed: {e}")))?;
        if !out.success() {
            return Err(BackupError::Vault(format!(
                "aws s3 cp exit {} : {}",
                out.status,
                out.stderr_string()
            )));
        }

        // 2. `vault operator raft snapshot restore <local_path>`
        let env: &[(&str, &str)] = &[
            ("VAULT_ADDR", &self.addr),
            ("VAULT_TOKEN", &token),
        ];
        let out = self
            .runner
            .run(
                "vault",
                &["operator", "raft", "snapshot", "restore", &local_path],
                env,
            )
            .await
            .map_err(|e| BackupError::Vault(format!("vault spawn failed: {e}")))?;
        if !out.success() {
            return Err(BackupError::Vault(format!(
                "vault restore exit {} : {}",
                out.status,
                out.stderr_string()
            )));
        }

        let _ = std::fs::remove_file(&local_path);
        Ok(())
    }
}

/// Compute size + SHA-256 of a local file. Streaming so even very
/// large Vault snapshots don't blow memory.
fn stat_file(path: &str) -> Result<(u64, String), BackupError> {
    use sha2::{Digest, Sha256};
    use std::fs::File;
    use std::io::Read;
    let mut f = File::open(path)?;
    let metadata = f.metadata()?;
    let size = metadata.len();
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let hex = hex::encode(hasher.finalize());
    Ok((size, hex))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_runner::MockCommandRunner;
    use std::io::Write;
    use tempfile::tempdir;

    fn write_token_file(dir: &std::path::Path) -> String {
        let path = dir.join("vault-token");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"hvs.test-token\n").unwrap();
        path.to_string_lossy().to_string()
    }

    fn write_dummy_snapshot(dir: &std::path::Path, label: &str) {
        // The MockCommandRunner doesn't actually execute vault, so
        // we pre-create the file pgBackRest the runner "would" have
        // written.
        let path = dir.join(format!("vault-snapshot-{label}.snap"));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"raft-snapshot-bytes").unwrap();
    }

    #[tokio::test]
    async fn snapshot_happy_path_uploads_and_returns_metadata() {
        let dir = tempdir().unwrap();
        let token_path = write_token_file(dir.path());
        write_dummy_snapshot(dir.path(), "label-x");

        let runner = Arc::new(
            MockCommandRunner::new()
                .expect_ok("vault operator raft snapshot save", "ok")
                .expect_ok("aws s3 cp", "uploaded"),
        );
        let s = VaultSnapshotter::new(
            runner,
            "https://vault:8200",
            token_path,
            "s3://efe-backups/vault",
            "prod",
        )
        .with_work_dir(dir.path().to_string_lossy().to_string());

        let meta = s
            .snapshot("label-x", BackupKind::VaultSnapshot)
            .await
            .unwrap();
        assert_eq!(meta.source, SnapshotSource::Vault);
        assert_eq!(meta.kind, BackupKind::VaultSnapshot);
        assert_eq!(meta.repo_path, "s3://efe-backups/vault/label-x.snap");
        assert_eq!(meta.size_bytes, b"raft-snapshot-bytes".len() as u64);
        assert_eq!(meta.sha256_manifest.len(), 64);
        assert!(meta.ok);
    }

    #[tokio::test]
    async fn snapshot_rejects_wrong_kind() {
        let dir = tempdir().unwrap();
        let token_path = write_token_file(dir.path());
        let runner = Arc::new(MockCommandRunner::new());
        let s = VaultSnapshotter::new(
            runner,
            "https://vault:8200",
            token_path,
            "s3://x",
            "prod",
        );
        let err = s.snapshot("x", BackupKind::Full).await.unwrap_err();
        assert!(matches!(err, BackupError::Config(_)));
    }

    #[tokio::test]
    async fn snapshot_propagates_vault_failure() {
        let dir = tempdir().unwrap();
        let token_path = write_token_file(dir.path());
        let runner = Arc::new(
            MockCommandRunner::new().expect_err("vault", 1, "permission denied"),
        );
        let s = VaultSnapshotter::new(
            runner,
            "https://vault:8200",
            token_path,
            "s3://x",
            "prod",
        )
        .with_work_dir(dir.path().to_string_lossy().to_string());
        let err = s
            .snapshot("x", BackupKind::VaultSnapshot)
            .await
            .unwrap_err();
        assert!(matches!(err, BackupError::Vault(_)));
        assert!(err.to_string().contains("permission denied"));
    }

    #[tokio::test]
    async fn verify_calls_aws_s3_ls() {
        let runner = Arc::new(MockCommandRunner::new().expect_ok("aws s3 ls", "obj"));
        let s = VaultSnapshotter::new(
            runner.clone(),
            "https://vault:8200",
            "/tmp/token",
            "s3://x",
            "prod",
        );
        s.verify("label").await.unwrap();
        let invs = runner.invocations();
        assert_eq!(invs.len(), 1);
        assert_eq!(invs[0].program, "aws");
    }

    #[tokio::test]
    async fn verify_propagates_missing_object() {
        let runner = Arc::new(
            MockCommandRunner::new().expect_err("aws", 1, "NoSuchKey"),
        );
        let s = VaultSnapshotter::new(
            runner,
            "https://vault:8200",
            "/tmp/token",
            "s3://x",
            "prod",
        );
        let err = s.verify("missing").await.unwrap_err();
        assert!(matches!(err, BackupError::Vault(_)));
    }

    #[test]
    fn stat_file_streams_large_input() {
        use std::io::Write;
        let dir = tempdir().unwrap();
        let path = dir.path().join("large.bin");
        let mut f = std::fs::File::create(&path).unwrap();
        // Write 200 KB of repeating bytes — exercises the chunked
        // hasher path.
        for _ in 0..(200 * 1024 / 32) {
            f.write_all(b"abcdefghijklmnopqrstuvwxyz012345").unwrap();
        }
        let path_s = path.to_string_lossy().to_string();
        let (size, hex_) = stat_file(&path_s).unwrap();
        assert_eq!(size, 200 * 1024);
        assert_eq!(hex_.len(), 64);
    }
}
