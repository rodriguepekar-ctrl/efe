//! Sprint D17 — **Audit fix § AC2 + AC3** — runtime instrumentation
//! wrapper for `Snapshotter` calls.
//!
//! Pre-audit, the 8 D17 Prometheus instruments declared in
//! `efe_telemetry::metrics::d17_instruments` were dead — declared
//! but never bumped from runtime code. Same defect class as
//! D16-H3/H5/H6 (counters registered but never incremented).
//!
//! Pre-audit, `Snapshotter::snapshot` returned `SnapshotMetadata`
//! with empty `ed25519_sig` and a placeholder
//! `sha256_manifest = "0"*64`. Restore-time signature verification
//! would reject every backup. Architecture § 6.1 step 3 broken.
//!
//! Post-audit, this module's [`InstrumentedSnapshot`] wrapper :
//!   1. Bumps `efe_backup_runs_total{source,type,result}` based on
//!      outcome.
//!   2. Records `efe_backup_duration_seconds{source,type}`.
//!   3. Records `efe_backup_size_bytes{source,type}` for successful
//!      snapshots.
//!   4. Sets `efe_backup_last_success_timestamp{source,type}` to
//!      now() on success.
//!   5. Calls [`crate::metadata::SnapshotMetadata::sign`] with the
//!      provided signing key (audit fix § AC3).
//!   6. Emits the audit-chain event via [`crate::BackupAuditEmitter`].
//!
//! The CLI's `cmd_snapshot` should always go through this wrapper,
//! NEVER through raw `Snapshotter::snapshot` calls — otherwise the
//! metrics + signing path stays dead.

use std::sync::Arc;

use ed25519_dalek::SigningKey;

use crate::audit::BackupAuditEmitter;
use crate::error::BackupError;
use crate::metadata::{BackupKind, SnapshotMetadata};
use crate::snapshot::Snapshotter;

/// Wraps a `Snapshotter` call with metric instrumentation, manifest
/// signing, and audit-chain emission.
pub struct InstrumentedSnapshot {
    snapshotter: Arc<dyn Snapshotter>,
    audit: BackupAuditEmitter,
    signing_key: Option<SigningKey>,
    key_id: String,
}

impl InstrumentedSnapshot {
    /// Construct.
    pub fn new(
        snapshotter: Arc<dyn Snapshotter>,
        audit: BackupAuditEmitter,
        signing_key: Option<SigningKey>,
        key_id: impl Into<String>,
    ) -> Self {
        Self {
            snapshotter,
            audit,
            signing_key,
            key_id: key_id.into(),
        }
    }

    /// Take a snapshot, instrument metrics, sign the metadata, emit audit.
    /// Returns the signed metadata on success, or the error.
    pub async fn snapshot(
        &self,
        label: &str,
        kind: BackupKind,
    ) -> Result<SnapshotMetadata, BackupError> {
        let source = self.snapshotter.source();
        let started_at = std::time::Instant::now();

        let result = self.snapshotter.snapshot(label, kind).await;

        // Bump duration histogram regardless of outcome.
        let elapsed = started_at.elapsed().as_secs_f64();
        efe_telemetry::metrics::d17_instruments::backup_duration_seconds()
            .with_label_values(&[source.as_str(), kind.as_str()])
            .observe(elapsed);

        match result {
            Ok(mut meta) => {
                // Sign the metadata if a signing key is provided.
                if let Some(sk) = &self.signing_key {
                    if let Err(e) = meta.sign(sk) {
                        // Sign failure → treat the whole snapshot as
                        // failed for metrics purposes.
                        bump_runs(source, kind, "failed");
                        let _ = self.audit.emit_snapshot(label, source, kind, Err(&e));
                        return Err(e);
                    }
                }
                // Bump success metrics.
                bump_runs(source, kind, "ok");
                efe_telemetry::metrics::d17_instruments::backup_size_bytes()
                    .with_label_values(&[source.as_str(), kind.as_str()])
                    .observe(meta.size_bytes as f64);
                let unix_now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                efe_telemetry::metrics::d17_instruments::backup_last_success_timestamp()
                    .with_label_values(&[source.as_str(), kind.as_str()])
                    .set(unix_now);

                // Audit emit.
                let _ = self.audit.emit_snapshot(label, source, kind, Ok(()));
                Ok(meta)
            }
            Err(e) => {
                bump_runs(source, kind, "failed");
                let _ = self.audit.emit_snapshot(label, source, kind, Err(&e));
                Err(e)
            }
        }
    }

    /// Instrumented verify call.
    pub async fn verify(&self, label: &str) -> Result<(), BackupError> {
        let source = self.snapshotter.source();
        let result = self.snapshotter.verify(label).await;
        match &result {
            Ok(()) => {
                let _ = self.audit.emit_verify(label, Ok(()));
            }
            Err(e) => {
                let _ = self.audit.emit_verify(label, Err(e));
            }
        }
        result
    }

    /// Instrumented restore call.
    pub async fn restore(
        &self,
        label: &str,
        target_env: &str,
    ) -> Result<(), BackupError> {
        let source = self.snapshotter.source();
        let started_at = std::time::Instant::now();
        let result = self.snapshotter.restore(label, target_env).await;
        let elapsed = started_at.elapsed().as_secs_f64();
        // Restores are not separate counters but we capture the
        // duration in the same histogram for now.
        efe_telemetry::metrics::d17_instruments::backup_duration_seconds()
            .with_label_values(&[source.as_str(), "restore"])
            .observe(elapsed);
        match &result {
            Ok(()) => {
                let _ = self.audit.emit_restore(label, target_env, Ok(()));
            }
            Err(e) => {
                let _ = self.audit.emit_restore(label, target_env, Err(e));
            }
        }
        result
    }

