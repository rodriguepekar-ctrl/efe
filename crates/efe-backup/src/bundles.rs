//! Sprint D17 — `Snapshotter` impl for embedder bundles via rclone.
//!
//! Cf. architecture § 5.4.
//!
//! Wraps `rclone sync` (with `--immutable` to refuse overwrites)
//! between the source bundles bucket (D16) and the backup bucket.
//! Then runs `rclone check --size-only` for an integrity sanity
//! check.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;

use crate::command_runner::CommandRunner;
use crate::error::BackupError;
use crate::metadata::{BackupKind, SnapshotMetadata, SnapshotSource};
use crate::snapshot::Snapshotter;

/// `rclone`-based bundle snapshotter.
#[derive(Debug)]
pub struct BundleSnapshotter {
    runner: Arc<dyn CommandRunner>,
    /// Source bucket, e.g. `efe-bundles-prod`.
    pub source_bucket: String,
    /// Destination bucket, e.g. `efe-bundle-backups-prod`.
    pub dest_bucket: String,
    /// Symbolic env tag.
    pub env: String,
    /// Whether to enforce `--immutable` on the destination side.
    pub immutable_dest: bool,
}

impl BundleSnapshotter {
    /// Construct.
    pub fn new(
        runner: Arc<dyn CommandRunner>,
        source_bucket: impl Into<String>,
        dest_bucket: impl Into<String>,
        env: impl Into<String>,
    ) -> Self {
        Self {
            runner,
            source_bucket: source_bucket.into(),
            dest_bucket: dest_bucket.into(),
            env: env.into(),
            immutable_dest: true,
        }
    }
}

#[async_trait]
impl Snapshotter for BundleSnapshotter {
    fn source(&self) -> SnapshotSource {
        SnapshotSource::Bundles
    }

