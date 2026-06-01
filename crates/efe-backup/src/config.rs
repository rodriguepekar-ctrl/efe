//! Sprint D17 — typed configuration for the backup orchestrator.
//!
//! Cf. `docs/bench/d17_backup_dr_architecture.md` § 9.2 (Helm
//! values type) + § 9.3 (ConfigMap rendering).
//!
//! Every `EFE_BACKUP_*` env var produced by the Helm
//! `templates/configmap.yaml` is consumed by [`BackupConfig::from_env`].
//! Defaults match the architecture-doc commitment (RPO 5 min, RTO
//! 30 min, retention full=4 weekly + diff=14 daily + WAL=14 daily).

use std::env;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::BackupError;

/// Resolved storage backend configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackupStorageConfig {
    /// Backend type ; only `"s3"` is implemented in v0.1.18.
    pub kind: String,
    /// Bucket name (e.g. `"efe-backups-prod"`).
    pub bucket: String,
    /// AWS region of the bucket (e.g. `"eu-west-1"`).
    pub region: String,
    /// Optional custom endpoint for S3-compatible stores like
    /// MinIO. If `None`, the AWS default endpoint is used.
    pub endpoint: Option<String>,
}

impl BackupStorageConfig {
    /// Default S3 prod-grade config used by tests / dev clusters.
    /// Production is expected to override every field.
    #[must_use]
    pub fn default_dev() -> Self {
        Self {
            kind: "s3".into(),
            bucket: "efe-backups-dev".into(),
            region: "eu-west-1".into(),
            endpoint: None,
        }
    }
}

/// Retention policy for each backup tier.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackupRetentionConfig {
    /// Number of weekly full backups to keep.
    pub full_weeks: u32,
    /// Number of daily diff backups to keep.
    pub diff_days: u32,
    /// Number of daily WAL archives to keep.
    pub wal_days: u32,
    /// Number of daily Vault snapshots to keep.
    pub vault_days: u32,
    /// Number of weekly bundle snapshots to keep.
    pub bundle_weeks: u32,
}

impl Default for BackupRetentionConfig {
    fn default() -> Self {
        Self {
            full_weeks: 4,
            diff_days: 14,
            wal_days: 14,
            vault_days: 30,
            bundle_weeks: 4,
        }
    }
}

/// Symbolic environment label used for cross-env restore guard
/// (T-DR-6 mitigation). Matches the `env=` field embedded in every
/// signed manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RestoreTargetEnv {
    /// Production environment ; restoring a non-prod backup here
    /// requires `--force-cross-env` + 4-eyes approval + DPO email.
    Prod,
    /// Staging environment.
    Staging,
    /// Ephemeral DR drill namespace (created weekly, destroyed at
    /// drill end).
    Drill,
    /// Free-form dev cluster ; cross-env restore guard relaxed.
    Dev,
}

impl RestoreTargetEnv {
    /// Stable string used in manifest `env` field + Prometheus
    /// labels.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Prod => "prod",
            Self::Staging => "staging",
            Self::Drill => "drill",
            Self::Dev => "dev",
        }
    }

    /// Parse from a free-form string (case-insensitive). Used by
    /// `BackupConfig::from_env` and the CLI `--target-env` arg.
    pub fn parse(s: &str) -> Result<Self, BackupError> {
        match s.to_ascii_lowercase().as_str() {
            "prod" | "production" => Ok(Self::Prod),
            "staging" | "stage" => Ok(Self::Staging),
            "drill" | "dr-drill" => Ok(Self::Drill),
            "dev" | "development" => Ok(Self::Dev),
            other => Err(BackupError::config(format!(
                "unknown restore target env: {other:?} (expected prod|staging|drill|dev)"
            ))),
        }
    }
}

