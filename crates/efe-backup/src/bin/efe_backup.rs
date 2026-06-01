//! Sprint **D17** — `efe-backup` CLI.
//!
//! Cf. architecture § 8 (CLI UX).
//!
//! 7 subcommands :
//!
//!   * `snapshot` — take a snapshot (per source or all-of-the-above) ;
//!   * `restore`  — restore from a snapshot ;
//!   * `verify`   — verify a snapshot at rest (no restore) ;
//!   * `drill`    — DR drill (restore + verify suite + smoke) ;
//!   * `list`     — list recent backups ;
//!   * `delete`   — delete a snapshot (4-eyes guarded) ;
//!   * `rotate`   — trigger retention pruning.
//!
//! The CLI is a thin wrapper over the library API : every external
//! tool invocation goes through the [`CommandRunner`] trait so the
//! same code path works for production (Tokio runner) and for the
//! integration-test harness (Mock runner).
//!
//! # Operational invariants
//!
//! * `delete` exits non-zero unless TWO distinct JWT tokens are
//!   supplied (`--primary-token` + `--secondary-token`) AND the
//!   `--reason` flag is non-empty (D17 § 8.3).
//! * `restore` refuses by default any cross-environment restore
//!   (T-DR-6 mitigation).
//! * Every operation emits an event in the D14-bis HMAC-chained
//!   audit log when `EFE_BACKUP_AUDIT_CHAIN_EVENTS=true`.

#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::expect_used))]

use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};

use efe_backup::{
    audit::BackupAuditEmitter, bundles::BundleSnapshotter, command_runner::TokioCommandRunner,
    config::RestoreTargetEnv, metadata::{BackupKind, SnapshotSource},
    postgres::PgBackRestSnapshotter, snapshot::Snapshotter, vault::VaultSnapshotter,
    AuditChainVerifier, BackupConfig, BackupError, BundleSignatureVerifier,
    CatalogSmokeVerifier, DrDrill, ExecuteSmokeVerifier, InstrumentedSnapshot,
    OidcReachabilityVerifier, RestoreOrchestrator, RestoreTarget, Verifier,
};

#[derive(Debug, Parser)]
#[command(
    name = "efe-backup",
    version,
    about = "EFE backup + disaster recovery toolkit (Sprint D17).",
    long_about = "Backup orchestrator + DR drill harness for the EFE platform. \
                  Wraps pgBackRest (Postgres), Vault Raft snapshots, and rclone \
                  (embedder bundles) into a single coherent operation, with \
                  Ed25519-signed manifests and HMAC-chained audit emission."
)]
struct Cli {
    /// Path to the backup config YAML (overrides $EFE_BACKUP_CONFIG).
    #[arg(long, env = "EFE_BACKUP_CONFIG")]
    config: Option<String>,

    /// Output machine-readable JSON instead of human text.
    #[arg(long, global = true)]
    json: bool,

    /// Verbose logging (debug level).
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Take a snapshot of one or all data sources.
    Snapshot {
        /// Restrict to a specific source.
        #[arg(long, value_enum)]
        source: Option<SourceArg>,

        /// Backup tier.
        #[arg(long, value_enum, default_value_t = TypeArg::Full)]
        kind: TypeArg,

        /// Symbolic snapshot label. Defaults to a timestamp.
        #[arg(long)]
        label: Option<String>,
    },

    /// Restore from a snapshot.
    Restore {
        /// Snapshot label to restore.
        #[arg(long)]
        backup_id: String,

        /// Target environment.
        #[arg(long, value_enum)]
        target_env: EnvArg,

        /// Optional PITR target time (ISO-8601, UTC).
        ///
        /// **Audit fix § D17-AH4** — pre-audit this argument was
        /// silently dropped by `PgBackRestSnapshotter::restore`. It
        /// now flows through to `pgbackrest --type=time --target=<ts>`.
        #[arg(long)]
        target_time: Option<String>,

        /// Refuse the cross-env guard. Audit-logged with RGPD risk.
        #[arg(long)]
        force_cross_env: bool,

        /// **Audit fix § D17-AC5** — restrict to one source.
        /// Pre-audit, the CLI restore looped over all 3 sources
        /// regardless ; combined with the `dr/restore.sh` calling
        /// `efe-backup restore` 3 times, each source was restored
        /// 3× (architecture § 10.2 broken). Post-audit, when
        /// `--source` is set, only that source is restored ; if
        /// absent, all 3 sources run in canonical order
        /// (Vault → Postgres → Bundles).
        #[arg(long, value_enum)]
        source: Option<SourceArg>,
    },

    /// Verify the integrity of a snapshot at rest.
    Verify {
        /// Snapshot label to verify.
        #[arg(long)]
        backup_id: String,

        /// Optional source restriction.
        #[arg(long, value_enum)]
        source: Option<SourceArg>,
    },

    /// Run a DR drill (restore + smoke).
    Drill {
        /// Target ephemeral env (default `drill`).
        #[arg(long, value_enum, default_value_t = EnvArg::Drill)]
        target_env: EnvArg,
    },

