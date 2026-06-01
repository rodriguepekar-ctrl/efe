//! Sprint **D17** — Backup + DR integration tests.
//!
//! These tests exercise the full pipeline using
//! [`efe_backup::MockSnapshotter`] + [`efe_backup::MockCommandRunner`].
//! They DO NOT need real pgBackRest / Vault / S3 / rclone binaries
//! installed.
//!
//! Cf. `docs/bench/d17_backup_dr_architecture.md` § 22.2.

use std::sync::Arc;

use chrono::Utc;
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

use efe_backup::audit::{AuditSink, BackupAuditEmitter};
use efe_backup::manifest::{
    sign_manifest_payload, BackupManifest, ManifestComponent, TrustStore, TrustStoreEntry,
};
use efe_backup::metadata::{BackupKind, SnapshotSource};
use efe_backup::snapshot::MockSnapshotter;
use efe_backup::verify::{run_suite, MockVerifier, Verifier, VerifierKind};
use efe_backup::{
    BackupAuditEmitter as _BackupAuditEmitterAlias,
    BackupConfig, BackupError, RestoreOrchestrator, RestoreTarget, RestoreTargetEnv, Snapshotter,
};

// ============================================================
// 1. End-to-end snapshot → manifest → restore round trip
// ============================================================

fn make_signed_manifest(
    env: RestoreTargetEnv,
    label: &str,
) -> (BackupManifest, Arc<TrustStore>, SigningKey) {
    let sk = SigningKey::generate(&mut OsRng);
    let vk = sk.verifying_key();
    let trust = TrustStore::from_entries(vec![TrustStoreEntry {
        key_id: "k1".into(),
        pubkey_hex: hex::encode(vk.to_bytes()),
        valid_from: Utc::now() - chrono::Duration::days(1),
        valid_until: None,
    }])
    .unwrap();
    let mut m = BackupManifest::draft(
        label,
        env,
        "efe-it",
        vec![ManifestComponent {
            source: "postgres".into(),
            path: "main/full/".into(),
            size_bytes: 100,
            sha256: "a".repeat(64),
        }],
    );
    sign_manifest_payload(&mut m, &sk, "k1").unwrap();
    (m, Arc::new(trust), sk)
}

fn make_orchestrator() -> (
    RestoreOrchestrator,
    Arc<MockSnapshotter>,
    Arc<MockSnapshotter>,
    Arc<MockSnapshotter>,
) {
    let pg = Arc::new(MockSnapshotter::new(SnapshotSource::Postgres));
    let vault = Arc::new(MockSnapshotter::new(SnapshotSource::Vault));
    let bundles = Arc::new(MockSnapshotter::new(SnapshotSource::Bundles));
    let audit = BackupAuditEmitter::new(None, false);
    let orch = RestoreOrchestrator::new(
        Arc::clone(&pg) as Arc<dyn Snapshotter>,
        Arc::clone(&vault) as Arc<dyn Snapshotter>,
        Arc::clone(&bundles) as Arc<dyn Snapshotter>,
        audit,
    );
    (orch, pg, vault, bundles)
}

#[tokio::test]
async fn end_to_end_snapshot_then_restore_round_trip() {
    // Orchestrate a full snapshot from each source, then sign a
    // manifest, then restore it. All happy path.
    let pg = MockSnapshotter::new(SnapshotSource::Postgres);
    let vault = MockSnapshotter::new(SnapshotSource::Vault);
    let bundles = MockSnapshotter::new(SnapshotSource::Bundles);

    let pg_meta = pg
        .snapshot("label-pg", BackupKind::Full)
        .await
        .unwrap();
    let vault_meta = vault
        .snapshot("label-vault", BackupKind::VaultSnapshot)
        .await
        .unwrap();
    let bundles_meta = bundles
        .snapshot("label-bundles", BackupKind::BundleSync)
        .await
        .unwrap();

    assert!(pg_meta.ok);
    assert!(vault_meta.ok);
    assert!(bundles_meta.ok);

    // Assemble + sign a manifest aggregating the 3 snapshots.
    let (manifest, trust, _sk) =
        make_signed_manifest(RestoreTargetEnv::Prod, "round-trip-label");
    let (orch, _pg, _vault, _bundles) = make_orchestrator();
    let orch = orch.with_trust_store(trust);
    let target = RestoreTarget::new("round-trip-label", RestoreTargetEnv::Prod);
    let outcome = orch.run(&manifest, &target).await.unwrap();
    assert_eq!(outcome.stages_ok, 3);
    assert_eq!(outcome.stages_failed, 0);
}

// ============================================================
// 2. Cross-env restore guard (T-DR-6 mitigation)
// ============================================================

