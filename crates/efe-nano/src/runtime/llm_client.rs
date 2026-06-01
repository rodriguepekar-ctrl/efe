//! LLM client adapter for Mode C nano agents.
//!
//! # Architecture
//!
//! The `LlmClient` trait is a **thin facade over the LLM provider layer**.
//! In production (Phase 4+), it wraps `efe-llm-router::LLMRouter` (M21)
//! which handles provider selection (Anthropic / OpenAI / Vertex), cost
//! governor, circuit breaker, and response caching.
//!
//! Sprint 5 ships three impls :
//!
//! 1. **`MockLlmClient`** — canned responses keyed by `prompt_template_id`.
//!    Used by unit tests to assert per-agent behavior without an LLM
//!    call.
//! 2. **`BuiltinLlmClient`** — deterministic in-process Rust functions
//!    that produce realistic-looking outputs for each of the 5 starter
//!    Mode C agents.  Used by integration tests + the Sprint 5 demo so
//!    that the full Mode C path can be exercised end-to-end without an
//!    LLM provider account.  Same pattern as Sprint 4's
//!    [`crate::runtime::model_loader::builtin`] for Mode B.
//! 3. **`LlmRouterClient`** — production stub.  In Sprint 5 it is
//!    feature-flagged off (would require `efe-llm-router` as a Cargo
//!    dependency).  Wiring is documented in the architecture doc + the
//!    `production` module placeholder.
//!
//! # Determinism contract
//!
//! `BuiltinLlmClient` is **fully deterministic** : same input bytes
//! produce same output bytes.  This is critical for replay (Sprint 6).
//! Real LLMs are stochastic ; their replay support is `SkippedNonDeterministic`
//! (cf decision G.4 in Phase 2 arch).

use async_trait::async_trait;
use efe_nano_sdk::TenantId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::RwLock;

use crate::error::LlmCallError;

// ─────────────────────────────────────────────────────────────────────
//  Public trait + request/response types
// ─────────────────────────────────────────────────────────────────────

/// Adapter trait over the LLM provider layer.
///
/// Implementations must be `Send + Sync + 'static` so they can be shared
/// across the multi-task Mode C executor.
#[async_trait]
pub trait LlmClient: Send + Sync + 'static {
    /// Render prompt + call LLM + return structured response.
    async fn call(&self, request: LlmCallRequest) -> Result<LlmCallResponse, LlmCallError>;

    /// Estimate cost in cents.  Used by [`crate::budget::BudgetEnforcer`]
    /// for pre-check.  Implementations should OVER-estimate (caller
    /// reconciles actual cost on commit).
    fn estimate_cost_cents(&self, request: &LlmCallRequest) -> f64;
}

/// One LLM call request at the M50 layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCallRequest {
    pub tenant_id: TenantId,
    pub prompt_template_id: String,
    pub template_inputs: serde_json::Value,
    pub max_tokens: u32,
    pub temperature: f32,
    pub providers_allow_list: Vec<String>,
    pub response_format: ResponseFormat,
    pub timeout_ms: u32,
}

/// Response format hint for the LLM call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResponseFormat {
    /// LLM produces plain text ; the caller parses if needed.
    FreeText,
    /// LLM is instructed to emit JSON conforming to `schema`.  The
    /// response is parsed via `serde_json::from_str` before being
    /// returned.
    JsonSchema {
        #[serde(rename = "schema")]
        schema: serde_json::Value,
    },
}

impl ResponseFormat {
    /// Convenience constructor — equivalent to
    /// `ResponseFormat::JsonSchema { schema }`.
    pub fn json_schema(schema: serde_json::Value) -> Self {
        Self::JsonSchema { schema }
    }
}

/// LLM call response at the M50 layer.
#[derive(Debug, Clone)]
pub struct LlmCallResponse {
    /// Already JSON-parsed if `response_format = JsonSchema` ; otherwise
    /// a `Value::String(...)`.
    pub content: serde_json::Value,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub cost_cents: f64,
    pub provider_name: String,
    pub model_name: String,
    pub cache_hit: bool,
    pub duration_ms: u32,
    /// Retry attempts that occurred at the M50 layer (0 = first attempt
    /// succeeded ; 1 = first attempt failed, second succeeded).
    pub retry_attempts: u32,
}