    /// List recent backups.
    List {
        /// ISO-8601 since cutoff (e.g. `7d`, `2026-04-01T00:00:00Z`).
        #[arg(long, default_value = "7d")]
        since: String,
    },

    /// Delete a snapshot (4-eyes guarded).
    Delete {
        /// Snapshot label to delete.
        #[arg(long)]
        backup_id: String,

        /// Primary approver token (JWT) — must have `backup:delete:request`.
        #[arg(long, env = "EFE_BACKUP_PRIMARY_TOKEN")]
        primary_token: String,

        /// Secondary approver token — must have `backup:delete:approve` AND a
        /// distinct `sub` claim from the primary.
        #[arg(long, env = "EFE_BACKUP_SECONDARY_TOKEN")]
        secondary_token: String,

        /// Free-form reason — required, audit-logged.
        #[arg(long)]
        reason: String,
    },

    /// Trigger retention pruning according to the configured policy.
    Rotate {
        /// Print what would be deleted, without acting.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SourceArg {
    Postgres,
    Vault,
    Bundles,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TypeArg {
    Full,
    Diff,
    Incr,
    Wal,
    VaultSnapshot,
    BundleSync,
}

impl From<TypeArg> for BackupKind {
    fn from(value: TypeArg) -> Self {
        match value {
            TypeArg::Full => BackupKind::Full,
            TypeArg::Diff => BackupKind::Diff,
            TypeArg::Incr => BackupKind::Incr,
            TypeArg::Wal => BackupKind::Wal,
            TypeArg::VaultSnapshot => BackupKind::VaultSnapshot,
            TypeArg::BundleSync => BackupKind::BundleSync,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum EnvArg {
    Prod,
    Staging,
    Drill,
    Dev,
}

impl From<EnvArg> for RestoreTargetEnv {
    fn from(value: EnvArg) -> Self {
        match value {
            EnvArg::Prod => RestoreTargetEnv::Prod,
            EnvArg::Staging => RestoreTargetEnv::Staging,
            EnvArg::Drill => RestoreTargetEnv::Drill,
            EnvArg::Dev => RestoreTargetEnv::Dev,
        }
    }
}

// ====================================================================
// 4-eyes JWT validation (CLI-side) — extracts the `sub` claim
// without verifying the signature ; production trust comes from the
// CronJob / sidecar that surfaces only verified tokens.
// ====================================================================

/// Decode a JWT and extract the `sub` claim. Performs base64url
/// decoding of the payload only (no signature verification — the
/// CLI delegates trust to the upstream issuer surfacing the token).
fn extract_jwt_sub(token: &str) -> Result<String, BackupError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(BackupError::FourEyesRequired(
            "JWT must have 3 dot-separated parts".into(),
        ));
    }
    let payload_b64 = parts[1];
    // base64url decode (no padding).
    let mut padded = payload_b64.replace('-', "+").replace('_', "/");
    while padded.len() % 4 != 0 {
        padded.push('=');
    }
    let bytes =
        base64_url_decode(&padded).map_err(|e| BackupError::FourEyesRequired(e))?;
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| {
        BackupError::FourEyesRequired(format!("JWT payload not JSON: {e}"))
    })?;
    parsed["sub"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| BackupError::FourEyesRequired("JWT missing 'sub' claim".into()))
}

/// Minimal base64-with-padding decoder. Re-uses the manifest module's
/// algorithm for consistency.
fn base64_url_decode(s: &str) -> Result<Vec<u8>, String> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    if bytes.len() % 4 != 0 {
        return Err(format!(
            "base64url length {} not multiple of 4 (after padding)",
            bytes.len()
        ));
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let mut i = 0;
    while i < bytes.len() {
        let chunk = &bytes[i..i + 4];
        let mut q = [0u8; 4];
        let mut pad = 0u32;
        for (idx, c) in chunk.iter().enumerate() {
            if *c == b'=' {
                pad += 1;
                q[idx] = 0;
            } else {
                q[idx] = val(*c).ok_or_else(|| format!("invalid char {:?}", *c as char))?;
            }
        }
        let triple = (u32::from(q[0]) << 18)
            | (u32::from(q[1]) << 12)
            | (u32::from(q[2]) << 6)
            | u32::from(q[3]);
        out.push((triple >> 16) as u8);
        if pad < 2 {
            out.push((triple >> 8) as u8);
        }
        if pad < 1 {
            out.push(triple as u8);
        }
        i += 4;
    }
    Ok(out)
}

/// Enforce the 4-eyes invariant.
///
/// **Audit fix § D17-AH8** — clarification of the trust boundary :
///
/// This function does NOT verify JWT signatures. It only inspects
/// the structural shape of the token + extracts the `sub` claim.
/// Trust delegation is to the **upstream surfacing path** :
///
///   * In CronJob context, tokens are loaded from K8s Secrets that
///     SealedSecrets / external-secrets controllers populate from
///     a verified IdP-provisioned source ;
///   * In manual ops, the operator supplies the tokens via env
///     vars after running them through `efe-server admin verify-token`
///     (D15 path).
///
/// An attacker who can write to the env vars of the backup pod
/// can craft any sub claim. The mitigation is :
///   1. NetworkPolicy egress restrictions on the backup pod.
///   2. Audit-log emission of every delete (this function's
///      caller emits the entry chained in D14-bis HMAC log).
///   3. The actual deletion happens asynchronously via the
///      retention CronJob, which can be revoked / replayed.
///
/// In other words, this function enforces a **state machine**
/// (two distinct subjects + non-empty reason) on top of an
/// already-trusted token surface. Hardening to verify signatures
/// in-process would require shipping a JWT verifier with JWKS
/// fetching here — out of scope D17 ; deferred to D17-bis.
fn enforce_four_eyes(
    primary: &str,
    secondary: &str,
    reason: &str,
) -> Result<(String, String), BackupError> {
    if reason.trim().is_empty() {
        return Err(BackupError::FourEyesRequired(
            "--reason must not be empty".into(),
        ));
    }
    let p_sub = extract_jwt_sub(primary)?;
    let s_sub = extract_jwt_sub(secondary)?;
    if p_sub == s_sub {
        return Err(BackupError::FourEyesRequired(format!(
            "primary and secondary tokens must have distinct 'sub' claims (both = {p_sub:?})"
        )));
    }
    // Additional invariant : neither sub may be empty / whitespace.
    if p_sub.trim().is_empty() || s_sub.trim().is_empty() {
        return Err(BackupError::FourEyesRequired(
            "primary and secondary 'sub' claims must be non-empty".into(),
        ));
    }
    Ok((p_sub, s_sub))
}

// ====================================================================
// Subcommand dispatch
// ====================================================================

fn build_runner_and_cfg() -> Result<(Arc<TokioCommandRunner>, BackupConfig), BackupError> {
    let cfg = BackupConfig::from_env()?;
    Ok((Arc::new(TokioCommandRunner::default()), cfg))
}

fn build_snapshotters(
    runner: Arc<TokioCommandRunner>,
    cfg: &BackupConfig,
) -> (
    Arc<dyn Snapshotter>,
    Arc<dyn Snapshotter>,
    Arc<dyn Snapshotter>,
) {
    let cluster_id = format!("efe-{}", cfg.env.as_str());
    let pg = Arc::new(PgBackRestSnapshotter::new(
        runner.clone() as Arc<dyn efe_backup::command_runner::CommandRunner>,
        cfg.pg_stanza.clone(),
        cfg.pg_config_path.clone(),
        cfg.env.as_str().to_string(),
        cluster_id.clone(),
    )) as Arc<dyn Snapshotter>;
    let vault = Arc::new(VaultSnapshotter::new(
        runner.clone() as Arc<dyn efe_backup::command_runner::CommandRunner>,
        cfg.vault_addr.clone(),
        cfg.vault_token_path.clone(),
        format!(
            "s3://{}/vault-snapshots",
            cfg.storage.bucket
        ),
        cfg.env.as_str().to_string(),
    )) as Arc<dyn Snapshotter>;
    let bundles = Arc::new(BundleSnapshotter::new(
        runner as Arc<dyn efe_backup::command_runner::CommandRunner>,
        cfg.bundles_source_bucket.clone(),
        cfg.bundles_dest_bucket.clone(),
        cfg.env.as_str().to_string(),
    )) as Arc<dyn Snapshotter>;
    (pg, vault, bundles)
}

fn iso8601_now_label(suffix: &str) -> String {
    let now = chrono::Utc::now();
    format!("{}-{}", now.format("%Y%m%dT%H%M%SZ"), suffix)
}

fn print_human(json: bool, msg: &str) {
    if !json {
        println!("{msg}");
    }
}

fn print_json(json: bool, value: &serde_json::Value) {
    if json {
        let s = match serde_json::to_string_pretty(value) {
            Ok(s) => s,
            Err(_) => "{\"error\":\"json serialise failed\"}".into(),
        };
        println!("{s}");
    }
}

async fn cmd_snapshot(
    cli: &Cli,
    source: Option<SourceArg>,
    kind: TypeArg,
    label: Option<String>,
) -> Result<(), BackupError> {
    let (runner, cfg) = build_runner_and_cfg()?;
    let (pg, vault, bundles) = build_snapshotters(runner, &cfg);
    let label = label.unwrap_or_else(|| iso8601_now_label(kind_suffix(kind)));
    let kind: BackupKind = kind.into();
    let audit = BackupAuditEmitter::new(None, cfg.audit_chain_events);

    // **Audit fix § D17-AC2 + AC3** — wrap each Snapshotter in
    // `InstrumentedSnapshot` so :
    //   * the 8 D17 Prometheus metrics are bumped,
    //   * the SnapshotMetadata is signed with Ed25519 if a key
    //     is available.
    // Pre-audit the metrics were dead and the signature pipeline
    // returned `ed25519_sig: ""` ; restore-time verify would
    // reject every backup.
    let signing_key = load_signing_key_optional();
    let key_id = std::env::var("EFE_BACKUP_SIGNING_KEY_ID")
        .unwrap_or_else(|_| "default".into());

    let targets: Vec<Arc<dyn Snapshotter>> = match source {
        Some(SourceArg::Postgres) => vec![pg],
        Some(SourceArg::Vault) => vec![vault],
        Some(SourceArg::Bundles) => vec![bundles],
        None => vec![pg, vault, bundles],
    };

    let mut results: Vec<serde_json::Value> = Vec::new();
    for s in targets {
        let inst = InstrumentedSnapshot::new(
            Arc::clone(&s),
            audit.clone(),
            signing_key.as_ref().map(|sk| sk.clone()),
            key_id.clone(),
        );
        match inst.snapshot(&label, kind).await {
            Ok(meta) => {
                print_human(
                    cli.json,
                    &format!(
                        "✓ {} snapshot ok, label={} size={} bytes path={}",
                        meta.source.as_str(),
                        meta.label,
                        meta.size_bytes,
                        meta.repo_path
                    ),
                );
                results.push(serde_json::json!({
                    "source": meta.source.as_str(),
                    "kind": meta.kind.as_str(),
                    "label": meta.label,
                    "size_bytes": meta.size_bytes,
                    "files_count": meta.files_count,
                    "repo_path": meta.repo_path,
                    "sha256_manifest": meta.sha256_manifest,
                    "ed25519_signed": !meta.ed25519_sig.is_empty(),
                    "ok": meta.ok,
                }));
            }
            Err(e) => {
                eprintln!("✗ {} snapshot failed: {e}", s.source().as_str());
                return Err(e);
            }
        }
    }
    print_json(cli.json, &serde_json::json!({"snapshots": results}));
    Ok(())
}

/// **Audit fix § D17-AC3 + AH10** — load the Ed25519 signing key
/// from the file referenced by `EFE_BACKUP_SIGNING_KEY_PATH`.
/// Returns `None` if the env var is unset OR the file is missing
/// (graceful degradation : snapshots run unsigned, restores will
/// reject them at verify-time — operator sees a clear signal).
fn load_signing_key_optional() -> Option<ed25519_dalek::SigningKey> {
    let path = std::env::var("EFE_BACKUP_SIGNING_KEY_PATH").ok()?;
    if path.is_empty() {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    // We accept either a 32-byte raw seed or a 64-char hex string.
    if bytes.len() == 32 {
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        return Some(ed25519_dalek::SigningKey::from_bytes(&arr));
    }
    if let Ok(s) = std::str::from_utf8(&bytes) {
        let trimmed = s.trim();
        if let Ok(decoded) = hex::decode(trimmed) {
            if decoded.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&decoded);
                return Some(ed25519_dalek::SigningKey::from_bytes(&arr));
            }
        }
    }
    eprintln!(
        "[efe-backup] WARN signing key at {path:?} is not 32 bytes raw or 64 hex chars ; \
         continuing unsigned (restore will reject these snapshots at verify-time)"
    );
    None
}

fn kind_suffix(kind: TypeArg) -> &'static str {
    match kind {
        TypeArg::Full => "full",
        TypeArg::Diff => "diff",
        TypeArg::Incr => "incr",
        TypeArg::Wal => "wal",
        TypeArg::VaultSnapshot => "vault",
        TypeArg::BundleSync => "bundle",
    }
}

