//! `ner_clinical` — biomedical NER extraction (DRUG / DISEASE / GENE / PHASE).
//!
//! Mode B — Local model (`ner_clinical_v1` v1.0.0).  TIER-1.
//!
//! Phase 2 ships a deterministic dictionary-based builtin
//! [`crate::runtime::model_loader::builtin::NerClinical`].
//! Sprint 8+ swap-in : real `BiomedBERT`-derived ONNX model.

use chrono::Utc;
use efe_nano_sdk::{
    AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
    IdempotencyPolicy, NanoAgent, NanoAgentId, NanoAgentVersion, ResourceLimits, StabilityTier,
};
use serde_json::json;

pub fn manifest() -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new("ner_clinical").expect("id valid"),
        version: NanoAgentVersion::new("1.0.0").expect("version valid"),
        description: "Biomedical NER : extract DRUG, DISEASE, GENE, PHASE entities from clinical text.".to_string(),
        author: Author::Efe,
        category: Category::EntityRecognition,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::LocalModel {
            model_id: "ner_clinical_v1".to_string(),
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
                            "kind": { "enum": ["DRUG", "DISEASE", "GENE", "PHASE"] },
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
            memory_mb: 2_048,
            cpu_ms: 5_000,
            wall_clock_ms: 10_000,
            llm_tokens: None,
            network_egress_kb: 0,
        },
        cost_estimate_cents: 0.1,
        latency_p99_ms_target: 200,
        audit: AuditPolicy::default(),
        compliance: ComplianceTags {
            gdpr: true,
            hipaa: true,
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
    fn manifest_is_mode_b() {
        match manifest().execution_mode {
            ExecutionMode::LocalModel { ref model_id, ref model_version } => {
                assert_eq!(model_id, "ner_clinical_v1");
                assert_eq!(model_version, "1.0.0");
            }
            other => panic!("expected LocalModel mode, got {other:?}"),
        }
    }

    #[test]
    fn manifest_flags_hipaa_and_dpia() {
        let m = manifest();
        assert!(m.compliance.gdpr);
        assert!(m.compliance.hipaa);
        assert!(m.compliance.dpia_required);
    }

    #[test]
    fn manifest_limits_for_mode_b() {
        let m = manifest();
        assert_eq!(m.limits.memory_mb, 2048);
        assert_eq!(m.limits.network_egress_kb, 0);
        assert!(m.cost_estimate_cents > 0.0);
    }
}
