//! **Sprint 8 — N11** : configuration for the Firecracker pool.
//!
//! Three layers :
//! - [`FirecrackerPoolConfig`] : pool-wide settings.
//! - [`KindConfig`] : per-kind / per-model parameters.
//! - [`SubPoolBudget`] : per-(kind, model, tenant_class) sizing.
//!
//! All configs are deserializable from JSON / TOML so the operator can
//! ship them via the Helm chart `values.yaml` without recompiling.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use super::super::VmKind;
use super::types::SubPoolKey;
#[cfg(test)]
use super::types::TenantClass;

// ─────────────────────────────────────────────────────────────────────
//  Errors
// ─────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid config : {0}")]
    Invalid(String),
    #[error("env var missing : {0}")]
    EnvMissing(String),
    #[error("env var unparseable : {var} : {reason}")]
    EnvUnparseable { var: String, reason: String },
    #[error("file read error : {0}")]
    FileRead(String),
    #[error("file parse error : {0}")]
    FileParse(String),
}

// ─────────────────────────────────────────────────────────────────────
//  FirecrackerPoolConfig
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : top-level configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirecrackerPoolConfig {
    /// Path to the Firecracker binary.  Linux-only ; ignored by
    /// `MockLifecycle`.
    pub binary_path: PathBuf,

    /// Path to a Linux kernel image suitable for Firecracker.  Linux-only.
    pub kernel_image: PathBuf,

    /// Root directory of cgroups v2.  Default `/sys/fs/cgroup/efe-nano`.
    pub cgroup_root: PathBuf,

    /// Directory for the per-VM Firecracker API sockets.
    /// Default `/var/run/efe-nano/sockets/`.
    pub api_socket_dir: PathBuf,

    /// Maximum time the lifecycle controller waits for a VM's vsock
    /// to come online after spawn.  Past this → `ProvisionFailed`.
    /// Default 500 ms.
    pub boot_budget: Duration,

    /// How often the reaper sweeps the pool.  Default 10 s.
    pub reaper_interval: Duration,

    /// Max time a single vsock RPC may take (request + response).
    /// Default 30 s.
    pub vsock_timeout: Duration,

    /// VMs idle longer than this are eligible for LRU eviction
    /// (down to `min_warm` floor).  Default 5 min.
    pub idle_eviction_after: Duration,

    /// Max VMs the pool will provision across ALL sub-pools.
    /// Acts as a hard ceiling against runaway allocation.
    /// Default 500.
    pub max_total_vms: u32,

    /// Per `(VmKind, model_id, model_version)` configuration.
    /// Keyed by a flat string so JSON round-trip is simple.
    pub per_kind: Vec<KindConfig>,

    /// Per-tenant-class sizing overrides.
    /// `None` ⇒ use the kind's defaults for every tenant_class.
    #[serde(default)]
    pub tenant_class_overrides: HashMap<String, SubPoolBudget>,
}

