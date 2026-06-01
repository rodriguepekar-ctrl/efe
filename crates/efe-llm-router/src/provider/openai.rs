//! OpenAI Chat Completions provider (M21 Phase B).
//!
//! Also re-used by VLLMProvider and HuggingFaceTGIProvider via custom
//! `base_url` since both expose OpenAI-compatible HTTP APIs.

use super::*;
use reqwest::Client;
use serde_json::json;
use std::time::Duration;
use tokio::time::timeout;

fn pricing_for(model: &str) -> CostModel {
    match model {
        "gpt-5" => CostModel {
            input_per_1m_tokens_usd: 10.0,
            output_per_1m_tokens_usd: 40.0,
        },
        "gpt-5-mini" => CostModel {
            input_per_1m_tokens_usd: 0.30,
            output_per_1m_tokens_usd: 1.20,
        },
        "gpt-4o" => CostModel {
            input_per_1m_tokens_usd: 2.50,
            output_per_1m_tokens_usd: 10.0,
        },
        _ => CostModel {
            input_per_1m_tokens_usd: 0.0,
            output_per_1m_tokens_usd: 0.0,
        },
    }
}

fn max_ctx_for(model: &str) -> u32 {
    match model {
        "gpt-5" => 256_000,
        _ => 128_000,
    }
}

pub struct OpenAIProvider {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
    name_cache: String,
    cost_override: Option<CostModel>,
    max_ctx_override: Option<u32>,
}

impl OpenAIProvider {
    pub fn new(api_key: String, model: String) -> Self {
        let name_cache = format!("openai:{}", model);
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .expect("reqwest client"),
            api_key,
            base_url: "https://api.openai.com/v1".into(),
            model,
            name_cache,
            cost_override: None,
            max_ctx_override: None,
        }
    }

    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url;
        self
    }

    /// Override displayed name (used by VLLMProvider / LlamaCppProvider).
    pub fn with_name(mut self, name: String) -> Self {
        self.name_cache = name;
        self
    }

    pub fn with_cost(mut self, cost: CostModel) -> Self {
        self.cost_override = Some(cost);
        self
    }

    pub fn with_max_context(mut self, max: u32) -> Self {
        self.max_ctx_override = Some(max);
        self
    }

    fn compute_cost(&self, ti: u32, to: u32) -> f64 {
        let cm = self.cost_model();
        (ti as f64) * cm.input_per_1m_tokens_usd / 1_000_000.0
            + (to as f64) * cm.output_per_1m_tokens_usd / 1_000_000.0
    }

    fn map_finish_reason(s: Option<&str>) -> FinishReason {
        match s.unwrap_or("") {
            "stop" => FinishReason::Stop,
            "length" => FinishReason::Length,
            "tool_calls" | "function_call" => FinishReason::ToolUse,
            "content_filter" => FinishReason::Safety,
            _ => FinishReason::Stop,
        }
    }
}

#[async_trait]
impl LLMProvider for OpenAIProvider {
    fn name(&self) -> &str {
        &self.name_cache
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn max_context_tokens(&self) -> u32 {
        self.max_ctx_override.unwrap_or_else(|| max_ctx_for(&self.model))
    }

    fn cost_model(&self) -> CostModel {
        self.cost_override.unwrap_or_else(|| pricing_for(&self.model))
    }

    async fn complete(
        &self,
        request: CompleteRequest,
    ) -> Result<CompleteResponse, ProviderError> {
        let mut messages = Vec::with_capacity(request.messages.len() + 1);
        if let Some(sp) = &request.system_prompt {
            messages.push(json!({"role": "system", "content": sp}));
        }
        for m in &request.messages {
            messages.push(json!({"role": m.role, "content": m.content}));
        }

        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "max_completion_tokens": request.max_tokens,
            "temperature": request.temperature,
        });
        if let Some(seed) = request.seed {
            body["seed"] = json!(seed);
        }
        if let Some(tools) = &request.tools {
            body["tools"] = json!(tools.iter().map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            }).collect::<Vec<_>>());
        }

        let req_fut = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .header("content-type", "application/json")
            .json(&body)
            .send();

        let resp = timeout(Duration::from_millis(request.timeout_ms as u64), req_fut)
            .await
            .map_err(|_| ProviderError::Timeout(self.name().into(), request.timeout_ms))?
            .map_err(|e| ProviderError::Unavailable(self.name().into(), e.to_string()))?;

        let status = resp.status();
        if status == 401 || status == 403 {
            return Err(ProviderError::AuthError(self.name().into()));
        }
        if status == 429 {
            return Err(ProviderError::RateLimited(self.name().into()));
        }
        if status.is_server_error() {
            return Err(ProviderError::Unavailable(
                self.name().into(),
                format!("{}", status),
            ));
        }
        if !status.is_success() {
            return Err(ProviderError::InvalidResponse(
                self.name().into(),
                format!("{}", status),
            ));
        }

        let json_resp: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::InvalidResponse(self.name().into(), e.to_string()))?;

        let choice = json_resp["choices"]
            .get(0)
            .ok_or_else(|| ProviderError::InvalidResponse(self.name().into(), "no choices".into()))?;
        let message = &choice["message"];
        let text = message["content"].as_str().unwrap_or("").to_string();
        let finish_reason = Self::map_finish_reason(choice["finish_reason"].as_str());

        let tool_calls = message["tool_calls"].as_array().map(|arr| {
            arr.iter()
                .map(|tc| ToolCall {
                    id: tc["id"].as_str().unwrap_or("").into(),
                    name: tc["function"]["name"].as_str().unwrap_or("").into(),
                    input: serde_json::from_str(
                        tc["function"]["arguments"].as_str().unwrap_or("{}"),
                    )
                    .unwrap_or(json!({})),
                })
                .collect::<Vec<_>>()
        }).filter(|v| !v.is_empty());

        let usage = &json_resp["usage"];
        let tokens_in = usage["prompt_tokens"].as_u64().unwrap_or(0) as u32;
        let tokens_out = usage["completion_tokens"].as_u64().unwrap_or(0) as u32;

        Ok(CompleteResponse {
            text,
            tokens_in,
            tokens_out,
            cost_usd: self.compute_cost(tokens_in, tokens_out),
            finish_reason,
            tool_calls,
            cache_hit: false,
            provider_name: self.name().into(),
            model_name: self.model.clone(),
        })
    }

    async fn health(&self) -> ProviderHealth {
        let req = CompleteRequest {
            system_prompt: None,
            messages: vec![Message::user("ping")],
            max_tokens: 10,
            temperature: 0.0,
            seed: None,
            timeout_ms: 10_000,
            tools: None,
        };
        let available = self.complete(req).await.is_ok();
        ProviderHealth {
            available,
            latency_p50_ms: None,
            latency_p99_ms: None,
            error_rate_30d: None,
            last_check_unix: chrono::Utc::now().timestamp(),
        }
    }
}
