// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21** : AgenticTask + AgenticResponse types.
//! **Sprint 18 — N26** : `AgenticAuditContext` extension on
//! `AgenticTask` to feed the N17 provenance collector.

use efe_nano_sdk::{
    GoldCaseId, KycTier, ModelIdentity, NanoAgentId, NanoAgentVersion, PiiStrategy,
    PublisherId, TenantId,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// **Sprint 18 — N26 + G1** : pre-call audit context carried
/// alongside every `AgenticTask`.  Populated by the caller (the
/// gateway handler / a swarm runner / an MCP tool) ; consumed
/// by `ClaudeBridge` to build the `PreCallProvenance` for N17
/// emission.
///
/// When `AgenticTask.audit_context.is_some() &&
/// bridge.provenance.is_some()`, the bridge produces exactly
/// one `LlmProvenanceRecord` per logical call.  When either is
/// `None`, no record is produced + a metric ticks (see N26
/// architecture § 3 G3 + I2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgenticAuditContext {
    /// **N26** : W3C trace id (or Uuid v4 fallback).
    pub trace_id: Uuid,
    /// **N26** : swarm request id from N21 orchestration
    /// (`SwarmReport.swarm_id`) ; fresh for direct agentic calls.
    pub swarm_request_id: Uuid,
    /// **N26** : operation id (`"op-1"`, `"agentic_proxy"`,
    /// `"claude_summarize"`, ...).
    pub op_id: String,
    /// **N26 + G11 + I12** : tenant id from the JWT claim.  The
    /// gateway handler OVERWRITES the client-supplied value
    /// with the JWT claim before dispatch.
    pub tenant_id: TenantId,
    /// **N26** : calling agent id.  For direct agentic calls
    /// (`POST /v1/agentic/call`) defaults to `"agentic_proxy"` ;
    /// for swarm-driven calls (Mode C nano) is the nano's id.
    pub agent_id: NanoAgentId,
    /// **N26** : calling agent semver.
    pub agent_version: NanoAgentVersion,
    /// **N26** : prompt template id (e.g. `"swarm_context_v1"`).
    pub prompt_template_id: String,
    /// **N26** : prompt template semver.
    pub prompt_template_version: String,
    /// **N26 + G7** : PII strategy applied to the bundle
    /// BEFORE this call.  The bundle's metadata is the source of
    /// truth ; this field captures the snapshot.
    #[serde(default)]
    pub pii_strategy_applied: PiiStrategy,
    /// **N26 + G7** : redaction tokens injected into the bundle.
    /// Empty when no redaction happened or the bundle has no
    /// redactions metadata.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redaction_token_set: Vec<String>,
    /// **N26 + G8** : publisher id snapshot at call time
    /// (`None` for direct calls without a publisher context).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_id: Option<PublisherId>,
    /// **N26 + G8 + I18** : publisher KYC tier snapshot at call
    /// time.  Captured separately from a live registry lookup
    /// so the audit chain stays faithful to "what was the tier
    /// when the call happened".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_kyc_tier: Option<KycTier>,
    /// **N26 + G9** : gold case id if this call is part of a
    /// gold test run.  `None` for production traffic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gold_case_id: Option<GoldCaseId>,
}

impl AgenticAuditContext {
    /// **N26** : construct a fresh default context for a direct
    /// `agentic_proxy` call.  Callers usually override `tenant_id`
    /// (mandatory) and `swarm_request_id` (if part of a swarm).
    pub fn agentic_proxy_default(tenant_id: TenantId) -> Self {
        Self {
            trace_id: Uuid::new_v4(),
            swarm_request_id: Uuid::new_v4(),
            op_id: "agentic_proxy".to_string(),
            tenant_id,
            agent_id: NanoAgentId::new("agentic_proxy")
                .expect("static agent id `agentic_proxy` is always valid"),
            agent_version: NanoAgentVersion::new("1.0.0")
                .expect("static version `1.0.0` is always valid"),
            prompt_template_id: "agentic_proxy_v1".to_string(),
            prompt_template_version: "1.0.0".to_string(),
            pii_strategy_applied: PiiStrategy::None,
            redaction_token_set: Vec::new(),
            publisher_id: None,
            publisher_kyc_tier: None,
            gold_case_id: None,
        }
    }
}

