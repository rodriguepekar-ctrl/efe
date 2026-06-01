//! `sentiment_nuanced` — multi-aspect sentiment analysis via LLM.
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
        id: NanoAgentId::new("sentiment_nuanced").expect("id valid"),
        version: NanoAgentVersion::new("1.0.0").expect("version valid"),
        description: "Multi-aspect sentiment analysis (positive/neutral/negative per aspect).".to_string(),
        author: Author::Efe,
        category: Category::Classification,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::LlmAugmented {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-5".to_string(),
            prompt_template_id: "m50.sentiment_nuanced.v1".to_string(),
            max_tokens: 500,
            temperature: 0.2,
        },
        // Sprint 7 — N5 : Mode C with replay-via-cache window.
        determinism: DeterminismLevel::LlmReplayable,
        idempotency: IdempotencyPolicy::auto_24h(),
        input_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["text"],
            "properties": {
                "text": { "type": "string", "minLength": 1, "maxLength": 20_000 }
            }
        }),
        output_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["overall_sentiment", "intensity", "aspects"],
            "properties": {
                "overall_sentiment": { "enum": ["positive", "neutral", "negative"] },
                "intensity": { "type": "number", "minimum": 0, "maximum": 1 },
                "aspects": {
                    "type": "array",
                    "maxItems": 10,
                    "items": {
                        "type": "object",
                        "required": ["aspect", "sentiment"],
                        "properties": {
                            "aspect": { "type": "string", "maxLength": 64 },
                            "sentiment": { "enum": ["positive", "neutral", "negative"] }
                        }
                    }
                }
            }
        }),
        error_schema: None,
        limits: ResourceLimits {
            memory_mb: 512,
            cpu_ms: 10_000,
            wall_clock_ms: 10_000,
            llm_tokens: Some(500),
            network_egress_kb: 20,
        },
        cost_estimate_cents: 1.0,
        latency_p99_ms_target: 2_500,
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
    fn manifest_output_aspects_max_10() {
        let m = manifest();
        let s = serde_json::to_string(&m.output_schema).unwrap();
        assert!(s.contains("\"maxItems\":10"));
    }

    #[test]
    fn manifest_uses_sentiment_template() {
        match manifest().execution_mode {
            ExecutionMode::LlmAugmented {
                ref prompt_template_id,
                ..
            } => assert_eq!(prompt_template_id, "m50.sentiment_nuanced.v1"),
            other => panic!("expected LlmAugmented, got {other:?}"),
        }
    }

    #[test]
    fn manifest_cost_around_one_cent() {
        let c = manifest().cost_estimate_cents;
        assert!((0.5..=2.0).contains(&c), "got {c}");
    }
}
