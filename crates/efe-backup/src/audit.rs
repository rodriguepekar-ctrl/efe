//! Sprint D17 — audit-chain emission for backup operations.
//!
//! Cf. architecture § 18.2.
//!
//! Every snapshot/restore/delete/verify operation MUST emit a
//! structured row into the D14-bis HMAC-chained audit log so the
//! backup activity is tamper-evident.
//!
//! In v0.1.18 we wrap an `Arc<dyn AuditSink>` (the same trait the
//! efe-server runtime uses) and emit JSON values matching the
//! D14-bis canonical event format.

use std::sync::Arc;

use chrono::Utc;
use serde::Serialize;
use serde_json::json;

use crate::error::BackupError;
use crate::metadata::{BackupKind, SnapshotSource};

/// Re-export of the trait we depend on.
pub use efe_telemetry::logs::audit::AuditSink;

/// Symbolic event-type label embedded in the audit row.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupEventKind {
    /// A snapshot operation completed (success or failure).
    SnapshotCompleted,
    /// A restore operation completed.
    RestoreCompleted,
    /// A 4-eyes-approved snapshot deletion.
    SnapshotDeleted,
    /// Verification of a snapshot at rest.
    SnapshotVerified,
    /// DR drill outcome.
    DrillCompleted,
    /// A configuration anomaly (e.g. RPO violation alert).
    Anomaly,
}

impl BackupEventKind {
    /// Stable string used in the JSON row.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SnapshotCompleted => "backup.snapshot",
            Self::RestoreCompleted => "backup.restore",
            Self::SnapshotDeleted => "backup.delete",
            Self::SnapshotVerified => "backup.verify",
            Self::DrillCompleted => "backup.drill",
            Self::Anomaly => "backup.anomaly",
        }
    }
}

/// Wraps an `Arc<dyn AuditSink>` and emits structured backup
/// events. Cheap to clone (just an Arc bump).
#[derive(Debug, Clone)]
pub struct BackupAuditEmitter {
    sink: Option<Arc<dyn AuditSink>>,
    /// Whether emission is enabled (per `BackupConfig::audit_chain_events`).
    enabled: bool,
}

impl BackupAuditEmitter {
    /// Construct from an optional sink. When `sink = None` and
    /// `enabled = true`, emission is logged via `tracing` only
    /// (degraded mode for clusters that don't ship an AuditSink).
    pub fn new(sink: Option<Arc<dyn AuditSink>>, enabled: bool) -> Self {
        Self { sink, enabled }
    }

    /// `true` iff emission writes to a real chain.
    #[must_use]
    pub fn is_chained(&self) -> bool {
        self.enabled && self.sink.is_some()
    }

    /// Emit a snapshot-completed event.
    pub fn emit_snapshot(
        &self,
        label: &str,
        source: SnapshotSource,
        kind: BackupKind,
        result: Result<(), &BackupError>,
    ) -> Result<(), BackupError> {
        let (status, error_kind, error_message) = match result {
            Ok(()) => ("ok", None, None),
            Err(e) => (
                "failed",
                Some(e.metric_label()),
                Some(format!("{e}")),
            ),
        };
        let payload = json!({
            "schema_version": "1.0",
            "timestamp": Utc::now().to_rfc3339(),
            "event": BackupEventKind::SnapshotCompleted.as_str(),
            "label": label,
            "source": source.as_str(),
            "kind": kind.as_str(),
            "status": status,
            "error_kind": error_kind,
            "error_message": error_message,
        });
        self.write(payload)
    }

    /// Emit a restore-completed event.
    pub fn emit_restore(
        &self,
        label: &str,
        target_env: &str,
        result: Result<(), &BackupError>,
    ) -> Result<(), BackupError> {
        let (status, error_kind, error_message) = match result {
            Ok(()) => ("ok", None, None),
            Err(e) => (
                "failed",
                Some(e.metric_label()),
                Some(format!("{e}")),
            ),
        };
        let payload = json!({
            "schema_version": "1.0",
            "timestamp": Utc::now().to_rfc3339(),
            "event": BackupEventKind::RestoreCompleted.as_str(),
            "label": label,
            "target_env": target_env,
            "status": status,
            "error_kind": error_kind,
            "error_message": error_message,
        });
        self.write(payload)
    }

    /// Emit a snapshot-deletion event (4-eyes approved).
    pub fn emit_delete(
        &self,
        label: &str,
        primary_sub: &str,
        secondary_sub: &str,
        reason: &str,
    ) -> Result<(), BackupError> {
        let payload = json!({
            "schema_version": "1.0",
            "timestamp": Utc::now().to_rfc3339(),
            "event": BackupEventKind::SnapshotDeleted.as_str(),
            "label": label,
            "primary_sub": primary_sub,
            "secondary_sub": secondary_sub,
            "reason": reason,
            "rgpd_risk": false,
        });
        self.write(payload)
    }

    /// Emit a verify event.
    pub fn emit_verify(
        &self,
        label: &str,
        result: Result<(), &BackupError>,
    ) -> Result<(), BackupError> {
        let (status, error_message) = match result {
            Ok(()) => ("ok", None),
            Err(e) => ("failed", Some(format!("{e}"))),
        };
        let payload = json!({
            "schema_version": "1.0",
            "timestamp": Utc::now().to_rfc3339(),
            "event": BackupEventKind::SnapshotVerified.as_str(),
            "label": label,
            "status": status,
            "error_message": error_message,
        });
        self.write(payload)
    }

