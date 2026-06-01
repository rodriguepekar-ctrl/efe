//! `intent_classify_generic` — keyword-scored intent classifier.
//!
//! Mode B — Local model (`intent_classify_v1` v1.0.0).  TIER-1.

use chrono::Utc;
use efe_nano_sdk::{
    AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
    IdempotencyPolicy, NanoAgent, NanoAgentId, NanoAgentVersion, ResourceLimits, StabilityTier,
};
use serde_json::json;

pub fn manifest() -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new("intent_classify_generic").expect("id valid"),
        version: NanoAgentVersion::new("1.0.0").expect("version valid"),
        description: "Generic intent classifier across 10 buckets (buy_product, complaint, request_info, etc.).".to_string(),
        author: Author::Efe,
        category: Category::Classification,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::LocalModel {
            model_id: "intent_classify_v1".to_string(),
            model_version: "1.0.0".to_string(),
        },
        determinism: DeterminismLevel::FullDeterministic,    // keyword-scoring is fully deterministic
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
            "required": ["intent", "score", "all_scores"],
            "properties": {
                "intent": { "type": "string" },
                "score": { "type": "number", "minimum": 0, "maximum": 1 },
                "all_scores": {
                    "type": "object",
                    "additionalProperties": { "type": "number" }
                }
            }
        }),
        error_schema: None,
        limits: ResourceLimits {
            memory_mb: 512,
            cpu_ms: 2_000,
            wall_clock_ms: 5_000,
            llm_tokens: None,
            network_egress_kb: 0,
        },
        cost_estimate_cents: 0.05,
        latency_p99_ms_target: 80,
        audit: AuditPolicy::default(),
        compliance: ComplianceTags::default(),
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
    use efe_nano_sdk::ExecutionMode;

    #[test]
    fn manifest_is_classification() {
        assert_eq!(manifest().category, Category::Classification);
    }

    #[test]
    fn manifest_is_mode_b_with_intent_model() {
        match manifest().execution_mode {
            ExecutionMode::LocalModel { ref model_id, .. } => {
                assert_eq!(model_id, "intent_classify_v1");
            }
            other => panic!("expected LocalModel, got {other:?}"),
        }
    }

    #[test]
    fn manifest_is_full_deterministic() {
        assert_eq!(
            manifest().determinism,
            DeterminismLevel::FullDeterministic
        );
    }

    #[test]
    fn manifest_no_compliance_flags() {
        // Generic intent classification has no inherent regulatory burden.
        let c = manifest().compliance;
        assert!(!c.gdpr);
        assert!(!c.hipaa);
        assert!(!c.has_any_high_risk());
    }
}
