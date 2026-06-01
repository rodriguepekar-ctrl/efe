//! Agent profile registry — human-readable resource + behavior preset.
//!
//! Profiles are loaded at startup from `agent-profiles.yaml` (mounted
//! via Helm). Each profile bundles : sandbox resources, default tool
//! whitelist, default approval policy, default LLM settings (provider,
//! model). Tenants can opt-in to specific profiles via
//! `tenant_feature.m25_allowed_profiles`.

use crate::sandbox::SandboxResources;
use crate::{AgentRunnerError, AgentRunnerResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    pub slug: String,
    pub display_name: String,
    pub description: String,
    pub resources: SandboxResources,
    pub default_tool_whitelist: Vec<String>,
    pub require_approval_for_writes: bool,
    pub default_provider: String,
    pub default_model: String,
    pub default_max_steps: u32,
    pub default_timeout_seconds: u32,
    pub default_max_cost_cents: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileFile {
    pub profiles: Vec<AgentProfile>,
}

#[derive(Debug, Clone, Default)]
pub struct ProfileRegistry {
    by_slug: HashMap<String, AgentProfile>,
}

impl ProfileRegistry {
    pub fn from_yaml(yaml: &str) -> AgentRunnerResult<Self> {
        let parsed: ProfileFile = serde_yaml::from_str(yaml)
            .map_err(|e| AgentRunnerError::InvalidInput(format!("parse profiles : {}", e)))?;
        let mut by_slug = HashMap::new();
        for p in parsed.profiles {
            if !p.slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(AgentRunnerError::InvalidInput(format!(
                    "profile slug '{}' contains forbidden chars", p.slug
                )));
            }
            if by_slug.contains_key(&p.slug) {
                return Err(AgentRunnerError::InvalidInput(format!(
                    "duplicate profile slug '{}'", p.slug
                )));
            }
            by_slug.insert(p.slug.clone(), p);
        }
        Ok(Self { by_slug })
    }

    pub fn from_default_file() -> AgentRunnerResult<Self> {
        Self::from_yaml(DEFAULT_PROFILES_YAML)
    }

    pub fn get(&self, slug: &str) -> AgentRunnerResult<&AgentProfile> {
        self.by_slug
            .get(slug)
            .ok_or_else(|| AgentRunnerError::ProfileUnknown(slug.to_string()))
    }

    pub fn enforce_tenant_allow(
        &self,
        slug: &str,
        allowed_profiles: &[String],
    ) -> AgentRunnerResult<&AgentProfile> {
        let p = self.get(slug)?;
        if !allowed_profiles.iter().any(|a| a == slug) {
            return Err(AgentRunnerError::ProfileNotAllowed(slug.to_string()));
        }
        Ok(p)
    }

    pub fn slugs(&self) -> Vec<&str> {
        self.by_slug.keys().map(String::as_str).collect()
    }
}

/// Default profile catalog, embedded as a fallback when no
/// `agent-profiles.yaml` is mounted (e.g. in unit tests).
pub const DEFAULT_PROFILES_YAML: &str = r#"
profiles:
  - slug: research_lite
    display_name: "Research lite"
    description: "Read-only research agent with web_search + fetch_url. No writes."
    resources:
      vcpus: 1
      memory_mib: 512
      rootfs_image: "ghcr.io/efe-ai/agent-core:2.0.0"
      kernel_image: "/opt/efe/firecracker/vmlinux"
      initramfs: null
    default_tool_whitelist: [web_search, fetch_url, efe_classify, efe_query_events]
    require_approval_for_writes: true
    default_provider: openai
    default_model: gpt-4o-mini
    default_max_steps: 25
    default_timeout_seconds: 1800
    default_max_cost_cents: 50

  - slug: catalog_curator
    display_name: "Catalog curator"
    description: "Catalog editing agent — writes require approval."
    resources:
      vcpus: 2
      memory_mib: 1024
      rootfs_image: "ghcr.io/efe-ai/agent-core:2.0.0"
      kernel_image: "/opt/efe/firecracker/vmlinux"
      initramfs: null
    default_tool_whitelist:
      [efe_classify, efe_query_events, efe_upsert_catalog, efe_retune_centroids]
    require_approval_for_writes: true
    default_provider: anthropic
    default_model: claude-3-5-sonnet
    default_max_steps: 50
    default_timeout_seconds: 3600
    default_max_cost_cents: 200

  - slug: triage_agent
    display_name: "Event triage"
    description: "Reads ingest events, classifies, optionally reroutes (write needs approval)."
    resources:
      vcpus: 1
      memory_mib: 512
      rootfs_image: "ghcr.io/efe-ai/agent-core:2.0.0"
      kernel_image: "/opt/efe/firecracker/vmlinux"
      initramfs: null
    default_tool_whitelist:
      [efe_classify, efe_query_events, efe_ingest_events]
    require_approval_for_writes: true
    default_provider: openai
    default_model: gpt-4o-mini
    default_max_steps: 30
    default_timeout_seconds: 1200
    default_max_cost_cents: 25
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_yaml_parses() {
        let r = ProfileRegistry::from_default_file().unwrap();
        assert!(r.get("research_lite").is_ok());
        assert!(r.get("catalog_curator").is_ok());
        assert!(r.get("triage_agent").is_ok());
        assert!(r.get("does_not_exist").is_err());
    }

    #[test]
    fn rejects_invalid_slug() {
        let bad = r#"profiles:
  - slug: "bad slug!"
    display_name: x
    description: x
    resources:
      vcpus: 1
      memory_mib: 256
      rootfs_image: "x"
      kernel_image: "x"
      initramfs: null
    default_tool_whitelist: [noop]
    require_approval_for_writes: false
    default_provider: openai
    default_model: gpt-4o-mini
    default_max_steps: 1
    default_timeout_seconds: 60
    default_max_cost_cents: 1
"#;
        let r = ProfileRegistry::from_yaml(bad);
        assert!(r.is_err());
    }

    #[test]
    fn rejects_duplicate_slug() {
        let dup = r#"profiles:
  - slug: a
    display_name: x
    description: x
    resources:
      vcpus: 1
      memory_mib: 256
      rootfs_image: "x"
      kernel_image: "x"
      initramfs: null
    default_tool_whitelist: [noop]
    require_approval_for_writes: false
    default_provider: openai
    default_model: gpt-4o-mini
    default_max_steps: 1
    default_timeout_seconds: 60
    default_max_cost_cents: 1
  - slug: a
    display_name: x
    description: x
    resources:
      vcpus: 1
      memory_mib: 256
      rootfs_image: "x"
      kernel_image: "x"
      initramfs: null
    default_tool_whitelist: [noop]
    require_approval_for_writes: false
    default_provider: openai
    default_model: gpt-4o-mini
    default_max_steps: 1
    default_timeout_seconds: 60
    default_max_cost_cents: 1
"#;
        let r = ProfileRegistry::from_yaml(dup);
        assert!(r.is_err());
    }

    #[test]
    fn enforce_tenant_allow_blocks_disallowed() {
        let r = ProfileRegistry::from_default_file().unwrap();
        let err = r.enforce_tenant_allow(
            "research_lite",
            &["triage_agent".into()],
        ).unwrap_err();
        assert!(matches!(err, AgentRunnerError::ProfileNotAllowed(_)));
    }

    #[test]
    fn enforce_tenant_allow_passes_when_allowed() {
        let r = ProfileRegistry::from_default_file().unwrap();
        let ok = r.enforce_tenant_allow(
            "research_lite",
            &["research_lite".into()],
        );
        assert!(ok.is_ok());
    }
}