    /// Emit a DR drill outcome.
    pub fn emit_drill(
        &self,
        drill_label: &str,
        rto_secs: u64,
        rto_target_secs: u64,
        success: bool,
    ) -> Result<(), BackupError> {
        let payload = json!({
            "schema_version": "1.0",
            "timestamp": Utc::now().to_rfc3339(),
            "event": BackupEventKind::DrillCompleted.as_str(),
            "drill_label": drill_label,
            "rto_secs": rto_secs,
            "rto_target_secs": rto_target_secs,
            "rto_within_target": rto_secs <= rto_target_secs,
            "success": success,
        });
        self.write(payload)
    }

    fn write(&self, payload: serde_json::Value) -> Result<(), BackupError> {
        if !self.enabled {
            return Ok(());
        }
        match &self.sink {
            Some(sink) => {
                sink.write_entry(payload);
                Ok(())
            }
            None => {
                tracing::info!(
                    target: "efe.backup.audit",
                    payload = %payload,
                    "backup audit event (no sink configured ; logging only)"
                );
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Test-only `AuditSink` that captures every entry.
    #[derive(Debug, Default)]
    struct CapturingSink {
        entries: Mutex<Vec<serde_json::Value>>,
    }

    impl AuditSink for CapturingSink {
        fn write_entry(&self, entry: serde_json::Value) {
            if let Ok(mut g) = self.entries.lock() {
                g.push(entry);
            }
        }
    }

    impl CapturingSink {
        fn count(&self) -> usize {
            self.entries.lock().map(|g| g.len()).unwrap_or(0)
        }

        fn last(&self) -> Option<serde_json::Value> {
            self.entries
                .lock()
                .ok()
                .and_then(|g| g.last().cloned())
        }
    }

    #[test]
    fn emit_snapshot_with_sink_writes_entry() {
        let sink = Arc::new(CapturingSink::default());
        let em = BackupAuditEmitter::new(Some(sink.clone()), true);
        em.emit_snapshot(
            "label-a",
            SnapshotSource::Postgres,
            BackupKind::Full,
            Ok(()),
        )
        .unwrap();
        assert_eq!(sink.count(), 1);
        let v = sink.last().unwrap();
        assert_eq!(v["event"], "backup.snapshot");
        assert_eq!(v["label"], "label-a");
        assert_eq!(v["status"], "ok");
        assert_eq!(v["source"], "postgres");
    }

    #[test]
    fn emit_snapshot_with_error_records_error_kind() {
        let sink = Arc::new(CapturingSink::default());
        let em = BackupAuditEmitter::new(Some(sink.clone()), true);
        let err = BackupError::PgBackRest("boom".into());
        em.emit_snapshot(
            "x",
            SnapshotSource::Postgres,
            BackupKind::Diff,
            Err(&err),
        )
        .unwrap();
        let v = sink.last().unwrap();
        assert_eq!(v["status"], "failed");
        assert_eq!(v["error_kind"], "pgbackrest_failed");
        assert!(v["error_message"]
            .as_str()
            .unwrap()
            .contains("boom"));
    }

    #[test]
    fn emit_with_disabled_returns_ok_and_writes_nothing() {
        let sink = Arc::new(CapturingSink::default());
        let em = BackupAuditEmitter::new(Some(sink.clone()), false);
        em.emit_snapshot(
            "x",
            SnapshotSource::Vault,
            BackupKind::VaultSnapshot,
            Ok(()),
        )
        .unwrap();
        assert_eq!(sink.count(), 0);
    }

    #[test]
    fn emit_without_sink_logs_only() {
        let em = BackupAuditEmitter::new(None, true);
        // No panic, returns Ok.
        em.emit_snapshot(
            "x",
            SnapshotSource::Vault,
            BackupKind::VaultSnapshot,
            Ok(()),
        )
        .unwrap();
        assert!(!em.is_chained());
    }

    #[test]
    fn emit_restore_records_target_env() {
        let sink = Arc::new(CapturingSink::default());
        let em = BackupAuditEmitter::new(Some(sink.clone()), true);
        em.emit_restore("label", "drill", Ok(())).unwrap();
        let v = sink.last().unwrap();
        assert_eq!(v["event"], "backup.restore");
        assert_eq!(v["target_env"], "drill");
    }

    #[test]
    fn emit_delete_records_4_eyes_subjects() {
        let sink = Arc::new(CapturingSink::default());
        let em = BackupAuditEmitter::new(Some(sink.clone()), true);
        em.emit_delete("label", "alice", "bob", "GDPR-erasure-T-XYZ")
            .unwrap();
        let v = sink.last().unwrap();
        assert_eq!(v["event"], "backup.delete");
        assert_eq!(v["primary_sub"], "alice");
        assert_eq!(v["secondary_sub"], "bob");
        assert_eq!(v["reason"], "GDPR-erasure-T-XYZ");
    }

    #[test]
    fn emit_drill_records_rto_within_target() {
        let sink = Arc::new(CapturingSink::default());
        let em = BackupAuditEmitter::new(Some(sink.clone()), true);
        em.emit_drill("drill-20260428", 1500, 1800, true).unwrap();
        let v = sink.last().unwrap();
        assert_eq!(v["event"], "backup.drill");
        assert_eq!(v["rto_secs"], 1500);
        assert_eq!(v["rto_within_target"], true);
        assert_eq!(v["success"], true);
    }

    #[test]
    fn emit_drill_marks_rto_exceeded() {
        let sink = Arc::new(CapturingSink::default());
        let em = BackupAuditEmitter::new(Some(sink.clone()), true);
        em.emit_drill("drill", 2400, 1800, false).unwrap();
        let v = sink.last().unwrap();
        assert_eq!(v["rto_within_target"], false);
        assert_eq!(v["success"], false);
    }
}
