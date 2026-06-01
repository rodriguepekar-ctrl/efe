//! Sprint D17 — snapshot metadata types shared between the
//! [`crate::Snapshotter`] trait, the [`crate::RestoreOrchestrator`],
//! and the `efe_backup_history` SQL row.
//!
//! Cf. architecture § 6.2 (`efe_backup_history` schema).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Symbolic data source category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SnapshotSource {
    /// Postgres database snapshot via pgBackRest (base or WAL).
    Postgres,
    /// Vault Raft snapshot (HMAC keys + OIDC config).
    Vault,
    /// rclone S3→S3 sync of the embedder bundles.
    Bundles,
}

impl SnapshotSource {
    /// Stable string used in metric labels + manifest fields.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Postgres => "postgres",
            Self::Vault => "vault",
            Self::Bundles => "bundles",
        }
    }

    /// Iterator over all variants — used by the orchestrator that
    /// runs every snapshotter in turn.
    pub fn all() -> impl Iterator<Item = Self> {
        [Self::Postgres, Self::Vault, Self::Bundles].into_iter()
    }
}

/// Backup tier (full / diff / incremental / WAL / vault-snapshot /
/// bundle-sync).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupKind {
    /// pgBackRest full base backup.
    Full,
    /// pgBackRest differential.
    Diff,
    /// pgBackRest incremental.
    Incr,
    /// WAL push (continuous archive).
    Wal,
    /// Vault `operator raft snapshot save`.
    VaultSnapshot,
    /// rclone bundles sync.
    BundleSync,
}

impl BackupKind {
    /// Stable string used in metric labels.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Diff => "diff",
            Self::Incr => "incr",
            Self::Wal => "wal",
            Self::VaultSnapshot => "vault_snapshot",
            Self::BundleSync => "bundle_sync",
        }
    }

    /// `true` iff this kind is considered a *base backup* eligible
    /// for the staleness alert (`D17BackupStale`). Diff + WAL are
    /// not because they are useless without a full.
    #[must_use]
    pub fn is_base_backup(&self) -> bool {
        matches!(self, Self::Full | Self::VaultSnapshot | Self::BundleSync)
    }
}

/// Full metadata describing one snapshot. Shared between the
/// snapshotter return value, the restore orchestrator input, and
/// the SQL row in `efe_backup_history`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    /// Symbolic snapshot label, e.g. `"20260428T060000Z-full"`.
    pub label: String,
    /// Source category.
    pub source: SnapshotSource,
    /// Backup kind.
    pub kind: BackupKind,
    /// Storage repo path, e.g. `"s3://efe-backups-prod/main/full/20260428T060000Z"`.
    pub repo_path: String,
    /// Total bytes reported by the underlying tool.
    pub size_bytes: u64,
    /// Number of files / objects counted by the underlying tool.
    pub files_count: u32,
    /// Hex SHA-256 of the manifest file produced for this snapshot.
    pub sha256_manifest: String,
    /// Detached Ed25519 signature of the manifest, base64-encoded.
    pub ed25519_sig: String,
    /// Wall-clock timestamp when the snapshot started.
    pub started_at: DateTime<Utc>,
    /// Wall-clock timestamp when the snapshot completed (success
    /// or failure).
    pub finished_at: DateTime<Utc>,
    /// Postgres LSN at start (None for non-postgres sources).
    pub pg_lsn_start: Option<String>,
    /// Postgres LSN at stop (None for non-postgres sources).
    pub pg_lsn_stop: Option<String>,
    /// `true` iff the snapshot completed without error.
    pub ok: bool,
    /// Error message if `ok = false` ; sanitised to avoid leaking
    /// sensitive data into the SQL row.
    pub error_message: Option<String>,
}

