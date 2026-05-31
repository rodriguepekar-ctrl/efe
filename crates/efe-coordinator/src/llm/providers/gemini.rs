// SPDX-License-Identifier: UNLICENSED
//! `GeminiProvider` — Google Generative Language API v1beta (Phase 3 R1a).
//!
//! Modèle défaut : `gemini-2.5-flash` ($0.075 in / $0.30 out par M tokens).

use std::time::{Duration, Instant};

use async_trait::async_trait;
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
use super::streaming::parse_gemini_sse;
use super::DEFAULT_GEMINI_MODEL;

use parking_lot::Mutex;
use std::sync::Arc;

const GEMINI_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";

/// Observabilité Gemini : usage du dernier call + safety blocks éventuels.
#[derive(Debug, Clone, Default)]
pub struct GeminiObservability {
    /// Tokens d'input du dernier call (depuis `usageMetadata`).
    pub last_prompt_tokens: Option<u32>,
    /// Tokens d'output du dernier call.
    pub last_candidates_tokens: Option<u32>,
    /// finishReason du dernier candidate (`STOP`, `MAX_TOKENS`, `SAFETY`, etc.).
    pub last_finish_reason: Option<String>,
    /// Si une catégorie safety a `blocked: true` au dernier call.
    pub last_safety_blocked: Option<String>,
}

/// Provider Google Gemini.
pub struct GeminiProvider {
    api_key: ApiKey,
    model: String,
    http: reqwest::Client,
    pricing: PricingTable,
    breaker: CircuitBreaker,
    retry: RetryPolicy,
    redactor: Option<PiiRedactor>,
    last_observability: Arc<Mutex<GeminiObservability>>,
    /// Budget de "thinking tokens" pour Gemini 2.5.
    ///
    /// - `Some(0)` : désactive complètement le thinking (Flash, Flash-lite).
    ///   **REQUIS** sinon Gemini 2.5 Flash consomme tous les tokens d'output
    ///   en raisonnement interne et renvoie une réponse vide.
    /// - `Some(N>0)` : cap explicite (Pro).
    /// - `Some(-1)` : dynamique (API choisit).
    /// - `None` : ne pas envoyer `thinkingConfig` (laisse l'API par défaut).
    ///
    /// Default automatique :
    /// - `*flash*` → `Some(0)` (désactivé)
    /// - `*pro*` → `None` (API default, mais on peut limiter avec `with_thinking_budget(N)`)
    /// - autre → `None`
    thinking_budget: Option<i32>,
}

impl GeminiProvider {
    /// Construit depuis `GOOGLE_AI_KEY`.  Modèle par défaut.
    ///
    /// Pas de garde de préfixe : Google émet des clés AI Studio sous plusieurs
    /// formats (`AIza…`, mais aussi `AQ…` sur les nouvelles clés). Un préfixe
    /// deviné côté client rejette à tort des clés valides ; la validation
    /// d'authentification appartient à l'API Google (400/403). On exige
    /// seulement une valeur non vide (`""` = accepte tout préfixe non vide).
    pub fn from_env() -> CoordResult<Self> {
        let key = ApiKey::from_env("GOOGLE_AI_KEY", "")?;
        Ok(Self::new(key, DEFAULT_GEMINI_MODEL))
    }

    /// Construit avec clé explicite + modèle au choix.
    ///
    /// Le `thinking_budget` par défaut dépend du modèle :
    /// - `*flash*` → `Some(0)` (désactivé — sinon Flash consomme tous les
    ///   tokens d'output en thinking interne et renvoie `text=""`).
    /// - `*pro*` → `None` (API default).
    #[must_use]
    pub fn new(api_key: ApiKey, model: impl Into<String>) -> Self {
        let model = model.into();
        let thinking_budget = if model.to_lowercase().contains("flash") {
            Some(0)
        } else {
            None
        };
        Self {
            api_key,
            model,
            http: build_http_client(),
            pricing: PricingTable::gemini(),
            breaker: CircuitBreaker::new(5, Duration::from_secs(60)),
            retry: RetryPolicy::default(),
            redactor: None,
            last_observability: Arc::new(Mutex::new(GeminiObservability::default())),
            thinking_budget,
        }
    }