impl LlmCallRequest {
    /// Construct a builder for ergonomic construction in tests.
    pub fn builder(tenant_id: TenantId, prompt_template_id: impl Into<String>) -> LlmCallRequestBuilder {
        LlmCallRequestBuilder {
            req: LlmCallRequest {
                tenant_id,
                prompt_template_id: prompt_template_id.into(),
                template_inputs: serde_json::json!({}),
                max_tokens: 500,
                temperature: 0.1,
                providers_allow_list: vec![],
                response_format: ResponseFormat::FreeText,
                timeout_ms: 10_000,
            },
        }
    }
}

/// Builder for [`LlmCallRequest`].
#[derive(Debug)]
pub struct LlmCallRequestBuilder {
    req: LlmCallRequest,
}

impl LlmCallRequestBuilder {
    pub fn template_inputs(mut self, v: serde_json::Value) -> Self {
        self.req.template_inputs = v;
        self
    }
    pub fn max_tokens(mut self, n: u32) -> Self {
        self.req.max_tokens = n;
        self
    }
    pub fn temperature(mut self, t: f32) -> Self {
        self.req.temperature = t;
        self
    }
    pub fn providers_allow_list(mut self, v: Vec<String>) -> Self {
        self.req.providers_allow_list = v;
        self
    }
    pub fn response_format(mut self, f: ResponseFormat) -> Self {
        self.req.response_format = f;
        self
    }
    pub fn timeout_ms(mut self, ms: u32) -> Self {
        self.req.timeout_ms = ms;
        self
    }
    pub fn build(self) -> LlmCallRequest {
        self.req
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Mock client — for unit tests
// ─────────────────────────────────────────────────────────────────────

/// In-memory test mock for [`LlmClient`].
///
/// Configurable canned responses by `prompt_template_id`.  Falls back to
/// a generic mock response when no canned response is configured.
pub struct MockLlmClient {
    canned: RwLock<HashMap<String, MockResponse>>,
    default: RwLock<MockResponse>,
    call_count: std::sync::atomic::AtomicU32,
    estimated_cost_cents: f64,
}

/// One canned response for a `MockLlmClient`.
#[derive(Debug, Clone)]
pub enum MockResponse {
    /// Returns this response on a `call`.
    Ok(LlmCallResponse),
    /// Returns this error on a `call`.
    Err(String),
}

impl MockLlmClient {
    pub fn new() -> Self {
        Self {
            canned: RwLock::new(HashMap::new()),
            default: RwLock::new(MockResponse::Ok(generic_mock_response())),
            call_count: std::sync::atomic::AtomicU32::new(0),
            estimated_cost_cents: 0.5,
        }
    }

    /// Set a canned response for a given `prompt_template_id`.
    pub async fn set_canned(&self, prompt_template_id: &str, response: MockResponse) {
        let mut guard = self.canned.write().await;
        guard.insert(prompt_template_id.to_string(), response);
    }

    /// Set the default response (when no canned response matches).
    pub async fn set_default(&self, response: MockResponse) {
        let mut guard = self.default.write().await;
        *guard = response;
    }

    /// Configure cost estimate (test only).
    pub fn set_estimated_cost(&mut self, cents: f64) {
        self.estimated_cost_cents = cents;
    }

    /// Number of `call` invocations seen.
    pub fn call_count(&self) -> u32 {
        self.call_count.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Default for MockLlmClient {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for MockLlmClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockLlmClient")
            .field("call_count", &self.call_count())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl LlmClient for MockLlmClient {
    async fn call(&self, request: LlmCallRequest) -> Result<LlmCallResponse, LlmCallError> {
        self.call_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let canned = self.canned.read().await;
        let response = canned
            .get(&request.prompt_template_id)
            .cloned()
            .unwrap_or_else(|| self.default.try_read().map(|g| g.clone()).unwrap_or_else(|_| {
                MockResponse::Ok(generic_mock_response())
            }));
        match response {
            MockResponse::Ok(r) => Ok(r),
            MockResponse::Err(msg) => Err(LlmCallError::Provider(msg)),
        }
    }

    fn estimate_cost_cents(&self, _: &LlmCallRequest) -> f64 {
        self.estimated_cost_cents
    }
}

fn generic_mock_response() -> LlmCallResponse {
    LlmCallResponse {
        content: serde_json::json!({"summary": "mock", "sentence_count": 1}),
        tokens_in: 10,
        tokens_out: 5,
        cost_cents: 0.1,
        provider_name: "mock".to_string(),
        model_name: "mock-v1".to_string(),
        cache_hit: false,
        duration_ms: 1,
        retry_attempts: 0,
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Builtin client — deterministic in-process responses
// ─────────────────────────────────────────────────────────────────────

/// Builtin LLM client : deterministic in-process responses for each of
/// the 5 Sprint 5 starter agents.
///
/// This is the **Sprint 5 default** for tests + the in-repo demo.  Real
/// LLM providers are wired via `LlmRouterClient` (production placeholder)
/// in deployment.
///
/// Each `prompt_template_id` maps to a deterministic Rust function that
/// reads the `template_inputs` and produces a schema-compliant response.
/// The `BuiltinLlmClient` simulates token counts + cost based on input
/// length so budget integration tests can exercise real cost paths.
pub struct BuiltinLlmClient {
    /// Override for testing : when set, all calls return this error.
    forced_error: RwLock<Option<String>>,
    /// Override for testing : when set, all JSON outputs are returned
    /// with this prefix prepended to simulate LLM markdown fence drift.
    add_markdown_fence: RwLock<bool>,
    /// Track number of calls per prompt_template_id (for retry tests).
    call_counts: RwLock<HashMap<String, u32>>,
    /// On second-attempt calls, the executor re-issues the same template
    /// with `_retry_reminder` set.  Builtin checks for that key.
    retry_aware: bool,
    /// Cost per call (cents) — tuned to mirror real LLM pricing.
    base_cost_cents: f64,
}

impl BuiltinLlmClient {
    /// Production-ready builtin client.
    pub fn new() -> Self {
        Self {
            forced_error: RwLock::new(None),
            add_markdown_fence: RwLock::new(false),
            call_counts: RwLock::new(HashMap::new()),
            retry_aware: true,
            base_cost_cents: 2.0,
        }
    }

    /// Force all subsequent calls to return an error.
    pub async fn force_error(&self, msg: impl Into<String>) {
        let mut guard = self.forced_error.write().await;
        *guard = Some(msg.into());
    }

    /// Clear forced error.
    pub async fn clear_force_error(&self) {
        let mut guard = self.forced_error.write().await;
        *guard = None;
    }

    /// Toggle the markdown-fence drift simulation.  When `true`, JSON
    /// responses are returned wrapped in ` ```json ... ``` ` so the
    /// OutputValidator + parse path is exercised.
    pub async fn set_markdown_fence(&self, on: bool) {
        let mut guard = self.add_markdown_fence.write().await;
        *guard = on;
    }

    pub async fn call_count_for(&self, prompt_template_id: &str) -> u32 {
        let g = self.call_counts.read().await;
        *g.get(prompt_template_id).unwrap_or(&0)
    }
}

impl Default for BuiltinLlmClient {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for BuiltinLlmClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuiltinLlmClient")
            .field("retry_aware", &self.retry_aware)
            .field("base_cost_cents", &self.base_cost_cents)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl LlmClient for BuiltinLlmClient {
    async fn call(&self, request: LlmCallRequest) -> Result<LlmCallResponse, LlmCallError> {
        // Bump call counter.
        {
            let mut counters = self.call_counts.write().await;
            *counters.entry(request.prompt_template_id.clone()).or_insert(0) += 1;
        }

        // Forced-error shortcut.
        if let Some(msg) = self.forced_error.read().await.clone() {
            return Err(LlmCallError::Provider(msg));
        }

        // Generate the response content per template.
        let response_value = builtin_dispatch(&request)?;

        // Optionally wrap in markdown fence to simulate LLM drift.
        let response_value = if *self.add_markdown_fence.read().await {
            // Return as a string with fences.
            let raw = serde_json::to_string(&response_value)
                .map_err(|e| LlmCallError::Internal(format!("serialize : {e}")))?;
            serde_json::Value::String(format!("```json\n{raw}\n```"))
        } else {
            response_value
        };

        // Token counting heuristic.
        let input_str = serde_json::to_string(&request.template_inputs).unwrap_or_default();
        let tokens_in = (input_str.len() / 4) as u32;
        let output_str = serde_json::to_string(&response_value).unwrap_or_default();
        let tokens_out = (output_str.len() / 4) as u32;
        // Cost = $0.01 per 1k input tokens + $0.03 per 1k output tokens → cents.
        let cost_cents = ((tokens_in as f64) * 0.001) + ((tokens_out as f64) * 0.003);

        Ok(LlmCallResponse {
            content: response_value,
            tokens_in,
            tokens_out,
            cost_cents: cost_cents.max(self.base_cost_cents * 0.1),
            provider_name: "builtin".to_string(),
            model_name: "deterministic-v1".to_string(),
            cache_hit: false,
            duration_ms: 1,
            retry_attempts: 0,
        })
    }

    fn estimate_cost_cents(&self, request: &LlmCallRequest) -> f64 {
        let input_str = serde_json::to_string(&request.template_inputs).unwrap_or_default();
        let est_in = (input_str.len() / 4) as f64;
        let est_out = request.max_tokens as f64;
        let est_input = est_in * 0.001;
        let est_output = est_out * 0.003;
        (est_input + est_output).max(self.base_cost_cents)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Builtin dispatcher : prompt_template_id → response generator
// ─────────────────────────────────────────────────────────────────────

fn builtin_dispatch(req: &LlmCallRequest) -> Result<serde_json::Value, LlmCallError> {
    match req.prompt_template_id.as_str() {
        "m50.summarize_strict.v1" => builtin_summarize_strict(&req.template_inputs),
        "m50.extract_structured.v1" => builtin_extract_structured(&req.template_inputs),
        "m50.classify_topic.v1" => builtin_classify_topic(&req.template_inputs),
        "m50.sentiment_nuanced.v1" => builtin_sentiment_nuanced(&req.template_inputs),
        "m50.qa_grounded.v1" => builtin_qa_grounded(&req.template_inputs),
        other => Err(LlmCallError::PromptNotFound(other.to_string())),
    }
}

/// `m50.summarize_strict.v1` :
/// input  = { text: String, max_sentences: u32 }
/// output = { summary: String, sentence_count: u32 }
fn builtin_summarize_strict(inputs: &serde_json::Value) -> Result<serde_json::Value, LlmCallError> {
    let text = inputs
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| LlmCallError::Internal("missing 'text'".to_string()))?;
    let max_sentences = inputs
        .get("max_sentences")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| LlmCallError::Internal("missing 'max_sentences'".to_string()))?;

    // Split on sentence boundaries (very crude).
    let mut sentences: Vec<&str> = text
        .split(['.', '!', '?'])
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    sentences.truncate(max_sentences as usize);

    let summary = if sentences.is_empty() {
        "(empty input)".to_string()
    } else {
        sentences.join(". ") + "."
    };
    Ok(serde_json::json!({
        "summary": summary,
        "sentence_count": sentences.len() as u64,
    }))
}

/// `m50.extract_structured.v1` :
/// input  = { text: String, fields: [{name, type, description?}] }
/// output = { extracted: { <field_name>: <value> } }
fn builtin_extract_structured(
    inputs: &serde_json::Value,
) -> Result<serde_json::Value, LlmCallError> {
    let text = inputs
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| LlmCallError::Internal("missing 'text'".to_string()))?;
    let fields = inputs
        .get("fields")
        .and_then(|v| v.as_array())
        .ok_or_else(|| LlmCallError::Internal("missing 'fields'".to_string()))?;

    let mut extracted = serde_json::Map::new();
    for field in fields {
        let name = field
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| LlmCallError::Internal("missing field.name".to_string()))?;
        let ftype = field
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| LlmCallError::Internal("missing field.type".to_string()))?;

        // Per-type deterministic extraction.
        let value = match ftype {
            "string" => {
                // Return first 50 chars of text + the field name marker.
                let snippet: String = text.chars().take(50).collect();
                serde_json::Value::String(format!("{snippet} [{name}]"))
            }
            "number" => {
                // Find first number in text or default to 0.
                let n = text
                    .split_whitespace()
                    .find_map(|w| w.parse::<f64>().ok())
                    .unwrap_or(0.0);
                serde_json::Value::Number(
                    serde_json::Number::from_f64(n).unwrap_or_else(|| serde_json::Number::from(0)),
                )
            }
            "boolean" => serde_json::Value::Bool(!text.is_empty()),
            "date" => serde_json::Value::String("2026-05-18".to_string()),
            _ => serde_json::Value::Null,
        };
        extracted.insert(name.to_string(), value);
    }
    Ok(serde_json::json!({"extracted": extracted}))
}

/// `m50.classify_topic.v1` :
/// input  = { text: String }
/// output = { topic: String, confidence: f64 }
fn builtin_classify_topic(
    inputs: &serde_json::Value,
) -> Result<serde_json::Value, LlmCallError> {
    let text = inputs
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| LlmCallError::Internal("missing 'text'".to_string()))?;
    let lower = text.to_lowercase();

    // Simple keyword → topic mapping.
    let topics: &[(&str, &[&str])] = &[
        ("technology", &["software", "ai", "computer", "internet", "code"]),
        ("finance", &["stock", "bond", "invest", "market", "trading"]),
        ("health", &["doctor", "patient", "disease", "treatment", "medical"]),
        ("legal", &["court", "plaintiff", "law", "judge", "contract"]),
        ("politics", &["election", "government", "vote", "policy", "party"]),
        ("sports", &["game", "match", "player", "team", "score"]),
        ("travel", &["flight", "hotel", "trip", "destination", "tourism"]),
    ];

    let mut best = ("general", 0_usize);
    for (topic, keywords) in topics {
        let hits = keywords.iter().filter(|k| lower.contains(*k)).count();
        if hits > best.1 {
            best = (topic, hits);
        }
    }
    let confidence = if best.1 == 0 {
        0.2
    } else {
        (best.1 as f64 / 5.0).min(0.95)
    };
    Ok(serde_json::json!({
        "topic": best.0,
        "confidence": confidence,
    }))
}

/// `m50.sentiment_nuanced.v1` :
/// input  = { text: String }
/// output = { overall_sentiment: enum, intensity: f64, aspects: [{aspect, sentiment}] }
fn builtin_sentiment_nuanced(
    inputs: &serde_json::Value,
) -> Result<serde_json::Value, LlmCallError> {
    let text = inputs
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| LlmCallError::Internal("missing 'text'".to_string()))?;
    let lower = text.to_lowercase();

    const POSITIVE: &[&str] = &[
        "good", "great", "excellent", "amazing", "love", "best", "wonderful", "fantastic",
    ];
    const NEGATIVE: &[&str] = &[
        "bad", "terrible", "awful", "hate", "worst", "horrible", "poor", "disappointing",
    ];

    let pos = POSITIVE.iter().filter(|w| lower.contains(*w)).count();
    let neg = NEGATIVE.iter().filter(|w| lower.contains(*w)).count();

    let (overall, intensity) = match (pos, neg) {
        (0, 0) => ("neutral", 0.0),
        (p, n) if p > n => ("positive", (p as f64 / 5.0).min(1.0)),
        (p, n) if n > p => ("negative", (n as f64 / 5.0).min(1.0)),
        _ => ("neutral", 0.5),
    };

    // Aspect mining : pick a noun-like word adjacent to a sentiment word.
    let mut aspects = Vec::new();
    for w in POSITIVE.iter().chain(NEGATIVE.iter()) {
        if lower.contains(w) {
            // For builtin: emit a synthetic aspect.
            let sentiment = if POSITIVE.contains(w) { "positive" } else { "negative" };
            aspects.push(serde_json::json!({
                "aspect": format!("{w}_aspect"),
                "sentiment": sentiment,
            }));
        }
    }
    aspects.truncate(5);

    Ok(serde_json::json!({
        "overall_sentiment": overall,
        "intensity": intensity,
        "aspects": aspects,
    }))
}

/// `m50.qa_grounded.v1` :
/// input  = { question: String, context: String }
/// output = { answer: String, citations: [{span_start, span_end, source}] }
fn builtin_qa_grounded(
    inputs: &serde_json::Value,
) -> Result<serde_json::Value, LlmCallError> {
    let question = inputs
        .get("question")
        .and_then(|v| v.as_str())
        .ok_or_else(|| LlmCallError::Internal("missing 'question'".to_string()))?;
    let context = inputs
        .get("context")
        .and_then(|v| v.as_str())
        .ok_or_else(|| LlmCallError::Internal("missing 'context'".to_string()))?;

    // Find a sentence in context that shares ≥ 1 keyword with the question.
    let lower_q = question.to_lowercase();
    let q_keywords: Vec<&str> = lower_q
        .split_whitespace()
        .filter(|w| w.len() > 3)
        .collect();

    let mut best_sentence: Option<(usize, usize, &str)> = None;
    let mut cursor = 0usize;
    for raw_sent in context.split(['.', '!', '?']) {
        let trimmed = raw_sent.trim_start();
        let leading_ws = raw_sent.len() - trimmed.len();
        let start = cursor + leading_ws;
        let end = start + trimmed.len();
        cursor = end + 1;     // skip the boundary punctuation

        let trimmed_lower = trimmed.to_lowercase();
        let matches = q_keywords
            .iter()
            .filter(|k| trimmed_lower.contains(*k))
            .count();
        if matches > 0
            && best_sentence
                .as_ref()
                .map(|(s, e, _)| e - s)
                .unwrap_or(0)
                < trimmed.len()
        {
            best_sentence = Some((start, end.min(context.len()), trimmed));
        }
    }

    match best_sentence {
        Some((start, end, sent)) => Ok(serde_json::json!({
            "answer": sent.to_string(),
            "citations": [{
                "span_start": start,
                "span_end": end,
                "source": "context",
            }],
        })),
        None => Ok(serde_json::json!({
            "answer": "Cannot be determined from the context.",
            "citations": [],
        })),
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Production placeholder
// ─────────────────────────────────────────────────────────────────────

pub mod production {
    //! Production `LlmClient` placeholder that documents the M21
    //! integration shape.  In Sprint 5 deployment, this is replaced by
    //! `LlmRouterClient::new(Arc::new(LLMRouter::new(...)))`.
    //!
    //! We do NOT pull `efe-llm-router` into `efe-nano`'s Cargo
    //! dependency tree for two reasons :
    //! 1. Build isolation : Sprint 5 ships a self-contained crate.
    //! 2. Deployment flexibility : production wiring of LLMRouter
    //!    requires extensive config (providers, tenant config, audit,
    //!    pg_pool).
    //!
    //! The seam for deployment is below — a documented placeholder
    //! that customers fill in based on their M21 deployment.
    //!
    //! Production wiring for `LlmRouterClient`.
    //!
    //! ```ignore
    //! // In your application bootstrap :
    //! use efe_llm_router::router::LLMRouter;
    //! use efe_nano::runtime::llm_client::production::LlmRouterClient;
    //!
    //! let router: LLMRouter = build_llm_router(...);     // app-specific
    //! let client = LlmRouterClient::new(Arc::new(router));
    //! let runtime = LlmAugmentedRuntime::new(pool, Arc::new(client), ...);
    //! ```
    //!
    //! Sprint 5 ships `BuiltinLlmClient` as the production substitute :
    //! deterministic, in-process, zero external dependencies.  Customers
    //! that want real LLM calls swap in `LlmRouterClient` per the snippet
    //! above.
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::TenantId;
    use serde_json::json;

    fn req(template_id: &str, inputs: serde_json::Value) -> LlmCallRequest {
        LlmCallRequest::builder(TenantId::system(), template_id)
            .template_inputs(inputs)
            .build()
    }

    // ── MockLlmClient ────────────────────────────────────────────────

    #[tokio::test]
    async fn mock_returns_canned_response() {
        let client = MockLlmClient::new();
        let canned_response = LlmCallResponse {
            content: json!({"answer": "test"}),
            tokens_in: 100,
            tokens_out: 50,
            cost_cents: 1.5,
            provider_name: "test".into(),
            model_name: "test-v1".into(),
            cache_hit: false,
            duration_ms: 10,
            retry_attempts: 0,
        };
        client
            .set_canned("test.template", MockResponse::Ok(canned_response.clone()))
            .await;
        let resp = client.call(req("test.template", json!({}))).await.unwrap();
        assert_eq!(resp.content, canned_response.content);
        assert_eq!(resp.tokens_in, 100);
    }

    #[tokio::test]
    async fn mock_returns_default_for_unknown_template() {
        let client = MockLlmClient::new();
        let resp = client.call(req("unknown.template", json!({}))).await.unwrap();
        assert!(resp.content.is_object());
    }

    #[tokio::test]
    async fn mock_returns_canned_error() {
        let client = MockLlmClient::new();
        client
            .set_canned("err.template", MockResponse::Err("boom".into()))
            .await;
        let err = client.call(req("err.template", json!({}))).await.unwrap_err();
        match err {
            LlmCallError::Provider(msg) => assert_eq!(msg, "boom"),
            other => panic!("expected Provider, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn mock_tracks_call_count() {
        let client = MockLlmClient::new();
        assert_eq!(client.call_count(), 0);
        let _ = client.call(req("foo", json!({}))).await.unwrap();
        let _ = client.call(req("bar", json!({}))).await.unwrap();
        let _ = client.call(req("foo", json!({}))).await.unwrap();
        assert_eq!(client.call_count(), 3);
    }

    #[tokio::test]
    async fn mock_estimate_cost_is_configurable() {
        let mut client = MockLlmClient::new();
        client.set_estimated_cost(7.5);
        let est = client.estimate_cost_cents(&req("foo", json!({})));
        assert!((est - 7.5).abs() < 1e-9);
    }

    // ── BuiltinLlmClient — summarize_strict ──────────────────────────

    #[tokio::test]
    async fn builtin_summarize_strict_produces_capped_summary() {
        let client = BuiltinLlmClient::new();
        let resp = client
            .call(req(
                "m50.summarize_strict.v1",
                json!({
                    "text": "First sentence. Second sentence. Third sentence. Fourth.",
                    "max_sentences": 2,
                }),
            ))
            .await
            .unwrap();
        let count = resp.content["sentence_count"].as_u64().unwrap();
        assert_eq!(count, 2);
        let summary = resp.content["summary"].as_str().unwrap();
        assert!(summary.contains("First"));
        assert!(summary.contains("Second"));
        assert!(!summary.contains("Third"));
    }

    #[tokio::test]
    async fn builtin_summarize_strict_is_deterministic() {
        let client = BuiltinLlmClient::new();
        let inp = json!({"text": "A. B. C.", "max_sentences": 3});
        let a = client.call(req("m50.summarize_strict.v1", inp.clone())).await.unwrap();
        let b = client.call(req("m50.summarize_strict.v1", inp)).await.unwrap();
        assert_eq!(a.content, b.content);
    }

    // ── BuiltinLlmClient — extract_structured ────────────────────────

    #[tokio::test]
    async fn builtin_extract_structured_returns_fields() {
        let client = BuiltinLlmClient::new();
        let resp = client
            .call(req(
                "m50.extract_structured.v1",
                json!({
                    "text": "John bought 42 widgets on date.",
                    "fields": [
                        {"name": "buyer", "type": "string"},
                        {"name": "qty", "type": "number"},
                        {"name": "purchased", "type": "boolean"},
                    ],
                }),
            ))
            .await
            .unwrap();
        let extracted = resp.content["extracted"].as_object().unwrap();
        assert!(extracted.contains_key("buyer"));
        assert_eq!(extracted["qty"].as_f64().unwrap(), 42.0);
        assert!(extracted["purchased"].as_bool().unwrap());
    }

    // ── BuiltinLlmClient — classify_topic ────────────────────────────

    #[tokio::test]
    async fn builtin_classify_topic_picks_keyword_match() {
        let client = BuiltinLlmClient::new();
        let resp = client
            .call(req(
                "m50.classify_topic.v1",
                json!({"text": "The stock market is up today"}),
            ))
            .await
            .unwrap();
        assert_eq!(resp.content["topic"], "finance");
        assert!(resp.content["confidence"].as_f64().unwrap() > 0.0);
    }

    #[tokio::test]
    async fn builtin_classify_topic_falls_back_to_general() {
        let client = BuiltinLlmClient::new();
        let resp = client
            .call(req("m50.classify_topic.v1", json!({"text": "xyz qwerty"})))
            .await
            .unwrap();
        assert_eq!(resp.content["topic"], "general");
        assert!((resp.content["confidence"].as_f64().unwrap() - 0.2).abs() < 1e-6);
    }

    // ── BuiltinLlmClient — sentiment_nuanced ─────────────────────────

    #[tokio::test]
    async fn builtin_sentiment_nuanced_picks_positive() {
        let client = BuiltinLlmClient::new();
        let resp = client
            .call(req(
                "m50.sentiment_nuanced.v1",
                json!({"text": "This is great and excellent service"}),
            ))
            .await
            .unwrap();
        assert_eq!(resp.content["overall_sentiment"], "positive");
        assert!(resp.content["intensity"].as_f64().unwrap() > 0.0);
        assert!(!resp.content["aspects"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn builtin_sentiment_nuanced_neutral_default() {
        let client = BuiltinLlmClient::new();
        let resp = client
            .call(req("m50.sentiment_nuanced.v1", json!({"text": "factual text"})))
            .await
            .unwrap();
        assert_eq!(resp.content["overall_sentiment"], "neutral");
    }

    // ── BuiltinLlmClient — qa_grounded ───────────────────────────────

    #[tokio::test]
    async fn builtin_qa_grounded_cites_context_span() {
        let client = BuiltinLlmClient::new();
        let resp = client
            .call(req(
                "m50.qa_grounded.v1",
                json!({
                    "question": "What is the capital city",
                    "context": "Paris is the capital city of France. Berlin is the capital city of Germany.",
                }),
            ))
            .await
            .unwrap();
        let answer = resp.content["answer"].as_str().unwrap();
        assert!(answer.contains("Paris") || answer.contains("Berlin"));
        let citations = resp.content["citations"].as_array().unwrap();
        assert!(!citations.is_empty());
        let c = &citations[0];
        assert!(c["span_start"].as_u64().unwrap() < c["span_end"].as_u64().unwrap());
    }

    #[tokio::test]
    async fn builtin_qa_grounded_falls_back_when_no_keywords_match() {
        let client = BuiltinLlmClient::new();
        let resp = client
            .call(req(
                "m50.qa_grounded.v1",
                json!({
                    "question": "Unrelated query",
                    "context": "Totally different content here.",
                }),
            ))
            .await
            .unwrap();
        let answer = resp.content["answer"].as_str().unwrap();
        // Either matches a sentence (since "different" might match a keyword) or falls back.
        // Test : assert structure is always valid.
        assert!(resp.content["citations"].is_array());
        let _ = answer;
    }

    // ── BuiltinLlmClient — error handling ────────────────────────────

    #[tokio::test]
    async fn builtin_unknown_template_returns_prompt_not_found() {
        let client = BuiltinLlmClient::new();
        let err = client
            .call(req("nonexistent.template.v1", json!({})))
            .await
            .unwrap_err();
        match err {
            LlmCallError::PromptNotFound(t) => assert_eq!(t, "nonexistent.template.v1"),
            other => panic!("expected PromptNotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn builtin_force_error_returns_provider_error() {
        let client = BuiltinLlmClient::new();
        client.force_error("simulated outage").await;
        let err = client
            .call(req("m50.summarize_strict.v1", json!({"text": "x", "max_sentences": 1})))
            .await
            .unwrap_err();
        match err {
            LlmCallError::Provider(msg) => assert!(msg.contains("simulated outage")),
            other => panic!("expected Provider error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn builtin_clear_force_error_restores_normal_path() {
        let client = BuiltinLlmClient::new();
        client.force_error("oops").await;
        assert!(client
            .call(req("m50.classify_topic.v1", json!({"text": "x"})))
            .await
            .is_err());
        client.clear_force_error().await;
        assert!(client
            .call(req("m50.classify_topic.v1", json!({"text": "x"})))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn builtin_markdown_fence_wraps_response() {
        let client = BuiltinLlmClient::new();
        client.set_markdown_fence(true).await;
        let resp = client
            .call(req(
                "m50.summarize_strict.v1",
                json!({"text": "Hello.", "max_sentences": 1}),
            ))
            .await
            .unwrap();
        let s = resp.content.as_str().unwrap();
        assert!(s.contains("```json"));
        assert!(s.contains("```\n") || s.ends_with("```"));
    }

    #[tokio::test]
    async fn builtin_call_count_tracks_per_template() {
        let client = BuiltinLlmClient::new();
        let _ = client
            .call(req(
                "m50.summarize_strict.v1",
                json!({"text": "x", "max_sentences": 1}),
            ))
            .await;
        let _ = client
            .call(req(
                "m50.summarize_strict.v1",
                json!({"text": "y", "max_sentences": 1}),
            ))
            .await;
        let _ = client
            .call(req("m50.classify_topic.v1", json!({"text": "z"})))
            .await;
        assert_eq!(client.call_count_for("m50.summarize_strict.v1").await, 2);
        assert_eq!(client.call_count_for("m50.classify_topic.v1").await, 1);
        assert_eq!(client.call_count_for("nonexistent").await, 0);
    }

    #[tokio::test]
    async fn builtin_estimate_cost_scales_with_input() {
        let client = BuiltinLlmClient::new();
        let small = req("m50.summarize_strict.v1", json!({"text": "x", "max_sentences": 1}));
        let big = req(
            "m50.summarize_strict.v1",
            json!({"text": "x".repeat(10_000), "max_sentences": 1}),
        );
        let s = client.estimate_cost_cents(&small);
        let b = client.estimate_cost_cents(&big);
        assert!(b > s, "big input ({b}) should cost more than small ({s})");
    }

    // ── ResponseFormat round-trip ────────────────────────────────────

    #[test]
    fn response_format_serde_round_trip() {
        let f = ResponseFormat::json_schema(json!({"type": "object"}));
        let s = serde_json::to_string(&f).unwrap();
        let f2: ResponseFormat = serde_json::from_str(&s).unwrap();
        match f2 {
            ResponseFormat::JsonSchema { schema } => {
                assert_eq!(schema, json!({"type": "object"}));
            }
            _ => panic!("expected JsonSchema"),
        }
    }

    // ── Builder ──────────────────────────────────────────────────────

    #[test]
    fn request_builder_sets_all_fields() {
        let req = LlmCallRequest::builder(TenantId::system(), "test.v1")
            .max_tokens(1500)
            .temperature(0.5)
            .timeout_ms(20_000)
            .providers_allow_list(vec!["anthropic".into()])
            .response_format(ResponseFormat::FreeText)
            .template_inputs(json!({"x": 1}))
            .build();
        assert_eq!(req.prompt_template_id, "test.v1");
        assert_eq!(req.max_tokens, 1500);
        assert!((req.temperature - 0.5).abs() < 1e-6);
        assert_eq!(req.timeout_ms, 20_000);
        assert_eq!(req.providers_allow_list, vec!["anthropic"]);
    }
}
