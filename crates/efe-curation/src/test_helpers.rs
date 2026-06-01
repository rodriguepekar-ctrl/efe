//! Test fixtures and mock implementations for the curation crate.
//!
//! Compiled only under `cfg(test)`. Provides stand-ins for the LlmClient,
//! PromptRenderer, and FeatureFlagProvider traits.

use crate::service::{ExecutionMode, FeatureFlagProvider, LlmClient, LlmResponse, PromptRenderer, RenderedPrompt};
use crate::{CurationResult, TenantCurationFlags, TenantId};
use std::sync::Mutex;
use uuid::Uuid;

#[allow(dead_code)]
pub struct MockFeatureFlags {
    pub server_m23: bool,
    pub server_m21: bool,
    pub server_internal_llm: bool,
    pub tenant: Mutex<std::collections::HashMap<String, TenantCurationFlags>>,
}

#[allow(dead_code)]
impl MockFeatureFlags {
    pub fn enabled() -> Self {
        Self {
            server_m23: true, server_m21: true, server_internal_llm: true,
            tenant: Mutex::new(std::collections::HashMap::new()),
        }
    }
    pub fn disabled() -> Self {
        Self {
            server_m23: false, server_m21: false, server_internal_llm: false,
            tenant: Mutex::new(std::collections::HashMap::new()),
        }
    }
    pub fn set_tenant(&self, tenant: &str, flags: TenantCurationFlags) {
        self.tenant.lock().unwrap().insert(tenant.to_string(), flags);
    }
}

#[async_trait::async_trait]
impl FeatureFlagProvider for MockFeatureFlags {
    fn server_m23_enabled(&self) -> bool { self.server_m23 }
    fn server_m21_enabled(&self) -> bool { self.server_m21 }
    fn server_use_internal_llm(&self) -> bool { self.server_internal_llm }
    async fn tenant_flags(&self, tenant: &TenantId) -> CurationResult<TenantCurationFlags> {
        Ok(self.tenant.lock().unwrap()
            .get(&tenant.0).cloned()
            .unwrap_or_default())
    }
}

#[allow(dead_code)]
pub struct MockLlm { pub canned: Mutex<Vec<String>> }

#[allow(dead_code)]
impl MockLlm {
    pub fn new(responses: Vec<&str>) -> Self {
        Self { canned: Mutex::new(responses.into_iter().rev().map(|s| s.to_string()).collect()) }
    }
}

#[async_trait::async_trait]
impl LlmClient for MockLlm {
    async fn call(
        &self, _tenant: &TenantId, _prompt: &str, _max_tokens: u32, _mode: ExecutionMode,
    ) -> CurationResult<LlmResponse> {
        let mut q = self.canned.lock().unwrap();
        let content = q.pop().unwrap_or_else(|| "{}".to_string());
        Ok(LlmResponse {
            content,
            provider: "mock".into(), model: "mock-1".into(),
            input_tokens: 100, output_tokens: 50,
            cost_cents: 1, latency_ms: 10, audit_id: Some(Uuid::new_v4()),
        })
    }
}

#[allow(dead_code)]
pub struct MockPromptRenderer;

#[async_trait::async_trait]
impl PromptRenderer for MockPromptRenderer {
    async fn render(
        &self, template_name: &str, version: u32,
        _tenant: &TenantId, _context: serde_json::Value,
    ) -> CurationResult<RenderedPrompt> {
        Ok(RenderedPrompt {
            body: format!("MOCK-PROMPT[{}|v{}]", template_name, version),
            template_name: template_name.into(),
            template_version: version,
        })
    }
}