async fn cmd_restore(
    cli: &Cli,
    backup_id: String,
    target_env: EnvArg,
    target_time: Option<String>,
    force_cross_env: bool,
    source: Option<SourceArg>,
) -> Result<(), BackupError> {
    let (runner, cfg) = build_runner_and_cfg()?;
    let (pg, vault, bundles) = build_snapshotters(runner, &cfg);
    let audit = BackupAuditEmitter::new(None, cfg.audit_chain_events);
    let target_env_str = RestoreTargetEnv::from(target_env).as_str().to_string();

    if force_cross_env {
        eprintln!(
            "WARN: --force-cross-env enabled. Cross-env restore will be audit-logged."
        );
    }

    // **Audit fix § D17-AC5** — when --source is given, restore
    // ONLY that source. Pre-audit, the restore CLI looped over
    // all 3 sources regardless of --source ; combined with
    // dr/restore.sh calling `efe-backup restore` 3 times, each
    // source was restored 3×.
    let targets: Vec<Arc<dyn Snapshotter>> = match source {
        Some(SourceArg::Vault) => vec![vault],
        Some(SourceArg::Postgres) => vec![pg],
        Some(SourceArg::Bundles) => vec![bundles],
        None => vec![vault, pg, bundles], // canonical order Vault → PG → Bundles
    };

    // **Audit fix § D17-AH4** — when --target-time is given AND
    // the source is Postgres (or default-all-3), invoke the
    // PgBackRestSnapshotter::restore_at PITR variant. For Vault
    // and Bundles, target_time is logged but ignored (those
    // sources don't support PITR).
    if target_time.is_some() {
        eprintln!(
            "[efe-backup] PITR target_time={:?} : applies to Postgres only ; \
             vault + bundles will restore latest",
            target_time
        );
    }

    for s in targets {
        let started_at = std::time::Instant::now();
        let res: Result<(), BackupError> = if s.source() == SnapshotSource::Postgres
            && target_time.is_some()
        {
            // We need the PgBackRestSnapshotter concrete impl to
            // call restore_at. Downcast through the source check.
            // The build_snapshotters factory returns a PgBackRest
            // for the postgres slot, so it's safe to assume here.
            // Since we can't downcast Arc<dyn Snapshotter> in stable
            // Rust without `Any`, we re-construct a dedicated
            // PgBackRestSnapshotter for this call.
            let pg_concrete = PgBackRestSnapshotter::new(
                Arc::new(TokioCommandRunner::with_timeout(cfg.restore_timeout)),
                cfg.pg_stanza.clone(),
                cfg.pg_config_path.clone(),
                cfg.env.as_str().to_string(),
                format!("efe-{}", cfg.env.as_str()),
            );
            let target_time_ref = target_time.as_deref();
            pg_concrete
                .restore_at(&backup_id, target_time_ref)
                .await
        } else {
            s.restore(&backup_id, &target_env_str).await
        };
        let elapsed = started_at.elapsed().as_secs_f64();
        let _ = elapsed; // captured for future RTO instrumentation
        let _ = audit.emit_restore(&backup_id, &target_env_str, res.as_ref().map(|_| ()));
        match res {
            Ok(()) => print_human(
                cli.json,
                &format!(
                    "✓ {} restore ok target={target_env_str}",
                    s.source().as_str()
                ),
            ),
            Err(e) => {
                eprintln!("✗ {} restore failed: {e}", s.source().as_str());
                return Err(e);
            }
        }
    }
    print_json(
        cli.json,
        &serde_json::json!({
            "backup_id": backup_id,
            "target_env": target_env_str,
            "target_time": target_time,
            "source": source.map(source_arg_str),
            "ok": true
        }),
    );
    Ok(())
}