    /// Override du `thinkingBudget` Gemini 2.5.  Voir la doc du champ.
    #[must_use]
    pub fn with_thinking_budget(mut self, budget: i32) -> Self {
        self.thinking_budget = Some(budget);
        self
    }

    /// Désactive l'envoi de `thinkingConfig` (laisse l'API choisir).
    #[must_use]
    pub fn without_thinking_config(mut self) -> Self {
        self.thinking_budget = None;
        self
    }

    /// Snapshot du thinking_budget effectif (pour debug + tests).
    #[must_use]
    pub fn thinking_budget(&self) -> Option<i32> {
        self.thinking_budget
    }

    /// Attache un redactor PII (closure) — invariant I15.
    #[must_use]
    pub fn with_redactor(mut self, r: PiiRedactor) -> Self {
        self.redactor = Some(r);
        self
    }

    /// Snapshot des dernières infos observées.
    #[must_use]
    pub fn last_observability(&self) -> GeminiObservability {
        self.last_observability.lock().clone()
    }

    /// Update le snapshot depuis un JSON de réponse Gemini.
    fn update_observability(&self, json_resp: &serde_json::Value) {
        let mut obs = self.last_observability.lock();
        obs.last_prompt_tokens = json_resp["usageMetadata"]["promptTokenCount"]
            .as_u64()
            .map(|v| v as u32);
        obs.last_candidates_tokens = json_resp["usageMetadata"]["candidatesTokenCount"]
            .as_u64()
            .map(|v| v as u32);
        obs.last_finish_reason = json_resp["candidates"][0]["finishReason"]
            .as_str()
            .map(String::from);
        // Cherche une safety category avec blocked=true.
        obs.last_safety_blocked = json_resp["candidates"][0]["safetyRatings"]
            .as_array()
            .and_then(|arr| {
                arr.iter()
                    .find(|r| r["blocked"].as_bool() == Some(true))
                    .and_then(|r| r["category"].as_str().map(String::from))
            });
        // Aussi blockReason au niveau prompt (cas 008_empty_candidates).
        if obs.last_safety_blocked.is_none() {
            obs.last_safety_blocked = json_resp["promptFeedback"]["blockReason"]
                .as_str()
                .map(String::from);
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

    #[allow(dead_code)]
    fn url_non_stream(&self) -> String {
        format!(
            "{}/{}:generateContent?key={}",
            GEMINI_BASE,
            self.model,
            self.api_key.expose()
        )
    }

    fn url_stream(&self) -> String {
        format!(
            "{}/{}:streamGenerateContent?alt=sse&key={}",
            GEMINI_BASE,
            self.model,
            self.api_key.expose()
        )
    }
}

#[async_trait]
impl LlmProvider for GeminiProvider {
    fn id(&self) -> ProviderId {
        format!("gemini_{}", self.model)
    }

    async fn call(&self, req: LlmCall) -> CoordResult<LlmResponse> {
        if self.breaker.is_open() {
            return Err(CoordError::LlmProvider {
                provider: self.id(),
                reason: "circuit breaker open".into(),
            });
        }
        let (system, user) = split_system_user(&req.prompt);
        // Invariant I15 : redact AVANT envoi.
        let user = apply_redactor(self.redactor.as_ref(), &user);
        let system = system.map(|s| apply_redactor(self.redactor.as_ref(), &s));
        let model = self.model.clone();
        let max_tokens = req.max_tokens.max(1);
        let temperature = req.temperature.clamp(0.0, 2.0);
        let api_key = self.api_key.clone();
        let http = self.http.clone();
        let thinking_budget = self.thinking_budget;

        let started = Instant::now();
        let resp = retry_with_backoff(&self.retry, || {
            let mut body = json!({
                "contents": [{
                    "role": "user",
                    "parts": [{"text": &user}]
                }],
                "generationConfig": {
                    "maxOutputTokens": max_tokens,
                    "temperature": temperature,
                }
            });
            // Gemini 2.5 thinking control (fix Flash: thinking_budget=0).
            if let Some(b) = thinking_budget {
                body["generationConfig"]["thinkingConfig"] = json!({
                    "thinkingBudget": b
                });
            }
            if let Some(s) = &system {
                body["systemInstruction"] = json!({"parts": [{"text": s}]});
            }
            let url = format!(
                "{}/{}:generateContent?key={}",
                GEMINI_BASE,
                model,
                api_key.expose()
            );
            let http = http.clone();
            let provider_id = format!("gemini_{model}");
            async move {
                let send = http
                    .post(&url)
                    .header("content-type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| CoordError::LlmProvider {
                        provider: provider_id.clone(),
                        reason: format!("transport: {e}"),
                    })?;
                let status = send.status();
                if !status.is_success() {
                    let body_text = send.text().await.unwrap_or_default();
                    return Err(CoordError::LlmProvider {
                        provider: provider_id,
                        reason: format!(
                            "HTTP {}: {}",
                            status.as_u16(),
                            truncate_error_body(&body_text)
                        ),
                    });
                }
                let json_resp: serde_json::Value = send.json().await.map_err(|e| {
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

        // Mise à jour observabilité (finish_reason, safety, usage).
        self.update_observability(&json_resp);

        // Concatène tous les parts.text du candidate[0].
        let text = json_resp["candidates"][0]["content"]["parts"]
            .as_array()
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|p| p["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();

        let input_tokens =
            json_resp["usageMetadata"]["promptTokenCount"].as_u64().unwrap_or(0) as u32;
        let output_tokens = json_resp["usageMetadata"]["candidatesTokenCount"]
            .as_u64()
            .unwrap_or(0) as u32;
        // R3.S1 (§2.5 + D7) : SINGLE SOURCE OF TRUTH = cost_microcents.
        // Gemini Flash $0.075/M = 7_500 µ¢/M EXACT (vs. arrondi 8 cents/M
        // legacy = +6.7% drift). cost_cents dérivé via `microcents_to_cents`.
        let cost_mc = self
            .pricing
            .calc_microcents(&self.model, input_tokens, output_tokens);
        let cost = crate::llm::microcents_to_cents(cost_mc);

        tracing::info!(
            provider = %self.id(),
            input_tokens,
            output_tokens,
            cost_cents = cost,
            cost_microcents = cost_mc,
            latency_ms = started.elapsed().as_millis() as u64,
            "gemini.call ok"
        );

        Ok(LlmResponse {
            text,
            tokens_generated: output_tokens,
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
        let mut body = json!({
            "contents": [{
                "role": "user",
                "parts": [{"text": user}]
            }],
            "generationConfig": {
                "maxOutputTokens": req.max_tokens.max(1),
                "temperature": req.temperature.clamp(0.0, 2.0),
            }
        });
        // Gemini 2.5 thinking control (cf. champ thinking_budget).
        if let Some(b) = self.thinking_budget {
            body["generationConfig"]["thinkingConfig"] = json!({
                "thinkingBudget": b
            });
        }
        if let Some(s) = system {
            body["systemInstruction"] = json!({"parts": [{"text": s}]});
        }
        let url = self.url_stream();
        let started = Instant::now();
        let resp = self
            .http
            .post(&url)
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
        let stream_result = parse_gemini_sse(byte_stream, tx).await;
        match stream_result {
            Ok(res) => {
                self.breaker.record_success();
                // R3.S1 (§2.5 + D7) : cost_cents dérivé de cost_microcents.
                let cost_mc = self.pricing.calc_microcents(
                    &self.model,
                    res.input_tokens,
                    res.output_tokens,
                );
                let cost = crate::llm::microcents_to_cents(cost_mc);
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
        // Endpoint /models existe et est cheap (pas d'inférence).
        let url = format!(
            "{}?key={}",
            GEMINI_BASE,
            self.api_key.expose()
        );
        let resp = self
            .http
            .get(&url)
            .timeout(Duration::from_secs(10))
            .send()
            .await;
        Ok(resp.map(|r| r.status().is_success()).unwrap_or(false))
    }
}

impl std::fmt::Debug for GeminiProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeminiProvider")
            .field("model", &self.model)
            .field("api_key", &self.api_key) // ApiKey redacte
            .field("breaker_state", &self.breaker.state())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_gemini_response_extracts_text() {
        let raw = json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "Hello"}, {"text": " world!"}],
                    "role": "model"
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 5,
                "candidatesTokenCount": 3,
                "totalTokenCount": 8
            }
        });
        let text = raw["candidates"][0]["content"]["parts"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(text, "Hello world!");
        assert_eq!(raw["usageMetadata"]["candidatesTokenCount"], 3);
    }

    #[test]
    fn pricing_flash_cheap() {
        let p = PricingTable::gemini();
        // 10_000 in + 10_000 out → cents/M = 8/30 → (80_000 + 300_000) / 1M ≈ 0.38 → 1
        let cost = p.calc("gemini-2.5-flash", 10_000, 10_000);
        assert!(cost <= 1);
    }

    #[test]
    fn id_format_includes_model() {
        let key = ApiKey::new("AIza-fake-test-key");
        let p = GeminiProvider::new(key, "gemini-2.5-flash");
        assert_eq!(p.id(), "gemini_gemini-2.5-flash");
    }

    #[test]
    fn debug_redacts_api_key() {
        let key = ApiKey::new("AIzaSyABCD1234EFGH5678IJKL");
        let p = GeminiProvider::new(key, DEFAULT_GEMINI_MODEL);
        let dbg = format!("{p:?}");
        assert!(!dbg.contains("ABCD1234"));
    }

    #[test]
    fn with_model_overrides() {
        let key = ApiKey::new("AIza-x");
        let p = GeminiProvider::new(key, DEFAULT_GEMINI_MODEL).with_model("gemini-2.5-pro");
        assert_eq!(p.id(), "gemini_gemini-2.5-pro");
    }

    #[test]
    fn url_stream_uses_sse() {
        let key = ApiKey::new("AIza-x");
        let p = GeminiProvider::new(key, DEFAULT_GEMINI_MODEL);
        let url = p.url_stream();
        assert!(url.contains("streamGenerateContent"));
        assert!(url.contains("alt=sse"));
        assert!(url.contains("key="));
    }

    #[test]
    fn url_non_stream_separate() {
        let key = ApiKey::new("AIza-x");
        let p = GeminiProvider::new(key, DEFAULT_GEMINI_MODEL);
        let url = p.url_non_stream();
        assert!(url.contains("generateContent"));
        assert!(!url.contains("streamGenerateContent"));
        assert!(url.contains("key="));
    }

    #[test]
    fn circuit_breaker_opens_after_threshold() {
        let key = ApiKey::new("AIza-x");
        let p = GeminiProvider::new(key, DEFAULT_GEMINI_MODEL);
        for _ in 0..5 {
            p.breaker.record_failure();
        }
        assert!(p.breaker.is_open());
    }

    #[test]
    fn default_thinking_budget_zero_for_flash() {
        let key = ApiKey::new("AIza-x");
        let p = GeminiProvider::new(key, "gemini-2.5-flash");
        assert_eq!(p.thinking_budget(), Some(0),
            "Flash MUST default to thinking_budget=0 to avoid empty responses");
    }

    #[test]
    fn default_thinking_budget_zero_for_flash_lite() {
        let key = ApiKey::new("AIza-x");
        let p = GeminiProvider::new(key, "gemini-2.5-flash-lite");
        assert_eq!(p.thinking_budget(), Some(0));
    }

    #[test]
    fn default_thinking_budget_none_for_pro() {
        let key = ApiKey::new("AIza-x");
        let p = GeminiProvider::new(key, "gemini-2.5-pro");
        assert_eq!(p.thinking_budget(), None,
            "Pro should default to None (API choice) — Pro thinking is meaningful");
    }

    #[test]
    fn with_thinking_budget_overrides() {
        let key = ApiKey::new("AIza-x");
        let p = GeminiProvider::new(key, "gemini-2.5-flash").with_thinking_budget(512);
        assert_eq!(p.thinking_budget(), Some(512));
    }

    #[test]
    fn without_thinking_config_clears() {
        let key = ApiKey::new("AIza-x");
        let p = GeminiProvider::new(key, "gemini-2.5-flash").without_thinking_config();
        assert_eq!(p.thinking_budget(), None);
    }
}
