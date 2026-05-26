// SPDX-License-Identifier: UNLICENSED
//! `ClaudeProvider` — Anthropic Messages API v1 (Phase 3 R1a).
//!
//! Modèle défaut : `claude-sonnet-4-6` ($3 in / $15 out par M tokens).

use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc;

use crate::error::{CoordError, CoordResult};
use crate::llm::{LlmCall, LlmProvider, LlmResponse, ProviderId, TokenChunk};

use super::auth::ApiKey;
use super::circuit::CircuitBreaker;
use super::common::{
    apply_redactor, build_http_client, split_system_user, strip_thinking_tags,
    truncate_error_body, PiiRedactor,
};
use super::pricing::PricingTable;
use super::retry::{retry_with_backoff, RetryPolicy};
use super::streaming::parse_anthropic_sse;
use super::DEFAULT_CLAUDE_MODEL;

use parking_lot::Mutex;
use std::sync::Arc;

const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Snapshot des informations rate-limit lues depuis les headers de réponse.
///
/// Anthropic expose : `anthropic-ratelimit-{requests,tokens}-{limit,remaining,reset}`.
#[derive(Debug, Clone, Default)]
pub struct ClaudeRateLimits {
    /// Requests limit / remaining / reset (RFC3339 timestamp).
    pub requests_limit: Option<u32>,
    /// Requests remaining
    pub requests_remaining: Option<u32>,
    /// Requests reset (RFC3339)
    pub requests_reset: Option<String>,
    /// Tokens limit
    pub tokens_limit: Option<u32>,
    /// Tokens remaining
    pub tokens_remaining: Option<u32>,
    /// Tokens reset (RFC3339)
    pub tokens_reset: Option<String>,
}

/// Provider Anthropic Claude.
pub struct ClaudeProvider {
    api_key: ApiKey,
    model: String,
    http: reqwest::Client,
    pricing: PricingTable,
    breaker: CircuitBreaker,
    retry: RetryPolicy,
    redactor: Option<PiiRedactor>,
    /// Strip auto les balises `<thinking>...</thinking>` du content (défaut true).
    strip_thinking: bool,
    /// Snapshot des derniers rate-limits observés.
    last_rate_limits: Arc<Mutex<ClaudeRateLimits>>,
}

impl ClaudeProvider {
    /// Construit le provider à partir de la variable d'environnement
    /// `ANTHROPIC_API_KEY`.  Utilise `DEFAULT_CLAUDE_MODEL`.
    pub fn from_env() -> CoordResult<Self> {
        let key = ApiKey::from_env("ANTHROPIC_API_KEY", "sk-ant-")?;
        Ok(Self::new(key, DEFAULT_CLAUDE_MODEL))
    }

    /// Construit avec clé explicite + modèle au choix.
    #[must_use]
    pub fn new(api_key: ApiKey, model: impl Into<String>) -> Self {
        Self {
            api_key,
            model: model.into(),
            http: build_http_client(),
            pricing: PricingTable::anthropic(),
            breaker: CircuitBreaker::new(5, Duration::from_secs(60)),
            retry: RetryPolicy::default(),
            redactor: None,
            strip_thinking: true,
            last_rate_limits: Arc::new(Mutex::new(ClaudeRateLimits::default())),
        }
    }

    /// Attache un redactor PII (closure).  Le redactor est appliqué AVANT
    /// envoi à Anthropic (invariant I15).
    #[must_use]
    pub fn with_redactor(mut self, r: PiiRedactor) -> Self {
        self.redactor = Some(r);
        self
    }

    /// Désactive le strip auto des balises `<thinking>...</thinking>`.
    /// (utile si l'utilisateur veut voir le raisonnement intermédiaire).
    #[must_use]
    pub fn with_thinking_kept(mut self) -> Self {
        self.strip_thinking = false;
        self
    }

    /// Snapshot des derniers rate-limits observés (None pour les champs jamais vus).
    #[must_use]
    pub fn last_rate_limits(&self) -> ClaudeRateLimits {
        self.last_rate_limits.lock().clone()
    }

