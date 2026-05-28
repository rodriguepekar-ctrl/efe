// SPDX-License-Identifier: UNLICENSED
//! `GptProvider` — OpenAI Chat Completions API v1 (Phase 3 R1b).
//!
//! Modèle défaut : `gpt-4o` ($2.50 in / $10 out par M tokens).
//!
//! ## Endpoint
//!
//! ```text
//! POST https://api.openai.com/v1/chat/completions
//! Headers:
//!   Authorization: Bearer $OPENAI_API_KEY
//!   content-type: application/json
//!   OpenAI-Organization: $OPENAI_ORG_ID  (optionnel)
//! ```
//!
//! ## Différences vs Anthropic/Gemini
//!
//! - Auth via `Authorization: Bearer ...` (pas `x-api-key`).
//! - Modèle déclaré dans le body JSON, pas dans le path.
//! - SSE termine sur `data: [DONE]\n\n` (sentinel literal).
//! - Tool calls dans `choices[0].message.tool_calls[]` (vs blocs `tool_use`).
//! - Pas de `<thinking>` natif (sauf si demandé explicitement).

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::json;
use tokio::sync::mpsc;

use crate::error::{CoordError, CoordResult};
use crate::llm::{LlmCall, LlmProvider, LlmResponse, ProviderId, TokenChunk};

use super::auth::ApiKey;
use super::circuit::CircuitBreaker;
use super::common::{
    apply_redactor, build_http_client, split_system_user, truncate_error_body, PiiRedactor,
};
use super::pricing::PricingTable;
use super::retry::{retry_with_backoff, RetryPolicy};
use super::streaming::parse_openai_sse;
use super::DEFAULT_GPT_MODEL;

const OPENAI_URL: &str = "https://api.openai.com/v1/chat/completions";

/// Snapshot des informations rate-limit lues depuis les headers OpenAI.
///
/// OpenAI expose `x-ratelimit-{limit,remaining,reset}-{requests,tokens}`.
/// Format de `reset_*` : durée humaine (`6s`, `1m30s`) — on stocke verbatim.
#[derive(Debug, Clone, Default)]
pub struct OpenAiRateLimits {
    /// `x-ratelimit-limit-requests`
    pub requests_limit: Option<u32>,
    /// `x-ratelimit-remaining-requests`
    pub requests_remaining: Option<u32>,
    /// `x-ratelimit-reset-requests` (ex: `6s`, `1m30s`).
    pub requests_reset: Option<String>,
    /// `x-ratelimit-limit-tokens`
    pub tokens_limit: Option<u32>,
    /// `x-ratelimit-remaining-tokens`
    pub tokens_remaining: Option<u32>,
    /// `x-ratelimit-reset-tokens`
    pub tokens_reset: Option<String>,
    /// Organisation associée (du header `openai-organization`).
    pub organization: Option<String>,
}

/// Provider OpenAI GPT.
pub struct GptProvider {
    api_key: ApiKey,
    model: String,
    /// Organization ID optionnel (header `OpenAI-Organization`).
    organization: Option<String>,
    http: reqwest::Client,
    pricing: PricingTable,
    breaker: CircuitBreaker,
    retry: RetryPolicy,
    redactor: Option<PiiRedactor>,
    last_rate_limits: Arc<Mutex<OpenAiRateLimits>>,
    /// Si `Some(true)`, inclut `stream_options.include_usage=true` dans les
    /// requêtes streaming pour recevoir `usage` dans le dernier chunk.
    /// Default `true` — utile pour le BudgetTracker.
    include_usage_in_stream: bool,
}

impl GptProvider {
    /// Construit depuis `OPENAI_API_KEY`. Modèle par défaut `gpt-4o`.
    ///
    /// Validate que la clé commence par `sk-` (clés legacy ET nouvelles
    /// `sk-proj-...` partagent ce préfixe).
    pub fn from_env() -> CoordResult<Self> {
        let key = ApiKey::from_env("OPENAI_API_KEY", "sk-")?;
        Ok(Self::new(key, DEFAULT_GPT_MODEL))
    }