/// Top-level backup orchestrator config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BackupConfig {
    /// `true` iff the operator opted in. When `false`, the
    /// orchestrator refuses every operation (defense in depth ;
    /// also prevents accidental ad-hoc backups in dev clusters).
    pub enabled: bool,

    /// Symbolic env label embedded in every manifest. Cross-env
    /// guard refuses restore when target_env != manifest.env.
    pub env: RestoreTargetEnv,

    /// Storage backend (S3 today).
    pub storage: BackupStorageConfig,

    /// Retention policy per tier.
    pub retention: BackupRetentionConfig,

    /// RPO target in seconds. Alert `D17BackupRpoViolation` fires
    /// on `efe_backup_rpo_seconds > rpo_target_seconds`.
    pub rpo_target_seconds: u64,

    /// RTO target in seconds. Used by the DR drill to assert the
    /// drill duration fits in the SLO budget.
    pub rto_target_seconds: u64,

    /// pgBackRest stanza name (typically `"main"`).
    pub pg_stanza: String,

    /// pgBackRest config file path (rendered by Helm).
    pub pg_config_path: String,

    /// Vault HTTP address (cluster-internal URL).
    pub vault_addr: String,

    /// Vault token file path (loaded from K8s Secret).
    pub vault_token_path: String,

    /// S3 bucket of the source bundles (D16) being backed up.
    pub bundles_source_bucket: String,

    /// S3 bucket of the backup destination (cross-region replicated).
    pub bundles_dest_bucket: String,

    /// Cross-region replication enabled for S3 backups.
    pub cross_region: bool,

    /// Whether backup operations chain into the D14-bis HMAC log.
    pub audit_chain_events: bool,

    /// Operation-level timeout for a single snapshot (default 30 min).
    pub snapshot_timeout: Duration,

    /// Operation-level timeout for a single restore (default 20 min).
    pub restore_timeout: Duration,
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            env: RestoreTargetEnv::Dev,
            storage: BackupStorageConfig::default_dev(),
            retention: BackupRetentionConfig::default(),
            rpo_target_seconds: 300, // 5 min
            rto_target_seconds: 1800, // 30 min
            pg_stanza: "main".into(),
            pg_config_path: "/etc/pgbackrest/pgbackrest.conf".into(),
            vault_addr: "https://vault.efe.svc.cluster.local:8200".into(),
            vault_token_path: "/var/run/secrets/efe/vault-token".into(),
            bundles_source_bucket: String::new(),
            bundles_dest_bucket: String::new(),
            cross_region: false,
            audit_chain_events: true,
            snapshot_timeout: Duration::from_secs(30 * 60),
            restore_timeout: Duration::from_secs(20 * 60),
        }
    }
}

impl BackupConfig {
    /// Parse from the process environment. Unknown / missing
    /// optional vars fall back to [`Default::default`].
    ///
    /// Variables consumed (cf. architecture § 9.3) :
    ///   * `EFE_BACKUP_ENABLED`
    ///   * `EFE_ENV` (re-used from the global EFE env tag)
    ///   * `EFE_BACKUP_STORAGE_TYPE`
    ///   * `EFE_BACKUP_STORAGE_BUCKET`
    ///   * `EFE_BACKUP_STORAGE_REGION`
    ///   * `EFE_BACKUP_STORAGE_ENDPOINT`
    ///   * `EFE_BACKUP_RPO_TARGET_S`
    ///   * `EFE_BACKUP_RTO_TARGET_S`
    ///   * `EFE_BACKUP_PG_STANZA`
    ///   * `EFE_BACKUP_PG_CONFIG_PATH`
    ///   * `EFE_BACKUP_VAULT_ADDR`
    ///   * `EFE_BACKUP_VAULT_TOKEN_PATH`
    ///   * `EFE_BACKUP_BUNDLES_SOURCE`
    ///   * `EFE_BACKUP_BUNDLES_DEST`
    ///   * `EFE_BACKUP_CROSS_REGION`
    ///   * `EFE_BACKUP_AUDIT_CHAIN_EVENTS`
    ///   * `EFE_BACKUP_SNAPSHOT_TIMEOUT_S`
    ///   * `EFE_BACKUP_RESTORE_TIMEOUT_S`
    pub fn from_env() -> Result<Self, BackupError> {
        Self::from_env_lookup(|k| env::var(k))
    }

