//! Shared test plumbing : in-memory tenant flags provider, helpers
//! for inserting fixture rows, etc.
//!
//! Compiled only with `cfg(test)` or the `test-utils` feature so it
//! never bloats the production binary.

use crate::profile::ProfileRegistry;
use crate::service::{AgentRuntimeService, CallerIdentity, TenantFlagsProvider};
use crate::{AgentRunnerResult, TenantAgentRuntimeFlags, TenantId};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Default)]
pub struct InMemoryFlagsProvider {
    inner: RwLock<HashMap<String, TenantAgentRuntimeFlags>>,
}

impl InMemoryFlagsProvider {
    pub fn new() -> Self { Self::default() }

    pub async fn set(&self, tenant: &TenantId, flags: TenantAgentRuntimeFlags) {
        self.inner.write().await.insert(tenant.0.clone(), flags);
    }
}

#[async_trait::async_trait]
impl TenantFlagsProvider for InMemoryFlagsProvider {
    async fn flags_for(
        &self,
        tenant: &TenantId,
    ) -> AgentRunnerResult<TenantAgentRuntimeFlags> {
        Ok(self
            .inner
            .read()
            .await
            .get(&tenant.0)
            .cloned()
            .unwrap_or_default())
    }
}

pub fn fully_enabled_flags(tenant: &str) -> TenantAgentRuntimeFlags {
    TenantAgentRuntimeFlags {
        agent_runtime_enabled: true,
        dpia_signed_at: Some(chrono::Utc::now() - chrono::Duration::days(7)),
        dpia_expires_at: Some(chrono::Utc::now() + chrono::Duration::days(180)),
        dpia_addendum_hash: Some(format!("abcdef{}", tenant)),
        cost_cap_per_run_usd: 5.00,
        timeout_minutes_max: 60,
        allowed_profiles: vec![
            "research_lite".into(),
            "catalog_curator".into(),
            "triage_agent".into(),
        ],
        allowed_providers: vec!["openai".into(), "anthropic".into()],
        memory_enabled: true,
        microvm_pool_size: 2,
        max_concurrent_runs: 5,
    }
}

pub fn admin_caller(tenant: &str) -> CallerIdentity {
    CallerIdentity {
        subject: format!("admin@{}", tenant),
        tenant: TenantId(tenant.into()),
        roles: vec!["admin".into()],
        token_age_seconds: 30,
    }
}

pub fn operator_caller(tenant: &str) -> CallerIdentity {
    CallerIdentity {
        subject: format!("op@{}", tenant),
        tenant: TenantId(tenant.into()),
        roles: vec!["operator".into()],
        token_age_seconds: 30,
    }
}

pub fn viewer_caller(tenant: &str) -> CallerIdentity {
    CallerIdentity {
        subject: format!("viewer@{}", tenant),
        tenant: TenantId(tenant.into()),
        roles: vec!["viewer".into()],
        token_age_seconds: 30,
    }
}

pub fn build_service(pg: sqlx::PgPool, flags: Arc<InMemoryFlagsProvider>) -> AgentRuntimeService {
    let flags_dyn: Arc<dyn TenantFlagsProvider> = flags;
    AgentRuntimeService::new(
        pg,
        flags_dyn,
        Arc::new(ProfileRegistry::from_default_file().expect("default profiles parse")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn flags_provider_stores_and_retrieves() {
        let p = InMemoryFlagsProvider::new();
        let t = TenantId("acme".into());
        p.set(&t, fully_enabled_flags("acme")).await;
        let got = p.flags_for(&t).await.unwrap();
        assert!(got.agent_runtime_enabled);
        assert!(got.dpia_valid());
    }

    #[tokio::test]
    async fn unknown_tenant_returns_default_flags() {
        let p = InMemoryFlagsProvider::new();
        let f = p.flags_for(&TenantId("nope".into())).await.unwrap();
        assert!(!f.agent_runtime_enabled);
    }
}