    /// Construit avec clé explicite + modèle au choix.
    #[must_use]
    pub fn new(api_key: ApiKey, model: impl Into<String>) -> Self {
        Self {
            api_key,
            model: model.into(),
            organization: None,
            http: build_http_client(),
            pricing: PricingTable::openai(),
            breaker: CircuitBreaker::new(5, Duration::from_secs(60)),
            retry: RetryPolicy::default(),
            redactor: None,
            last_rate_limits: Arc::new(Mutex::new(OpenAiRateLimits::default())),
            include_usage_in_stream: true,
        }
    }

    /// Override du modèle.
    #[must_use]
    pub fn with_model(mut self, m: impl Into<String>) -> Self {
        self.model = m.into();
        self
    }

    /// Attache un organization ID (header `OpenAI-Organization`).
    /// Requis pour les comptes multi-org afin d'attribuer la facturation.
    #[must_use]
    pub fn with_organization(mut self, org: impl Into<String>) -> Self {
        self.organization = Some(org.into());
        self
    }

    /// Attache un redactor PII (invariant I15).
    #[must_use]
    pub fn with_redactor(mut self, r: PiiRedactor) -> Self {
        self.redactor = Some(r);
        self
    }

    /// Override de la politique de retry.
    #[must_use]
    pub fn with_retry(mut self, r: RetryPolicy) -> Self {
        self.retry = r;
        self
    }

    /// Active/désactive `stream_options.include_usage` dans les calls streaming.
    /// Default `true` — recommandé pour avoir le coût exact post-stream.
    #[must_use]
    pub fn with_include_usage_in_stream(mut self, include: bool) -> Self {
        self.include_usage_in_stream = include;
        self
    }

    /// Snapshot des derniers rate-limits observés.
    #[must_use]
    pub fn last_rate_limits(&self) -> OpenAiRateLimits {
        self.last_rate_limits.lock().clone()
    }

    /// Suffixe ID redacté.
    #[must_use]
    pub fn key_suffix(&self) -> String {
        self.api_key.suffix_redacted()
    }

    /// Extrait texte + tool_calls éventuels d'une réponse OpenAI.
    /// Convention : tool_calls sérialisés en `[tool_call id=X name=Y]\n{args}\n[/tool_call]`
    /// pour cohérence avec le format Claude `[tool_use ...]`.
    pub(crate) fn extract_text_and_tools(json_resp: &serde_json::Value) -> String {
        let message = &json_resp["choices"][0]["message"];
        let mut out = String::new();
        if let Some(t) = message["content"].as_str() {
            out.push_str(t);
        }
        if let Some(tool_calls) = message["tool_calls"].as_array() {
            for tc in tool_calls {
                let id = tc["id"].as_str().unwrap_or("?");
                let func = &tc["function"];
                let name = func["name"].as_str().unwrap_or("?");
                let args = func["arguments"].as_str().unwrap_or("{}");
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str(&format!(
                    "[tool_call id={id} name={name}]\n{args}\n[/tool_call]"
                ));
            }
        }
        out
    }
}

/// Helper : snapshot des headers OpenAI rate-limit.
fn snapshot_rate_limits_into(
    target: &Arc<Mutex<OpenAiRateLimits>>,
    headers: &reqwest::header::HeaderMap,
) {
    let mut rl = target.lock();
    let u32_of = |k: &str| -> Option<u32> {
        headers
            .get(k)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok())
    };
    let str_of = |k: &str| -> Option<String> {
        headers
            .get(k)
            .and_then(|v| v.to_str().ok())
            .map(String::from)
    };
    if let Some(v) = u32_of("x-ratelimit-limit-requests") {
        rl.requests_limit = Some(v);
    }
    if let Some(v) = u32_of("x-ratelimit-remaining-requests") {
        rl.requests_remaining = Some(v);
    }
    if let Some(v) = str_of("x-ratelimit-reset-requests") {
        rl.requests_reset = Some(v);
    }
    if let Some(v) = u32_of("x-ratelimit-limit-tokens") {
        rl.tokens_limit = Some(v);
    }
    if let Some(v) = u32_of("x-ratelimit-remaining-tokens") {
        rl.tokens_remaining = Some(v);
    }
    if let Some(v) = str_of("x-ratelimit-reset-tokens") {
        rl.tokens_reset = Some(v);
    }
    if let Some(v) = str_of("openai-organization") {
        rl.organization = Some(v);
    }
}

