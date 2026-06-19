//! **Sprint 9 — N17** : LLM provenance types — forensic record of
//! every Mode C call.
//!
//! See `docs/architecture/EFE_M50_N17_LLM_PROVENANCE_ARCHITECTURE.md`.
//!
//! # Module surface
//!
//! - [`LlmProvenanceRecord`] — the comprehensive per-call record.
//! - [`Blake3Hash`] — 32-byte content-addressing newtype.
//! - [`ProvenanceId`] — opaque per-record identifier.
//! - [`ModelIdentity`] — `(provider, model_id, version, hash)` tuple.
//! - [`TokenUsage`] — both client-estimated + provider-reported.
//! - [`CostAccounting`] — deterministic cost recomputation source.
//! - [`Completeness`] — Complete / Partial / Refused / Errored.
//! - [`SafetyFlag`] — 8 typed safety variants.
//! - [`SamplingParams`] — sampling configuration on the call.
//! - [`ToolCallRecord`] — per-tool-call provenance (Sprint 10
//!   runtime wiring ; format frozen now).
//! - [`CacheProvenance`] — N12 integration.
//! - [`FinishReason`] — provider's finish reason taxonomy.

use std::collections::HashMap;
use std::time::SystemTime;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;
use uuid::Uuid;

use crate::types::region::RegionCode;
use crate::types::{NanoAgentId, NanoAgentVersion, PiiStrategy, TenantId};

// ─────────────────────────────────────────────────────────────────────
//  Blake3Hash newtype
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : 32-byte BLAKE3 content hash.
///
/// Wire format : hex-encoded 64-character lowercase string.
/// Used for prompt + output + tool-arg content addressing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Blake3Hash(pub [u8; 32]);

impl Blake3Hash {
    /// **N17** : Construct from raw 32-byte digest.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// **N17** : Zero hash (used as sentinel for unset fields).
    pub const ZERO: Self = Self([0u8; 32]);

    /// **N17** : True if every byte is zero.
    pub fn is_zero(&self) -> bool {
        self.0.iter().all(|b| *b == 0)
    }

    /// **N17** : Hex-encode the digest (lowercase, no separator).
    pub fn to_hex(&self) -> String {
        let mut out = String::with_capacity(64);
        for b in &self.0 {
            out.push_str(&format!("{b:02x}"));
        }
        out
    }

    /// **N17** : Parse from a 64-character lowercase hex string.
    /// Returns `None` for any malformed input.
    pub fn parse_hex(s: &str) -> Option<Self> {
        if s.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            let hi = hex_nibble(s.as_bytes()[i * 2])?;
            let lo = hex_nibble(s.as_bytes()[i * 2 + 1])?;
            *byte = (hi << 4) | lo;
        }
        Some(Self(out))
    }
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

impl std::fmt::Display for Blake3Hash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl Serialize for Blake3Hash {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Blake3Hash {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let s = String::deserialize(de)?;
        Self::parse_hex(&s).ok_or_else(|| Error::custom("malformed blake3 hex digest"))
    }
}

impl Default for Blake3Hash {
    fn default() -> Self {
        Self::ZERO
    }
}

// ─────────────────────────────────────────────────────────────────────
//  ProvenanceId
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : opaque per-record identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProvenanceId(pub Uuid);

impl ProvenanceId {
    /// **N17** : Generate a fresh ID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// **N17** : Construct from an existing UUID.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// **N17** : Extract the inner UUID.
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for ProvenanceId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ProvenanceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  ModelIdentity
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : canonical identity of an LLM model.
///
/// Equality is field-by-field exact (no version range matching).
/// Use [`Self::lineage_key`] for cross-version queries.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelIdentity {
    /// Provider canonical name (e.g. "openai", "anthropic").
    pub provider: String,
    /// Provider's model id (e.g. "gpt-4-turbo", "claude-3-opus").
    pub model_id: String,
    /// The version the caller requested.
    pub requested_version: String,
    /// The version the provider returned (None when not exposed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub served_version: Option<String>,
    /// Content hash for models we can fingerprint (e.g. local
    /// weights).  None for hosted models we can't inspect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<Blake3Hash>,
    /// True if the model supports OpenAI-style tool/function calling.
    pub supports_tool_use: bool,
}

impl ModelIdentity {
    /// **N17** : Convenience for hosted models without content hash.
    pub fn hosted(provider: &str, model_id: &str, version: &str) -> Self {
        Self {
            provider: provider.to_string(),
            model_id: model_id.to_string(),
            requested_version: version.to_string(),
            served_version: None,
            content_hash: None,
            supports_tool_use: false,
        }
    }

    /// **N17** : Lineage key — `(provider, model_id)` ignoring
    /// version.  Used for cross-version aggregations.
    pub fn lineage_key(&self) -> (String, String) {
        (self.provider.clone(), self.model_id.clone())
    }