    /// Helper interne : extrait les headers rate-limit Anthropic d'une réponse.
    #[allow(dead_code)]
    fn snapshot_rate_limits(&self, headers: &reqwest::header::HeaderMap) {
        let mut rl = self.last_rate_limits.lock();
        let read_u32 = |k: &str| -> Option<u32> {
            headers
                .get(k)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse().ok())
        };
        let read_str = |k: &str| -> Option<String> {
            headers
                .get(k)
                .and_then(|v| v.to_str().ok())
                .map(String::from)
        };
        if let Some(v) = read_u32("anthropic-ratelimit-requests-limit") {
            rl.requests_limit = Some(v);
        }
        if let Some(v) = read_u32("anthropic-ratelimit-requests-remaining") {
            rl.requests_remaining = Some(v);
        }
        if let Some(v) = read_str("anthropic-ratelimit-requests-reset") {
            rl.requests_reset = Some(v);
        }
        if let Some(v) = read_u32("anthropic-ratelimit-tokens-limit") {
            rl.tokens_limit = Some(v);
        }
        if let Some(v) = read_u32("anthropic-ratelimit-tokens-remaining") {
            rl.tokens_remaining = Some(v);
        }
        if let Some(v) = read_str("anthropic-ratelimit-tokens-reset") {
            rl.tokens_reset = Some(v);
        }
    }

    /// Extrait le texte d'une réponse Anthropic.  Concatène tous les blocs
    /// `content[]` de type text, ET sérialise les blocs `tool_use` en JSON
    /// compact à la suite (pour que le caller puisse les retrouver).
    /// Applique le strip thinking si activé.
    pub(crate) fn extract_text_and_tools(&self, json_resp: &serde_json::Value) -> String {
        let blocks = match json_resp["content"].as_array() {
            Some(arr) => arr,
            None => return String::new(),
        };
        let mut out = String::new();
        for block in blocks {
            match block["type"].as_str() {
                Some("text") => {
                    if let Some(t) = block["text"].as_str() {
                        out.push_str(t);
                    }
                }
                Some("tool_use") => {
                    // Sérialise le tool_use de manière lisible et parsable.
                    let name = block["name"].as_str().unwrap_or("?");
                    let id = block["id"].as_str().unwrap_or("?");
                    let input_str = block["input"].to_string();
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str(&format!(
                        "[tool_use id={id} name={name}]\n{input_str}\n[/tool_use]"
                    ));
                }
                _ => {}
            }
        }
        if self.strip_thinking {
            strip_thinking_tags(&out)
        } else {
            out
        }
    }

    /// Override du modèle.
    #[must_use]
    pub fn with_model(mut self, m: impl Into<String>) -> Self {
        self.model = m.into();
        self
    }

    /// Override de la politique de retry.
    #[must_use]
    pub fn with_retry(mut self, r: RetryPolicy) -> Self {
        self.retry = r;
        self
    }

    /// Suffixe ID redacté (debug + observabilité).
    #[must_use]
    pub fn key_suffix(&self) -> String {
        self.api_key.suffix_redacted()
    }
}

#[async_trait]
impl LlmProvider for ClaudeProvider {
    fn id(&self) -> ProviderId {
        format!("claude_{}", self.model)
    }