#[tokio::test]
async fn cross_env_restore_refused_by_default() {
    let (orch, _pg, _vault, _bundles) = make_orchestrator();
    let (manifest, trust, _) =
        make_signed_manifest(RestoreTargetEnv::Prod, "cross-env-label");
    let orch = orch.with_trust_store(trust);
    let target = RestoreTarget::new("cross-env-label", RestoreTargetEnv::Staging);
    let err = orch.run(&manifest, &target).await.unwrap_err();
    assert!(matches!(err, BackupError::CrossEnvRefused(_)));
}

#[tokio::test]
async fn cross_env_restore_allowed_with_flag() {
    let (orch, _pg, _vault, _bundles) = make_orchestrator();
    let (manifest, trust, _) =
        make_signed_manifest(RestoreTargetEnv::Prod, "cross-env-label");
    let orch = orch.with_trust_store(trust);
    let mut target = RestoreTarget::new("cross-env-label", RestoreTargetEnv::Staging);
    target.force_cross_env = true;
    let outcome = orch.run(&manifest, &target).await.unwrap();
    assert_eq!(outcome.stages_ok, 3);
}

// ============================================================
// 3. First-stage failure aborts pipeline
// ============================================================

#[tokio::test]
async fn first_stage_failure_aborts_pipeline() {
    let (orch, pg, vault, bundles) = make_orchestrator();
    vault.expect_restore(Err(BackupError::Vault("vault outage".into())));
    let (manifest, trust, _) =
        make_signed_manifest(RestoreTargetEnv::Prod, "abort-label");
    let orch = orch.with_trust_store(trust);
    let target = RestoreTarget::new("abort-label", RestoreTargetEnv::Prod);
    let err = orch.run(&manifest, &target).await.unwrap_err();
    assert!(matches!(err, BackupError::Vault(_)));
    // Vault was attempted, the others were NOT.
    assert_eq!(vault.restore_calls().len(), 1);
    assert_eq!(pg.restore_calls().len(), 0);
    assert_eq!(bundles.restore_calls().len(), 0);
}

// ============================================================
// 4. Audit chain emission
// ============================================================

#[derive(Debug, Default)]
struct CapturingSink {
    entries: std::sync::Mutex<Vec<serde_json::Value>>,
}

impl AuditSink for CapturingSink {
    fn write_entry(&self, entry: serde_json::Value) {
        if let Ok(mut g) = self.entries.lock() {
            g.push(entry);
        }
    }
}

