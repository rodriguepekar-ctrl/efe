//! Sprint D17 — `Snapshotter` trait + a generic `MockSnapshotter`
//! used by integration tests.
//!
//! Concrete impls live in [`crate::postgres`], [`crate::vault`],
//! and [`crate::bundles`].

use std::sync::Mutex;

use async_trait::async_trait;

use crate::error::BackupError;
use crate::metadata::{BackupKind, SnapshotMetadata, SnapshotSource};

/// Trait implemented by every backup data source. The orchestrator
/// holds an `Arc<dyn Snapshotter>` per registered source.
#[async_trait]
pub trait Snapshotter: Send + Sync + std::fmt::Debug {
    /// Symbolic source name (`postgres`, `vault`, `bundles`).
    fn source(&self) -> SnapshotSource;

    /// Take a snapshot of the underlying source. The returned
    /// [`SnapshotMetadata`] is what the orchestrator records in
    /// the `efe_backup_history` table.
    async fn snapshot(
        &self,
        label: &str,
        kind: BackupKind,
    ) -> Result<SnapshotMetadata, BackupError>;

    /// Verify the integrity of an at-rest snapshot. Called by the
    /// CLI `verify` subcommand and by the DR drill before a
    /// restore drill kick.
    async fn verify(&self, label: &str) -> Result<(), BackupError>;

    /// Restore from a snapshot. Called by [`crate::RestoreOrchestrator`].
    async fn restore(
        &self,
        label: &str,
        target_env: &str,
    ) -> Result<(), BackupError>;
}

/// Test-only snapshotter that records every call and returns
/// canned outputs. Usable across all tests that need a
/// deterministic Snapshotter without spawning real processes.
#[derive(Debug)]
pub struct MockSnapshotter {
    source: SnapshotSource,
    inner: Mutex<MockState>,
}

#[derive(Debug, Default)]
struct MockState {
    snapshots: Vec<(String, BackupKind)>,
    verifies: Vec<String>,
    restores: Vec<(String, String)>,
    next_snapshot_result: Option<Result<SnapshotMetadata, BackupError>>,
    next_verify_result: Option<Result<(), BackupError>>,
    next_restore_result: Option<Result<(), BackupError>>,
}

impl MockSnapshotter {
    /// New mock for the given source. Every operation returns
    /// `Ok` with default-shaped metadata until [`Self::expect_*`]
    /// is called.
    #[must_use]
    pub fn new(source: SnapshotSource) -> Self {
        Self {
            source,
            inner: Mutex::new(MockState::default()),
        }
    }

    /// Set the value returned by the next `snapshot` call.
    pub fn expect_snapshot(&self, result: Result<SnapshotMetadata, BackupError>) {
        if let Ok(mut g) = self.inner.lock() {
            g.next_snapshot_result = Some(result);
        }
    }

    /// Set the value returned by the next `verify` call.
    pub fn expect_verify(&self, result: Result<(), BackupError>) {
        if let Ok(mut g) = self.inner.lock() {
            g.next_verify_result = Some(result);
        }
    }

    /// Set the value returned by the next `restore` call.
    pub fn expect_restore(&self, result: Result<(), BackupError>) {
        if let Ok(mut g) = self.inner.lock() {
            g.next_restore_result = Some(result);
        }
    }

    /// Recorded snapshot calls.
    pub fn snapshot_calls(&self) -> Vec<(String, BackupKind)> {
        match self.inner.lock() {
            Ok(g) => g.snapshots.clone(),
            Err(_) => vec![],
        }
    }

    /// Recorded verify calls.
    pub fn verify_calls(&self) -> Vec<String> {
        match self.inner.lock() {
            Ok(g) => g.verifies.clone(),
            Err(_) => vec![],
        }
    }

    /// Recorded restore calls.
    pub fn restore_calls(&self) -> Vec<(String, String)> {
        match self.inner.lock() {
            Ok(g) => g.restores.clone(),
            Err(_) => vec![],
        }
    }
}

#[async_trait]
impl Snapshotter for MockSnapshotter {
    fn source(&self) -> SnapshotSource {
        self.source
    }

    async fn snapshot(
        &self,
        label: &str,
        kind: BackupKind,
    ) -> Result<SnapshotMetadata, BackupError> {
        let mut g = self.inner.lock().map_err(|_| {
            BackupError::Config("mock snapshotter mutex poisoned".into())
        })?;
        g.snapshots.push((label.to_string(), kind));
        if let Some(canned) = g.next_snapshot_result.take() {
            return canned;
        }
        Ok(SnapshotMetadata::ok_now(
            label,
            self.source,
            kind,
            format!("s3://test/{}/{}", self.source.as_str(), label),
        ))
    }

    async fn verify(&self, label: &str) -> Result<(), BackupError> {
        let mut g = self.inner.lock().map_err(|_| {
            BackupError::Config("mock snapshotter mutex poisoned".into())
        })?;
        g.verifies.push(label.to_string());
        match g.next_verify_result.take() {
            Some(r) => r,
            None => Ok(()),
        }
    }

    async fn restore(
        &self,
        label: &str,
        target_env: &str,
    ) -> Result<(), BackupError> {
        let mut g = self.inner.lock().map_err(|_| {
            BackupError::Config("mock snapshotter mutex poisoned".into())
        })?;
        g.restores.push((label.to_string(), target_env.to_string()));
        match g.next_restore_result.take() {
            Some(r) => r,
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_snapshotter_default_returns_ok() {
        let m = MockSnapshotter::new(SnapshotSource::Postgres);
        let meta = m.snapshot("label", BackupKind::Full).await.unwrap();
        assert_eq!(meta.label, "label");
        assert_eq!(meta.source, SnapshotSource::Postgres);
        assert_eq!(meta.kind, BackupKind::Full);
        assert!(meta.ok);
    }

    #[tokio::test]
    async fn mock_snapshotter_records_calls() {
        let m = MockSnapshotter::new(SnapshotSource::Vault);
        let _ = m.snapshot("a", BackupKind::Full).await.unwrap();
        let _ = m.snapshot("b", BackupKind::Diff).await.unwrap();
        let _ = m.verify("a").await.unwrap();
        let _ = m.restore("a", "prod").await.unwrap();
        let snaps = m.snapshot_calls();
        assert_eq!(snaps.len(), 2);
        assert_eq!(snaps[0], ("a".into(), BackupKind::Full));
        assert_eq!(snaps[1], ("b".into(), BackupKind::Diff));
        assert_eq!(m.verify_calls(), vec!["a".to_string()]);
        assert_eq!(m.restore_calls(), vec![("a".into(), "prod".into())]);
    }

    #[tokio::test]
    async fn mock_snapshotter_returns_canned_error() {
        let m = MockSnapshotter::new(SnapshotSource::Postgres);
        m.expect_snapshot(Err(BackupError::PgBackRest("boom".into())));
        let err = m.snapshot("x", BackupKind::Full).await.unwrap_err();
        assert!(matches!(err, BackupError::PgBackRest(_)));
    }

    #[tokio::test]
    async fn mock_snapshotter_canned_error_only_consumed_once() {
        let m = MockSnapshotter::new(SnapshotSource::Postgres);
        m.expect_verify(Err(BackupError::Verify("first".into())));
        let _ = m.verify("a").await.unwrap_err();
        // Second call falls back to default Ok.
        m.verify("b").await.unwrap();
    }
}