    /// Pluggable env lookup for tests.
    pub fn from_env_lookup<F>(lookup: F) -> Result<Self, BackupError>
    where
        F: for<'a> Fn(&'a str) -> Result<String, env::VarError>,
    {
        let mut cfg = Self::default();

        if let Ok(v) = lookup("EFE_BACKUP_ENABLED") {
            cfg.enabled = parse_bool(&v).map_err(|_| {
                BackupError::config(format!("EFE_BACKUP_ENABLED invalid: {v:?}"))
            })?;
        }
        if let Ok(v) = lookup("EFE_ENV") {
            cfg.env = RestoreTargetEnv::parse(&v)?;
        }
        if let Ok(v) = lookup("EFE_BACKUP_STORAGE_TYPE") {
            cfg.storage.kind = v;
        }
        if let Ok(v) = lookup("EFE_BACKUP_STORAGE_BUCKET") {
            cfg.storage.bucket = v;
        }
        if let Ok(v) = lookup("EFE_BACKUP_STORAGE_REGION") {
            cfg.storage.region = v;
        }
        if let Ok(v) = lookup("EFE_BACKUP_STORAGE_ENDPOINT") {
            if !v.is_empty() {
                cfg.storage.endpoint = Some(v);
            }
        }
        if let Ok(v) = lookup("EFE_BACKUP_RPO_TARGET_S") {
            cfg.rpo_target_seconds = v.parse().map_err(|_| {
                BackupError::config(format!("EFE_BACKUP_RPO_TARGET_S invalid: {v:?}"))
            })?;
        }
        if let Ok(v) = lookup("EFE_BACKUP_RTO_TARGET_S") {
            cfg.rto_target_seconds = v.parse().map_err(|_| {
                BackupError::config(format!("EFE_BACKUP_RTO_TARGET_S invalid: {v:?}"))
            })?;
        }
        if let Ok(v) = lookup("EFE_BACKUP_PG_STANZA") {
            cfg.pg_stanza = v;
        }
        if let Ok(v) = lookup("EFE_BACKUP_PG_CONFIG_PATH") {
            cfg.pg_config_path = v;
        }
        if let Ok(v) = lookup("EFE_BACKUP_VAULT_ADDR") {
            cfg.vault_addr = v;
        }
        if let Ok(v) = lookup("EFE_BACKUP_VAULT_TOKEN_PATH") {
            cfg.vault_token_path = v;
        }
        if let Ok(v) = lookup("EFE_BACKUP_BUNDLES_SOURCE") {
            cfg.bundles_source_bucket = v;
        }
        if let Ok(v) = lookup("EFE_BACKUP_BUNDLES_DEST") {
            cfg.bundles_dest_bucket = v;
        }
        if let Ok(v) = lookup("EFE_BACKUP_CROSS_REGION") {
            cfg.cross_region = parse_bool(&v).map_err(|_| {
                BackupError::config(format!("EFE_BACKUP_CROSS_REGION invalid: {v:?}"))
            })?;
        }
        if let Ok(v) = lookup("EFE_BACKUP_AUDIT_CHAIN_EVENTS") {
            cfg.audit_chain_events = parse_bool(&v).map_err(|_| {
                BackupError::config(format!(
                    "EFE_BACKUP_AUDIT_CHAIN_EVENTS invalid: {v:?}"
                ))
            })?;
        }
        if let Ok(v) = lookup("EFE_BACKUP_SNAPSHOT_TIMEOUT_S") {
            let secs: u64 = v.parse().map_err(|_| {
                BackupError::config(format!("EFE_BACKUP_SNAPSHOT_TIMEOUT_S invalid: {v:?}"))
            })?;
            cfg.snapshot_timeout = Duration::from_secs(secs);
        }
        if let Ok(v) = lookup("EFE_BACKUP_RESTORE_TIMEOUT_S") {
            let secs: u64 = v.parse().map_err(|_| {
                BackupError::config(format!("EFE_BACKUP_RESTORE_TIMEOUT_S invalid: {v:?}"))
            })?;
            cfg.restore_timeout = Duration::from_secs(secs);
        }

        cfg.validate()?;
        Ok(cfg)
    }

    /// Internal-consistency checks. Called by [`Self::from_env_lookup`]
    /// and by the CLI before any operation.
    pub fn validate(&self) -> Result<(), BackupError> {
        if !self.enabled {
            // Validation is relaxed for the disabled path : the
            // operator may set EFE_BACKUP_ENABLED=false without
            // populating any of the dependent vars.
            return Ok(());
        }
        if self.storage.bucket.is_empty() {
            return Err(BackupError::config(
                "EFE_BACKUP_STORAGE_BUCKET is required when enabled=true",
            ));
        }
        if self.storage.region.is_empty() {
            return Err(BackupError::config(
                "EFE_BACKUP_STORAGE_REGION is required when enabled=true",
            ));
        }
        if self.rpo_target_seconds == 0 {
            return Err(BackupError::config("RPO target must be > 0"));
        }
        if self.rto_target_seconds == 0 {
            return Err(BackupError::config("RTO target must be > 0"));
        }
        if self.rpo_target_seconds >= self.rto_target_seconds {
            // RPO < RTO is the canonical inequality (you must be
            // able to detect data loss before you decide to bascule).
            return Err(BackupError::config(format!(
                "RPO ({} s) must be strictly < RTO ({} s)",
                self.rpo_target_seconds, self.rto_target_seconds
            )));
        }
        if self.bundles_source_bucket.is_empty() {
            return Err(BackupError::config(
                "EFE_BACKUP_BUNDLES_SOURCE is required when enabled=true",
            ));
        }
        if self.bundles_dest_bucket.is_empty() {
            return Err(BackupError::config(
                "EFE_BACKUP_BUNDLES_DEST is required when enabled=true",
            ));
        }
        Ok(())
    }
}