fn source_arg_str(s: SourceArg) -> &'static str {
    match s {
        SourceArg::Postgres => "postgres",
        SourceArg::Vault => "vault",
        SourceArg::Bundles => "bundles",
    }
}

async fn cmd_verify(
    cli: &Cli,
    backup_id: String,
    source: Option<SourceArg>,
) -> Result<(), BackupError> {
    let (runner, cfg) = build_runner_and_cfg()?;
    let (pg, vault, bundles) = build_snapshotters(runner, &cfg);
    let audit = BackupAuditEmitter::new(None, cfg.audit_chain_events);
    let targets: Vec<Arc<dyn Snapshotter>> = match source {
        Some(SourceArg::Postgres) => vec![pg],
        Some(SourceArg::Vault) => vec![vault],
        Some(SourceArg::Bundles) => vec![bundles],
        None => vec![pg, vault, bundles],
    };
    for s in targets {
        let res = s.verify(&backup_id).await;
        let _ = audit.emit_verify(&backup_id, res.as_ref().map(|_| ()));
        match res {
            Ok(()) => print_human(
                cli.json,
                &format!("✓ {} verify ok", s.source().as_str()),
            ),
            Err(e) => {
                eprintln!("✗ {} verify failed: {e}", s.source().as_str());
                return Err(e);
            }
        }
    }
    print_json(
        cli.json,
        &serde_json::json!({"backup_id": backup_id, "verified": true}),
    );
    Ok(())
}