/// **N21** : the task handed to the agentic LLM after the swarm
/// pre-analysis completes.
///
/// **Sprint 18 N26 additive** : `audit_context` carries the
/// pre-call provenance metadata so the bridge can emit an
/// `LlmProvenanceRecord` (N17).  `None` preserves Sprint 13
/// behaviour : the bridge dispatches without emitting any
/// provenance record (a metric ticks).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgenticTask {
    pub system_prompt: String,
    pub user_message: String,
    pub max_tokens: u32,
    pub temperature: f32,
    /// **N26 + G1** : optional pre-call audit context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_context: Option<AgenticAuditContext>,
}

impl Default for AgenticTask {
    fn default() -> Self {
        Self {
            system_prompt: String::new(),
            user_message: String::new(),
            max_tokens: 4_096,
            temperature: 0.0,
            audit_context: None,
        }
    }
}

impl AgenticTask {
    /// **N21** : Construct a task with sensible defaults.
    pub fn new(user_message: impl Into<String>) -> Self {
        Self {
            user_message: user_message.into(),
            ..Default::default()
        }
    }

    pub fn with_system(mut self, system_prompt: impl Into<String>) -> Self {
        self.system_prompt = system_prompt.into();
        self
    }

    pub fn with_max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = n;
        self
    }

    pub fn with_temperature(mut self, t: f32) -> Self {
        self.temperature = t;
        self
    }

    /// **N26 + G1** : attach an audit context for N17 provenance
    /// emission.
    pub fn with_audit_context(mut self, ctx: AgenticAuditContext) -> Self {
        self.audit_context = Some(ctx);
        self
    }
}

/// **N21** : the response from the agentic LLM.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgenticResponse {
    pub text: String,
    pub stop_reason: StopReason,
    pub usage: TokenUsage,
    pub model_identity: ModelIdentity,
    pub cost_microcents: i64,
}

/// **N21** : token usage from the agentic LLM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    ToolUse,
    StopSequence,
    Other,
}

