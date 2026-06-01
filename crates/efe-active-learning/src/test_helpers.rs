//! Test fixtures and mock implementations for the active learning crate.
//!
//! Compiled under `cfg(test)` OR feature `test-utils` so integration
//! tests under `tests/` can reach the in-process mocks.

use crate::llm_worker::{ClassValidator, LlmClient, LlmResponse, PromptRenderer, RenderedPrompt};
use crate::queue::QueueRow;
use crate::sampler::{CatalogContext, SamplerConfig, TenantContext, TenantRegistry};
use crate::service::FeatureFlagProvider;
use crate::{ActiveLearningResult, CatalogId, TenantActiveLearningFlags, TenantId};
use sqlx::PgPool;
use std::sync::Mutex;
use uuid::Uuid;

#[allow(dead_code)]
pub struct MockFeatureFlags {
    pub server_m24: bool,
    pub server_m21: bool,
    pub server_m23: bool,
    pub tenant: Mutex<std::collections::HashMap<String, TenantActiveLearningFlags>>,
}

#[allow(dead_code)]
impl MockFeatureFlags {
    pub fn enabled() -> Self {
        Self {
            server_m24: true, server_m21: true, server_m23: true,
            tenant: Mutex::new(std::collections::HashMap::new()),
        }
    }
    pub fn disabled() -> Self {
        Self {
            server_m24: false, server_m21: false, server_m23: false,
            tenant: Mutex::new(std::collections::HashMap::new()),
        }
    }
    pub fn set_tenant(&self, tenant: &str, flags: TenantActiveLearningFlags) {
        self.tenant.lock().unwrap().insert(tenant.to_string(), flags);
    }
}

#[async_trait::async_trait]
impl FeatureFlagProvider for MockFeatureFlags {
    fn server_m24_enabled(&self) -> bool { self.server_m24 }
    fn server_m21_enabled(&self) -> bool { self.server_m21 }
    fn server_m23_enabled(&self) -> bool { self.server_m23 }
    async fn tenant_flags(&self, tenant: &TenantId) -> ActiveLearningResult<TenantActiveLearningFlags> {
        Ok(self.tenant.lock().unwrap().get(&tenant.0).cloned().unwrap_or_default())
    }
}

#[allow(dead_code)]
pub struct MockLlmClient {
    pub canned: Mutex<Vec<String>>,
}

#[allow(dead_code)]
impl MockLlmClient {
    pub fn new(responses: Vec<&str>) -> Self {
        Self {
            canned: Mutex::new(
                responses.into_iter().rev().map(|s| s.to_string()).collect(),
            ),
        }
    }
}

#[async_trait::async_trait]
impl LlmClient for MockLlmClient {
    async fn label(
        &self, _tenant: &TenantId, _prompt: &str, _max_tokens: u32,
    ) -> ActiveLearningResult<LlmResponse> {
        let mut q = self.canned.lock().unwrap();
        let content = q.pop().unwrap_or_else(|| "{}".to_string());
        Ok(LlmResponse {
            content,
            provider: "mock".into(),
            model: "mock-1".into(),
            input_tokens: 100,
            output_tokens: 50,
            cost_cents: 1,
            latency_ms: 10,
            audit_id: Some(Uuid::new_v4()),
        })
    }
}

#[allow(dead_code)]
pub struct MockPromptRenderer;

#[async_trait::async_trait]
impl PromptRenderer for MockPromptRenderer {
    async fn render(
        &self, _tenant: &TenantId, queue_row: &QueueRow,
    ) -> ActiveLearningResult<RenderedPrompt> {
        Ok(RenderedPrompt {
            body: format!(
                "MOCK-PROMPT(catalog={}, sentence={})",
                queue_row.catalog_id, queue_row.sentence_redacted,
            ),
            template_name: "label_classify_v1".into(),
            template_version: 1,
        })
    }
}

#[allow(dead_code)]
pub struct AcceptingClassValidator;

#[async_trait::async_trait]
impl ClassValidator for AcceptingClassValidator {
    async fn is_valid(
        &self, _pg: &PgPool, _tenant: &TenantId, _catalog_id: &str, _class_id: &str,
    ) -> ActiveLearningResult<bool> {
        Ok(true)
    }
}

#[allow(dead_code)]
pub struct StaticTenantRegistry {
    pub tenants: Vec<TenantContext>,
}

#[async_trait::async_trait]
impl TenantRegistry for StaticTenantRegistry {
    async fn iter_active_with_al(&self) -> ActiveLearningResult<Vec<TenantContext>> {
        // Clone the static config — TenantContext doesn't derive Clone so
        // we reconstruct each time.
        Ok(self.tenants.iter().map(|t| TenantContext {
            id: t.id.clone(),
            catalogs: t.catalogs.iter().map(|c| CatalogContext {
                id: c.id.clone(), al_enabled: c.al_enabled,
            }).collect(),
            config: t.config.clone(),
        }).collect())
    }
}