impl SnapshotMetadata {
    /// Convenience builder for tests + ad-hoc construction.
    pub fn ok_now(
        label: impl Into<String>,
        source: SnapshotSource,
        kind: BackupKind,
        repo_path: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            label: label.into(),
            source,
            kind,
            repo_path: repo_path.into(),
            size_bytes: 0,
            files_count: 0,
            sha256_manifest: "0".repeat(64),
            ed25519_sig: String::new(),
            started_at: now,
            finished_at: now,
            pg_lsn_start: None,
            pg_lsn_stop: None,
            ok: true,
            error_message: None,
        }
    }

    /// Duration of the snapshot in seconds.
    #[must_use]
    pub fn duration_seconds(&self) -> f64 {
        let delta = self.finished_at - self.started_at;
        delta.num_milliseconds().max(0) as f64 / 1000.0
    }

    /// **Audit fix § D17-AC3** — sign the snapshot metadata
    /// canonical bytes with `signing_key`, populating `sha256_manifest`
    /// + `ed25519_sig`. The signed metadata is what gets persisted
    /// to `efe_backup_history` and what the post-restore verifier
    /// re-checks against the trust store.
    ///
    /// Pre-audit, snapshotter impls returned metadata with empty
    /// `ed25519_sig` and a placeholder `sha256_manifest = "0"*64`.
    /// Restore-time signature verification would then reject every
    /// backup as "tampered" — the signing pipeline was broken.
    ///
    /// The signature covers a canonical JSON form of `Self` with
    /// the `sha256_manifest`, `ed25519_sig` and `error_message`
    /// fields blanked, in the same way [`crate::manifest::sign_manifest_payload`]
    /// signs the `BackupManifest` body.
    pub fn sign(
        &mut self,
        signing_key: &ed25519_dalek::SigningKey,
    ) -> Result<(), crate::error::BackupError> {
        use ed25519_dalek::Signer;
        use sha2::Digest;

        // Canonical body : copy with signature fields blanked.
        let mut canon = self.clone();
        canon.sha256_manifest = String::new();
        canon.ed25519_sig = String::new();
        // The error_message is environment-dependent (timestamps,
        // hostnames, etc.) — exclude from the signed canonical
        // form so the signature is reproducible.
        canon.error_message = None;
        let body = serde_json::to_vec(&canon)?;

        let mut hasher = sha2::Sha256::new();
        hasher.update(&body);
        let digest = hasher.finalize();
        let sha_hex = hex::encode(digest);
        let sig: ed25519_dalek::Signature = signing_key.sign(&body);
        let sig_b64 = crate::manifest::base64_encode_signature(&sig.to_bytes());

        self.sha256_manifest = sha_hex;
        self.ed25519_sig = sig_b64;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_source_round_trip_string() {
        for s in [
            SnapshotSource::Postgres,
            SnapshotSource::Vault,
            SnapshotSource::Bundles,
        ] {
            let back: SnapshotSource =
                serde_json::from_value(serde_json::json!(s.as_str())).unwrap();
            assert_eq!(back, s);
        }
    }

    #[test]
    fn snapshot_source_all_lists_all_variants() {
        let all: Vec<_> = SnapshotSource::all().collect();
        assert_eq!(all.len(), 3);
        assert!(all.contains(&SnapshotSource::Postgres));
        assert!(all.contains(&SnapshotSource::Vault));
        assert!(all.contains(&SnapshotSource::Bundles));
    }

    #[test]
    fn backup_kind_is_base_backup() {
        assert!(BackupKind::Full.is_base_backup());
        assert!(BackupKind::VaultSnapshot.is_base_backup());
        assert!(BackupKind::BundleSync.is_base_backup());
        assert!(!BackupKind::Diff.is_base_backup());
        assert!(!BackupKind::Incr.is_base_backup());
        assert!(!BackupKind::Wal.is_base_backup());
    }

    #[test]
    fn snapshot_metadata_ok_now_has_consistent_timestamps() {
        let m = SnapshotMetadata::ok_now(
            "label",
            SnapshotSource::Postgres,
            BackupKind::Full,
            "s3://b",
        );
        assert!(m.ok);
        assert!(m.error_message.is_none());
        // started_at and finished_at are the same instant in this
        // builder.
        assert_eq!(m.started_at, m.finished_at);
        assert_eq!(m.duration_seconds(), 0.0);
    }
}