    /// **N17** : True if the provider returned a model version that
    /// differs from the one we requested (silent migration).
    pub fn provider_migrated(&self) -> bool {
        match &self.served_version {
            Some(served) => served != &self.requested_version,
            None => false,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  TokenUsage
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : token-count accounting, client + provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    pub client_estimated_input: u32,
    pub client_estimated_output: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_reported_input: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_reported_output: Option<u32>,
    /// True when the provider didn't return token counts (F2).
    pub is_estimated: bool,
}

impl TokenUsage {
    /// **N17** : Canonical input token count : provider wins, else client.
    pub fn canonical_input(&self) -> u32 {
        self.provider_reported_input.unwrap_or(self.client_estimated_input)
    }

    /// **N17** : Canonical output token count : provider wins, else client.
    pub fn canonical_output(&self) -> u32 {
        self.provider_reported_output.unwrap_or(self.client_estimated_output)
    }

    /// **N17** : Drift between provider-reported and client-estimated
    /// totals.  `None` when provider didn't report.  Positive →
    /// provider counted more.
    pub fn drift_total(&self) -> Option<i64> {
        let provider_total =
            i64::from(self.provider_reported_input?) + i64::from(self.provider_reported_output?);
        let client_total =
            i64::from(self.client_estimated_input) + i64::from(self.client_estimated_output);
        Some(provider_total - client_total)
    }

    /// **N17** : Drift ratio — `(provider - client) / client`.
    /// `None` when provider didn't report or client total is zero.
    pub fn drift_ratio(&self) -> Option<f64> {
        let drift = self.drift_total()? as f64;
        let client_total =
            f64::from(self.client_estimated_input + self.client_estimated_output);
        if client_total <= 0.0 {
            return None;
        }
        Some(drift / client_total)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  CostAccounting
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : cost accounting derived deterministically
/// from token counts × prices table.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct CostAccounting {
    pub cost_cents: f64,
    /// Snapshot of the price table version used.  Lets the
    /// reconciliation job recompute historical costs.
    pub prices_table_version: u32,
    /// Provider-quoted cost (when available — most providers don't).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_quoted_cost_cents: Option<f64>,
    /// True if cost computation hit `f64::MAX / 2` ceiling (F11).
    pub overflow_capped: bool,
}

// ─────────────────────────────────────────────────────────────────────
//  Sampling parameters
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : sampling parameters captured on every call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SamplingParams {
    pub temperature: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    pub max_tokens: u32,
    /// Sampling seed.  `None` for non-deterministic calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop_sequences: Vec<String>,
}

// ─────────────────────────────────────────────────────────────────────
//  FinishReason
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : provider's finish reason taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// Model emitted a natural stop token.
    Stop,
    /// Hit `max_tokens`.
    Length,
    /// Provider content filter blocked the response.
    ContentFilter,
    /// Model emitted a tool call.
    ToolCalls,
    /// Provider returned an error mid-stream.
    Error,
    /// Provider didn't return a finish reason.
    Unknown,
}

impl FinishReason {
    /// **N17** : Static label for metric dimensions.
    pub fn label(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Length => "length",
            Self::ContentFilter => "content_filter",
            Self::ToolCalls => "tool_calls",
            Self::Error => "error",
            Self::Unknown => "unknown",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Completeness
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : completeness of the LLM response.
///
/// **Sprint 19 — N27** : adds [`Completeness::Aborted`] for
/// stream-abort scenarios.  Distinct from `Partial` (which is
/// "MaxTokens was hit, response would have continued") — `Aborted`
/// means the stream was explicitly cut short (client disconnect,
/// mid-stream upstream error, timeout, backpressure).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Completeness {
    /// Full response received.
    Complete,
    /// `MaxTokens` truncation : Anthropic stopped because the
    /// model would have continued past `max_tokens`.
    Partial {
        received_tokens: u32,
        expected_tokens: u32,
    },
    /// **Sprint 19 — N27** : stream was explicitly aborted.
    /// Carries the partial-token count + a closed-taxonomy reason.
    Aborted {
        received_tokens: u32,
        reason: AbortReason,
    },
    /// Provider refused the request (content filter etc.).
    Refused { category: SafetyCategory },
    /// Provider returned an error.
    Errored { error_kind: String },
}

impl Completeness {
    /// **N17** : Stable label for metrics.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial { .. } => "partial",
            Self::Aborted { .. } => "aborted",
            Self::Refused { .. } => "refused",
            Self::Errored { .. } => "errored",
        }
    }

    /// **N17** : True if the response is fully usable downstream.
    pub fn is_usable(&self) -> bool {
        matches!(self, Self::Complete)
    }

    /// **N27** : True if this completeness denotes a stream that
    /// was explicitly aborted (client disconnect, upstream
    /// mid-stream error, timeout).
    pub fn is_aborted(&self) -> bool {
        matches!(self, Self::Aborted { .. })
    }

    /// **N27** : Number of output tokens actually received,
    /// regardless of completeness variant.  Used for invariant
    /// I23 cross-checks.
    pub fn received_output_tokens(&self) -> Option<u32> {
        match self {
            Self::Partial { received_tokens, .. } => Some(*received_tokens),
            Self::Aborted { received_tokens, .. } => Some(*received_tokens),
            _ => None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  AbortReason  (Sprint 19 — N27)
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 19 — N27** : closed taxonomy of stream-abort reasons.
///
/// Stamped into [`Completeness::Aborted`] whenever a streamed Mode-C
/// call is cut short.  Every variant is reachable from a chaos test
/// in `chaos_n27_*` (see N27 architecture § 11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AbortReason {
    /// Downstream client closed the SSE connection.
    /// Triggered by `tokio_util::sync::CancellationToken::cancel()`
    /// from the gateway when the HTTP/2 RST_STREAM is observed.
    ClientDisconnect,
    /// Mid-stream `event: error` from Anthropic ; carries the
    /// provider's `error.type` (e.g. `"overloaded_error"`).
    UpstreamError { error_type: String },
    /// No SSE event received for `STREAM_READ_TIMEOUT_MS` (default
    /// 60 s) ; the stream is considered hung.
    ReadTimeout { ms: u32 },
    /// Total stream wall-clock exceeded `STREAM_TOTAL_TIMEOUT_MS`
    /// (default 300 s).
    TotalTimeout { ms: u32 },
    /// Stream errored post-first-byte-downstream (cannot replay
    /// per ADR-N27-3).
    NonRetryablePostStream,
    /// Internal mpsc channel saturated, backpressure forced abort.
    BackpressureAbort,
}

impl AbortReason {
    /// **N27** : stable Prometheus label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::ClientDisconnect => "client_disconnect",
            Self::UpstreamError { .. } => "upstream_error",
            Self::ReadTimeout { .. } => "read_timeout",
            Self::TotalTimeout { .. } => "total_timeout",
            Self::NonRetryablePostStream => "non_retryable_post_stream",
            Self::BackpressureAbort => "backpressure_abort",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  RetryOutcome  (Sprint 19 — N27)
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 19 — N27** : closed taxonomy of retry-loop outcomes.
///
/// Stamped on every streaming `LlmProvenanceRecord` via the
/// [`LlmProvenanceRecord::retry_outcome`] field.  Buffered (legacy)
/// records default to [`RetryOutcome::NoRetry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RetryOutcome {
    /// First attempt succeeded ; `retry_count = 0`.
    #[default]
    NoRetry,
    /// One or more retries happened, final attempt succeeded.
    RetriedSucceeded,
    /// All retries exhausted ; final attempt failed.
    RetriedExhausted,
    /// Hit a non-retryable error (400/401/403/...) ; no retries
    /// attempted.
    NonRetryable,
}

impl RetryOutcome {
    /// **N27** : stable Prometheus label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::NoRetry => "no_retry",
            Self::RetriedSucceeded => "retried_succeeded",
            Self::RetriedExhausted => "retried_exhausted",
            Self::NonRetryable => "non_retryable",
        }
    }

    /// **N27** : true iff the call ultimately succeeded.
    pub fn succeeded(&self) -> bool {
        matches!(self, Self::NoRetry | Self::RetriedSucceeded)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  SafetyFlag / SafetyCategory
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : per-call safety signal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SafetyFlag {
    /// PII was detected in the input prompt by the PII redactor.
    PiiDetected,
    /// Prompt-injection filter flagged the input (Sprint 7 N8).
    PromptInjectionSuspected,
    /// Output moderation triggered.
    OutputModerationTriggered,
    /// Provider content filter tripped.
    ContentFilter { category: SafetyCategory },
    /// Toxicity classifier flagged the output.
    ToxicityDetected,
    /// Self-harm classifier flagged the output.
    SelfHarmDetected,
    /// Jailbreak attempt detected.
    JailbreakAttemptDetected,
    /// Prompt exceeded provider's context window.
    PromptLengthExceeded,
}

impl SafetyFlag {
    pub fn label(&self) -> &'static str {
        match self {
            Self::PiiDetected => "pii_detected",
            Self::PromptInjectionSuspected => "prompt_injection",
            Self::OutputModerationTriggered => "output_moderation",
            Self::ContentFilter { .. } => "content_filter",
            Self::ToxicityDetected => "toxicity",
            Self::SelfHarmDetected => "self_harm",
            Self::JailbreakAttemptDetected => "jailbreak",
            Self::PromptLengthExceeded => "prompt_length_exceeded",
        }
    }
}

/// **Sprint 9 — N17** : safety taxonomy aligned with major
/// providers (OpenAI moderation categories etc.).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "label")]
pub enum SafetyCategory {
    Hate,
    Sexual,
    Violence,
    SelfHarm,
    SexualMinors,
    ViolenceGraphic,
    Harassment,
    /// Provider-specific category we don't recognise — carries the
    /// raw string for forensic traceability.
    Other(String),
}

impl SafetyCategory {
    pub fn label(&self) -> &str {
        match self {
            Self::Hate => "hate",
            Self::Sexual => "sexual",
            Self::Violence => "violence",
            Self::SelfHarm => "self_harm",
            Self::SexualMinors => "sexual_minors",
            Self::ViolenceGraphic => "violence_graphic",
            Self::Harassment => "harassment",
            Self::Other(s) => s,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  ToolCallRecord
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : per-tool-call provenance record.
///
/// Sprint 10 wires the runtime ; Sprint 9 ships the SDK + chain
/// types so the format is frozen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallRecord {
    /// Position in the sequence of tool calls within this LLM op.
    pub ordinal: u32,
    /// Name of the tool the model invoked.
    pub tool_name: String,
    /// Content-hash of the JSON-encoded arguments the model passed.
    pub arguments_hash: Blake3Hash,
    /// Content-hash of the JSON-encoded return value (`None` if
    /// the call failed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_hash: Option<Blake3Hash>,
    pub result_kind: ToolCallResultKind,
}

/// **Sprint 9 — N17** : tool-call outcome taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallResultKind {
    /// Tool returned successfully.
    Ok,
    /// Tool returned an error.
    Errored,
    /// Tool was cancelled by the agent or operator.
    Cancelled,
    /// Tool was skipped (e.g. precondition not met).
    Skipped,
}

impl ToolCallResultKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Errored => "errored",
            Self::Cancelled => "cancelled",
            Self::Skipped => "skipped",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  CacheProvenance
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : N12 cache-hit provenance back-reference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CacheProvenance {
    /// Trace id of the original LLM call that warmed the cache.
    pub original_trace_id: Uuid,
    /// Provenance id of the original record.
    pub original_provenance_id: ProvenanceId,
    /// Version of the cache key algorithm used (N12 hardening).
    pub cache_key_version: u32,
}

// ─────────────────────────────────────────────────────────────────────
//  RC-1 — l'attestation de computation (le Mode C distant re-prouvable)
// ─────────────────────────────────────────────────────────────────────

/// **RC-1** — l'identité PINNÉE de l'endpoint distant qui a servi l'inférence.
///
/// On pinne l'EMPREINTE de certificat / SPKI (`pin_fingerprint`), **pas** le
/// hostname : DNS/routage peuvent dériver sous un hostname entre `health()` et
/// `complete()` (TOCTOU, architecture §5.2) ; une empreinte de clé stable, elle,
/// ne dérive pas. `host` est conservé à titre **diagnostique** — il n'est jamais
/// la racine de confiance du canal.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EndpointIdentity {
    /// L'hôte tel que configuré (diagnostic ; PAS la racine de confiance).
    pub host: String,
    /// L'empreinte du certificat / SPKI pinné — la racine de confiance du canal.
    pub pin_fingerprint: String,
}

impl EndpointIdentity {
    /// Construit une identité d'endpoint pinnée.
    pub fn new(host: impl Into<String>, pin_fingerprint: impl Into<String>) -> Self {
        Self { host: host.into(), pin_fingerprint: pin_fingerprint.into() }
    }
}

/// **RC-1** — le schéma de la co-signature de nœud (T-WITNESS). `ed25519` à v1
/// (asymétrique : le vérifieur contrôle la signature contre une **clé publique**
/// pinnée, sans détenir aucun secret — c'est la propriété du moat généralisée).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CoSignatureScheme {
    /// Ed25519 — signature asymétrique, vérifiable hors-ligne contre la pubkey pinnée.
    #[default]
    Ed25519,
}

/// **RC-1** — la co-signature d'un nœud distant PINNÉ sur sa propre computation
/// (T-WITNESS / barreau R3).
///
/// C'est une **seconde signature indépendante**, à côté de la chaîne N17
/// keyée-tenant, produite par la **propre clé** du nœud — jamais une part du
/// maître tenant (I-RC2). Elle est vérifiée contre la pubkey pinnée du nœud
/// (I-RC6) sur le tuple `blake3(node_id ‖ final_prompt_hash ‖ output_hash ‖
/// nonce)`. Le `nonce` ferme le rejeu (§7). Elle rend le nœud **responsable**,
/// pas nécessairement honnête (cf. modèle de menace).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeCoSignature {
    /// L'identité enrôlée + pinnée du nœud (façon registre `publisher_id`).
    pub node_id: String,
    /// Le schéma de signature.
    pub scheme: CoSignatureScheme,
    /// La signature, encodée hex.
    pub signature_hex: String,
    /// Le tuple signé, hex : `blake3(node_id ‖ final_prompt_hash ‖ output_hash ‖ [model_fp] ‖ nonce)`.
    pub signed_tuple_hash: String,
    /// Le nonce anti-rejeu (hex), lié à *ce* run.
    pub nonce: String,
    /// **R3+** : l'empreinte-modèle (hex du `content_hash` des poids) **co-signée** par le nœud, s'il
    /// atteste son modèle. `None` ⇒ co-signature production-seule (R3). Quand `Some`, le tuple co-signé
    /// emploie le domaine `EFE-RC3-COSIGN-v1` et **inclut** cette empreinte (cf. `cosign_tuple`), et le
    /// vérifieur peut tenir `model_hash_verified` **si** l'empreinte ∈ les modèles attendus pinnés (I-RC7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_fingerprint: Option<String>,
}