impl FirecrackerPoolConfig {
    /// Construct from environment variables.  Used at runtime when the
    /// operator wants to override Helm defaults via `kubectl set env`.
    pub fn from_env() -> Result<Self, ConfigError> {
        // Sprint 8 ship : the per-kind configs and tenant overrides are
        // expected to come from a JSON file pointed to by env, NOT from
        // individual env vars (would explode the surface).  This keeps
        // the env-var contract small.
        let cfg_path = std::env::var("EFE_NANO_FIRECRACKER_CONFIG")
            .map_err(|_| ConfigError::EnvMissing("EFE_NANO_FIRECRACKER_CONFIG".to_string()))?;
        let bytes = std::fs::read(&cfg_path)
            .map_err(|e| ConfigError::FileRead(format!("{cfg_path}: {e}")))?;
        let cfg: FirecrackerPoolConfig = serde_json::from_slice(&bytes)
            .map_err(|e| ConfigError::FileParse(format!("{cfg_path}: {e}")))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Reasonable defaults for unit testing — no real Firecracker.
    pub fn for_testing() -> Self {
        Self {
            binary_path: PathBuf::from("/dev/null/firecracker"),
            kernel_image: PathBuf::from("/dev/null/vmlinux"),
            cgroup_root: PathBuf::from("/dev/null/cgroup"),
            api_socket_dir: PathBuf::from("/tmp/efe-test-sockets"),
            boot_budget: Duration::from_millis(500),
            reaper_interval: Duration::from_secs(1),
            vsock_timeout: Duration::from_secs(5),
            idle_eviction_after: Duration::from_secs(300),
            max_total_vms: 100,
            per_kind: vec![
                KindConfig::testing_mode_b("test_model", "1.0.0"),
                KindConfig::testing_mode_c(),
            ],
            tenant_class_overrides: HashMap::new(),
        }
    }

    /// Validate the config — called at load time.  Catches typos +
    /// budget mistakes before they cause runtime ProvisionFailed loops.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.boot_budget < Duration::from_millis(100) {
            return Err(ConfigError::Invalid(format!(
                "boot_budget too small : {:?} (min 100ms)",
                self.boot_budget
            )));
        }
        if self.reaper_interval < Duration::from_millis(500) {
            return Err(ConfigError::Invalid(format!(
                "reaper_interval too small : {:?} (min 500ms)",
                self.reaper_interval
            )));
        }
        if self.vsock_timeout < Duration::from_millis(100) {
            return Err(ConfigError::Invalid(format!(
                "vsock_timeout too small : {:?} (min 100ms)",
                self.vsock_timeout
            )));
        }
        if self.max_total_vms == 0 {
            return Err(ConfigError::Invalid("max_total_vms must be ≥ 1".into()));
        }
        if self.per_kind.is_empty() {
            return Err(ConfigError::Invalid(
                "at least one KindConfig required".into(),
            ));
        }
        // Validate each KindConfig + total budget vs cap.
        let mut total_min_warm: u32 = 0;
        let mut total_max: u32 = 0;
        for kc in &self.per_kind {
            kc.validate()?;
            total_min_warm = total_min_warm.saturating_add(kc.min_warm);
            total_max = total_max.saturating_add(kc.max_total);
        }
        if total_min_warm > self.max_total_vms {
            return Err(ConfigError::Invalid(format!(
                "sum of min_warm ({total_min_warm}) exceeds max_total_vms ({})",
                self.max_total_vms
            )));
        }
        if total_max > self.max_total_vms.saturating_mul(2) {
            // Allow over-subscription up to 2x (warm + in-flight) but flag
            // if higher — likely a misconfig.
            tracing::warn!(
                target: "efe_nano::firecracker",
                total_max_per_kind = total_max,
                pool_max = self.max_total_vms,
                "per-kind max_total sum > 2× pool max ; oversubscription"
            );
        }
        Ok(())
    }

    /// Look up the config for a specific sub-pool key.  Falls back to
    /// `per_kind` then to `KindConfig::default_for(kind)`.
    pub fn for_subpool(&self, key: &SubPoolKey) -> KindConfig {
        for kc in &self.per_kind {
            if kc.matches(key) {
                let mut out = kc.clone();
                if let Some(ovr) = self
                    .tenant_class_overrides
                    .get(key.tenant_class.as_str())
                {
                    out.apply_budget(ovr);
                }
                return out;
            }
        }
        KindConfig::default_for(key)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  KindConfig
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : per-(kind, model, version) parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KindConfig {
    pub kind: VmKind,
    /// Mode B only : the model the VM is pre-baked with.  Mode C uses
    /// the empty string.
    pub model_id: String,
    pub model_version: String,

    /// Path to the rootfs ext4 image for this kind.  Linux-only ; ignored
    /// by `MockLifecycle`.
    pub rootfs_path: PathBuf,

    /// Target floor of warm VMs maintained by the reaper.
    pub min_warm: u32,

    /// Hard ceiling : never have more than `max_total` VMs (warm +
    /// in-use) for this kind.
    pub max_total: u32,

    /// Memory cap per VM (megabytes).  Enforced via cgroup memory.max.
    pub memory_mb: u32,

    /// vCPU count per VM.
    pub vcpus: u32,

    /// cgroup cpu.max quota in microseconds per period.  `1_000_000` =
    /// 1 full core ; `500_000` = 0.5 core.
    pub cgroup_cpu_quota_us: i64,

    /// cgroup memory.high (soft limit, bytes).
    pub cgroup_memory_high: u64,

    /// cgroup memory.max (hard limit, bytes).  VM is OOM-killed past this.
    pub cgroup_memory_max: u64,

    /// cgroup io.weight (1..1000) — weighted share of block I/O bandwidth.
    pub cgroup_blkio_weight: u64,
}

impl KindConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.max_total == 0 {
            return Err(ConfigError::Invalid(format!(
                "KindConfig {}/{} max_total=0",
                self.model_id, self.model_version
            )));
        }
        if self.min_warm > self.max_total {
            return Err(ConfigError::Invalid(format!(
                "KindConfig {}/{} min_warm={} > max_total={}",
                self.model_id, self.model_version, self.min_warm, self.max_total
            )));
        }
        if self.memory_mb < 32 {
            return Err(ConfigError::Invalid(format!(
                "KindConfig {}/{} memory_mb={} too small (min 32)",
                self.model_id, self.model_version, self.memory_mb
            )));
        }
        if self.vcpus == 0 {
            return Err(ConfigError::Invalid(format!(
                "KindConfig {}/{} vcpus=0",
                self.model_id, self.model_version
            )));
        }
        if self.cgroup_blkio_weight < 1 || self.cgroup_blkio_weight > 1000 {
            return Err(ConfigError::Invalid(format!(
                "KindConfig {}/{} cgroup_blkio_weight={} out of [1, 1000]",
                self.model_id, self.model_version, self.cgroup_blkio_weight
            )));
        }
        Ok(())
    }

    pub fn matches(&self, key: &SubPoolKey) -> bool {
        self.kind == key.kind
            && self.model_id == key.model_id
            && self.model_version == key.model_version
    }

    pub fn apply_budget(&mut self, override_budget: &SubPoolBudget) {
        if let Some(min_warm) = override_budget.min_warm {
            self.min_warm = min_warm;
        }
        if let Some(max_total) = override_budget.max_total {
            self.max_total = max_total;
        }
    }

    /// Test default for a Mode B kind.
    pub fn testing_mode_b(model_id: &str, model_version: &str) -> Self {
        Self {
            kind: VmKind::ModelInference,
            model_id: model_id.to_string(),
            model_version: model_version.to_string(),
            rootfs_path: PathBuf::from(format!("/dev/null/{model_id}.ext4")),
            min_warm: 2,
            max_total: 10,
            memory_mb: 256,
            vcpus: 1,
            cgroup_cpu_quota_us: 1_000_000,
            cgroup_memory_high: 230_686_720,   // 220 MB
            cgroup_memory_max: 268_435_456,    // 256 MB
            cgroup_blkio_weight: 100,
        }
    }

    /// Test default for a Mode C kind.
    pub fn testing_mode_c() -> Self {
        Self {
            kind: VmKind::LlmCaller,
            model_id: String::new(),
            model_version: String::new(),
            rootfs_path: PathBuf::from("/dev/null/llm_caller.ext4"),
            min_warm: 2,
            max_total: 20,
            memory_mb: 512,
            vcpus: 2,
            cgroup_cpu_quota_us: 2_000_000,
            cgroup_memory_high: 461_373_440,
            cgroup_memory_max: 536_870_912,
            cgroup_blkio_weight: 100,
        }
    }

    /// Sensible default for a sub-pool key that has no explicit config.
    /// Used at the boundary when a customer creates a tenant_class that
    /// the operator hasn't pre-provisioned.
    pub fn default_for(key: &SubPoolKey) -> Self {
        match key.kind {
            VmKind::ModelInference => Self::testing_mode_b(&key.model_id, &key.model_version),
            VmKind::LlmCaller => Self::testing_mode_c(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  SubPoolBudget — tenant_class-level override
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : per-tenant-class sizing override.  Set on the
/// pool config to give e.g. TIER-AUDIT customers a guaranteed
/// reservation of `min_warm` VMs even if other classes are saturating.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubPoolBudget {
    pub min_warm: Option<u32>,
    pub max_total: Option<u32>,
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn for_testing_validates() {
        let cfg = FirecrackerPoolConfig::for_testing();
        cfg.validate().expect("default test config must validate");
    }

    #[test]
    fn validate_rejects_zero_boot_budget() {
        let mut cfg = FirecrackerPoolConfig::for_testing();
        cfg.boot_budget = Duration::from_millis(0);
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("boot_budget"));
    }

    #[test]
    fn validate_rejects_min_warm_exceeding_max_total() {
        let mut cfg = FirecrackerPoolConfig::for_testing();
        cfg.per_kind[0].min_warm = 999;
        cfg.per_kind[0].max_total = 10;
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("min_warm"));
    }

    #[test]
    fn validate_rejects_min_warm_sum_over_pool_cap() {
        let mut cfg = FirecrackerPoolConfig::for_testing();
        cfg.max_total_vms = 1;
        cfg.per_kind[0].min_warm = 5;
        cfg.per_kind[0].max_total = 10;
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("exceeds max_total_vms"));
    }

    #[test]
    fn validate_rejects_invalid_blkio_weight() {
        let mut cfg = FirecrackerPoolConfig::for_testing();
        cfg.per_kind[0].cgroup_blkio_weight = 2000;
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("blkio_weight"));
    }

    #[test]
    fn validate_rejects_zero_vcpus() {
        let mut cfg = FirecrackerPoolConfig::for_testing();
        cfg.per_kind[0].vcpus = 0;
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("vcpus"));
    }

    #[test]
    fn validate_rejects_tiny_memory() {
        let mut cfg = FirecrackerPoolConfig::for_testing();
        cfg.per_kind[0].memory_mb = 10;
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("memory_mb"));
    }

    #[test]
    fn for_subpool_matches_model_kind() {
        let cfg = FirecrackerPoolConfig::for_testing();
        let kc = cfg.for_subpool(&SubPoolKey::mode_b(
            "test_model",
            "1.0.0",
            TenantClass::Tier1,
        ));
        assert_eq!(kc.model_id, "test_model");
        assert_eq!(kc.model_version, "1.0.0");
    }

    #[test]
    fn for_subpool_falls_back_for_unknown_model() {
        let cfg = FirecrackerPoolConfig::for_testing();
        let kc = cfg.for_subpool(&SubPoolKey::mode_b(
            "unknown_model",
            "1.0.0",
            TenantClass::Tier1,
        ));
        assert_eq!(kc.model_id, "unknown_model");
        // default_for returns testing defaults
        assert_eq!(kc.min_warm, 2);
    }

    #[test]
    fn for_subpool_applies_tenant_class_override() {
        let mut cfg = FirecrackerPoolConfig::for_testing();
        cfg.tenant_class_overrides.insert(
            "tier_audit".into(),
            SubPoolBudget {
                min_warm: Some(20),
                max_total: Some(100),
            },
        );
        let kc = cfg.for_subpool(&SubPoolKey::mode_b(
            "test_model",
            "1.0.0",
            TenantClass::TierAudit,
        ));
        assert_eq!(kc.min_warm, 20);
        assert_eq!(kc.max_total, 100);
    }

    #[test]
    fn for_subpool_does_not_apply_unknown_tenant_class_override() {
        let mut cfg = FirecrackerPoolConfig::for_testing();
        cfg.tenant_class_overrides.insert(
            "tier_audit".into(),
            SubPoolBudget {
                min_warm: Some(20),
                max_total: Some(100),
            },
        );
        let kc = cfg.for_subpool(&SubPoolKey::mode_b(
            "test_model",
            "1.0.0",
            TenantClass::Tier1,
        ));
        // Tier1 doesn't match the override key → defaults preserved.
        assert_eq!(kc.min_warm, 2);
        assert_eq!(kc.max_total, 10);
    }

    #[test]
    fn kind_config_validates_clean_defaults() {
        let kc = KindConfig::testing_mode_b("foo", "1.0.0");
        kc.validate().unwrap();
        let kc = KindConfig::testing_mode_c();
        kc.validate().unwrap();
    }

    #[test]
    fn kind_config_matches_correctly() {
        let kc = KindConfig::testing_mode_b("ner_clinical", "1.0.0");
        assert!(kc.matches(&SubPoolKey::mode_b(
            "ner_clinical",
            "1.0.0",
            TenantClass::Tier1
        )));
        assert!(!kc.matches(&SubPoolKey::mode_b(
            "ner_clinical",
            "1.0.1",
            TenantClass::Tier1
        )));
        assert!(!kc.matches(&SubPoolKey::mode_b(
            "other",
            "1.0.0",
            TenantClass::Tier1
        )));
        assert!(!kc.matches(&SubPoolKey::mode_c(TenantClass::Tier1)));
    }

    #[test]
    fn json_round_trip() {
        let cfg = FirecrackerPoolConfig::for_testing();
        let s = serde_json::to_string(&cfg).unwrap();
        let back: FirecrackerPoolConfig = serde_json::from_str(&s).unwrap();
        back.validate().unwrap();
        assert_eq!(back.per_kind.len(), cfg.per_kind.len());
    }
}
