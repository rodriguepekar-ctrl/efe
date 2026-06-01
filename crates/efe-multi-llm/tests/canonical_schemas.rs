// SPDX-License-Identifier: UNLICENSED
//! Tests : 5 canonical schemas v1.0.0 (présence assets + roundtrip).

use efe_multi_llm::{
    CanonicalArgument, CanonicalClaim, CanonicalCode, CanonicalPlan, CanonicalSchemaId,
    CanonicalSynthesis, CANONICAL_SCHEMA_VERSION,
};

fn asset_path(rel: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets/schemas")
        .join(rel)
}

#[test]
fn schema_version_v1() {
    assert_eq!(CANONICAL_SCHEMA_VERSION, "1.0.0");
}

#[test]
fn all_5_schemas_exist_on_disk() {
    for sub in [
        "canonical/plan_v1.json",
        "canonical/code_v1.json",
        "canonical/claim_v1.json",
        "canonical/argument_v1.json",
        "canonical/synthesis_v1.json",
    ] {
        let p = asset_path(sub);
        assert!(p.exists(), "missing schema asset: {}", p.display());
    }
}

#[test]
fn all_schemas_parse_as_valid_json() {
    for sub in [
        "canonical/plan_v1.json",
        "canonical/code_v1.json",
        "canonical/claim_v1.json",
        "canonical/argument_v1.json",
        "canonical/synthesis_v1.json",
    ] {
        let s = std::fs::read_to_string(asset_path(sub)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert!(v.is_object());
        assert!(v.get("$schema").is_some());
        assert!(v.get("title").is_some());
    }
}

#[test]
fn plan_validates_against_schema() {
    let schema_str = std::fs::read_to_string(asset_path("canonical/plan_v1.json")).unwrap();
    let schema: serde_json::Value = serde_json::from_str(&schema_str).unwrap();
    let validator = jsonschema::JSONSchema::compile(&schema).unwrap();
    let p = CanonicalPlan {
        plan_id: uuid::Uuid::now_v7(),
        title: "Test".into(),
        steps: vec![efe_multi_llm::canonical::PlanStep {
            id: 1,
            action: "do".into(),
            deps: vec![],
            estimated_duration_min: Some(5),
        }],
        rationale: "because".into(),
    };
    let value = serde_json::to_value(&p).unwrap();
    let result = validator.validate(&value);
    assert!(result.is_ok(), "plan failed schema validation: {value:?}");
}

#[test]
fn code_validates_against_schema() {
    let schema_str = std::fs::read_to_string(asset_path("canonical/code_v1.json")).unwrap();
    let schema: serde_json::Value = serde_json::from_str(&schema_str).unwrap();
    let validator = jsonschema::JSONSchema::compile(&schema).unwrap();
    let c = CanonicalCode {
        language: efe_multi_llm::canonical::CodeLanguage::Rust,
        source: "fn main() {}".into(),
        tests: vec![],
        deps: vec![],
        complexity_estimate: Some(efe_multi_llm::canonical::CodeComplexity::Simple),
    };
    let value = serde_json::to_value(&c).unwrap();
    let result = validator.validate(&value);
    assert!(result.is_ok());
}

#[test]
fn claim_validates_against_schema() {
    let schema_str = std::fs::read_to_string(asset_path("canonical/claim_v1.json")).unwrap();
    let schema: serde_json::Value = serde_json::from_str(&schema_str).unwrap();
    let validator = jsonschema::JSONSchema::compile(&schema).unwrap();
    let c = CanonicalClaim {
        claim: "Paris is in France".into(),
        evidence: vec![],
        confidence: 0.9,
        domain: "geography".into(),
    };
    let value = serde_json::to_value(&c).unwrap();
    assert!(validator.validate(&value).is_ok());
}

#[test]
fn argument_validates_against_schema() {
    let schema_str = std::fs::read_to_string(asset_path("canonical/argument_v1.json")).unwrap();
    let schema: serde_json::Value = serde_json::from_str(&schema_str).unwrap();
    let validator = jsonschema::JSONSchema::compile(&schema).unwrap();
    let a = CanonicalArgument {
        thesis: "Rust is safer".into(),
        reasoning: vec!["Memory safety".into()],
        sources: vec![],
        addresses_counter: None,
        stance: efe_multi_llm::canonical::ArgumentStance::Pro,
    };
    let value = serde_json::to_value(&a).unwrap();
    assert!(validator.validate(&value).is_ok());
}

#[test]
fn synthesis_validates_against_schema() {
    let schema_str = std::fs::read_to_string(asset_path("canonical/synthesis_v1.json")).unwrap();
    let schema: serde_json::Value = serde_json::from_str(&schema_str).unwrap();
    let validator = jsonschema::JSONSchema::compile(&schema).unwrap();
    let s = CanonicalSynthesis {
        summary: "Multi-LLM synthesis".into(),
        citations: vec![],
        uncertainty: String::new(),
        sections: vec![],
    };
    let value = serde_json::to_value(&s).unwrap();
    assert!(validator.validate(&value).is_ok());
}

#[test]
fn code_rejects_invalid_language() {
    let schema_str = std::fs::read_to_string(asset_path("canonical/code_v1.json")).unwrap();
    let schema: serde_json::Value = serde_json::from_str(&schema_str).unwrap();
    let validator = jsonschema::JSONSchema::compile(&schema).unwrap();
    // Construire un objet "code" manuellement avec un langage invalide.
    let invalid = serde_json::json!({"language": "cobol", "source": "x"});
    let result = validator.validate(&invalid);
    assert!(result.is_err(), "cobol should be rejected by enum");
}

#[test]
fn claim_rejects_confidence_above_1() {
    let schema_str = std::fs::read_to_string(asset_path("canonical/claim_v1.json")).unwrap();
    let schema: serde_json::Value = serde_json::from_str(&schema_str).unwrap();
    let validator = jsonschema::JSONSchema::compile(&schema).unwrap();
    let invalid = serde_json::json!({"claim": "x", "confidence": 1.5});
    assert!(validator.validate(&invalid).is_err());
}

#[test]
fn argument_rejects_invalid_stance() {
    let schema_str = std::fs::read_to_string(asset_path("canonical/argument_v1.json")).unwrap();
    let schema: serde_json::Value = serde_json::from_str(&schema_str).unwrap();
    let validator = jsonschema::JSONSchema::compile(&schema).unwrap();
    let invalid = serde_json::json!({"thesis": "x", "reasoning": ["a"], "stance": "MAYBE"});
    assert!(validator.validate(&invalid).is_err());
}

#[test]
fn schema_id_short_names_documented() {
    let names: Vec<_> = [
        CanonicalSchemaId::PlanV1,
        CanonicalSchemaId::CodeV1,
        CanonicalSchemaId::ClaimV1,
        CanonicalSchemaId::ArgumentV1,
        CanonicalSchemaId::SynthesisV1,
    ]
    .iter()
    .map(|s| s.short())
    .collect();
    assert_eq!(names.len(), 5);
    assert!(names.iter().all(|n| n.ends_with("_v1")));
}