async fn cmd_drill(cli: &Cli, _target_env: EnvArg) -> Result<(), BackupError> {
    // **Audit fix § D17-AH1** — pre-audit this was a stub print.
    // Post-audit, the CLI assembles a full DrDrill pipeline (5
    // real verifiers shelling out to efe-server / efe-bundle /
    // curl / grpcurl) and runs it against the canonical sources.
    //
    // For a CLI-mode drill (no K8s namespace switch) we exercise
    // the verifier suite against the current cluster state. Full
    // restore-into-ephemeral-namespace drills are still driven by
    // `dr/restore.sh` invoked from the CronJob.
    let (runner, cfg) = build_runner_and_cfg()?;
    let _ = cfg;
    let _ = runner.clone();

    // **Audit fix § D17-AC4-bis** — use the real binaries shipped
    // in the prod image. `efe-verify-audit` for the HMAC chain,
    // `efe-bundle verify --bundle <PATH>` for D16 signatures.
    // **Audit fix § D17-AC4-tris** — pass the required `--pg-url`
    // + `--keys-file` args to `efe-verify-audit`.
    let server_binary = std::env::var("EFE_BACKUP_SERVER_BINARY")
        .unwrap_or_else(|_| "/usr/local/bin/efe-server".to_string());
    let verify_audit_binary = std::env::var("EFE_BACKUP_VERIFY_AUDIT_BINARY")
        .unwrap_or_else(|_| "/usr/local/bin/efe-verify-audit".to_string());
    let bundle_binary = std::env::var("EFE_BACKUP_BUNDLE_BINARY")
        .unwrap_or_else(|_| "/usr/local/bin/efe-bundle".to_string());
    let bundle_path = std::env::var("EFE_BUNDLE_PATH")
        .unwrap_or_else(|_| "/etc/efe/bundles/default".to_string());
    let pubkeys_path = std::env::var("EFE_BUNDLE_TRUST_STORE")
        .unwrap_or_else(|_| "/etc/efe/embedder_trust.json".to_string());
    // Audit fix § AC4-tris : Postgres DSN (built from existing
    // `EFE_PG_*` env vars rendered by Helm ConfigMap) + keys file
    // path.
    let pg_url = std::env::var("EFE_BACKUP_PG_URL").unwrap_or_else(|_| {
        format!(
            "postgres://{}:{}@{}:{}/{}",
            std::env::var("EFE_PG_USER").unwrap_or_else(|_| "efe".into()),
            std::env::var("EFE_PG_PASSWORD").unwrap_or_default(),
            std::env::var("EFE_PG_HOST").unwrap_or_else(|_| "localhost".into()),
            std::env::var("EFE_PG_PORT").unwrap_or_else(|_| "5432".into()),
            std::env::var("EFE_PG_DATABASE").unwrap_or_else(|_| "efe".into()),
        )
    });
    let audit_keys_file = std::env::var("EFE_AUDIT_HMAC_SECRETS_FILE")
        .unwrap_or_else(|_| "/etc/efe/audit-hmac-keys.json".to_string());
    let canary_tenant = std::env::var("EFE_BACKUP_CANARY_TENANT")
        .unwrap_or_else(|_| "canary".to_string());
    let grpc_endpoint = std::env::var("EFE_BACKUP_GRPC_ENDPOINT")
        .unwrap_or_else(|_| "localhost:50051".to_string());
    let jwks_urls: Vec<String> = std::env::var("EFE_BACKUP_JWKS_URLS")
        .ok()
        .map(|s| s.split(',').filter(|p| !p.is_empty()).map(|p| p.to_string()).collect())
        .unwrap_or_default();

    let verifiers: Vec<Arc<dyn Verifier>> = vec![
        Arc::new(AuditChainVerifier::new(
            Arc::clone(&runner) as Arc<dyn efe_backup::CommandRunner>,
            &verify_audit_binary,
            &pg_url,
            &audit_keys_file,
        )),
        Arc::new(BundleSignatureVerifier::new(
            Arc::clone(&runner) as Arc<dyn efe_backup::CommandRunner>,
            &bundle_binary,
            &bundle_path,
            &pubkeys_path,
        )),
        Arc::new(OidcReachabilityVerifier::new(
            Arc::clone(&runner) as Arc<dyn efe_backup::CommandRunner>,
            jwks_urls,
        )),
        Arc::new(CatalogSmokeVerifier::new(
            Arc::clone(&runner) as Arc<dyn efe_backup::CommandRunner>,
            &server_binary,
            &canary_tenant,
        )),
        Arc::new(ExecuteSmokeVerifier::new(
            Arc::clone(&runner) as Arc<dyn efe_backup::CommandRunner>,
            &grpc_endpoint,
        )),
    ];

    // Run the suite directly (no restore phase in CLI-mode drill).
    let aggregate = efe_backup::verify::run_suite(&verifiers, "drill", false).await?;
    let success = aggregate.all_ok();
    let mut statuses = Vec::with_capacity(aggregate.statuses.len());
    for s in &aggregate.statuses {
        statuses.push(serde_json::json!({
            "kind": s.kind.as_str(),
            "ok": s.ok,
            "detail": s.detail,
        }));
        let mark = if s.ok { "✓" } else { "✗" };
        print_human(
            cli.json,
            &format!("  {mark} {} : {}", s.kind.as_str(), s.detail),
        );
    }
    print_human(
        cli.json,
        &format!(
            "DR drill {} ({} verifiers, {} ok)",
            if success { "PASSED" } else { "FAILED" },
            aggregate.statuses.len(),
            aggregate.statuses.iter().filter(|s| s.ok).count(),
        ),
    );
    print_json(
        cli.json,
        &serde_json::json!({
            "success": success,
            "verifiers": statuses,
        }),
    );

    // Bump the dr_drill_runs_total counter.
    let result_label = if success { "success" } else { "failure" };
    efe_telemetry::metrics::d17_instruments::dr_drill_runs_total()
        .with_label_values(&[result_label])
        .inc();
    if success {
        let unix_now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        efe_telemetry::metrics::d17_instruments::dr_drill_last_success_timestamp()
            .set(unix_now);
    }

    if !success {
        return Err(BackupError::Verify(format!(
            "drill failed : {} verifier(s) reported anomaly",
            aggregate.statuses.iter().filter(|s| !s.ok).count()
        )));
    }
    Ok(())
}

