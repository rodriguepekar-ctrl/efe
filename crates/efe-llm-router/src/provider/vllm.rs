//! vLLM self-host provider — wraps OpenAIProvider with custom base_url.
//!
//! Cost model = $0 (GPU rental amortised separately).

use super::*;
use crate::provider::openai::OpenAIProvider;

const FREE_COST: CostModel = CostModel {
    input_per_1m_tokens_usd: 0.0,
    output_per_1m_tokens_usd: 0.0,
};

pub struct VLLMProvider {
    inner: OpenAIProvider,
    name_cache: String,
    max_context: u32,
}

impl VLLMProvider {
    /// `server_url` example : `http://localhost:8000/v1`.
    pub fn new(server_url: String, model: String, max_context: u32) -> Self {
        let name = format!("vllm:{}", model);
        // vLLM ignores api_key but the OpenAI client requires non-empty.
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
impl LLMProvider for VLLMProvider {
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