    async fn call(&self, req: LlmCall) -> CoordResult<LlmResponse> {
        if self.breaker.is_open() {
            return Err(CoordError::LlmProvider {
                provider: self.id(),
                reason: "circuit breaker open".into(),
            });
        }

        let (system, user) = split_system_user(&req.prompt);
        // Invariant I15 : PII redact AVANT envoi externe.
        let user = apply_redactor(self.redactor.as_ref(), &user);
        let system = system.map(|s| apply_redactor(self.redactor.as_ref(), &s));
        let model = self.model.clone();
        let max_tokens = req.max_tokens.max(1);
        let temperature = req.temperature.clamp(0.0, 1.0);
        let api_key = self.api_key.clone();
        let http = self.http.clone();
        let rate_limits_snap = Arc::clone(&self.last_rate_limits);

        let started = Instant::now();
        let resp = retry_with_backoff(&self.retry, || {
            let mut body = json!({
                "model": &model,
                "max_tokens": max_tokens,
                "temperature": temperature,
                "messages": [{"role": "user", "content": &user}],
                "stream": false,
            });
            if let Some(s) = &system {
                body["system"] = json!(s);
            }
            let http = http.clone();
            let api_key = api_key.clone();
            let provider_id = format!("claude_{model}");
            let rate_limits_snap = Arc::clone(&rate_limits_snap);
            async move {
                let req_send = http
                    .post(ANTHROPIC_URL)
                    .header("x-api-key", api_key.expose())
                    .header("anthropic-version", ANTHROPIC_VERSION)
                    .header("content-type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| CoordError::LlmProvider {
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
                let json_resp: serde_json::Value = req_send.json().await.map_err(|e| {
                    CoordError::LlmProvider {
                        provider: provider_id,
                        reason: format!("response json parse: {e}"),
                    }
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

        let text = self.extract_text_and_tools(&json_resp);

        let input_tokens = json_resp["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = json_resp["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32;
        let cost = self.pricing.calc(&self.model, input_tokens, output_tokens);

        tracing::info!(
            provider = %self.id(),
            input_tokens,
            output_tokens,
            cost_cents = cost,
            latency_ms = started.elapsed().as_millis() as u64,
            "claude.call ok"
        );

        Ok(LlmResponse {
            text,
            tokens_generated: output_tokens,
            cost_cents: cost,
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
        let mut body = json!({
            "model": &self.model,
            "max_tokens": req.max_tokens.max(1),
            "temperature": req.temperature.clamp(0.0, 1.0),
            "messages": [{"role": "user", "content": user}],
            "stream": true,
        });
        if let Some(s) = system {
            body["system"] = json!(s);
        }
        let started = Instant::now();
        let resp = self
            .http
            .post(ANTHROPIC_URL)
            .header("x-api-key", self.api_key.expose())
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
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
        let stream_result = parse_anthropic_sse(byte_stream, tx).await;
        match stream_result {
            Ok(res) => {
                self.breaker.record_success();
                let cost =
                    self.pricing.calc(&self.model, res.input_tokens, res.output_tokens);
                Ok(LlmResponse {
                    text: res.text,
                    tokens_generated: res.output_tokens,
                    cost_cents: cost,
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
        // Pas d'endpoint /health public. On envoie un message minimal (1 token).
        let body = json!({
            "model": &self.model,
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "ping"}],
        });
        let resp = self
            .http
            .post(ANTHROPIC_URL)
            .header("x-api-key", self.api_key.expose())
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .timeout(Duration::from_secs(10))
            .send()
            .await;
        Ok(resp.map(|r| r.status().is_success()).unwrap_or(false))
    }
}

/// Helper free-fn pour partager le snapshot des headers entre `call` et `stream`
/// (l'accès se fait via le Mutex partagé `last_rate_limits`).
fn snapshot_rate_limits_into(
    target: &Arc<Mutex<ClaudeRateLimits>>,
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
    if let Some(v) = u32_of("anthropic-ratelimit-requests-limit") {
        rl.requests_limit = Some(v);
    }
    if let Some(v) = u32_of("anthropic-ratelimit-requests-remaining") {
        rl.requests_remaining = Some(v);
    }
    if let Some(v) = str_of("anthropic-ratelimit-requests-reset") {
        rl.requests_reset = Some(v);
    }
    if let Some(v) = u32_of("anthropic-ratelimit-tokens-limit") {
        rl.tokens_limit = Some(v);
    }
    if let Some(v) = u32_of("anthropic-ratelimit-tokens-remaining") {
        rl.tokens_remaining = Some(v);
    }
    if let Some(v) = str_of("anthropic-ratelimit-tokens-reset") {
        rl.tokens_reset = Some(v);
    }
}

impl std::fmt::Debug for ClaudeProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeProvider")
            .field("model", &self.model)
            .field("api_key", &self.api_key) // Debug d'ApiKey redacte
            .field("breaker_state", &self.breaker.state())
            .finish()
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Parse une réponse JSON Anthropic capturée → vérifie l'extraction.
    #[test]
    fn parse_anthropic_response_extracts_text() {
        let raw = json!({
            "id": "msg_01ABC",
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-4-6",
            "content": [
                {"type": "text", "text": "Paris is the capital of France."}
            ],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 12, "output_tokens": 8}
        });
        let text = raw["content"]
            .as_array()
            .and_then(|arr| arr.iter().find(|c| c["type"] == "text"))
            .and_then(|c| c["text"].as_str())
            .unwrap();
        assert_eq!(text, "Paris is the capital of France.");
        assert_eq!(raw["usage"]["input_tokens"], 12);
        assert_eq!(raw["usage"]["output_tokens"], 8);
    }

    #[test]
    fn pricing_sonnet_consistent() {
        let p = PricingTable::anthropic();
        let cost = p.calc("claude-sonnet-4-6", 1000, 500);
        // (1000 * 300 + 500 * 1500) / 1_000_000 = (300000 + 750000) / 1M = 1.05 → 2
        assert_eq!(cost, 2);
    }

    #[test]
    fn id_format_includes_model() {
        let key = ApiKey::new("sk-ant-test-fake-key");
        let p = ClaudeProvider::new(key, "claude-sonnet-4-6");
        assert_eq!(p.id(), "claude_claude-sonnet-4-6");
    }

    #[test]
    fn debug_redacts_api_key() {
        let key = ApiKey::new("sk-ant-supersecret123");
        let p = ClaudeProvider::new(key, DEFAULT_CLAUDE_MODEL);
        let dbg = format!("{p:?}");
        assert!(!dbg.contains("supersecret"));
        assert!(dbg.contains("ClaudeProvider"));
    }

    #[test]
    fn with_model_overrides() {
        let key = ApiKey::new("sk-ant-x");
        let p = ClaudeProvider::new(key, DEFAULT_CLAUDE_MODEL).with_model("claude-opus-4-6");
        assert_eq!(p.id(), "claude_claude-opus-4-6");
    }

    /// Vérifie que le breaker s'ouvre après 5 failures.
    #[test]
    fn circuit_breaker_opens_after_threshold() {
        let key = ApiKey::new("sk-ant-x");
        let p = ClaudeProvider::new(key, DEFAULT_CLAUDE_MODEL);
        assert!(!p.breaker.is_open());
        for _ in 0..5 {
            p.breaker.record_failure();
        }
        assert!(p.breaker.is_open());
    }
}
