//! `qa_grounded` — grounded Q&A with citations via LLM.
//!
//! Mode C — LLM-augmented (Opus-class, highest-quality model).  TIER-1.

use chrono::Utc;
use efe_nano_sdk::{
    AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
    IdempotencyPolicy, NanoAgent, NanoAgentId, NanoAgentVersion, PiiStrategy, ResourceLimits,
    StabilityTier,
};
use serde_json::json;

pub fn manifest() -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new("qa_grounded").expect("id valid"),
        version: NanoAgentVersion::new("1.0.0").expect("version valid"),
        description: "Answer a question given a context, citing source spans.".to_string(),
        author: Author::Efe,
        category: Category::StructuredExtract,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::LlmAugmented {
            provider: "anthropic".to_string(),
            model: "claude-opus-4-5".to_string(),
            prompt_template_id: "m50.qa_grounded.v1".to_string(),
            max_tokens: 1_500,
            temperature: 0.0,
        },
        // Sprint 7 — N5 : Mode C with replay-via-cache window (90j default).
        determinism: DeterminismLevel::LlmReplayable,
        idempotency: IdempotencyPolicy::Manual { ttl_sec: 3_600 },
        input_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["question", "context"],
            "properties": {
                "question": { "type": "string", "minLength": 1, "maxLength": 1_000 },
                "context": { "type": "string", "minLength": 1, "maxLength": 50_000 }
            }
        }),
        output_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["answer", "citations"],
            "properties": {
                "answer": { "type": "string", "minLength": 1 },
                "citations": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["span_start", "span_end", "source"],
                        "properties": {
                            "span_start": { "type": "integer", "minimum": 0 },
                            "span_end": { "type": "integer", "minimum": 0 },
                            "source": { "type": "string", "maxLength": 256 }
                        }
                    }
                }
            }
        }),
        error_schema: None,
        limits: ResourceLimits {
            memory_mb: 1_024,
            cpu_ms: 15_000,
            wall_clock_ms: 30_000,
            llm_tokens: Some(1_500),
            network_egress_kb: 100,
        },
        cost_estimate_cents: 3.0,
        latency_p99_ms_target: 4_000,
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
    fn manifest_is_structured_extract() {
        assert_eq!(manifest().category, Category::StructuredExtract);
    }

    #[test]
    fn manifest_uses_opus_model() {
        match manifest().execution_mode {
            ExecutionMode::LlmAugmented { ref model, .. } => {
                assert!(model.contains("opus"));
            }
            other => panic!("expected LlmAugmented, got {other:?}"),
        }
    }

    #[test]
    fn manifest_supports_50kb_context() {
        let m = manifest();
        let s = serde_json::to_string(&m.input_schema).unwrap();
        assert!(s.contains("\"maxLength\":50000"));
    }

    #[test]
    fn manifest_cost_highest_among_mode_c() {
        // qa_grounded uses Opus → most expensive.
        let c = manifest().cost_estimate_cents;
        assert!(c >= 2.5, "expected ≥ 2.5 cents for Opus, got {c}");
    }
}
