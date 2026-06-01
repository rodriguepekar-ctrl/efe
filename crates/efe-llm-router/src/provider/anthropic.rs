//! Anthropic Messages API provider (M21 Phase B).

use super::*;
use reqwest::Client;
use serde_json::json;
use std::time::Duration;
use tokio::time::timeout;

/// Pricing pinned to 2026 list prices ; bump on price-list change.
fn pricing_for(model: &str) -> CostModel {
    match model {
        "claude-opus-4-7" => CostModel {
            input_per_1m_tokens_usd: 15.0,
            output_per_1m_tokens_usd: 75.0,
        },
        "claude-sonnet-4-5" => CostModel {
            input_per_1m_tokens_usd: 3.0,
            output_per_1m_tokens_usd: 15.0,
        },
        "claude-haiku-4" => CostModel {
            input_per_1m_tokens_usd: 0.30,
            output_per_1m_tokens_usd: 1.50,
        },
        _ => CostModel {
            input_per_1m_tokens_usd: 0.0,
            output_per_1m_tokens_usd: 0.0,
        },
    }
}

fn max_ctx_for(model: &str) -> u32 {
    match model {
        "claude-opus-4-7" => 1_000_000,
        _ => 200_000,
    }
}

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
    name_cache: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String, model: String) -> Self {
        let name_cache = format!("anthropic:{}", model);
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .expect("reqwest client"),
            api_key,
            base_url: "https://api.anthropic.com/v1".into(),
            model,
            name_cache,
        }
    }

    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url;
        self
    }

    fn compute_cost(&self, ti: u32, to: u32) -> f64 {
        let cm = pricing_for(&self.model);
        (ti as f64) * cm.input_per_1m_tokens_usd / 1_000_000.0
            + (to as f64) * cm.output_per_1m_tokens_usd / 1_000_000.0
    }

    fn map_stop_reason(s: Option<&str>) -> FinishReason {
        match s.unwrap_or("") {
            "end_turn" | "stop_sequence" => FinishReason::Stop,
            "max_tokens" => FinishReason::Length,
            "tool_use" => FinishReason::ToolUse,
            _ => FinishReason::Stop,
        }
    }
}

#[async_trait]
impl LLMProvider for AnthropicProvider {
    fn name(&self) -> &str {
        &self.name_cache
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn max_context_tokens(&self) -> u32 {
        max_ctx_for(&self.model)
    }

    fn cost_model(&self) -> CostModel {
        pricing_for(&self.model)
    }

    async fn complete(
        &self,
        request: CompleteRequest,
    ) -> Result<CompleteResponse, ProviderError> {
        let mut body = json!({
            "model": self.model,
            "max_tokens": request.max_tokens,
            "temperature": request.temperature,
            "messages": request.messages.iter().map(|m| {
                json!({"role": m.role, "content": m.content})
            }).collect::<Vec<_>>(),
        });
        if let Some(sp) = &request.system_prompt {
            body["system"] = json!(sp);
        }
        if let Some(tools) = &request.tools {
            body["tools"] = serde_json::to_value(tools)
                .map_err(|e| ProviderError::InvalidResponse(self.name().into(), e.to_string()))?;
        }

        let req_fut = self
            .client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
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
            let body_text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::InvalidResponse(
                self.name().into(),
                format!("{}: {}", status, body_text),
            ));
        }

        let json_resp: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::InvalidResponse(self.name().into(), e.to_string()))?;

        let text = json_resp
            .get("content")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|b| {
                        if b.get("type").and_then(|v| v.as_str()) == Some("text") {
                            b.get("text").and_then(|v| v.as_str()).map(String::from)
                        } else {
                            None
                        }
                    })
                    .collect::<String>()
            })
            .unwrap_or_default();

        let tool_calls = json_resp
            .get("content")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|b| {
                        if b.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                            Some(ToolCall {
                                id: b.get("id").and_then(|v| v.as_str()).unwrap_or("").into(),
                                name: b.get("name").and_then(|v| v.as_str()).unwrap_or("").into(),
                                input: b.get("input").cloned().unwrap_or(json!({})),
                            })
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty());

        let usage = &json_resp["usage"];
        let tokens_in = usage["input_tokens"].as_u64().unwrap_or(0) as u32;
        let tokens_out = usage["output_tokens"].as_u64().unwrap_or(0) as u32;
        let stop_reason = json_resp["stop_reason"].as_str();

        Ok(CompleteResponse {
            text,
            tokens_in,
            tokens_out,
            cost_usd: self.compute_cost(tokens_in, tokens_out),
            finish_reason: Self::map_stop_reason(stop_reason),
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