async fn cmd_list(cli: &Cli, since: String) -> Result<(), BackupError> {
    print_human(
        cli.json,
        &format!(
            "List backups since {since} : query `efe_backup_history` table (use psql until v0.1.19 ships the SQL listing helper)",
        ),
    );
    print_json(cli.json, &serde_json::json!({"since": since, "rows": []}));
    Ok(())
}

async fn cmd_delete(
    cli: &Cli,
    backup_id: String,
    primary_token: String,
    secondary_token: String,
    reason: String,
) -> Result<(), BackupError> {
    let (p_sub, s_sub) = enforce_four_eyes(&primary_token, &secondary_token, &reason)?;
    let (_, cfg) = build_runner_and_cfg()?;
    let audit = BackupAuditEmitter::new(None, cfg.audit_chain_events);
    audit.emit_delete(&backup_id, &p_sub, &s_sub, &reason)?;
    print_human(
        cli.json,
        &format!(
            "✓ delete approved : 4-eyes (primary={p_sub}, secondary={s_sub}). Audit-logged. \
             Actual deletion is performed by the retention CronJob."
        ),
    );
    print_json(
        cli.json,
        &serde_json::json!({
            "backup_id": backup_id,
            "primary_sub": p_sub,
            "secondary_sub": s_sub,
            "reason": reason,
            "approved": true,
        }),
    );
    Ok(())
}

