//! `extract_structured` — LLM-based structured field extraction.
//!
//! Mode C — LLM-augmented.  TIER-1.

use chrono::Utc;
use efe_nano_sdk::{
    AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
    IdempotencyPolicy, NanoAgent, NanoAgentId, NanoAgentVersion, PiiStrategy, ResourceLimits,
    StabilityTier,
};
use serde_json::json;

pub fn manifest() -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new("extract_structured").expect("id valid"),
        version: NanoAgentVersion::new("1.0.0").expect("version valid"),
        description: "LLM-based structured extraction with caller-defined fields.".to_string(),
        author: Author::Efe,
        category: Category::StructuredExtract,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::LlmAugmented {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-5".to_string(),
            prompt_template_id: "m50.extract_structured.v1".to_string(),
            max_tokens: 2_000,
            temperature: 0.1,
        },
        // Sprint 7 — N5 : Mode C with replay-via-cache window.
        determinism: DeterminismLevel::LlmReplayable,
        idempotency: IdempotencyPolicy::Manual { ttl_sec: 3_600 },
        input_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["text", "fields"],
            "properties": {
                "text": { "type": "string", "minLength": 1, "maxLength": 50_000 },
                "fields": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 20,
                    "items": {
                        "type": "object",
                        "required": ["name", "type"],
                        "properties": {
                            "name": { "type": "string", "pattern": "^[a-z_][a-z0-9_]*$" },
                            "type": { "enum": ["string", "number", "boolean", "date"] },
                            "description": { "type": "string", "maxLength": 200 }
                        }
                    }
                }
            }
        }),
        output_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["extracted"],
            "properties": {
                "extracted": { "type": "object" }
            }
        }),
        error_schema: None,
        limits: ResourceLimits {
            memory_mb: 512,
            cpu_ms: 10_000,
            wall_clock_ms: 30_000,
            llm_tokens: Some(2_000),
            network_egress_kb: 50,
        },
        cost_estimate_cents: 2.5,
        latency_p99_ms_target: 5_000,
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
    fn manifest_uses_extract_structured_template() {
        match manifest().execution_mode {
            ExecutionMode::LlmAugmented {
                ref prompt_template_id,
                ..
            } => assert_eq!(prompt_template_id, "m50.extract_structured.v1"),
            other => panic!("expected LlmAugmented, got {other:?}"),
        }
    }

    #[test]
    fn manifest_p99_target_under_5s() {
        assert!(manifest().latency_p99_ms_target <= 5_000);
    }

    #[test]
    fn manifest_cost_estimate_reasonable() {
        let c = manifest().cost_estimate_cents;
        assert!(c > 0.0 && c < 10.0);
    }
}