    async fn snapshot(
        &self,
        label: &str,
        kind: BackupKind,
    ) -> Result<SnapshotMetadata, BackupError> {
        if kind != BackupKind::BundleSync {
            return Err(BackupError::config(format!(
                "BundleSnapshotter requires kind=BundleSync, got {kind:?}"
            )));
        }
        let started_at = Utc::now();
        let src = format!("s3:{}/", self.source_bucket);
        let dst = format!("s3:{}/snapshot-{}/", self.dest_bucket, label);
        let mut args: Vec<&str> = vec!["sync", &src, &dst];
        if self.immutable_dest {
            args.push("--immutable");
        }

        let out = self
            .runner
            .run("rclone", &args, &[])
            .await
            .map_err(|e| BackupError::Rclone(format!("rclone spawn failed: {e}")))?;
        if !out.success() {
            return Err(BackupError::Rclone(format!(
                "rclone sync exit {} : {}",
                out.status,
                out.stderr_string()
            )));
        }

        // Run integrity check post-sync.
        let check_out = self
            .runner
            .run("rclone", &["check", "--size-only", &src, &dst], &[])
            .await
            .map_err(|e| BackupError::Rclone(format!("rclone check spawn failed: {e}")))?;
        if !check_out.success() {
            return Err(BackupError::Rclone(format!(
                "rclone check exit {} : {} (sync produced inconsistent dest)",
                check_out.status,
                check_out.stderr_string()
            )));
        }

        // Parse stdout for sync stats. The format is :
        //   Transferred: 5 / 5, 100%
        // We accept any non-empty output as success ; the exit code
        // already gates correctness.
        let (size_bytes, files_count) = parse_rclone_stats(&out.stdout);

        let finished_at = Utc::now();
        Ok(SnapshotMetadata {
            label: label.to_string(),
            source: SnapshotSource::Bundles,
            kind,
            repo_path: dst,
            size_bytes,
            files_count,
            sha256_manifest: "0".repeat(64),
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
        let src = format!("s3:{}/", self.source_bucket);
        let dst = format!("s3:{}/snapshot-{}/", self.dest_bucket, label);
        let out = self
            .runner
            .run("rclone", &["check", "--size-only", &src, &dst], &[])
            .await
            .map_err(|e| BackupError::Rclone(format!("rclone spawn failed: {e}")))?;
        if !out.success() {
            return Err(BackupError::Rclone(format!(
                "rclone check exit {} : {}",
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
        // Restore is the reverse of snapshot : sync from backup
        // bucket back to the original source bucket.
        let src = format!("s3:{}/snapshot-{}/", self.dest_bucket, label);
        let dst = format!("s3:{}/", self.source_bucket);
        let out = self
            .runner
            .run("rclone", &["sync", &src, &dst], &[])
            .await
            .map_err(|e| BackupError::Rclone(format!("rclone spawn failed: {e}")))?;
        if !out.success() {
            return Err(BackupError::Rclone(format!(
                "rclone restore exit {} : {}",
                out.status,
                out.stderr_string()
            )));
        }
        Ok(())
    }
}

/// Parse the `Transferred: N / M, X%` line from rclone stdout.
/// Returns `(0, 0)` on parse failure (success path is guaranteed
/// by the exit code).
fn parse_rclone_stats(stdout: &[u8]) -> (u64, u32) {
    let s = String::from_utf8_lossy(stdout);
    let mut bytes_total: u64 = 0;
    let mut files_total: u32 = 0;
    for line in s.lines() {
        let lc = line.to_ascii_lowercase();
        if let Some(rest) = lc.strip_prefix("transferred:") {
            // "Transferred: 1.2 KiB / 5 KiB, 24%, 100 B/s, ETA ..."
            // "Transferred: 5 / 5, 100%"
            let parts: Vec<&str> = rest.split('/').collect();
            if parts.len() >= 2 {
                if let Some(b) = parts[0].split_whitespace().next() {
                    let trimmed = b.trim_end_matches(|c: char| !c.is_ascii_digit() && c != '.');
                    if let Ok(n) = trimmed.parse::<f64>() {
                        bytes_total = n as u64;
                    }
                }
                if let Some(f) = parts[1].split_whitespace().next() {
                    // Strip any trailing punctuation (`,`, `%`, etc.).
                    let trimmed = f.trim_end_matches(|c: char| !c.is_ascii_digit());
                    if let Ok(n) = trimmed.parse::<u32>() {
                        files_total = n;
                    }
                }
            }
        }
    }
    (bytes_total, files_total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_runner::MockCommandRunner;

    #[tokio::test]
    async fn snapshot_happy_path_runs_sync_then_check() {
        let runner = Arc::new(
            MockCommandRunner::new()
                .expect_ok("rclone sync", "Transferred: 5 / 5, 100%")
                .expect_ok("rclone check", "0 differences"),
        );
        let s = BundleSnapshotter::new(
            runner.clone(),
            "efe-bundles-prod",
            "efe-bundle-backups-prod",
            "prod",
        );
        let meta = s
            .snapshot("20260428T070000Z", BackupKind::BundleSync)
            .await
            .unwrap();
        assert_eq!(meta.source, SnapshotSource::Bundles);
        assert_eq!(meta.kind, BackupKind::BundleSync);
        assert!(meta.repo_path.contains("efe-bundle-backups-prod"));
        assert!(meta.ok);
        // Two invocations : sync + check.
        assert_eq!(runner.invocation_count(), 2);
    }

    #[tokio::test]
    async fn snapshot_propagates_check_failure() {
        let runner = Arc::new(
            MockCommandRunner::new()
                .expect_ok("rclone sync", "")
                .expect_err("rclone check", 1, "1 differences found"),
        );
        let s = BundleSnapshotter::new(
            runner,
            "efe-bundles-prod",
            "efe-bundle-backups-prod",
            "prod",
        );
        let err = s
            .snapshot("x", BackupKind::BundleSync)
            .await
            .unwrap_err();
        assert!(matches!(err, BackupError::Rclone(_)));
        assert!(err.to_string().contains("1 differences found"));
    }

    #[tokio::test]
    async fn snapshot_rejects_wrong_kind() {
        let runner = Arc::new(MockCommandRunner::new());
        let s = BundleSnapshotter::new(runner, "src", "dst", "prod");
        let err = s.snapshot("x", BackupKind::Full).await.unwrap_err();
        assert!(matches!(err, BackupError::Config(_)));
    }

    #[tokio::test]
    async fn restore_runs_reverse_sync() {
        let runner = Arc::new(
            MockCommandRunner::new().expect_ok("rclone sync", "ok"),
        );
        let s = BundleSnapshotter::new(
            runner.clone(),
            "efe-bundles-prod",
            "efe-bundle-backups-prod",
            "prod",
        );
        s.restore("label", "drill").await.unwrap();
        let invs = runner.invocations();
        assert_eq!(invs.len(), 1);
        // src must be the backup bucket, dst the source bucket.
        assert!(invs[0].args[1].starts_with("s3:efe-bundle-backups-prod/snapshot-label/"));
        assert_eq!(invs[0].args[2], "s3:efe-bundles-prod/");
    }

    #[test]
    fn parse_rclone_stats_handles_typical_output() {
        let out = b"Transferred: 5 / 5, 100%\nElapsed time: 1.2s\n";
        let (bytes, files) = parse_rclone_stats(out);
        assert_eq!(bytes, 5);
        assert_eq!(files, 5);
    }

    #[test]
    fn parse_rclone_stats_returns_zero_on_garbage() {
        let (bytes, files) = parse_rclone_stats(b"hello world");
        assert_eq!(bytes, 0);
        assert_eq!(files, 0);
    }
}