#[async_trait]
impl LlmProvider for GptProvider {
    fn id(&self) -> ProviderId {
        format!("gpt_{}", self.model)
    }

    async fn call(&self, req: LlmCall) -> CoordResult<LlmResponse> {
        if self.breaker.is_open() {
            return Err(CoordError::LlmProvider {
                provider: self.id(),
                reason: "circuit breaker open".into(),
            });
        }
        let (system, user) = split_system_user(&req.prompt);
        // Invariant I15 : redact AVANT envoi externe.
        let user = apply_redactor(self.redactor.as_ref(), &user);
        let system = system.map(|s| apply_redactor(self.redactor.as_ref(), &s));
        let model = self.model.clone();
        let max_tokens = req.max_tokens.max(1);
        // OpenAI accepte temperature 0..2.
        let temperature = req.temperature.clamp(0.0, 2.0);
        let api_key = self.api_key.clone();
        let org = self.organization.clone();
        let http = self.http.clone();
        let rate_limits_snap = Arc::clone(&self.last_rate_limits);

        let started = Instant::now();
        let resp = retry_with_backoff(&self.retry, || {
            // Construit le tableau de messages selon présence du system prompt.
            let messages: Vec<serde_json::Value> = match &system {
                Some(s) => vec![
                    json!({"role": "system", "content": s}),
                    json!({"role": "user", "content": &user}),
                ],
                None => vec![json!({"role": "user", "content": &user})],
            };
            let body = json!({
                "model": &model,
                "messages": messages,
                "max_tokens": max_tokens,
                "temperature": temperature,
                "stream": false,
            });
            let http = http.clone();
            let api_key = api_key.clone();
            let org = org.clone();
            let rate_limits_snap = Arc::clone(&rate_limits_snap);
            let provider_id = format!("gpt_{model}");
            async move {
                let mut request = http
                    .post(OPENAI_URL)
                    .header("authorization", format!("Bearer {}", api_key.expose()))
                    .header("content-type", "application/json")
                    .json(&body);
                if let Some(o) = &org {
                    request = request.header("openai-organization", o);
                }
                let req_send = request.send().await.map_err(|e| CoordError::LlmProvider {
                    provider: provider_id.clone(),
                    reason: format!("transport: {e}"),
                })?;
                let status = req_send.status();
                // Snapshot rate-limits MÊME en cas d'erreur (utile pour 429).
                snapshot_rate_limits_into(&rate_limits_snap, req_send.headers());
                if !status.is_success() {
                    let body_text = req_send.text().await.unwrap_or_default();
                    return Err(CoordError::LlmProvider {
                        provider: provider_id,
                        reason: format!(
                            "HTTP {}: {}",
                            status.as_u16(),
                            truncate_error_body(&body_text)
                        ),
                    });
                }
                let json_resp: serde_json::Value =
                    req_send.json().await.map_err(|e| CoordError::LlmProvider {
                        provider: provider_id,
                        reason: format!("response json parse: {e}"),
                    })?;
                Ok(json_resp)
            }
        })
        .await;

        let json_resp = match resp {
            Ok(v) => {
                self.breaker.record_success();
                v
            }
            Err(e) => {
                self.breaker.record_failure();
                return Err(e);
            }
        };

        let text = Self::extract_text_and_tools(&json_resp);
        let prompt_tokens = json_resp["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32;
        let completion_tokens =
            json_resp["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32;
        let cost = self.pricing.calc(&self.model, prompt_tokens, completion_tokens);
        // R3.S1 : précision µ¢ (= cost × 1000 pour OpenAI, pas de sub-cent
        // dans la grille OpenAI mais on harmonise l'API).
        let cost_mc =
            self.pricing
                .calc_microcents(&self.model, prompt_tokens, completion_tokens);

        tracing::info!(
            provider = %self.id(),
            prompt_tokens,
            completion_tokens,
            cost_cents = cost,
            cost_microcents = cost_mc,
            latency_ms = started.elapsed().as_millis() as u64,
            "gpt.call ok"
        );

        Ok(LlmResponse {
            text,
            tokens_generated: completion_tokens,
            cost_cents: cost,
            cost_microcents: cost_mc,
            latency_ms: started.elapsed().as_millis() as u64,
            provider: self.id(),
            self_quality: 1.0,
        })
    }

    async fn stream(
        &self,
        req: LlmCall,
        tx: mpsc::Sender<TokenChunk>,
    ) -> CoordResult<LlmResponse> {
        if self.breaker.is_open() {
            return Err(CoordError::LlmProvider {
                provider: self.id(),
                reason: "circuit breaker open".into(),
            });
        }
        let (system, user) = split_system_user(&req.prompt);
        let user = apply_redactor(self.redactor.as_ref(), &user);
        let system = system.map(|s| apply_redactor(self.redactor.as_ref(), &s));
        let messages: Vec<serde_json::Value> = match &system {
            Some(s) => vec![
                json!({"role": "system", "content": s}),
                json!({"role": "user", "content": user}),
            ],
            None => vec![json!({"role": "user", "content": user})],
        };
        let mut body = json!({
            "model": &self.model,
            "messages": messages,
            "max_tokens": req.max_tokens.max(1),
            "temperature": req.temperature.clamp(0.0, 2.0),
            "stream": true,
        });
        if self.include_usage_in_stream {
            body["stream_options"] = json!({"include_usage": true});
        }
        let started = Instant::now();
        let mut request = self
            .http
            .post(OPENAI_URL)
            .header("authorization", format!("Bearer {}", self.api_key.expose()))
            .header("content-type", "application/json")
            .json(&body);
        if let Some(o) = &self.organization {
            request = request.header("openai-organization", o);
        }
        let resp = request.send().await.map_err(|e| {
            self.breaker.record_failure();
            CoordError::LlmProvider {
                provider: self.id(),
                reason: format!("transport: {e}"),
            }
        })?;
        // Snapshot rate-limits MÊME en cas d'erreur.
        snapshot_rate_limits_into(&self.last_rate_limits, resp.headers());
        if !resp.status().is_success() {
            self.breaker.record_failure();
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(CoordError::LlmProvider {
                provider: self.id(),
                reason: format!(
                    "HTTP {}: {}",
                    status.as_u16(),
                    truncate_error_body(&body_text)
                ),
            });
        }
        let byte_stream = resp.bytes_stream();
        let stream_result = parse_openai_sse(byte_stream, tx).await;
        match stream_result {
            Ok(res) => {
                self.breaker.record_success();
                let cost = self.pricing.calc(&self.model, res.input_tokens, res.output_tokens);
                // R3.S1 : précision µ¢ pour stream() également.
                let cost_mc = self.pricing.calc_microcents(
                    &self.model,
                    res.input_tokens,
                    res.output_tokens,
                );
                Ok(LlmResponse {
                    text: res.text,
                    tokens_generated: res.output_tokens,
                    cost_cents: cost,
                    cost_microcents: cost_mc,
                    latency_ms: started.elapsed().as_millis() as u64,
                    provider: self.id(),
                    self_quality: 1.0,
                })
            }
            Err(e) => {
                self.breaker.record_failure();
                Err(e)
            }
        }
    }

    async fn health(&self) -> CoordResult<bool> {
        // Endpoint `/v1/models` (GET) — cheap, pas d'inférence.
        let url = "https://api.openai.com/v1/models";
        let mut request = self
            .http
            .get(url)
            .header("authorization", format!("Bearer {}", self.api_key.expose()))
            .timeout(Duration::from_secs(10));
        if let Some(o) = &self.organization {
            request = request.header("openai-organization", o);
        }
        let resp = request.send().await;
        Ok(resp.map(|r| r.status().is_success()).unwrap_or(false))
    }
}

impl std::fmt::Debug for GptProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GptProvider")
            .field("model", &self.model)
            .field("api_key", &self.api_key) // ApiKey redacte
            .field("organization", &self.organization)
            .field("breaker_state", &self.breaker.state())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_response_extracts_text() {
        let raw = json!({
            "id": "chatcmpl-A",
            "object": "chat.completion",
            "created": 1716738900,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "Paris is the capital of France."},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 14, "completion_tokens": 8, "total_tokens": 22}
        });
        let text = GptProvider::extract_text_and_tools(&raw);
        assert_eq!(text, "Paris is the capital of France.");
        assert_eq!(raw["usage"]["prompt_tokens"], 14);
        assert_eq!(raw["usage"]["completion_tokens"], 8);
        assert_eq!(raw["choices"][0]["finish_reason"], "stop");
    }

    #[test]
    fn parse_response_with_tool_call() {
        let raw = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_abc",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"location\":\"Paris\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });
        let text = GptProvider::extract_text_and_tools(&raw);
        assert!(text.contains("[tool_call id=call_abc name=get_weather]"));
        assert!(text.contains("Paris"));
        assert!(text.contains("[/tool_call]"));
    }

    #[test]
    fn parse_response_with_text_and_tool_call_combined() {
        let raw = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "Let me check the weather.",
                    "tool_calls": [{
                        "id": "call_x",
                        "type": "function",
                        "function": {"name": "get_weather", "arguments": "{\"city\":\"Lyon\"}"}
                    }]
                }
            }]
        });
        let text = GptProvider::extract_text_and_tools(&raw);
        assert!(text.starts_with("Let me check the weather."));
        assert!(text.contains("[tool_call id=call_x name=get_weather]"));
    }

    #[test]
    fn pricing_gpt4o_default() {
        let p = PricingTable::openai();
        // GPT-4o : 1000 in + 500 out → (1000 × 250 + 500 × 1000) / 1M = 0.75 → 1
        let cost = p.calc("gpt-4o", 1000, 500);
        assert_eq!(cost, 1);
    }

    #[test]
    fn id_format_includes_model() {
        let key = ApiKey::new("sk-test-fake");
        let p = GptProvider::new(key, "gpt-4o");
        assert_eq!(p.id(), "gpt_gpt-4o");
    }

    #[test]
    fn debug_redacts_api_key() {
        let key = ApiKey::new("sk-proj-supersecret123");
        let p = GptProvider::new(key, DEFAULT_GPT_MODEL);
        let dbg = format!("{p:?}");
        assert!(!dbg.contains("supersecret"));
        assert!(dbg.contains("GptProvider"));
    }

    #[test]
    fn with_model_overrides() {
        let key = ApiKey::new("sk-x");
        let p = GptProvider::new(key, DEFAULT_GPT_MODEL).with_model("gpt-4o-mini");
        assert_eq!(p.id(), "gpt_gpt-4o-mini");
    }

    #[test]
    fn with_organization_stored() {
        let key = ApiKey::new("sk-x");
        let p = GptProvider::new(key, DEFAULT_GPT_MODEL).with_organization("org-abc123");
        assert_eq!(p.organization, Some("org-abc123".to_string()));
    }

    #[test]
    fn include_usage_in_stream_default_true() {
        let key = ApiKey::new("sk-x");
        let p = GptProvider::new(key, DEFAULT_GPT_MODEL);
        assert!(p.include_usage_in_stream);
    }

    #[test]
    fn include_usage_can_be_disabled() {
        let key = ApiKey::new("sk-x");
        let p = GptProvider::new(key, DEFAULT_GPT_MODEL).with_include_usage_in_stream(false);
        assert!(!p.include_usage_in_stream);
    }

    #[test]
    fn circuit_breaker_opens_after_threshold() {
        let key = ApiKey::new("sk-x");
        let p = GptProvider::new(key, DEFAULT_GPT_MODEL);
        for _ in 0..5 {
            p.breaker.record_failure();
        }
        assert!(p.breaker.is_open());
    }

    #[test]
    fn extract_handles_null_content() {
        // Cas tool_use seul (content=null).
        let raw = json!({
            "choices": [{
                "message": {"role": "assistant", "content": null, "tool_calls": []}
            }]
        });
        let text = GptProvider::extract_text_and_tools(&raw);
        assert_eq!(text, "");
    }

    #[test]
    fn extract_handles_missing_choices() {
        let raw = json!({"object": "chat.completion"});
        let text = GptProvider::extract_text_and_tools(&raw);
        assert_eq!(text, "");
    }
}