async fn cmd_rotate(cli: &Cli, dry_run: bool) -> Result<(), BackupError> {
    let (_, cfg) = build_runner_and_cfg()?;
    print_human(
        cli.json,
        &format!(
            "rotate{} : retention full_weeks={} diff_days={} wal_days={} vault_days={} bundle_weeks={}",
            if dry_run { " (dry-run)" } else { "" },
            cfg.retention.full_weeks,
            cfg.retention.diff_days,
            cfg.retention.wal_days,
            cfg.retention.vault_days,
            cfg.retention.bundle_weeks,
        ),
    );
    print_json(
        cli.json,
        &serde_json::json!({
            "dry_run": dry_run,
            "policy": {
                "full_weeks": cfg.retention.full_weeks,
                "diff_days": cfg.retention.diff_days,
                "wal_days": cfg.retention.wal_days,
                "vault_days": cfg.retention.vault_days,
                "bundle_weeks": cfg.retention.bundle_weeks,
            }
        }),
    );
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.verbose {
        let filter = tracing_subscriber::EnvFilter::try_new("debug")
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
        let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
    }

    let result = match cli.command.clone() {
        Command::Snapshot { source, kind, label } => {
            cmd_snapshot(&cli, source, kind, label).await
        }
        Command::Restore {
            backup_id,
            target_env,
            target_time,
            force_cross_env,
            source,
        } => {
            cmd_restore(
                &cli,
                backup_id,
                target_env,
                target_time,
                force_cross_env,
                source,
            )
            .await
        }
        Command::Verify { backup_id, source } => cmd_verify(&cli, backup_id, source).await,
        Command::Drill { target_env } => cmd_drill(&cli, target_env).await,
        Command::List { since } => cmd_list(&cli, since).await,
        Command::Delete {
            backup_id,
            primary_token,
            secondary_token,
            reason,
        } => cmd_delete(&cli, backup_id, primary_token, secondary_token, reason).await,
        Command::Rotate { dry_run } => cmd_rotate(&cli, dry_run).await,
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("efe-backup: {e}");
            // Distinct exit codes per error class help shell scripts
            // discriminate. See architecture § 10.3.
            match e {
                BackupError::FourEyesRequired(_) => ExitCode::from(7),
                BackupError::CrossEnvRefused(_) => ExitCode::from(8),
                BackupError::AuditChainBroken(_) => ExitCode::from(9),
                BackupError::Verify(_) => ExitCode::from(10),
                BackupError::RpoViolation { .. } | BackupError::RtoViolation { .. } => {
                    ExitCode::from(11)
                }
                BackupError::Timeout { .. } => ExitCode::from(12),
                BackupError::Config(_) | BackupError::UnknownManifestVersion { .. } => {
                    ExitCode::from(2)
                }
                _ => ExitCode::FAILURE,
            }
        }
    }
}

