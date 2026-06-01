//! Sprint **D17** — Backup + Disaster Recovery orchestration crate.
//!
//! This crate is the implementation companion of
//! `docs/bench/d17_backup_dr_architecture.md`. It ships :
//!
//!   * a `Snapshotter` trait + 3 production impls
//!     (`PgBackRestSnapshotter`, `VaultSnapshotter`,
//!     `BundleSnapshotter`) plus mock impls for tests ;
//!   * a `RestoreOrchestrator` that re-runs each snapshot in the
//!     correct order (Vault → Postgres → bundles → smoke verify) ;
//!   * a `Verifier` trait + 5 post-restore checks
//!     (HMAC chain integrity D14-bis, bundle signatures D16, OIDC
//!     reachability D15, catalog smoke, Execute smoke) ;
//!   * a `DrDrill` orchestrator that runs the full restore cycle
//!     in an ephemeral namespace + measures RTO ;
//!   * an `AuditEmitter` wrapper that chains every backup
//!     operation into the D14-bis HMAC log ;
//!   * a `BackupConfig` with `from_env()` parsing of every
//!     `EFE_BACKUP_*` variable rendered by the Helm chart.
//!
//! The CLI lives at `src/bin/efe_backup.rs` and provides 7
//! subcommands : `snapshot`, `restore`, `verify`, `drill`, `list`,
//! `delete` (4-eyes guarded), `rotate`.
//!
//! # Operational status
//!
//! v0.1.18 ships with the **library + CLI architecture complete**
//! and **unit/integration tests fully passing**. The shell-out to
//! the actual external tools (pgBackRest, Vault, rclone) is wrapped
//! by a `CommandRunner` trait : production uses the
//! `TokioCommandRunner` which spawns real processes ; tests use
//! `MockCommandRunner` which returns canned responses. End-to-end
//! integration with the real binaries is exercised via the
//! `dr/restore.sh` runbook and the weekly DR drill CronJob (see
//! Helm templates).

#![warn(missing_docs)]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::expect_used))]

pub mod audit;
pub mod bundles;
pub mod command_runner;
pub mod config;
pub mod drill;
pub mod error;
pub mod manifest;
pub mod metadata;
pub mod postgres;
pub mod restore;
/// **Audit fix § AC2 + AC3** — runtime instrumentation wrapper
/// (metrics + manifest signing + audit emission) for Snapshotter
/// calls.
pub mod runtime_instrument;
pub mod snapshot;
pub mod vault;
pub mod verifiers;
pub mod verify;

pub use audit::BackupAuditEmitter;
pub use command_runner::{CommandOutput, CommandRunner, MockCommandRunner, TokioCommandRunner};
pub use config::{BackupConfig, BackupRetentionConfig, BackupStorageConfig, RestoreTargetEnv};
pub use drill::{DrDrill, DrDrillOutcome};
pub use error::BackupError;
pub use manifest::{
    sign_manifest_payload, verify_manifest_payload, verify_snapshot_metadata, BackupManifest,
    ManifestComponent, TrustStore,
};
pub use metadata::{BackupKind, SnapshotMetadata, SnapshotSource};
pub use restore::{RestoreOrchestrator, RestoreOutcome, RestoreTarget};
pub use runtime_instrument::InstrumentedSnapshot;
pub use snapshot::{Snapshotter, MockSnapshotter};
pub use verifiers::{
    AuditChainVerifier, BundleSignatureVerifier, CatalogSmokeVerifier, ExecuteSmokeVerifier,
    OidcReachabilityVerifier,
};
pub use verify::{VerifierKind, VerifierStatus, VerifyAggregate, VerifyError, Verifier};

/// Schema version of the `BackupManifest` JSON. Pinned because the
/// architecture § 27 Anti-corruption clause requires a strict
/// reader that refuses unknown manifest versions.
pub const MANIFEST_VERSION: &str = "1.0.0";
