//! `efe-agent-runner` — M25 EXPERIMENTAL agent runtime.
//!
//! In-cluster Firecracker microVM sandbox for LLM-driven autonomous
//! agents. Hard-isolated per-tenant, tool-whitelisted, step-approved,
//! cost-capped, DPIA-gated.
//!
//! **CRITICAL** : feature-gated behind `EFE_FEATURE_AGENT_RUNTIME_ENABLED`
//! AND requires Helm chart `efe-agent-runner` deployed separately
//! (CRDs, NetworkPolicy, RBAC). Default OFF.
//!
//! **Status** : EXPERIMENTAL. Single design-partner pilot. Sunset clause
//! if < 2 tenants at 12 months.
//!
//! See `docs/EFE_M25_AGENT_RUNTIME_ARCHITECTURE.md` for full spec.

pub mod approval;
pub mod audit;
pub mod cost;
pub mod controller;
pub mod controller_reaper;   // M26
pub mod error;
pub mod flags;               // D93 — production PgTenantFlagsProvider
pub mod memory;
pub mod metrics;
pub mod profile;
pub mod run;
pub mod sandbox;
pub mod service;
pub mod state;
pub mod template;
pub mod tool_acl;
pub mod webhook;             // M26

#[cfg(any(test, feature = "test-utils"))]
pub mod test_helpers;

pub use error::{AgentRunnerError, AgentRunnerResult};
pub use flags::PgTenantFlagsProvider;   // D93 re-export

// Re-export common types from efe-curation (M23) so callers don't need
// a second import.
pub use efe_curation::{CatalogId, ClassId, TenantId};

/// Crate / EFE version. M25 = EFE 2.0.
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Tenant feature flags relevant to M25.
#[derive(Debug, Clone)]
pub struct TenantAgentRuntimeFlags {
    pub agent_runtime_enabled: bool,
    pub dpia_signed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub dpia_expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub dpia_addendum_hash: Option<String>,
    pub cost_cap_per_run_usd: f64,
    pub timeout_minutes_max: u32,
    pub allowed_profiles: Vec<String>,
    pub allowed_providers: Vec<String>,
    pub memory_enabled: bool,
    pub microvm_pool_size: u32,
    pub max_concurrent_runs: u32,
}

impl Default for TenantAgentRuntimeFlags {
    fn default() -> Self {
        Self {
            agent_runtime_enabled: false,
            dpia_signed_at: None,
            dpia_expires_at: None,
            dpia_addendum_hash: None,
            cost_cap_per_run_usd: 0.0,        // 0 = all rejected
            timeout_minutes_max: 60,
            allowed_profiles: vec![],
            allowed_providers: vec![],
            memory_enabled: false,
            microvm_pool_size: 2,
            max_concurrent_runs: 5,
        }
    }
}

impl TenantAgentRuntimeFlags {
    /// True if DPIA is currently valid (signed AND not expired).
    pub fn dpia_valid(&self) -> bool {
        match (self.dpia_signed_at, self.dpia_expires_at) {
            (Some(_), Some(exp)) => exp > chrono::Utc::now(),
            _ => false,
        }
    }
}

/// Validate startup dependencies — M25 requires M21 + KVM + CRDs.
pub fn validate_startup() -> Result<(), String> {
    let m25 = env_bool("EFE_FEATURE_AGENT_RUNTIME_ENABLED");
    if !m25 { return Ok(()); }

    let m21 = env_bool("EFE_FEATURE_M21_LLMROUTER");
    if !m21 {
        return Err(
            "M25 requires M21 LLMRouter to be ON (set EFE_FEATURE_M21_LLMROUTER=true)".into()
        );
    }

    // KVM availability check — soft warn (full check requires probing /dev/kvm
    // which only works in production environment).
    if !std::path::Path::new("/dev/kvm").exists() {
        tracing::warn!(
            "EFE_FEATURE_AGENT_RUNTIME_ENABLED=true but /dev/kvm not present. \
             M25 requires KVM hardware virtualization. Make sure the node has \
             KVM enabled or this is a controller-only deployment."
        );
    }

    Ok(())
}

fn env_bool(key: &str) -> bool {
    matches!(std::env::var(key).as_deref(), Ok("true") | Ok("1") | Ok("yes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_default_safe() {
        let f = TenantAgentRuntimeFlags::default();
        assert!(!f.agent_runtime_enabled);
        assert!(!f.dpia_valid());
        assert_eq!(f.cost_cap_per_run_usd, 0.0);
    }

    #[test]
    fn dpia_valid_requires_signed_and_unexpired() {
        let mut f = TenantAgentRuntimeFlags::default();
        // Signed but no expiry → invalid
        f.dpia_signed_at = Some(chrono::Utc::now());
        assert!(!f.dpia_valid());
        // Signed + future expiry → valid
        f.dpia_expires_at = Some(chrono::Utc::now() + chrono::Duration::days(30));
        assert!(f.dpia_valid());
        // Past expiry → invalid
        f.dpia_expires_at = Some(chrono::Utc::now() - chrono::Duration::days(1));
        assert!(!f.dpia_valid());
    }
}
