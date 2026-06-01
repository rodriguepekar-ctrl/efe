//! `ner_legal` — legal NER extraction (CASE / CITATION / PARTY / CLAUSE).
//!
//! Mode B — Local model (`ner_legal_v1` v1.0.0).  TIER-1.

use chrono::Utc;
use efe_nano_sdk::{
    AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
    IdempotencyPolicy, NanoAgent, NanoAgentId, NanoAgentVersion, ResourceLimits, StabilityTier,
};
use serde_json::json;

pub fn manifest() -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new("ner_legal").expect("id valid"),
        version: NanoAgentVersion::new("1.0.0").expect("version valid"),
        description: "Legal NER : extract CASE, CITATION, PARTY, CLAUSE entities from legal text.".to_string(),
        author: Author::Efe,
        category: Category::EntityRecognition,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::LocalModel {
            model_id: "ner_legal_v1".to_string(),
            model_version: "1.0.0".to_string(),
        },
        // Sprint 7 — N5 : Mode B = seeded local model (strict replay hash match).
        determinism: DeterminismLevel::Seeded,
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
            "required": ["entities", "count"],
            "properties": {
                "count": { "type": "integer", "minimum": 0 },
                "entities": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["text", "kind", "start", "end", "score"],
                        "properties": {
                            "text": { "type": "string" },
                            "kind": { "enum": ["CASE", "CITATION", "PARTY", "CLAUSE"] },
                            "start": { "type": "integer", "minimum": 0 },
                            "end": { "type": "integer", "minimum": 0 },
                            "score": { "type": "number", "minimum": 0, "maximum": 1 }
                        }
                    }
                }
            }
        }),
        error_schema: None,
        limits: ResourceLimits {
            memory_mb: 1_536,
            cpu_ms: 4_000,
            wall_clock_ms: 8_000,
            llm_tokens: None,
            network_egress_kb: 0,
        },
        cost_estimate_cents: 0.08,
        latency_p99_ms_target: 150,
        audit: AuditPolicy::default(),
        compliance: ComplianceTags {
            gdpr: true,
            dpia_required: true,
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
    fn manifest_is_entity_recognition() {
        assert_eq!(manifest().category, Category::EntityRecognition);
    }

    #[test]
    fn manifest_is_mode_b_with_legal_model() {
        match manifest().execution_mode {
            ExecutionMode::LocalModel { ref model_id, .. } => {
                assert_eq!(model_id, "ner_legal_v1");
            }
            other => panic!("expected LocalModel, got {other:?}"),
        }
    }

    #[test]
    fn manifest_does_not_flag_hipaa() {
        // Legal NER touches GDPR but not HIPAA.
        let m = manifest();
        assert!(m.compliance.gdpr);
        assert!(!m.compliance.hipaa);
    }

    #[test]
    fn manifest_latency_target_below_200ms() {
        assert!(manifest().latency_p99_ms_target <= 200);
    }
}