impl StopReason {
    pub fn label(&self) -> &'static str {
        match self {
            Self::EndTurn => "end_turn",
            Self::MaxTokens => "max_tokens",
            Self::ToolUse => "tool_use",
            Self::StopSequence => "stop_sequence",
            Self::Other => "other",
        }
    }

    /// **N21** : Parse Anthropic's `stop_reason` string into a variant.
    pub fn from_anthropic(s: &str) -> Self {
        match s {
            "end_turn" => Self::EndTurn,
            "max_tokens" => Self::MaxTokens,
            "tool_use" => Self::ToolUse,
            "stop_sequence" => Self::StopSequence,
            _ => Self::Other,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  AgenticEvent + AbortReason  (Sprint 19 — N27)
// ─────────────────────────────────────────────────────────────────────

// Re-export the SDK's AbortReason at the bridge crate level so
// downstream consumers don't have to depend on efe-nano-sdk
// directly just to pattern-match an event payload.
pub use efe_nano_sdk::AbortReason;

use efe_nano_sdk::RetryOutcome;

use crate::error::BridgeError;

/// **Sprint 19 — N27** : a single event in a streaming agentic
/// response.
///
/// Forwarded from the bridge to the gateway (which re-encodes
/// into SSE for HTTP clients).  Some variants are
/// internal-only — they're emitted on the bridge's mpsc channel
/// for instrumentation but the gateway strips them before they
/// hit the wire (see [`AgenticEvent::is_downstream_visible`] +
/// ADR-N27-7).
#[derive(Debug, Clone)]
pub enum AgenticEvent {
    /// **Internal** : a Claude turn began.
    /// Not forwarded downstream.
    MessageStart {
        turn: u8,
        provider_request_id: Option<String>,
        input_tokens: u32,
    },
    /// **Downstream-visible** : incremental text from the LLM.
    TextDelta(String),
    /// **Internal** : turn-1 tool-use block began.  The bridge
    /// will execute the tool and chain a turn 2 ; the downstream
    /// consumer only sees the final natural-language answer.
    ToolUseStart {
        tool_id: String,
        tool_name: String,
    },
    /// **Internal** : partial tool input JSON.  Accumulated
    /// server-side and parsed once at [`AgenticEvent::ToolUseComplete`].
    ToolUseDelta(String),
    /// **Internal** : tool-use block complete.  Holds the parsed
    /// input.
    ToolUseComplete {
        tool_id: String,
        tool_name: String,
        input: serde_json::Value,
    },
    /// **Internal** : per-turn final usage + latency.
    TurnUsage {
        turn: u8,
        usage: TokenUsage,
        latency_ms: u32,
    },
    /// **Downstream-visible** : stream is finished, here's the
    /// full final summary.  This is the LAST visible event of a
    /// successful stream.
    Done {
        stop_reason: StopReason,
        total_usage: TokenUsage,
        model_identity: ModelIdentity,
        cost_microcents: i64,
        time_to_first_token_ms: Option<u32>,
        provider_response_time_ms: u32,
        retry_count: u8,
        retry_outcome: RetryOutcome,
    },
    /// **Downstream-visible** : terminal error.
    Error(BridgeError),
}

impl AgenticEvent {
    /// **N27 + ADR-N27-7** : should this event be forwarded to
    /// the downstream HTTP/SSE client by default ?
    ///
    /// Returns `true` only for the user-facing variants
    /// (`TextDelta`, `Done`, `Error`) ; the rest are internal
    /// instrumentation.
    pub fn is_downstream_visible(&self) -> bool {
        matches!(
            self,
            AgenticEvent::TextDelta(_) | AgenticEvent::Done { .. } | AgenticEvent::Error(_)
        )
    }

    /// **N27** : stable Prometheus / log label.
    pub fn label(&self) -> &'static str {
        match self {
            AgenticEvent::MessageStart { .. } => "message_start",
            AgenticEvent::TextDelta(_) => "text_delta",
            AgenticEvent::ToolUseStart { .. } => "tool_use_start",
            AgenticEvent::ToolUseDelta(_) => "tool_use_delta",
            AgenticEvent::ToolUseComplete { .. } => "tool_use_complete",
            AgenticEvent::TurnUsage { .. } => "turn_usage",
            AgenticEvent::Done { .. } => "done",
            AgenticEvent::Error(_) => "error",
        }
    }

    /// **N27** : a `Done` event with safe defaults.  Used as a
    /// pseudo-finalizer by the [`AgenticBridge::call_with_context`]
    /// default impl when an underlying bridge has no native
    /// streaming surface and must synthesise events.
    pub fn done_buffered(
        text_already_buffered_len: usize,
        stop_reason: StopReason,
        usage: TokenUsage,
        model_identity: ModelIdentity,
        cost_microcents: i64,
    ) -> Self {
        let _ = text_already_buffered_len; // reserved for future TTFT shaping
        AgenticEvent::Done {
            stop_reason,
            total_usage: usage,
            model_identity,
            cost_microcents,
            time_to_first_token_ms: None,
            provider_response_time_ms: 0,
            retry_count: 0,
            retry_outcome: RetryOutcome::NoRetry,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_default_max_tokens() {
        let t = AgenticTask::default();
        assert_eq!(t.max_tokens, 4_096);
        assert_eq!(t.temperature, 0.0);
    }

    #[test]
    fn task_builder_chain() {
        let t = AgenticTask::new("hello")
            .with_system("You are a helpful assistant.")
            .with_max_tokens(2_048)
            .with_temperature(0.7);
        assert_eq!(t.user_message, "hello");
        assert_eq!(t.system_prompt, "You are a helpful assistant.");
        assert_eq!(t.max_tokens, 2_048);
        assert_eq!(t.temperature, 0.7);
    }

    #[test]
    fn stop_reason_round_trip_via_anthropic_strings() {
        for s in ["end_turn", "max_tokens", "tool_use", "stop_sequence", "unknown"] {
            let r = StopReason::from_anthropic(s);
            // Round-trip the label() back to the original (unknown
            // maps to "other").
            if s == "unknown" {
                assert_eq!(r.label(), "other");
            } else {
                assert_eq!(r.label(), s);
            }
        }
    }

    #[test]
    fn stop_reason_serde_round_trip() {
        for r in [
            StopReason::EndTurn,
            StopReason::MaxTokens,
            StopReason::ToolUse,
            StopReason::StopSequence,
            StopReason::Other,
        ] {
            let json = serde_json::to_string(&r).unwrap();
            let back: StopReason = serde_json::from_str(&json).unwrap();
            assert_eq!(back, r);
        }
    }

    #[test]
    fn token_usage_default() {
        let u = TokenUsage::default();
        assert_eq!(u.input_tokens, 0);
        assert_eq!(u.output_tokens, 0);
    }

    #[test]
    fn task_default_has_no_audit_context() {
        let t = AgenticTask::default();
        assert!(t.audit_context.is_none());
    }

    #[test]
    fn audit_context_agentic_proxy_default_uses_tenant_id() {
        let tenant = TenantId::new();
        let ctx = AgenticAuditContext::agentic_proxy_default(tenant);
        assert_eq!(ctx.tenant_id, tenant);
        assert_eq!(ctx.agent_id.as_str(), "agentic_proxy");
        assert_eq!(ctx.agent_version.as_str(), "1.0.0");
        assert_eq!(ctx.op_id, "agentic_proxy");
        assert_eq!(ctx.prompt_template_id, "agentic_proxy_v1");
        assert!(ctx.publisher_id.is_none());
        assert!(ctx.gold_case_id.is_none());
        assert!(ctx.redaction_token_set.is_empty());
        // Trace id is fresh per call.
        let ctx2 = AgenticAuditContext::agentic_proxy_default(tenant);
        assert_ne!(ctx.trace_id, ctx2.trace_id);
    }

    #[test]
    fn audit_context_serde_round_trip_minimal() {
        let ctx = AgenticAuditContext::agentic_proxy_default(TenantId::new());
        let json = serde_json::to_string(&ctx).unwrap();
        let back: AgenticAuditContext = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ctx);
    }

    #[test]
    fn audit_context_serde_skips_none_options() {
        let ctx = AgenticAuditContext::agentic_proxy_default(TenantId::new());
        let json = serde_json::to_string(&ctx).unwrap();
        // None fields are stripped.
        assert!(!json.contains("publisher_id"));
        assert!(!json.contains("publisher_kyc_tier"));
        assert!(!json.contains("gold_case_id"));
        // Empty redaction set is stripped too.
        assert!(!json.contains("redaction_token_set"));
    }

    #[test]
    fn task_with_audit_context_builder() {
        let tenant = TenantId::new();
        let ctx = AgenticAuditContext::agentic_proxy_default(tenant);
        let t = AgenticTask::new("hi").with_audit_context(ctx.clone());
        assert_eq!(t.audit_context, Some(ctx));
    }

    #[test]
    fn task_with_audit_context_serde_round_trip() {
        let tenant = TenantId::new();
        let ctx = AgenticAuditContext::agentic_proxy_default(tenant);
        let t = AgenticTask::new("hi").with_audit_context(ctx);
        let json = serde_json::to_string(&t).unwrap();
        let back: AgenticTask = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn task_without_audit_context_deserialises_from_legacy_payload() {
        // Sprint 13 callers serialised the task without
        // `audit_context`.  The new field must default to `None`.
        let legacy_json = r#"{
            "system_prompt": "",
            "user_message": "hello",
            "max_tokens": 4096,
            "temperature": 0.0
        }"#;
        let t: AgenticTask = serde_json::from_str(legacy_json).unwrap();
        assert!(t.audit_context.is_none());
    }

    #[test]
    fn agentic_response_serde_round_trip() {
        let r = AgenticResponse {
            text: "Hello".into(),
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
            },
            model_identity: ModelIdentity::hosted("anthropic", "claude-sonnet-4-6", "2024-06"),
            cost_microcents: 250,
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: AgenticResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }
}