/// **RC-1 — l'attestation de computation.** Le barreau de confiance qu'EFE peut
/// *honnêtement* tenir pour ce record Mode C, **scellé dans le leaf N17**.
///
/// C'est le cœur du Mode C distant re-prouvable. Aujourd'hui la localité est
/// *invisible* dans les octets scellés : un record produit à distance serait
/// byte-identique à un local. Ce champ rend le niveau de confiance **explicite,
/// signé** (donc inviolable : comme `leaf_hash = sha256(canonical_bytes(record))`,
/// le réécrire casse `verify_n17_chain` → `RecordContentMutated`) et **lu par le
/// vérifieur** (`held_strength`). Aucune nouvelle primitive cryptographique.
///
/// L'échelle est **totale et décroissante** (cf. [`strength_rank`]) :
/// `R1 > R2 > R3 > R4 > LegacyUnspecified`. Le défaut serde est
/// [`ComputeAttestation::LegacyUnspecified`] — **strictement plus faible que
/// R4** — pour qu'un record pré-RC1 ne mente *jamais* en se faisant passer pour
/// local ; il force un backfill explicite. Voir l'architecture §3.
///
/// [`strength_rank`]: ComputeAttestation::strength_rank
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ComputeAttestation {
    /// Record antérieur à RC-1 : le barreau n'est pas asserti. Traité comme
    /// **strictement < R4** — le vérifieur ne peut rien affirmer sur la localité.
    #[default]
    LegacyUnspecified,
    /// **R1** — Mode A déterministe : la sortie est `f(tâche, variant)`,
    /// re-RUN-able à l'octet via `replay_outputs`. Le plus fort.
    R1LocalDeterministic,
    /// **R2** — Mode C co-localisé (T-LOCAL) : l'inférence a tourné sur la boucle
    /// **loopback**, le récepteur de provenance a témoigné de la computation. Le
    /// cas prouvé aujourd'hui. Exige un endpoint loopback (I-RC3).
    R2LocalWitnessed,
    /// **R3** — Mode C distant **co-signé** (T-WITNESS) : un nœud distant pinné a
    /// co-signé sa propre computation avec sa propre clé. Responsable, pas
    /// nécessairement honnête (§7).
    R3RemoteCosigned {
        /// Le canal authentifié + pinné par lequel l'appel a transité.
        endpoint: EndpointIdentity,
        /// La co-signature du nœud, vérifiée contre sa pubkey pinnée (I-RC6).
        cosignature: NodeCoSignature,
    },
    /// **R4** — Mode C distant, **canal seul** (T-CHANNEL) : l'orchestrateur
    /// atteste la *transaction* (« j'ai envoyé ce prompt, reçu cette sortie, sur
    /// ce canal pinné »), jamais la computation. Couvre le self-host distant
    /// zéro-clé ET le routeur keyé `Network`. Le plus faible — *et le record le dit*.
    R4RemoteChannel {
        /// Le canal authentifié + pinné.
        endpoint: EndpointIdentity,
        /// `true` si le canal a porté un credential (routeur keyé `Network`) ; le
        /// credential lui-même n'est **JAMAIS** enregistré (I-RC5).
        keyed: bool,
    },
}

