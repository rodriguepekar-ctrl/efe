//! `efe-curation` — M23 LLM-assisted taxonomy curation.
//!
//! This crate provides the backend for curation workflows :
//!   - Drift detection (KL divergence on confidence distributions)
//!   - Proposal lifecycle (5 tools : split / merge / explain / synth / audit)
//!   - Taxonomy version DAG (parent-child fork-merge, signed)
//!   - Semantic diff engine (anchor-level + class-level)
//!   - Rollout / canary / auto-rollback
//!   - Audit chain (HMAC, mirrors `llm_call_audit`)
//!
//! All functionality is gated behind `EFE_FEATURE_M23_CURATION_ENABLED` ;
//! when OFF, this crate is dead-code-eliminated.
//!
//! See `docs/EFE_M23_TAXONOMY_CURATION_ARCHITECTURE.md` for the full spec.

pub mod audit;
pub mod drift;
pub mod error;
pub mod metrics;
pub mod proposal;
pub mod rollout;
pub mod schemas;
pub mod semantic_diff;
pub mod service;
pub mod version_dag;

// `test_helpers` is exposed under the `test-utils` feature so integration
// tests under `tests/` can reach in-process mocks. Unit tests under
// `cfg(test)` get them automatically.
#[cfg(any(test, feature = "test-utils"))]
pub mod test_helpers;

pub use error::{CurationError, CurationResult};

/// Curation crate version. Bumped per architectural change ; not the gRPC
/// service version (which is `v1` and stable).
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Feature flag check helper. Reads from environment + tenant features.
pub fn is_enabled_for_tenant(tenant_features: &TenantCurationFlags) -> bool {
    let server_on = std::env::var("EFE_FEATURE_M23_CURATION_ENABLED")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);
    server_on && tenant_features.curation_enabled
}

/// Subset of tenant feature flags relevant to M23. Resolved per-call.
#[derive(Debug, Clone)]
pub struct TenantCurationFlags {
    pub curation_enabled: bool,
    pub allow_internal_llm: bool,
    pub require_4_eyes: bool,
    pub drift_detection_enabled: bool,
    pub drift_auto_trigger_proposal: bool,
    pub drift_kl_threshold: f64,
    pub drift_conf_drop_threshold: f64,
    pub drift_support_threshold_pct: f64,
    pub canary_traffic_percent: f64,
    pub canary_duration_hours: u32,
    pub canary_min_samples: u32,
    pub proposal_retention_days: u32,
    pub curation_budget_cents_per_day: i64,
}

impl Default for TenantCurationFlags {
    fn default() -> Self {
        Self {
            curation_enabled: false,
            allow_internal_llm: false,
            require_4_eyes: false,
            drift_detection_enabled: false,
            drift_auto_trigger_proposal: false,
            drift_kl_threshold: 0.15,
            drift_conf_drop_threshold: 0.10,
            drift_support_threshold_pct: 0.40,
            canary_traffic_percent: 0.05,
            canary_duration_hours: 24,
            canary_min_samples: 1000,
            proposal_retention_days: 90,
            curation_budget_cents_per_day: 0,
        }
    }
}

/// Strong-typed wrapper for tenant ID — prevents accidental cross-tenant leak.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct TenantId(pub String);

impl std::fmt::Display for TenantId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Strong-typed wrapper for catalog ID.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct CatalogId(pub String);

/// Strong-typed wrapper for class ID — kebab-case enforced at construction.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct ClassId(pub String);

impl ClassId {
    pub fn new(s: impl Into<String>) -> Result<Self, CurationError> {
        let s: String = s.into();
        if !is_valid_kebab_case(&s) {
            return Err(CurationError::InvalidClassId(s));
        }
        Ok(Self(s))
    }
}

/// Validates a string is valid kebab-case (lowercase ASCII + hyphens, no
/// leading/trailing hyphens, no consecutive hyphens, length 2-64).
pub fn is_valid_kebab_case(s: &str) -> bool {
    if s.len() < 2 || s.len() > 64 {
        return false;
    }
    if s.starts_with('-') || s.ends_with('-') {
        return false;
    }
    if s.contains("--") {
        return false;
    }
    s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod kebab_tests {
    use super::*;

    #[test]
    fn valid_kebab() {
        assert!(is_valid_kebab_case("ok"));
        assert!(is_valid_kebab_case("support-issue"));
        assert!(is_valid_kebab_case("class-123"));
        assert!(is_valid_kebab_case("a-b-c-d"));
    }

    #[test]
    fn invalid_kebab() {
        assert!(!is_valid_kebab_case(""));
        assert!(!is_valid_kebab_case("a"));
        assert!(!is_valid_kebab_case("UPPERCASE"));
        assert!(!is_valid_kebab_case("-leading"));
        assert!(!is_valid_kebab_case("trailing-"));
        assert!(!is_valid_kebab_case("double--hyphen"));
        assert!(!is_valid_kebab_case("with_underscore"));
        assert!(!is_valid_kebab_case("with space"));
        assert!(!is_valid_kebab_case(&"x".repeat(65)));
    }

    #[test]
    fn class_id_constructor() {
        assert!(ClassId::new("valid-id").is_ok());
        assert!(ClassId::new("Invalid").is_err());
    }
}
