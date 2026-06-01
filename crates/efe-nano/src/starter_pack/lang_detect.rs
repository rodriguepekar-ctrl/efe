//! `lang_detect` — language detection via function-word frequency.
//!
//! Mode B — Local model (`lang_detect_v1` v1.0.0).  TIER-1.

use chrono::Utc;
use efe_nano_sdk::{
    AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
    IdempotencyPolicy, NanoAgent, NanoAgentId, NanoAgentVersion, ResourceLimits, StabilityTier,
};
use serde_json::json;

pub fn manifest() -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new("lang_detect").expect("id valid"),
        version: NanoAgentVersion::new("1.0.0").expect("version valid"),
        description: "Detect language of free-text using function-word frequency (en/fr/de/es/it/pt/nl).".to_string(),
        author: Author::Efe,
        category: Category::Classification,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::LocalModel {
            model_id: "lang_detect_v1".to_string(),
            model_version: "1.0.0".to_string(),
        },
        determinism: DeterminismLevel::FullDeterministic,
        idempotency: IdempotencyPolicy::auto_24h(),
        input_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["text"],
            "properties": {
                "text": { "type": "string", "minLength": 1, "maxLength": 100_000 }
            }
        }),
        output_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["language", "iso_639_1", "confidence", "per_language_scores"],
            "properties": {
                "language": { "type": "string" },
                "iso_639_1": { "type": "string", "pattern": "^[a-z]{2,3}$" },
                "confidence": { "type": "number", "minimum": 0, "maximum": 1 },
                "per_language_scores": {
                    "type": "object",
                    "additionalProperties": { "type": "number" }
                }
            }
        }),
        error_schema: None,
        limits: ResourceLimits {
            memory_mb: 128,
            cpu_ms: 500,
            wall_clock_ms: 2_000,
            llm_tokens: None,
            network_egress_kb: 0,
        },
        cost_estimate_cents: 0.01,
        latency_p99_ms_target: 20,
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
    fn manifest_is_mode_b_with_lang_detect_model() {
        match manifest().execution_mode {
            ExecutionMode::LocalModel { ref model_id, .. } => {
                assert_eq!(model_id, "lang_detect_v1");
            }
            other => panic!("expected LocalModel, got {other:?}"),
        }
    }

    #[test]
    fn manifest_is_deterministic() {
        assert_eq!(
            manifest().determinism,
            DeterminismLevel::FullDeterministic
        );
    }

    #[test]
    fn manifest_fast_p99_target() {
        // Language detection is the cheapest Mode B agent ; ≤ 20 ms p99.
        assert!(manifest().latency_p99_ms_target <= 20);
    }
}
