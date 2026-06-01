//! Mistral La Plateforme provider — uses OpenAI-compat endpoint.

use super::*;
use crate::provider::openai::OpenAIProvider;

pub struct MistralProvider {
    inner: OpenAIProvider,
    name_cache: String,
    cost: CostModel,
}

impl MistralProvider {
    pub fn new(api_key: String, model: String) -> Self {
        let name = format!("mistral:{}", model);
        let cost = match model.as_str() {
            "mistral-large-3" => CostModel {
                input_per_1m_tokens_usd: 2.0,
                output_per_1m_tokens_usd: 6.0,
            },
            _ => CostModel {
                input_per_1m_tokens_usd: 0.0,
                output_per_1m_tokens_usd: 0.0,
            },
        };
        let inner = OpenAIProvider::new(api_key, model.clone())
            .with_base_url("https://api.mistral.ai/v1".into())
            .with_name(name.clone())
            .with_cost(cost);
        Self {
            inner,
            name_cache: name,
            cost,
        }
    }
}

#[async_trait]
impl LLMProvider for MistralProvider {
    fn name(&self) -> &str {
        &self.name_cache
    }
    fn supports_tools(&self) -> bool {
        true
    }
    fn max_context_tokens(&self) -> u32 {
        128_000
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