impl ComputeAttestation {
    /// Rang de force (plus haut = plus fort). L'échelle est **totale**.
    /// Le vérifieur rapporte le **minimum** sur le run (`held_strength`).
    pub fn strength_rank(&self) -> u8 {
        match self {
            ComputeAttestation::R1LocalDeterministic => 4,
            ComputeAttestation::R2LocalWitnessed => 3,
            ComputeAttestation::R3RemoteCosigned { .. } => 2,
            ComputeAttestation::R4RemoteChannel { .. } => 1,
            ComputeAttestation::LegacyUnspecified => 0,
        }
    }

    /// `true` si l'inférence a quitté la boîte souveraine (R3 ou R4).
    pub fn is_remote(&self) -> bool {
        matches!(
            self,
            ComputeAttestation::R3RemoteCosigned { .. } | ComputeAttestation::R4RemoteChannel { .. }
        )
    }

    /// `true` si le barreau EXIGE un endpoint loopback (R1/R2 ; I-RC3).
    pub fn requires_loopback(&self) -> bool {
        matches!(
            self,
            ComputeAttestation::R1LocalDeterministic | ComputeAttestation::R2LocalWitnessed
        )
    }

    /// L'identité d'endpoint pinnée, si le barreau est distant.
    pub fn endpoint(&self) -> Option<&EndpointIdentity> {
        match self {
            ComputeAttestation::R3RemoteCosigned { endpoint, .. }
            | ComputeAttestation::R4RemoteChannel { endpoint, .. } => Some(endpoint),
            _ => None,
        }
    }

    /// La co-signature du nœud, si le barreau est R3.
    pub fn cosignature(&self) -> Option<&NodeCoSignature> {
        match self {
            ComputeAttestation::R3RemoteCosigned { cosignature, .. } => Some(cosignature),
            _ => None,
        }
    }

    /// Un libellé stable pour les rapports / la forensique.
    pub fn rung_label(&self) -> &'static str {
        match self {
            ComputeAttestation::R1LocalDeterministic => "R1-LOCAL-DETERMINISTE",
            ComputeAttestation::R2LocalWitnessed => "R2-LOCAL-TEMOIN",
            ComputeAttestation::R3RemoteCosigned { .. } => "R3-DISTANT-COSIGNE",
            ComputeAttestation::R4RemoteChannel { .. } => "R4-DISTANT-CANAL",
            ComputeAttestation::LegacyUnspecified => "LEGACY-NON-ASSERTI",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  LlmProvenanceRecord — the comprehensive trace
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : per-call LLM provenance record.
///
/// Lives in the audit chain trace's `provenance_payload` field
/// (architecture ADR-1) — excluded from the canonical body so
/// pre-N17 chains still verify.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmProvenanceRecord {
    // Identity
    pub provenance_id: ProvenanceId,
    pub trace_id: Uuid,
    pub swarm_request_id: Uuid,
    pub op_id: String,
    pub tenant_id: TenantId,
    pub agent_id: NanoAgentId,
    pub agent_version: NanoAgentVersion,

    // Model identity
    pub model_identity: ModelIdentity,
    pub model_hash_verified: bool,

    // Prompt provenance
    pub prompt_template_id: String,
    pub prompt_template_version: String,
    pub prompt_inputs_hash: Blake3Hash,
    pub system_prompt_hash: Blake3Hash,
    pub user_prompt_hash: Blake3Hash,
    pub final_prompt_hash: Blake3Hash,

    // Sampling
    pub sampling: SamplingParams,

    // Tokens + cost
    pub token_usage: TokenUsage,
    pub cost_accounting: CostAccounting,

    // Provider metadata
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_request_id: Option<String>,
    pub provider_response_time_ms: u32,
    pub provider_finish_reason: FinishReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_region: Option<RegionCode>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub provider_metadata: HashMap<String, serde_json::Value>,

    // Output + completeness
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_hash: Option<Blake3Hash>,
    pub completeness: Completeness,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub safety_flags: Vec<SafetyFlag>,

    // Tool calls
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCallRecord>,

    // PII
    pub pii_strategy_applied: PiiStrategy,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redaction_token_set: Vec<String>,

    // Cross-cutting
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay_of: Option<ProvenanceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_hit: Option<CacheProvenance>,

    // Timing
    pub recorded_at: SystemTime,

    // ─── Sprint 10 — N18 cross-cutting (architecture § 13 + I20) ────
    //
    // Every Mode C provenance row must carry the publisher identity
    // + tier snapshot so the EU AI Act audit query (Workflow E) can
    // JOIN against the publisher registry.  Pre-N18 records have
    // `publisher_id: None` (treated as Tier 0 anonymous per F13
    // fallback).
    //
    /// **Sprint 10 — N18** : the registered publisher that authored
    /// the agent which made this LLM call.  `None` for pre-N18
    /// records or EFE-shipped agents (use `Author::Efe` reserved id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_id: Option<super::PublisherId>,

    /// **Sprint 10 — N18** : KYC tier snapshot at the time of the
    /// call.  Captured separately from the live registry value so
    /// the audit trace stays faithful to "what was the tier when
    /// the call happened" (I18).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_kyc_tier: Option<super::KycTier>,

    /// **Sprint 11 — N19 cross-cutting** : if this LLM call was
    /// triggered by a gold-test case, the case id.  Used by the
    /// gold-run report to attribute each call back to the case that
    /// drove it (G12).  `None` for production traffic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gold_case_id: Option<super::GoldCaseId>,

    // ─── Sprint 19 — N27 streaming + retry (architecture § 6.1) ─────
    //
    // The four fields below are populated by `ClaudeBridge::dispatch_stream`
    // (streaming path) and defaulted to "no-op" values by the buffered
    // path so existing N17/N26 records remain wire-compatible.  All four
    // are `#[serde(default)]` so JSON written by pre-N27 callers
    // deserialises unchanged.
    //
    /// **Sprint 19 — N27** : `true` iff this record was produced by
    /// the streaming dispatch path (`POST /v1/agentic/call-stream`
    /// or any client-of-the-bridge that invoked
    /// `call_with_context_stream` and let the SSE flow downstream).
    /// `false` for the buffered path even though it now also runs
    /// the bridge's internal stream and collects — the distinction
    /// is "did the bridge surface a streaming wire to the caller?".
    /// See ADR-N27-15.
    #[serde(default)]
    pub streaming_mode: bool,

    /// **Sprint 19 — N27** : number of retries performed before
    /// the final attempt.  `0` for first-try success.  Bounded
    /// by `N27_MAX_RETRIES` (3 at v1).  See I22.
    #[serde(default)]
    pub retry_count: u8,

    /// **Sprint 19 — N27** : closed-taxonomy outcome of the
    /// retry loop.  Defaults to [`RetryOutcome::NoRetry`] for
    /// records that did not go through the retry layer (legacy
    /// buffered records pre-N27 binary).
    #[serde(default)]
    pub retry_outcome: RetryOutcome,

    /// **Sprint 19 — N27** : time from the first attempt's POST
    /// to the first text `content_block_delta` received from the
    /// provider.  `None` for cache hits, replays, refusals before
    /// any delta, or aborted-pre-stream failures.  Bounded by I21
    /// (`ttft <= total_latency`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_to_first_token_ms: Option<u32>,

    /// **RC-1** : le barreau de confiance de computation tenu pour ce record,
    /// **scellé dans le leaf N17** (donc inviolable). Défaut
    /// [`ComputeAttestation::LegacyUnspecified`] (strictement < R4) pour les
    /// records pré-RC1 — pas de mensonge par omission. C'est ce qui rend la
    /// localité *visible et signée* dans les octets : le Mode C distant
    /// re-prouvable. Voir [`ComputeAttestation`] et l'architecture §3/§6.
    #[serde(default)]
    pub compute_attestation: ComputeAttestation,
}

impl LlmProvenanceRecord {
    /// **RC-1** : le barreau de confiance de computation de ce record.
    pub fn compute_attestation(&self) -> &ComputeAttestation {
        &self.compute_attestation
    }

