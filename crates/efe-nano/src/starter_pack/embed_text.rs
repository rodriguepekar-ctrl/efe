//! `embed_text` — deterministic 384-dim text embedder.
//!
//! Mode B — Local model (`embedder_v1` v1.0.0).  TIER-1.
//!
//! Phase 2 ships a SHA-256-seeded deterministic xorshift embedder
//! (cf [`crate::runtime::model_loader::builtin::Embedder`]).  Sprint 8+
//! swap-in : real MiniLM-L6-v2 ONNX model with semantic similarity.

use chrono::Utc;
use efe_nano_sdk::{
    AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
    IdempotencyPolicy, NanoAgent, NanoAgentId, NanoAgentVersion, ResourceLimits, StabilityTier,
};
use serde_json::json;

pub fn manifest() -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new("embed_text").expect("id valid"),
        version: NanoAgentVersion::new("1.0.0").expect("version valid"),
        description: "Produce a 384-dimensional unit-norm embedding vector for the input text.".to_string(),
        author: Author::Efe,
        category: Category::Other,    // Embedding doesn't map to other categories cleanly
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::LocalModel {
            model_id: "embedder_v1".to_string(),
            model_version: "1.0.0".to_string(),
        },
        determinism: DeterminismLevel::FullDeterministic,
        idempotency: IdempotencyPolicy::auto_24h(),
        input_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["text"],
            "properties": {
                "text": { "type": "string", "minLength": 1, "maxLength": 50_000 }
            }
        }),
        output_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["embedding", "dimension", "model"],
            "properties": {
                "embedding": {
                    "type": "array",
                    "items": { "type": "number" },
                    "minItems": 384,
                    "maxItems": 384
                },
                "dimension": { "const": 384 },
                "model": { "type": "string" }
            }
        }),
        error_schema: None,
        limits: ResourceLimits {
            memory_mb: 256,
            cpu_ms: 1_000,
            wall_clock_ms: 3_000,
            llm_tokens: None,
            network_egress_kb: 0,
        },
        cost_estimate_cents: 0.05,
        latency_p99_ms_target: 100,
        audit: AuditPolicy::default(),
        compliance: ComplianceTags {
            // Text may contain PII — flag GDPR.
            gdpr: true,
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
    use efe_nano_sdk::ExecutionMode;

    #[test]
    fn manifest_is_other_category() {
        assert_eq!(manifest().category, Category::Other);
    }

    #[test]
    fn manifest_is_mode_b_with_embedder_model() {
        match manifest().execution_mode {
            ExecutionMode::LocalModel { ref model_id, .. } => {
                assert_eq!(model_id, "embedder_v1");
            }
            other => panic!("expected LocalModel, got {other:?}"),
        }
    }

    #[test]
    fn manifest_output_schema_pins_dim_to_384() {
        let m = manifest();
        let schema_str = serde_json::to_string(&m.output_schema).unwrap();
        assert!(schema_str.contains("\"const\":384"));
    }

    #[test]
    fn manifest_flags_gdpr() {
        // Embeddings of text may inadvertently encode PII features.
        assert!(manifest().compliance.gdpr);
    }
}
