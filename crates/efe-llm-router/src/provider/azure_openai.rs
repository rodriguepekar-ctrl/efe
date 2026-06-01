//! Azure OpenAI provider — wraps OpenAIProvider with Azure auth +
//! deployment URL pattern.

use super::*;
use crate::provider::openai::OpenAIProvider;

pub struct AzureOpenAIProvider {
    inner: OpenAIProvider,
    name_cache: String,
    cost: CostModel,
    max_context: u32,
}

impl AzureOpenAIProvider {
    /// `endpoint` example : `https://my-resource.openai.azure.com`.
    /// `deployment` is the Azure OpenAI deployment name.
    pub fn new(
        endpoint: String,
        deployment: String,
        api_version: String,
        api_key: String,
        cost: CostModel,
    ) -> Self {
        let base = format!(
            "{}/openai/deployments/{}",
            endpoint.trim_end_matches('/'),
            deployment
        );
        let name = format!("azure-openai:{}", deployment);
        let inner = OpenAIProvider::new(api_key, deployment.clone())
            .with_base_url(base)
            .with_name(name.clone())
            .with_cost(cost);
        Self {
            inner,
            name_cache: name,
            cost,
            max_context: 128_000,
        }
    }

    pub fn with_max_context(mut self, max: u32) -> Self {
        self.max_context = max;
        self.inner = self.inner.with_max_context(max);
        self
    }
}

#[async_trait]
impl LLMProvider for AzureOpenAIProvider {
    fn name(&self) -> &str {
        &self.name_cache
    }
    fn supports_tools(&self) -> bool {
        true
    }
    fn max_context_tokens(&self) -> u32 {
        self.max_context
    }
    fn cost_model(&self) -> CostModel {
        self.cost
    }
    async fn complete(
        &self,
        request: CompleteRequest,
    ) -> Result<CompleteResponse, ProviderError> {
        self.inner.complete(request).await
    }
    async fn health(&self) -> ProviderHealth {
        self.inner.health().await
    }
}