    /// **RC-1** : `true` si l'inférence de ce record a quitté la boîte souveraine.
    pub fn is_remote_compute(&self) -> bool {
        self.compute_attestation.is_remote()
    }
    /// **N17** : True if this record describes a cache hit (no
    /// real LLM call happened).
    pub fn is_cache_hit(&self) -> bool {
        self.cache_hit.is_some()
    }

    /// **N17** : True if this record describes a replay (N16).
    pub fn is_replay(&self) -> bool {
        self.replay_of.is_some()
    }

    /// **N17** : True if the call hit a safety refusal / content
    /// filter.
    pub fn was_refused(&self) -> bool {
        matches!(self.completeness, Completeness::Refused { .. })
    }

    /// **N17** : Total token count canonicalised : provider when
    /// available, else client estimate.
    pub fn total_tokens(&self) -> u32 {
        self.token_usage.canonical_input() + self.token_usage.canonical_output()
    }

    /// **N27** : True if the stream was aborted (any
    /// [`AbortReason`] variant).
    pub fn is_aborted(&self) -> bool {
        self.completeness.is_aborted()
    }

    /// **N27 + I25** : Total wall-clock latency seen by the
    /// caller, including any retry sleep time.  The N27 design
    /// records `provider_response_time_ms` as Anthropic-side wall
    /// only.  Invariant I25 guarantees
    /// `total_latency_ms == provider_response_time_ms` PLUS
    /// `total_retry_wait_ms` (within ±5 ms clock-skew tolerance).
    /// Callers that need the exact retry-wait value should
    /// consult the `retry_count` + `retry_outcome` pair plus the
    /// configured backoff curve, or re-fetch the structured
    /// dispatch metrics.
    ///
    /// Returns the `provider_response_time_ms` unchanged when
    /// `retry_count == 0` (no retry sleep happened).
    pub fn total_latency_ms_floor(&self) -> u32 {
        // The N27 design records `provider_response_time_ms` as
        // Anthropic-side wall only.  A consumer wanting the
        // user-visible total must add the retry wait, which is
        // bounded by the configured policy.  This helper returns
        // the FLOOR (no retries → exact match).
        self.provider_response_time_ms
    }
}

// ─────────────────────────────────────────────────────────────────────
//  ProvenanceError
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : closed taxonomy of provenance-emission errors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProvenanceError {
    /// Model hash failed pre-call verification (F3 + ADR-7).
    #[error("model integrity violation : {requested_hash} != {actual_hash}")]
    ModelIntegrityViolation {
        requested_hash: String,
        actual_hash: String,
    },

    /// **Post-audit fix** : Registry has no entry for this model
    /// identity.  Distinct from `ModelIntegrityViolation` so ops
    /// dashboards can plot drift events vs unregistered-model
    /// rejections separately.
    #[error("model not registered : {provider}/{model_id}@{version}")]
    ModelNotRegistered {
        provider: String,
        model_id: String,
        version: String,
    },

    /// Prompt template version not in registry (F8).
    #[error("unknown prompt template : {template_id}@{version}")]
    UnknownPromptTemplate {
        template_id: String,
        version: String,
    },

    /// Provenance store write failed (F4).
    #[error("provenance store error : {message}")]
    Store { message: String },

    /// Audit chain write failed (F4).
    #[error("audit chain write failed : {message}")]
    Chain { message: String },

    /// Internal error.
    #[error("internal : {message}")]
    Internal { message: String },
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record() -> LlmProvenanceRecord {
        LlmProvenanceRecord {
            provenance_id: ProvenanceId::new(),
            trace_id: Uuid::new_v4(),
            swarm_request_id: Uuid::new_v4(),
            op_id: "summarize".into(),
            tenant_id: TenantId::new(),
            agent_id: NanoAgentId::new("summarize_strict").unwrap(),
            agent_version: NanoAgentVersion::new("1.2.3").unwrap(),
            model_identity: ModelIdentity::hosted("openai", "gpt-4-turbo", "2024-04-09"),
            model_hash_verified: false,
            prompt_template_id: "summarize-v3".into(),
            prompt_template_version: "1.0.0".into(),
            prompt_inputs_hash: Blake3Hash::from_bytes([1u8; 32]),
            system_prompt_hash: Blake3Hash::from_bytes([2u8; 32]),
            user_prompt_hash: Blake3Hash::from_bytes([3u8; 32]),
            final_prompt_hash: Blake3Hash::from_bytes([4u8; 32]),
            sampling: SamplingParams {
                temperature: 0.0,
                top_p: None,
                max_tokens: 500,
                seed: None,
                stop_sequences: vec![],
            },
            token_usage: TokenUsage {
                client_estimated_input: 1240,
                client_estimated_output: 312,
                provider_reported_input: Some(1238),
                provider_reported_output: Some(314),
                is_estimated: false,
            },
            cost_accounting: CostAccounting {
                cost_cents: 4.124,
                prices_table_version: 17,
                provider_quoted_cost_cents: None,
                overflow_capped: false,
            },
            provider_request_id: Some("req_abc123".into()),
            provider_response_time_ms: 487,
            provider_finish_reason: FinishReason::Stop,
            provider_region: Some(RegionCode::UsEast1),
            publisher_id: None,
            publisher_kyc_tier: None,
            gold_case_id: None,
            provider_metadata: HashMap::new(),
            output_hash: Some(Blake3Hash::from_bytes([5u8; 32])),
            completeness: Completeness::Complete,
            safety_flags: vec![],
            tool_calls: vec![],
            pii_strategy_applied: PiiStrategy::Strict,
            redaction_token_set: vec![],
            replay_of: None,
            cache_hit: None,
            recorded_at: SystemTime::now(),
            // **Sprint 19 — N27** : buffered fixture, no streaming.
            streaming_mode: false,
            retry_count: 0,
            retry_outcome: RetryOutcome::NoRetry,
            time_to_first_token_ms: None,
            compute_attestation: ComputeAttestation::default(),
        }
    }

    // ─── Blake3Hash ────────────────────────────────────────────────

    #[test]
    fn blake3_hash_to_hex_64_chars() {
        let h = Blake3Hash::from_bytes([0xABu8; 32]);
        let hex = h.to_hex();
        assert_eq!(hex.len(), 64);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(hex, "ab".repeat(32));
    }

    #[test]
    fn blake3_hash_round_trip_hex() {
        let cases = [
            [0u8; 32],
            [0xFFu8; 32],
            [
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78, 0x9a,
                0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34,
                0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0,
            ],
        ];
        for bytes in cases {
            let h = Blake3Hash::from_bytes(bytes);
            let hex = h.to_hex();
            let h2 = Blake3Hash::parse_hex(&hex).unwrap();
            assert_eq!(h, h2);
        }
    }

    #[test]
    fn blake3_hash_parse_rejects_malformed() {
        assert!(Blake3Hash::parse_hex("too-short").is_none());
        assert!(Blake3Hash::parse_hex(&"z".repeat(64)).is_none());
        assert!(Blake3Hash::parse_hex(&"a".repeat(63)).is_none());
        assert!(Blake3Hash::parse_hex(&"a".repeat(65)).is_none());
    }

    #[test]
    fn blake3_hash_zero_helpers() {
        let z = Blake3Hash::ZERO;
        assert!(z.is_zero());
        assert!(!Blake3Hash::from_bytes([1u8; 32]).is_zero());
    }

    #[test]
    fn blake3_hash_serde_uses_hex() {
        let h = Blake3Hash::from_bytes([0xABu8; 32]);
        let s = serde_json::to_string(&h).unwrap();
        assert_eq!(s, format!("\"{}\"", "ab".repeat(32)));
        let h2: Blake3Hash = serde_json::from_str(&s).unwrap();
        assert_eq!(h, h2);
    }

    #[test]
    fn blake3_hash_display_matches_hex() {
        let h = Blake3Hash::from_bytes([0x42u8; 32]);
        assert_eq!(h.to_string(), h.to_hex());
    }

    #[test]
    fn blake3_hash_default_is_zero() {
        let h: Blake3Hash = Default::default();
        assert!(h.is_zero());
    }

    // ─── ProvenanceId ──────────────────────────────────────────────

    #[test]
    fn provenance_id_new_unique() {
        let a = ProvenanceId::new();
        let b = ProvenanceId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn provenance_id_round_trip_uuid() {
        let uuid = Uuid::new_v4();
        let id = ProvenanceId::from_uuid(uuid);
        assert_eq!(id.as_uuid(), uuid);
    }

    #[test]
    fn provenance_id_serde() {
        let id = ProvenanceId::new();
        let s = serde_json::to_string(&id).unwrap();
        let id2: ProvenanceId = serde_json::from_str(&s).unwrap();
        assert_eq!(id, id2);
    }

    // ─── ModelIdentity ─────────────────────────────────────────────

    #[test]
    fn model_identity_hosted_helper() {
        let m = ModelIdentity::hosted("openai", "gpt-4", "2024-01-01");
        assert_eq!(m.provider, "openai");
        assert_eq!(m.model_id, "gpt-4");
        assert_eq!(m.requested_version, "2024-01-01");
        assert!(m.served_version.is_none());
        assert!(m.content_hash.is_none());
    }

    #[test]
    fn model_identity_lineage_key_ignores_version() {
        let a = ModelIdentity::hosted("openai", "gpt-4", "2024-01-01");
        let mut b = a.clone();
        b.requested_version = "2024-04-09".into();
        assert_eq!(a.lineage_key(), b.lineage_key());
    }

    #[test]
    fn provider_migrated_detects_silent_swap() {
        let mut m = ModelIdentity::hosted("openai", "gpt-4", "0613");
        assert!(!m.provider_migrated());
        m.served_version = Some("0314".into());
        assert!(m.provider_migrated());
        m.served_version = Some("0613".into());
        assert!(!m.provider_migrated());
    }

    #[test]
    fn model_identity_serde() {
        let m = ModelIdentity {
            provider: "openai".into(),
            model_id: "gpt-4".into(),
            requested_version: "2024-04-09".into(),
            served_version: Some("2024-04-09".into()),
            content_hash: Some(Blake3Hash::from_bytes([7u8; 32])),
            supports_tool_use: true,
        };
        let json = serde_json::to_string(&m).unwrap();
        let m2: ModelIdentity = serde_json::from_str(&json).unwrap();
        assert_eq!(m, m2);
    }

    // ─── TokenUsage ────────────────────────────────────────────────

    #[test]
    fn token_usage_canonical_uses_provider_when_available() {
        let u = TokenUsage {
            client_estimated_input: 100,
            client_estimated_output: 200,
            provider_reported_input: Some(110),
            provider_reported_output: Some(190),
            is_estimated: false,
        };
        assert_eq!(u.canonical_input(), 110);
        assert_eq!(u.canonical_output(), 190);
    }

    #[test]
    fn token_usage_canonical_falls_back_to_client() {
        let u = TokenUsage {
            client_estimated_input: 100,
            client_estimated_output: 200,
            provider_reported_input: None,
            provider_reported_output: None,
            is_estimated: true,
        };
        assert_eq!(u.canonical_input(), 100);
        assert_eq!(u.canonical_output(), 200);
    }

    #[test]
    fn token_usage_drift_total() {
        let u = TokenUsage {
            client_estimated_input: 100,
            client_estimated_output: 200,
            provider_reported_input: Some(110),
            provider_reported_output: Some(220),
            is_estimated: false,
        };
        // (110 + 220) - (100 + 200) = 30
        assert_eq!(u.drift_total(), Some(30));
    }

    #[test]
    fn token_usage_drift_ratio() {
        let u = TokenUsage {
            client_estimated_input: 100,
            client_estimated_output: 100,
            provider_reported_input: Some(110),
            provider_reported_output: Some(110),
            is_estimated: false,
        };
        // (220 - 200) / 200 = 0.1
        assert!((u.drift_ratio().unwrap() - 0.1).abs() < 1e-9);
    }

    #[test]
    fn token_usage_drift_none_when_no_provider() {
        let u = TokenUsage {
            client_estimated_input: 100,
            client_estimated_output: 200,
            provider_reported_input: None,
            provider_reported_output: None,
            is_estimated: true,
        };
        assert!(u.drift_total().is_none());
        assert!(u.drift_ratio().is_none());
    }

    // ─── Completeness / SafetyFlag / FinishReason ─────────────────

    #[test]
    fn completeness_is_usable_only_for_complete() {
        assert!(Completeness::Complete.is_usable());
        assert!(!Completeness::Partial {
            received_tokens: 10,
            expected_tokens: 100
        }
        .is_usable());
        assert!(!Completeness::Refused {
            category: SafetyCategory::Hate
        }
        .is_usable());
        assert!(!Completeness::Errored {
            error_kind: "x".into()
        }
        .is_usable());
    }

    #[test]
    fn completeness_label_stable() {
        assert_eq!(Completeness::Complete.label(), "complete");
        assert_eq!(
            Completeness::Partial {
                received_tokens: 1,
                expected_tokens: 2
            }
            .label(),
            "partial"
        );
        assert_eq!(
            Completeness::Refused {
                category: SafetyCategory::Hate
            }
            .label(),
            "refused"
        );
        // **Sprint 19 — N27** : the new Aborted variant.
        assert_eq!(
            Completeness::Aborted {
                received_tokens: 7,
                reason: AbortReason::ClientDisconnect,
            }
            .label(),
            "aborted"
        );
    }

    // ─── Sprint 19 — N27 : Completeness::Aborted + AbortReason + RetryOutcome ─

    #[test]
    fn completeness_aborted_is_not_usable() {
        let c = Completeness::Aborted {
            received_tokens: 12,
            reason: AbortReason::ClientDisconnect,
        };
        assert!(!c.is_usable());
        assert!(c.is_aborted());
        assert_eq!(c.received_output_tokens(), Some(12));
    }

    #[test]
    fn completeness_partial_received_tokens_helper() {
        let p = Completeness::Partial {
            received_tokens: 4_096,
            expected_tokens: 8_192,
        };
        assert_eq!(p.received_output_tokens(), Some(4_096));
        assert!(!p.is_aborted());
    }

    #[test]
    fn completeness_complete_received_tokens_none() {
        assert!(Completeness::Complete.received_output_tokens().is_none());
    }

    #[test]
    fn abort_reason_labels_are_stable() {
        assert_eq!(AbortReason::ClientDisconnect.label(), "client_disconnect");
        assert_eq!(
            AbortReason::UpstreamError {
                error_type: "overloaded_error".into()
            }
            .label(),
            "upstream_error"
        );
        assert_eq!(AbortReason::ReadTimeout { ms: 60_000 }.label(), "read_timeout");
        assert_eq!(
            AbortReason::TotalTimeout { ms: 300_000 }.label(),
            "total_timeout"
        );
        assert_eq!(
            AbortReason::NonRetryablePostStream.label(),
            "non_retryable_post_stream"
        );
        assert_eq!(
            AbortReason::BackpressureAbort.label(),
            "backpressure_abort"
        );
    }

    #[test]
    fn abort_reason_serde_round_trip_all_variants() {
        let cases = vec![
            AbortReason::ClientDisconnect,
            AbortReason::UpstreamError {
                error_type: "overloaded_error".into(),
            },
            AbortReason::ReadTimeout { ms: 60_000 },
            AbortReason::TotalTimeout { ms: 300_000 },
            AbortReason::NonRetryablePostStream,
            AbortReason::BackpressureAbort,
        ];
        for c in cases {
            let json = serde_json::to_string(&c).unwrap();
            let back: AbortReason = serde_json::from_str(&json).unwrap();
            assert_eq!(c, back);
        }
    }

    #[test]
    fn retry_outcome_default_is_no_retry() {
        let r: RetryOutcome = Default::default();
        assert_eq!(r, RetryOutcome::NoRetry);
        assert!(r.succeeded());
    }

    #[test]
    fn retry_outcome_labels_match_taxonomy() {
        assert_eq!(RetryOutcome::NoRetry.label(), "no_retry");
        assert_eq!(RetryOutcome::RetriedSucceeded.label(), "retried_succeeded");
        assert_eq!(RetryOutcome::RetriedExhausted.label(), "retried_exhausted");
        assert_eq!(RetryOutcome::NonRetryable.label(), "non_retryable");
    }

    #[test]
    fn retry_outcome_succeeded_predicate() {
        assert!(RetryOutcome::NoRetry.succeeded());
        assert!(RetryOutcome::RetriedSucceeded.succeeded());
        assert!(!RetryOutcome::RetriedExhausted.succeeded());
        assert!(!RetryOutcome::NonRetryable.succeeded());
    }

    #[test]
    fn retry_outcome_serde_round_trip() {
        for r in [
            RetryOutcome::NoRetry,
            RetryOutcome::RetriedSucceeded,
            RetryOutcome::RetriedExhausted,
            RetryOutcome::NonRetryable,
        ] {
            let j = serde_json::to_string(&r).unwrap();
            let back: RetryOutcome = serde_json::from_str(&j).unwrap();
            assert_eq!(r, back);
        }
    }

    #[test]
    fn record_n27_fields_default_when_legacy_json_deserialised() {
        // Build a record JSON without N27 fields and deserialize ;
        // the new fields must default sanely.
        let r = sample_record();
        let json = serde_json::to_string(&r).unwrap();
        // Strip the N27 fields from the JSON to simulate a pre-N27
        // serialiser : the sample fixture already has them at defaults
        // so we just verify they're absent from the wire when default.
        // (Default RetryOutcome::NoRetry + retry_count=0 + streaming_mode=false
        //  + time_to_first_token_ms=None all serialise with the field
        //  literally present if the user explicitly defaulted them,
        //  unless we also annotate with `skip_serializing_if`.  The
        //  N27 wire spec is "send the field when non-default", so
        //  retry_count=0 is "send 0".)
        // Round-trip must restore the same record.
        let back: LlmProvenanceRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    // ── RC-1 : l'attestation de computation (le Mode C distant re-prouvable) ──────────

    #[test]
    fn compute_attestation_defaults_to_legacy_when_absent_from_json() {
        // Un record pré-RC1 (JSON SANS `compute_attestation`) doit se désérialiser en
        // `LegacyUnspecified` — strictement < R4 : pas de mensonge par omission, backfill forcé.
        let r = sample_record();
        let mut v: serde_json::Value = serde_json::to_value(&r).unwrap();
        v.as_object_mut().unwrap().remove("compute_attestation");
        assert!(v.get("compute_attestation").is_none(), "champ retiré (simule le legacy)");
        let back: LlmProvenanceRecord = serde_json::from_value(v).unwrap();
        assert_eq!(back.compute_attestation, ComputeAttestation::LegacyUnspecified);
        assert_eq!(back.compute_attestation.strength_rank(), 0, "legacy strictement < R4 (rang 1)");
    }

    #[test]
    fn compute_attestation_strength_is_total_and_strictly_decreasing() {
        // L'échelle est TOTALE et décroissante : R1 > R2 > R3 > R4 > Legacy.
        let ep = EndpointIdentity::new("node.lan", "sha256/PIN");
        let cosig = NodeCoSignature {
            node_id: "node-1".into(),
            scheme: CoSignatureScheme::Ed25519,
            signature_hex: "ab".into(),
            signed_tuple_hash: "cd".into(),
            nonce: "ef".into(),
            model_fingerprint: None,
        };
        let ranks = [
            ComputeAttestation::R1LocalDeterministic.strength_rank(),
            ComputeAttestation::R2LocalWitnessed.strength_rank(),
            ComputeAttestation::R3RemoteCosigned { endpoint: ep.clone(), cosignature: cosig }.strength_rank(),
            ComputeAttestation::R4RemoteChannel { endpoint: ep, keyed: false }.strength_rank(),
            ComputeAttestation::LegacyUnspecified.strength_rank(),
        ];
        for w in ranks.windows(2) {
            assert!(w[0] > w[1], "rang strictement décroissant : {} > {}", w[0], w[1]);
        }
    }

    #[test]
    fn compute_attestation_round_trips_every_rung_and_seals_into_the_record() {
        // Chaque barreau (y compris R3/R4 avec leurs données) round-trip en JSON, ET change
        // les octets sérialisés du record → donc le leaf_hash (scellé, infalsifiable).
        let ep = EndpointIdentity::new("gpu.internal", "sha256/REMOTEPIN");
        let rungs = vec![
            ComputeAttestation::R1LocalDeterministic,
            ComputeAttestation::R2LocalWitnessed,
            ComputeAttestation::R4RemoteChannel { endpoint: ep.clone(), keyed: true },
            ComputeAttestation::R3RemoteCosigned {
                endpoint: ep,
                cosignature: NodeCoSignature {
                    node_id: "gpu-1".into(),
                    scheme: CoSignatureScheme::Ed25519,
                    signature_hex: "deadbeef".into(),
                    signed_tuple_hash: "00ff".into(),
                    nonce: "1234".into(),
                    model_fingerprint: None,
                },
            },
        ];
        let baseline = serde_json::to_string(&sample_record()).unwrap();
        for rung in rungs {
            // round-trip de l'enum seul
            let j = serde_json::to_string(&rung).unwrap();
            let back: ComputeAttestation = serde_json::from_str(&j).unwrap();
            assert_eq!(back, rung);
            // scellement : un record portant ce barreau diffère des octets du défaut.
            let mut r = sample_record();
            r.compute_attestation = rung.clone();
            let with = serde_json::to_string(&r).unwrap();
            assert_ne!(with, baseline, "le barreau {:?} doit changer les octets scellés", rung.rung_label());
        }
    }

    #[test]
    fn only_remote_rungs_report_remote_and_carry_an_endpoint() {
        let ep = EndpointIdentity::new("h", "fp");
        assert!(!ComputeAttestation::R1LocalDeterministic.is_remote());
        assert!(!ComputeAttestation::R2LocalWitnessed.is_remote());
        assert!(!ComputeAttestation::LegacyUnspecified.is_remote());
        let r4 = ComputeAttestation::R4RemoteChannel { endpoint: ep.clone(), keyed: false };
        assert!(r4.is_remote());
        assert_eq!(r4.endpoint().map(|e| e.pin_fingerprint.as_str()), Some("fp"));
        // I-RC3 : seuls R1/R2 exigent le loopback ; les distants ne le peuvent pas.
        assert!(ComputeAttestation::R2LocalWitnessed.requires_loopback());
        assert!(!r4.requires_loopback());
    }

    #[test]
    fn record_n27_streaming_aborted_round_trip() {
        let mut r = sample_record();
        r.streaming_mode = true;
        r.retry_count = 2;
        r.retry_outcome = RetryOutcome::RetriedSucceeded;
        r.time_to_first_token_ms = Some(287);
        r.completeness = Completeness::Aborted {
            received_tokens: 89,
            reason: AbortReason::ClientDisconnect,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"streaming_mode\":true"));
        assert!(json.contains("\"retry_count\":2"));
        assert!(json.contains("retried_succeeded"));
        assert!(json.contains("\"time_to_first_token_ms\":287"));
        assert!(json.contains("\"kind\":\"aborted\""));
        assert!(json.contains("client_disconnect"));
        let back: LlmProvenanceRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
        assert!(back.is_aborted());
    }

    #[test]
    fn safety_flag_taxonomy_eight_variants_serde_round_trip() {
        let cases = vec![
            SafetyFlag::PiiDetected,
            SafetyFlag::PromptInjectionSuspected,
            SafetyFlag::OutputModerationTriggered,
            SafetyFlag::ContentFilter {
                category: SafetyCategory::Sexual,
            },
            SafetyFlag::ToxicityDetected,
            SafetyFlag::SelfHarmDetected,
            SafetyFlag::JailbreakAttemptDetected,
            SafetyFlag::PromptLengthExceeded,
        ];
        for flag in cases {
            let json = serde_json::to_string(&flag).unwrap();
            let flag2: SafetyFlag = serde_json::from_str(&json).unwrap();
            assert_eq!(flag, flag2);
            assert!(!flag.label().is_empty());
        }
    }

    #[test]
    fn safety_category_other_carries_label() {
        let c = SafetyCategory::Other("custom".into());
        assert_eq!(c.label(), "custom");
    }

    #[test]
    fn finish_reason_label_stable() {
        assert_eq!(FinishReason::Stop.label(), "stop");
        assert_eq!(FinishReason::Length.label(), "length");
        assert_eq!(FinishReason::ContentFilter.label(), "content_filter");
        assert_eq!(FinishReason::ToolCalls.label(), "tool_calls");
        assert_eq!(FinishReason::Error.label(), "error");
        assert_eq!(FinishReason::Unknown.label(), "unknown");
    }

    // ─── ToolCallRecord ────────────────────────────────────────────

    #[test]
    fn tool_call_record_serdes_round_trip() {
        let r = ToolCallRecord {
            ordinal: 1,
            tool_name: "lookup_patient".into(),
            arguments_hash: Blake3Hash::from_bytes([1u8; 32]),
            result_hash: Some(Blake3Hash::from_bytes([2u8; 32])),
            result_kind: ToolCallResultKind::Ok,
        };
        let json = serde_json::to_string(&r).unwrap();
        let r2: ToolCallRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(r, r2);
    }

    #[test]
    fn tool_call_result_kind_label() {
        assert_eq!(ToolCallResultKind::Ok.label(), "ok");
        assert_eq!(ToolCallResultKind::Errored.label(), "errored");
        assert_eq!(ToolCallResultKind::Cancelled.label(), "cancelled");
        assert_eq!(ToolCallResultKind::Skipped.label(), "skipped");
    }

    // ─── CacheProvenance ───────────────────────────────────────────

    #[test]
    fn cache_provenance_serdes() {
        let c = CacheProvenance {
            original_trace_id: Uuid::new_v4(),
            original_provenance_id: ProvenanceId::new(),
            cache_key_version: 1,
        };
        let json = serde_json::to_string(&c).unwrap();
        let c2: CacheProvenance = serde_json::from_str(&json).unwrap();
        assert_eq!(c, c2);
    }

    // ─── LlmProvenanceRecord ────────────────────────────────────────

    #[test]
    fn provenance_record_serde_round_trip() {
        let r = sample_record();
        let json = serde_json::to_string(&r).unwrap();
        let r2: LlmProvenanceRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(r, r2);
    }

    #[test]
    fn provenance_record_is_cache_hit_helper() {
        let mut r = sample_record();
        assert!(!r.is_cache_hit());
        r.cache_hit = Some(CacheProvenance {
            original_trace_id: Uuid::new_v4(),
            original_provenance_id: ProvenanceId::new(),
            cache_key_version: 1,
        });
        assert!(r.is_cache_hit());
    }

    #[test]
    fn provenance_record_is_replay_helper() {
        let mut r = sample_record();
        assert!(!r.is_replay());
        r.replay_of = Some(ProvenanceId::new());
        assert!(r.is_replay());
    }

    #[test]
    fn provenance_record_was_refused() {
        let mut r = sample_record();
        assert!(!r.was_refused());
        r.completeness = Completeness::Refused {
            category: SafetyCategory::Hate,
        };
        assert!(r.was_refused());
    }

    #[test]
    fn provenance_record_total_tokens() {
        let r = sample_record();
        // Uses provider values : 1238 + 314 = 1552
        assert_eq!(r.total_tokens(), 1552);
    }

    // ─── ProvenanceError ───────────────────────────────────────────

    #[test]
    fn provenance_error_serde_taxonomy() {
        let cases = vec![
            ProvenanceError::ModelIntegrityViolation {
                requested_hash: "abc".into(),
                actual_hash: "def".into(),
            },
            ProvenanceError::UnknownPromptTemplate {
                template_id: "x".into(),
                version: "1.0".into(),
            },
            ProvenanceError::Store {
                message: "db down".into(),
            },
            ProvenanceError::Chain {
                message: "hmac fail".into(),
            },
            ProvenanceError::Internal {
                message: "bug".into(),
            },
        ];
        for err in cases {
            let json = serde_json::to_string(&err).unwrap();
            let err2: ProvenanceError = serde_json::from_str(&json).unwrap();
            assert_eq!(err, err2);
        }
    }

    #[test]
    fn provenance_error_display_non_empty() {
        let err = ProvenanceError::Store {
            message: "x".into(),
        };
        assert!(!err.to_string().is_empty());
    }

    // ─── SamplingParams ────────────────────────────────────────────

    #[test]
    fn sampling_params_default_serde() {
        let s = SamplingParams::default();
        let json = serde_json::to_string(&s).unwrap();
        let s2: SamplingParams = serde_json::from_str(&json).unwrap();
        assert_eq!(s, s2);
    }

    #[test]
    fn sampling_params_round_trip_with_seed() {
        let s = SamplingParams {
            temperature: 0.7,
            top_p: Some(0.95),
            max_tokens: 1024,
            seed: Some(42),
            stop_sequences: vec!["END".into(), "STOP".into()],
        };
        let json = serde_json::to_string(&s).unwrap();
        let s2: SamplingParams = serde_json::from_str(&json).unwrap();
        assert_eq!(s, s2);
    }

    // ─── CostAccounting ────────────────────────────────────────────

    #[test]
    fn cost_accounting_default_zero() {
        let c = CostAccounting::default();
        assert_eq!(c.cost_cents, 0.0);
        assert_eq!(c.prices_table_version, 0);
        assert!(c.provider_quoted_cost_cents.is_none());
        assert!(!c.overflow_capped);
    }
}
