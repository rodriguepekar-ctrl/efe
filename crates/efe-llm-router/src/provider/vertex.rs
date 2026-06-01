//! Google Vertex AI Gemini provider (M21 Phase B).
//!
//! Uses the `generateContent` REST endpoint. Auth via GCP access
//! token (refreshed externally — production switches to
//! `google-cloud-auth` Rust crate).

use super::*;
use reqwest::Client;
use serde_json::json;
use std::time::Duration;
use tokio::time::timeout;

fn pricing_for(model: &str) -> CostModel {
    match model {
        "gemini-2.5-pro" => CostModel {
            input_per_1m_tokens_usd: 1.25,
            output_per_1m_tokens_usd: 10.0,
        },
        "gemini-2.5-flash" => CostModel {
            input_per_1m_tokens_usd: 0.075,
            output_per_1m_tokens_usd: 0.30,
        },
        _ => CostModel {
            input_per_1m_tokens_usd: 0.0,
            output_per_1m_tokens_usd: 0.0,
        },
    }
}

fn max_ctx_for(model: &str) -> u32 {
    match model {
        "gemini-2.5-pro" => 2_000_000,
        _ => 1_000_000,
    }
}

pub struct VertexProvider {
    client: Client,
    project: String,
    location: String,
    model: String,
    access_token: String,
    name_cache: String,
}

impl VertexProvider {
    pub fn new(project: String, location: String, model: String, access_token: String) -> Self {
        let name_cache = format!("vertex:{}", model);
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .expect("reqwest client"),
            project,
            location,
            model,
            access_token,
            name_cache,
        }
    }

    fn endpoint(&self) -> String {
        format!(
            "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/google/models/{}:generateContent",
            self.location, self.project, self.location, self.model,
        )
    }

    fn compute_cost(&self, ti: u32, to: u32) -> f64 {
        let cm = pricing_for(&self.model);
        (ti as f64) * cm.input_per_1m_tokens_usd / 1_000_000.0
            + (to as f64) * cm.output_per_1m_tokens_usd / 1_000_000.0
    }
}

#[async_trait]
impl LLMProvider for VertexProvider {
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
        let mut contents = Vec::with_capacity(request.messages.len());
        if let Some(sp) = &request.system_prompt {
            contents.push(json!({
                "role": "user",
                "parts": [{"text": format!("[System]\n{}", sp)}],
            }));
        }
        for m in &request.messages {
            let role = if m.role == "assistant" { "model" } else { "user" };
            contents.push(json!({
                "role": role,
                "parts": [{"text": m.content}],
            }));
        }

        let body = json!({
            "contents": contents,
            "generationConfig": {
                "maxOutputTokens": request.max_tokens,
                "temperature": request.temperature,
            },
        });

        let req_fut = self
            .client
            .post(self.endpoint())
            .bearer_auth(&self.access_token)
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
        if !status.is_success() {
            return Err(ProviderError::Unavailable(
                self.name().into(),
                format!("{}", status),
            ));
        }

        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::InvalidResponse(self.name().into(), e.to_string()))?;

        let text = v["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let usage = &v["usageMetadata"];
        let tokens_in = usage["promptTokenCount"].as_u64().unwrap_or(0) as u32;
        let tokens_out = usage["candidatesTokenCount"].as_u64().unwrap_or(0) as u32;

        Ok(CompleteResponse {
            text,
            tokens_in,
            tokens_out,
            cost_usd: self.compute_cost(tokens_in, tokens_out),
            finish_reason: FinishReason::Stop,
            tool_calls: None,
            cache_hit: false,
            provider_name: self.name().into(),
            model_name: self.model.clone(),
        })
    }

    async fn health(&self) -> ProviderHealth {
        ProviderHealth {
            available: !self.access_token.is_empty(),
            latency_p50_ms: None,
            latency_p99_ms: None,
            error_rate_30d: None,
            last_check_unix: chrono::Utc::now().timestamp(),
        }
    }
}
