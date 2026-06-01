//! `summarize_strict` — LLM summarization with strict sentence-count cap.
//!
//! Mode C — LLM-augmented.  TIER-1.
//!
//! # Inputs
//!
//! ```json
//! { "text": "<source text>", "max_sentences": <1-10> }
//! ```
//!
//! # Outputs
//!
//! ```json
//! { "summary": "<summary text>", "sentence_count": <int> }
//! ```
//!
//! # Compliance
//!
//! - `gdpr: true` — text may contain personal data.
//! - `pii_redaction_strategy: PreLlmRedact` (Sprint 5 default for Mode C).

use chrono::Utc;
use efe_nano_sdk::{
    AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
    IdempotencyPolicy, NanoAgent, NanoAgentId, NanoAgentVersion, PiiStrategy, ResourceLimits,
    StabilityTier,
};
use serde_json::json;

pub fn manifest() -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new("summarize_strict").expect("id valid"),
        version: NanoAgentVersion::new("1.0.0").expect("version valid"),
        description: "LLM-based summarization with strict sentence-count cap.".to_string(),
        author: Author::Efe,
        category: Category::Summarization,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::LlmAugmented {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-5".to_string(),
            prompt_template_id: "m50.summarize_strict.v1".to_string(),
            max_tokens: 800,
            temperature: 0.3,
        },
        // Sprint 7 — N5 : Mode C with replay-via-cache window.
        determinism: DeterminismLevel::LlmReplayable,
        idempotency: IdempotencyPolicy::Manual { ttl_sec: 3_600 },
        input_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["text", "max_sentences"],
            "properties": {
                "text": { "type": "string", "minLength": 1, "maxLength": 50_000 },
                "max_sentences": { "type": "integer", "minimum": 1, "maximum": 10 }
            }
        }),
        output_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["summary", "sentence_count"],
            "properties": {
                "summary": { "type": "string", "minLength": 1 },
                "sentence_count": { "type": "integer", "minimum": 0, "maximum": 10 }
            }
        }),
        error_schema: None,
        limits: ResourceLimits {
            memory_mb: 512,
            cpu_ms: 10_000,
            wall_clock_ms: 30_000,
            llm_tokens: Some(800),
            network_egress_kb: 50,
        },
        cost_estimate_cents: 2.0,
        latency_p99_ms_target: 3_000,
        audit: AuditPolicy::default(),
        compliance: ComplianceTags {
            gdpr: true,
            pii_redaction_strategy: PiiStrategy::PreLlmRedact,
            ..ComplianceTags::default()
        },
        mcp_tool_metadata: None,
        signature: None,
        co_publishing: None,
        gold_set_ref: None,
        created_at: Utc::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_summarization() {
        assert_eq!(manifest().category, Category::Summarization);
    }

    #[test]
    fn manifest_is_mode_c_anthropic() {
        match manifest().execution_mode {
            ExecutionMode::LlmAugmented {
                ref provider,
                ref model,
                ..
            } => {
                assert_eq!(provider, "anthropic");
                assert!(model.starts_with("claude-sonnet"));
            }
            other => panic!("expected LlmAugmented, got {other:?}"),
        }
    }

    #[test]
    fn manifest_default_pii_redaction_is_pre_llm_redact() {
        let m = manifest();
        assert_eq!(m.compliance.pii_redaction_strategy, PiiStrategy::PreLlmRedact);
        assert!(m.compliance.gdpr);
    }

    #[test]
    fn manifest_p99_target_under_3s() {
        assert!(manifest().latency_p99_ms_target <= 3_000);
    }
}
