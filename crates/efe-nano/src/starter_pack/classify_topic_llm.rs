//! `classify_topic_llm` — LLM-based free-form topic classification.
//!
//! Mode C — LLM-augmented (cheap model variant).  TIER-1.

use chrono::Utc;
use efe_nano_sdk::{
    AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
    IdempotencyPolicy, NanoAgent, NanoAgentId, NanoAgentVersion, PiiStrategy, ResourceLimits,
    StabilityTier,
};
use serde_json::json;

pub fn manifest() -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new("classify_topic_llm").expect("id valid"),
        version: NanoAgentVersion::new("1.0.0").expect("version valid"),
        description: "Free-form topic classification via LLM (no closed taxonomy).".to_string(),
        author: Author::Efe,
        category: Category::Classification,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::LlmAugmented {
            provider: "openai".to_string(),
            model: "gpt-5-mini".to_string(),
            prompt_template_id: "m50.classify_topic.v1".to_string(),
            max_tokens: 200,
            temperature: 0.0,
        },
        // Sprint 7 — N5 : Mode C with replay-via-cache window.
        determinism: DeterminismLevel::LlmReplayable,
        idempotency: IdempotencyPolicy::auto_24h(),
        input_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["text"],
            "properties": {
                "text": { "type": "string", "minLength": 1, "maxLength": 10_000 }
            }
        }),
        output_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["topic", "confidence"],
            "properties": {
                "topic": { "type": "string", "minLength": 1, "maxLength": 64 },
                "confidence": { "type": "number", "minimum": 0, "maximum": 1 }
            }
        }),
        error_schema: None,
        limits: ResourceLimits {
            memory_mb: 256,
            cpu_ms: 5_000,
            wall_clock_ms: 10_000,
            llm_tokens: Some(200),
            network_egress_kb: 10,
        },
        cost_estimate_cents: 0.5,
        latency_p99_ms_target: 2_000,
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
    fn manifest_is_classification() {
        assert_eq!(manifest().category, Category::Classification);
    }

    #[test]
    fn manifest_uses_cheap_openai_model() {
        match manifest().execution_mode {
            ExecutionMode::LlmAugmented {
                ref provider, ref model, ..
            } => {
                assert_eq!(provider, "openai");
                assert!(model.contains("mini"));    // cheap variant
            }
            other => panic!("expected LlmAugmented, got {other:?}"),
        }
    }

    #[test]
    fn manifest_cost_below_1_cent() {
        let c = manifest().cost_estimate_cents;
        assert!(c < 1.0, "expected sub-cent cost for cheap classifier, got {c}");
    }

    #[test]
    fn manifest_p99_under_2s() {
        assert!(manifest().latency_p99_ms_target <= 2_000);
    }
}