impl CapturingSink {
    fn entries(&self) -> Vec<serde_json::Value> {
        self.entries.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

#[tokio::test]
async fn audit_chain_emission_full_lifecycle() {
    let sink = Arc::new(CapturingSink::default());
    let audit = BackupAuditEmitter::new(Some(sink.clone()), true);

    // Snapshot
    audit
        .emit_snapshot(
            "label-a",
            SnapshotSource::Postgres,
            BackupKind::Full,
            Ok(()),
        )
        .unwrap();
    // Verify
    audit.emit_verify("label-a", Ok(())).unwrap();
    // Restore
    audit.emit_restore("label-a", "drill", Ok(())).unwrap();
    // Drill
    audit.emit_drill("drill-1", 1500, 1800, true).unwrap();
    // Delete (4-eyes)
    audit
        .emit_delete("label-a", "alice", "bob", "GDPR-erasure-T-XYZ")
        .unwrap();

    let events = sink.entries();
    assert_eq!(events.len(), 5);
    let kinds: Vec<&str> = events.iter().map(|e| e["event"].as_str().unwrap()).collect();
    assert_eq!(
        kinds,
        vec![
            "backup.snapshot",
            "backup.verify",
            "backup.restore",
            "backup.drill",
            "backup.delete",
        ]
    );
    // Per-event payload sanity checks.
    assert_eq!(events[0]["source"], "postgres");
    assert_eq!(events[0]["kind"], "full");
    assert_eq!(events[3]["rto_within_target"], true);
    assert_eq!(events[4]["primary_sub"], "alice");
    assert_eq!(events[4]["secondary_sub"], "bob");
}

#[tokio::test]
async fn audit_emitter_records_failure_with_metric_label() {
    let sink = Arc::new(CapturingSink::default());
    let audit = BackupAuditEmitter::new(Some(sink.clone()), true);
    let err = BackupError::Rclone("S3 5xx".into());
    audit
        .emit_snapshot(
            "label-b",
            SnapshotSource::Bundles,
            BackupKind::BundleSync,
            Err(&err),
        )
        .unwrap();
    let events = sink.entries();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["status"], "failed");
    assert_eq!(events[0]["error_kind"], "rclone_failed");
}

// ============================================================
// 5. Verifier suite — ordering + fail-fast
// ============================================================

#[tokio::test]
async fn verifier_suite_runs_all_5_kinds_in_order() {
    let suite: Vec<Arc<dyn Verifier>> = vec![
        Arc::new(MockVerifier::ok(VerifierKind::AuditChain)),
        Arc::new(MockVerifier::ok(VerifierKind::BundleSignatures)),
        Arc::new(MockVerifier::ok(VerifierKind::OidcIssuers)),
        Arc::new(MockVerifier::ok(VerifierKind::CatalogSmoke)),
        Arc::new(MockVerifier::ok(VerifierKind::ExecuteSmoke)),
    ];
    let agg = run_suite(&suite, "drill", false).await.unwrap();
    assert!(agg.all_ok());
    assert_eq!(agg.statuses.len(), 5);
    assert_eq!(agg.statuses[0].kind, VerifierKind::AuditChain);
    assert_eq!(agg.statuses[4].kind, VerifierKind::ExecuteSmoke);
}

#[tokio::test]
async fn verifier_suite_fail_fast_on_audit_chain_break() {
    let suite: Vec<Arc<dyn Verifier>> = vec![
        Arc::new(MockVerifier::anomaly(
            VerifierKind::AuditChain,
            "row 5042 hmac mismatch",
        )),
        Arc::new(MockVerifier::ok(VerifierKind::BundleSignatures)),
    ];
    let agg = run_suite(&suite, "drill", true).await.unwrap();
    assert!(!agg.all_ok());
    assert_eq!(agg.statuses.len(), 1);
    let f = agg.first_failure().unwrap();
    assert_eq!(f.kind, VerifierKind::AuditChain);
    assert!(f.detail.contains("hmac mismatch"));
}

// ============================================================
// 6. Manifest invariants & version pinning
// ============================================================

#[tokio::test]
async fn manifest_with_unknown_version_rejected() {
    let mut m = BackupManifest::draft(
        "x",
        RestoreTargetEnv::Dev,
        "c",
        vec![ManifestComponent {
            source: "postgres".into(),
            path: "p".into(),
            size_bytes: 1,
            sha256: "a".repeat(64),
        }],
    );
    m.version = "9.9.9".into();
    let err = m.check_invariants().unwrap_err();
    assert!(matches!(err, BackupError::UnknownManifestVersion { .. }));
}

#[tokio::test]
async fn manifest_with_empty_env_rejected() {
    let mut m = BackupManifest::draft(
        "x",
        RestoreTargetEnv::Dev,
        "c",
        vec![ManifestComponent {
            source: "postgres".into(),
            path: "p".into(),
            size_bytes: 1,
            sha256: "a".repeat(64),
        }],
    );
    m.env = String::new();
    let err = m.check_invariants().unwrap_err();
    assert!(matches!(err, BackupError::ManifestInvariant(_)));
}

// ============================================================
// 7. BackupConfig env parsing (full happy path)
// ============================================================

#[tokio::test]
async fn backup_config_from_env_full_happy_path() {
    let pairs: Vec<(&str, &str)> = vec![
        ("EFE_BACKUP_ENABLED", "true"),
        ("EFE_ENV", "prod"),
        ("EFE_BACKUP_STORAGE_BUCKET", "efe-backups-prod"),
        ("EFE_BACKUP_STORAGE_REGION", "eu-west-1"),
        ("EFE_BACKUP_RPO_TARGET_S", "300"),
        ("EFE_BACKUP_RTO_TARGET_S", "1800"),
        ("EFE_BACKUP_BUNDLES_SOURCE", "efe-bundles-prod"),
        ("EFE_BACKUP_BUNDLES_DEST", "efe-bundle-backups-prod"),
        ("EFE_BACKUP_CROSS_REGION", "true"),
        ("EFE_BACKUP_PG_STANZA", "main"),
        ("EFE_BACKUP_VAULT_ADDR", "https://vault.efe.svc:8200"),
    ];
    let lookup = move |key: &str| -> Result<String, std::env::VarError> {
        for (k, v) in &pairs {
            if *k == key {
                return Ok((*v).to_string());
            }
        }
        Err(std::env::VarError::NotPresent)
    };
    let cfg = BackupConfig::from_env_lookup(lookup).unwrap();
    assert!(cfg.enabled);
    assert_eq!(cfg.env, RestoreTargetEnv::Prod);
    assert_eq!(cfg.storage.bucket, "efe-backups-prod");
    assert_eq!(cfg.rpo_target_seconds, 300);
    assert_eq!(cfg.rto_target_seconds, 1800);
    assert!(cfg.cross_region);
}

// Re-export so unused-import doesn't fire.
#[allow(unused_imports)]
use _BackupAuditEmitterAlias as _;