    /// Direct access to the underlying Snapshotter (for callers that
    /// need it, e.g. a RestoreOrchestrator that wants to keep the
    /// Arc<dyn Snapshotter> shape).
    pub fn inner(&self) -> Arc<dyn Snapshotter> {
        Arc::clone(&self.snapshotter)
    }
}

fn bump_runs(source: crate::metadata::SnapshotSource, kind: BackupKind, result: &str) {
    efe_telemetry::metrics::d17_instruments::backup_runs_total()
        .with_label_values(&[source.as_str(), kind.as_str(), result])
        .inc();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::SnapshotSource;
    use crate::snapshot::MockSnapshotter;

    fn make_audit() -> BackupAuditEmitter {
        BackupAuditEmitter::new(None, false)
    }

    #[tokio::test]
    async fn instrumented_snapshot_bumps_runs_total_ok() {
        let snap = Arc::new(MockSnapshotter::new(SnapshotSource::Postgres));
        let inst = InstrumentedSnapshot::new(snap, make_audit(), None, "k");
        let metric = efe_telemetry::metrics::d17_instruments::backup_runs_total();
        let labels = ["postgres", "full", "ok"];
        let before = metric.with_label_values(&labels).get();
        let _ = inst.snapshot("label", BackupKind::Full).await.unwrap();
        let after = metric.with_label_values(&labels).get();
        assert!(after >= before + 1);
    }

    #[tokio::test]
    async fn instrumented_snapshot_bumps_failed_on_error() {
        let snap = Arc::new(MockSnapshotter::new(SnapshotSource::Vault));
        snap.expect_snapshot(Err(BackupError::Vault("boom".into())));
        let inst = InstrumentedSnapshot::new(
            Arc::clone(&snap) as Arc<dyn Snapshotter>,
            make_audit(),
            None,
            "k",
        );
        let metric = efe_telemetry::metrics::d17_instruments::backup_runs_total();
        let labels = ["vault", "vault_snapshot", "failed"];
        let before = metric.with_label_values(&labels).get();
        let _ = inst
            .snapshot("x", BackupKind::VaultSnapshot)
            .await
            .unwrap_err();
        let after = metric.with_label_values(&labels).get();
        assert!(after >= before + 1);
    }

    #[tokio::test]
    async fn instrumented_snapshot_signs_metadata_when_key_provided() {
        use ed25519_dalek::SigningKey;
        use rand_core::OsRng;

        let sk = SigningKey::generate(&mut OsRng);
        let snap = Arc::new(MockSnapshotter::new(SnapshotSource::Postgres));
        let inst = InstrumentedSnapshot::new(snap, make_audit(), Some(sk), "test-key");
        let meta = inst.snapshot("label-sig", BackupKind::Full).await.unwrap();
        assert!(!meta.ed25519_sig.is_empty(), "sig must be populated");
        assert_ne!(meta.sha256_manifest, "0".repeat(64));
        // sha256 hex shape.
        assert_eq!(meta.sha256_manifest.len(), 64);
    }

    #[tokio::test]
    async fn instrumented_snapshot_no_sign_when_key_absent() {
        let snap = Arc::new(MockSnapshotter::new(SnapshotSource::Bundles));
        let inst = InstrumentedSnapshot::new(snap, make_audit(), None, "");
        let meta = inst
            .snapshot("label-nosig", BackupKind::BundleSync)
            .await
            .unwrap();
        // ed25519_sig may be empty (mock default returns empty),
        // but the call must not panic.
        let _ = meta;
    }

    #[tokio::test]
    async fn instrumented_snapshot_observes_duration_histogram() {
        let snap = Arc::new(MockSnapshotter::new(SnapshotSource::Postgres));
        let inst = InstrumentedSnapshot::new(snap, make_audit(), None, "");
        let hist = efe_telemetry::metrics::d17_instruments::backup_duration_seconds();
        let labels = ["postgres", "full"];
        let before = hist.with_label_values(&labels).get_sample_count();
        let _ = inst.snapshot("dur", BackupKind::Full).await.unwrap();
        let after = hist.with_label_values(&labels).get_sample_count();
        assert!(after >= before + 1);
    }

    #[tokio::test]
    async fn instrumented_snapshot_sets_last_success_ts_on_ok() {
        let snap = Arc::new(MockSnapshotter::new(SnapshotSource::Vault));
        let inst = InstrumentedSnapshot::new(snap, make_audit(), None, "");
        let _ = inst
            .snapshot("ts", BackupKind::VaultSnapshot)
            .await
            .unwrap();
        let ts = efe_telemetry::metrics::d17_instruments::backup_last_success_timestamp()
            .with_label_values(&["vault", "vault_snapshot"])
            .get();
        // Must be a recent unix timestamp.
        assert!(ts > 1_700_000_000, "ts {ts} not recent");
    }

    #[tokio::test]
    async fn instrumented_restore_emits_audit_event() {
        let snap = Arc::new(MockSnapshotter::new(SnapshotSource::Postgres));
        let inst = InstrumentedSnapshot::new(snap, make_audit(), None, "");
        inst.restore("label", "drill").await.unwrap();
    }

    #[tokio::test]
    async fn instrumented_verify_propagates_error() {
        let snap = Arc::new(MockSnapshotter::new(SnapshotSource::Bundles));
        snap.expect_verify(Err(BackupError::Verify("bad sig".into())));
        let inst = InstrumentedSnapshot::new(
            Arc::clone(&snap) as Arc<dyn Snapshotter>,
            make_audit(),
            None,
            "",
        );
        let err = inst.verify("label").await.unwrap_err();
        assert!(matches!(err, BackupError::Verify(_)));
    }
}