fn parse_bool(s: &str) -> Result<bool, ()> {
    match s.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "y" | "on" => Ok(true),
        "false" | "0" | "no" | "n" | "off" | "" => Ok(false),
        _ => Err(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::env::VarError;

    fn lookup_factory(
        map: HashMap<&'static str, &'static str>,
    ) -> impl for<'a> Fn(&'a str) -> Result<String, VarError> {
        move |k: &str| {
            map.get(k)
                .map(|s| (*s).to_string())
                .ok_or(VarError::NotPresent)
        }
    }

    #[test]
    fn default_is_disabled() {
        let cfg = BackupConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.rpo_target_seconds, 300);
        assert_eq!(cfg.rto_target_seconds, 1800);
        assert_eq!(cfg.env, RestoreTargetEnv::Dev);
    }

    #[test]
    fn restore_target_env_parse_canonical_forms() {
        assert_eq!(
            RestoreTargetEnv::parse("prod").unwrap(),
            RestoreTargetEnv::Prod
        );
        assert_eq!(
            RestoreTargetEnv::parse("PRODUCTION").unwrap(),
            RestoreTargetEnv::Prod
        );
        assert_eq!(
            RestoreTargetEnv::parse("staging").unwrap(),
            RestoreTargetEnv::Staging
        );
        assert_eq!(
            RestoreTargetEnv::parse("drill").unwrap(),
            RestoreTargetEnv::Drill
        );
        assert_eq!(
            RestoreTargetEnv::parse("dev").unwrap(),
            RestoreTargetEnv::Dev
        );
    }

    #[test]
    fn restore_target_env_parse_rejects_unknown() {
        let err = RestoreTargetEnv::parse("preprod").unwrap_err();
        assert!(matches!(err, BackupError::Config(_)));
    }

    #[test]
    fn from_env_with_defaults_returns_disabled_config() {
        let cfg = BackupConfig::from_env_lookup(lookup_factory(HashMap::new())).unwrap();
        assert!(!cfg.enabled);
    }

    #[test]
    fn from_env_overrides() {
        let mut m = HashMap::new();
        m.insert("EFE_BACKUP_ENABLED", "true");
        m.insert("EFE_ENV", "prod");
        m.insert("EFE_BACKUP_STORAGE_BUCKET", "efe-backups-prod");
        m.insert("EFE_BACKUP_STORAGE_REGION", "eu-west-3");
        m.insert("EFE_BACKUP_RPO_TARGET_S", "120");
        m.insert("EFE_BACKUP_RTO_TARGET_S", "900");
        m.insert("EFE_BACKUP_BUNDLES_SOURCE", "efe-bundles");
        m.insert("EFE_BACKUP_BUNDLES_DEST", "efe-bundle-backups");
        m.insert("EFE_BACKUP_CROSS_REGION", "true");
        let cfg = BackupConfig::from_env_lookup(lookup_factory(m)).unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.env, RestoreTargetEnv::Prod);
        assert_eq!(cfg.storage.region, "eu-west-3");
        assert_eq!(cfg.rpo_target_seconds, 120);
        assert_eq!(cfg.rto_target_seconds, 900);
        assert!(cfg.cross_region);
    }

    #[test]
    fn validate_rejects_enabled_without_bucket() {
        let mut m = HashMap::new();
        m.insert("EFE_BACKUP_ENABLED", "true");
        let err = BackupConfig::from_env_lookup(lookup_factory(m)).unwrap_err();
        assert!(matches!(err, BackupError::Config(_)));
    }

    #[test]
    fn validate_rejects_rpo_geq_rto() {
        let mut m = HashMap::new();
        m.insert("EFE_BACKUP_ENABLED", "true");
        m.insert("EFE_BACKUP_STORAGE_BUCKET", "b");
        m.insert("EFE_BACKUP_STORAGE_REGION", "r");
        m.insert("EFE_BACKUP_RPO_TARGET_S", "1800");
        m.insert("EFE_BACKUP_RTO_TARGET_S", "1800");
        m.insert("EFE_BACKUP_BUNDLES_SOURCE", "s");
        m.insert("EFE_BACKUP_BUNDLES_DEST", "d");
        let err = BackupConfig::from_env_lookup(lookup_factory(m)).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("RPO") && msg.contains("RTO"));
    }

    #[test]
    fn validate_rejects_invalid_bool() {
        let mut m = HashMap::new();
        m.insert("EFE_BACKUP_ENABLED", "banana");
        let err = BackupConfig::from_env_lookup(lookup_factory(m)).unwrap_err();
        assert!(matches!(err, BackupError::Config(_)));
    }

    #[test]
    fn parse_bool_canonical_forms() {
        assert!(parse_bool("true").unwrap());
        assert!(parse_bool("1").unwrap());
        assert!(parse_bool("YES").unwrap());
        assert!(!parse_bool("false").unwrap());
        assert!(!parse_bool("0").unwrap());
        assert!(!parse_bool("").unwrap());
    }

    #[test]
    fn target_env_str_round_trip() {
        for env in [
            RestoreTargetEnv::Prod,
            RestoreTargetEnv::Staging,
            RestoreTargetEnv::Drill,
            RestoreTargetEnv::Dev,
        ] {
            let s = env.as_str();
            let back = RestoreTargetEnv::parse(s).unwrap();
            assert_eq!(env, back);
        }
    }

    #[test]
    fn retention_default_matches_architecture() {
        let r = BackupRetentionConfig::default();
        assert_eq!(r.full_weeks, 4);
        assert_eq!(r.diff_days, 14);
        assert_eq!(r.wal_days, 14);
        assert_eq!(r.vault_days, 30);
        assert_eq!(r.bundle_weeks, 4);
    }
}
