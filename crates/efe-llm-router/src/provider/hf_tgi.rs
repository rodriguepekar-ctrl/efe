//! HuggingFace Text Generation Inference provider — OpenAI-compat.

use super::*;
use crate::provider::openai::OpenAIProvider;

const FREE_COST: CostModel = CostModel {
    input_per_1m_tokens_usd: 0.0,
    output_per_1m_tokens_usd: 0.0,
};

pub struct HuggingFaceTGIProvider {
    inner: OpenAIProvider,
    name_cache: String,
    max_context: u32,
}

impl HuggingFaceTGIProvider {
    pub fn new(server_url: String, model: String, max_context: u32) -> Self {
        let name = format!("hf-tgi:{}", model);
        let inner = OpenAIProvider::new("EMPTY".into(), model.clone())
            .with_base_url(server_url)
            .with_name(name.clone())
            .with_cost(FREE_COST)
            .with_max_context(max_context);
        Self {
            inner,
            name_cache: name,
            max_context,
        }
    }
}

#[async_trait]
impl LLMProvider for HuggingFaceTGIProvider {
    fn name(&self) -> &str {
        &self.name_cache
    }
    fn supports_tools(&self) -> bool {
        false // TGI tool support varies by model — conservative default
    }
    fn max_context_tokens(&self) -> u32 {
        self.max_context
    }
    fn cost_model(&self) -> CostModel {
        FREE_COST
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