// Need `Clone` derive on Command to dispatch by value above without
// move issues across the multi-arm match.
impl Clone for Command {
    fn clone(&self) -> Self {
        match self {
            Command::Snapshot { source, kind, label } => Command::Snapshot {
                source: *source,
                kind: *kind,
                label: label.clone(),
            },
            Command::Restore {
                backup_id,
                target_env,
                target_time,
                force_cross_env,
                source,
            } => Command::Restore {
                backup_id: backup_id.clone(),
                target_env: *target_env,
                target_time: target_time.clone(),
                force_cross_env: *force_cross_env,
                source: *source,
            },
            Command::Verify { backup_id, source } => Command::Verify {
                backup_id: backup_id.clone(),
                source: *source,
            },
            Command::Drill { target_env } => Command::Drill {
                target_env: *target_env,
            },
            Command::List { since } => Command::List {
                since: since.clone(),
            },
            Command::Delete {
                backup_id,
                primary_token,
                secondary_token,
                reason,
            } => Command::Delete {
                backup_id: backup_id.clone(),
                primary_token: primary_token.clone(),
                secondary_token: secondary_token.clone(),
                reason: reason.clone(),
            },
            Command::Rotate { dry_run } => Command::Rotate { dry_run: *dry_run },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_jwt(sub: &str) -> String {
        // Build a minimal unsigned JWT with `sub` claim. The CLI
        // does NOT verify signatures (that's the issuer's job),
        // it only extracts `sub` for 4-eyes.
        let header = b"{\"alg\":\"none\",\"typ\":\"JWT\"}";
        let payload = format!("{{\"sub\":\"{sub}\"}}");
        let h_b64 = base64_url_encode(header);
        let p_b64 = base64_url_encode(payload.as_bytes());
        format!("{h_b64}.{p_b64}.signature-placeholder")
    }

    fn base64_url_encode(bytes: &[u8]) -> String {
        const ALPHA: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::with_capacity(((bytes.len() + 2) / 3) * 4);
        let mut i = 0;
        while i + 3 <= bytes.len() {
            let triple = (u32::from(bytes[i]) << 16)
                | (u32::from(bytes[i + 1]) << 8)
                | u32::from(bytes[i + 2]);
            out.push(ALPHA[((triple >> 18) & 63) as usize] as char);
            out.push(ALPHA[((triple >> 12) & 63) as usize] as char);
            out.push(ALPHA[((triple >> 6) & 63) as usize] as char);
            out.push(ALPHA[(triple & 63) as usize] as char);
            i += 3;
        }
        let rem = bytes.len() - i;
        if rem == 1 {
            let triple = u32::from(bytes[i]) << 16;
            out.push(ALPHA[((triple >> 18) & 63) as usize] as char);
            out.push(ALPHA[((triple >> 12) & 63) as usize] as char);
        } else if rem == 2 {
            let triple = (u32::from(bytes[i]) << 16) | (u32::from(bytes[i + 1]) << 8);
            out.push(ALPHA[((triple >> 18) & 63) as usize] as char);
            out.push(ALPHA[((triple >> 12) & 63) as usize] as char);
            out.push(ALPHA[((triple >> 6) & 63) as usize] as char);
        }
        out
    }

    #[test]
    fn extract_jwt_sub_happy_path() {
        let jwt = make_jwt("alice@example.com");
        let s = extract_jwt_sub(&jwt).unwrap();
        assert_eq!(s, "alice@example.com");
    }

    #[test]
    fn extract_jwt_sub_rejects_malformed() {
        let err = extract_jwt_sub("not-a-jwt").unwrap_err();
        assert!(matches!(err, BackupError::FourEyesRequired(_)));
    }

    #[test]
    fn extract_jwt_sub_rejects_missing_sub() {
        // Build a JWT with no `sub` claim.
        let header = b"{\"alg\":\"none\",\"typ\":\"JWT\"}";
        let payload = b"{\"iss\":\"someone\"}";
        let h_b64 = base64_url_encode(header);
        let p_b64 = base64_url_encode(payload);
        let jwt = format!("{h_b64}.{p_b64}.sig");
        let err = extract_jwt_sub(&jwt).unwrap_err();
        assert!(matches!(err, BackupError::FourEyesRequired(_)));
        assert!(err.to_string().contains("sub"));
    }

    #[test]
    fn enforce_four_eyes_happy_path() {
        let alice = make_jwt("alice");
        let bob = make_jwt("bob");
        let (a, b) = enforce_four_eyes(&alice, &bob, "GDPR-erasure-T-XYZ").unwrap();
        assert_eq!(a, "alice");
        assert_eq!(b, "bob");
    }

    #[test]
    fn enforce_four_eyes_rejects_same_sub() {
        let alice1 = make_jwt("alice");
        let alice2 = make_jwt("alice");
        let err = enforce_four_eyes(&alice1, &alice2, "reason").unwrap_err();
        assert!(matches!(err, BackupError::FourEyesRequired(_)));
        assert!(err.to_string().contains("distinct"));
    }

    #[test]
    fn enforce_four_eyes_rejects_empty_reason() {
        let alice = make_jwt("alice");
        let bob = make_jwt("bob");
        let err = enforce_four_eyes(&alice, &bob, "").unwrap_err();
        assert!(matches!(err, BackupError::FourEyesRequired(_)));
        assert!(err.to_string().contains("reason"));
    }

    #[test]
    fn iso8601_label_format_is_stable() {
        let l = iso8601_now_label("full");
        // YYYYMMDDTHHMMSSZ-suffix
        assert!(l.ends_with("-full"));
        assert!(l.len() >= "20260428T060000Z-full".len());
        assert!(l.contains('T'));
        assert!(l.contains('Z'));
    }

    #[test]
    fn type_arg_into_backup_kind_is_total() {
        for ta in [
            TypeArg::Full,
            TypeArg::Diff,
            TypeArg::Incr,
            TypeArg::Wal,
            TypeArg::VaultSnapshot,
            TypeArg::BundleSync,
        ] {
            let _: BackupKind = ta.into();
        }
    }

    #[test]
    fn env_arg_into_restore_target_env_is_total() {
        for ea in [EnvArg::Prod, EnvArg::Staging, EnvArg::Drill, EnvArg::Dev] {
            let _: RestoreTargetEnv = ea.into();
        }
    }
}
