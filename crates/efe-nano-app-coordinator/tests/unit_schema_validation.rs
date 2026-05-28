// SPDX-License-Identifier: UNLICENSED
//! Tests de validation des schémas I/O — sans dépendance à un llama-server.
//!
//! On valide que :
//! 1. Les types Rust sérialisent vers le JSON attendu par le schéma.
//! 2. Les outputs typiques passent toutes les contraintes.
//! 3. Les outputs invalides échouent à la validation.
//! 4. Les enums sérialisent dans les bons cases.

use efe_nano_app_coordinator::types::*;
use validator::Validate as _;

#[test]
fn analyzer_full_output_valid_passes() {
    let out = AnalyzerOutput {
        entities: vec![
            Entity {
                text: "Apple".into(),
                kind: EntityKind::Org,
                start_char: 0,
                end_char: 5,
                confidence: 0.97,
            },
            Entity {
                text: "$90B".into(),
                kind: EntityKind::Money,
                start_char: 15,
                end_char: 19,
                confidence: 0.99,
            },
        ],
        intent: IntentClass::Factual,
        sentiment: 0.2,
        language: IsoLang::En,
        one_line_summary: "Apple Q3 revenue $90B".into(),
        confidence: 0.95,
    };
    assert!(out.validate().is_ok());
    assert!(out.validate_semantics().is_ok());
}

#[test]
fn analyzer_too_many_entities_fails() {
    let mut entities = Vec::with_capacity(21);
    for i in 0..21 {
        entities.push(Entity {
            text: format!("entity{i}"),
            kind: EntityKind::Other,
            start_char: 0,
            end_char: 5,
            confidence: 0.9,
        });
    }
    let out = AnalyzerOutput {
        entities,
        intent: IntentClass::Factual,
        sentiment: 0.0,
        language: IsoLang::En,
        one_line_summary: "x".into(),
        confidence: 0.5,
    };
    assert!(out.validate().is_err());
}

#[test]
fn analyzer_sentiment_too_high_fails() {
    let out = AnalyzerOutput {
        entities: vec![],
        intent: IntentClass::Factual,
        sentiment: 2.5, // > 1.0
        language: IsoLang::En,
        one_line_summary: "x".into(),
        confidence: 0.5,
    };
    assert!(out.validate().is_err());
}

#[test]
fn analyzer_inverted_offsets_caught_by_semantics() {
    let out = AnalyzerOutput {
        entities: vec![Entity {
            text: "X".into(),
            kind: EntityKind::Other,
            start_char: 10,
            end_char: 5, // INVERSÉ
            confidence: 0.9,
        }],
        intent: IntentClass::Factual,
        sentiment: 0.0,
        language: IsoLang::En,
        one_line_summary: "x".into(),
        confidence: 0.9,
    };
    assert!(out.validate().is_ok()); // basic ranges OK
    assert!(out.validate_semantics().is_err()); // semantic catches it
}

#[test]
fn analyzer_json_roundtrip_preserves_data() {
    let original = AnalyzerOutput {
        entities: vec![Entity {
            text: "Crédit Agricole".into(),
            kind: EntityKind::Org,
            start_char: 3,
            end_char: 18,
            confidence: 0.97,
        }],
        intent: IntentClass::Analytical,
        sentiment: 0.4,
        language: IsoLang::Fr,
        one_line_summary: "résumé".into(),
        confidence: 0.91,
    };
    let json = serde_json::to_string(&original).unwrap();
    let parsed: AnalyzerOutput = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.entities.len(), 1);
    assert_eq!(parsed.entities[0].text, "Crédit Agricole");
    assert_eq!(parsed.entities[0].kind, EntityKind::Org);
    assert_eq!(parsed.language, IsoLang::Fr);
    assert_eq!(parsed.intent, IntentClass::Analytical);
}

#[test]
fn verifier_supported_without_source_fails_consistency() {
    let out = VerifierOutput {
        verdict: Verdict::Supported,
        best_source_id: None, // illegal pour SUPPORTED
        similarity: 0.9,
        contradictions: vec![],
        severity: VerifierSeverity::Info,
        explanation: None,
    };
    assert!(out.validate_consistency().is_err());
}

#[test]
fn verifier_contradicted_with_all_fields_valid() {
    let out = VerifierOutput {
        verdict: Verdict::Contradicted,
        best_source_id: Some("src-a1".into()),
        similarity: 0.7,
        contradictions: vec!["src-a2".into(), "src-a3".into()],
        severity: VerifierSeverity::Error,
        explanation: Some("Two sources state the opposite explicitly.".into()),
    };
    assert!(out.validate().is_ok());
    assert!(out.validate_consistency().is_ok());
}

#[test]
fn compressor_must_preserve_enforced() {
    let out = CompressorOutput {
        summary: "Fed maintains 5.25% rate per FOMC.".into(),
        word_count: 6,
        preserved: vec!["Fed".into(), "FOMC".into()],
        omitted_for_brevity: vec![],
        compression_ratio: 8.0,
    };
    let must = vec!["Fed".to_string(), "FOMC".to_string()];
    assert!(out.validate_preservation(&must).is_ok());

    let must_extra = vec!["Fed".to_string(), "FOMC".to_string(), "MISSING".into()];
    assert!(out.validate_preservation(&must_extra).is_err());
}

#[test]
fn reformulator_canonical_can_be_any_json() {
    let json = r#"{
      "canonical": {"deeply": {"nested": ["values", 42, true, null]}},
      "schema_id": "fact_v1",
      "extraction_warnings": []
    }"#;
    let out: ReformulatorOutput = serde_json::from_str(json).unwrap();
    assert!(out.validate().is_ok());
    assert_eq!(out.schema_id, SchemaId::FactV1);
}

#[test]
fn router_serializes_correctly() {
    use std::collections::HashMap;
    let mut caps = HashMap::new();
    caps.insert("FACTUAL_QA".into(), 0.9);
    let meta = LlmMetadata {
        cost_input_per_million: 1500,
        cost_output_per_million: 7500,
        // R3.S1 : 0 → _resolved() fallback à cents × 1000.
        cost_input_per_million_microcents: 0,
        cost_output_per_million_microcents: 0,
        p50_latency_ms: 900,
        p99_latency_ms: 3000,
        success_rate: 0.99,
        capability_scores: caps,
        circuit_open: false,
    };
    let s = serde_json::to_string(&meta).unwrap();
    let back: LlmMetadata = serde_json::from_str(&s).unwrap();
    assert!(!back.circuit_open);
    assert!((back.success_rate - 0.99).abs() < 1e-6);
}

#[test]
fn filter_routing_decision_serializes_with_tag() {
    let d = RoutingDecision::Send {
        priority: 8,
        reason: "high relevance".into(),
    };
    let s = serde_json::to_string(&d).unwrap();
    assert!(s.contains("\"type\":\"Send\""));
    assert!(s.contains("\"priority\":8"));
}

#[test]
fn schemas_files_exist_and_parse_as_json() {
    let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for name in [
        "analyzer_v1.json",
        "reformulator_v1.json",
        "compressor_v1.json",
        "verifier_v1.json",
    ] {
        let path = crate_dir.join("assets/schemas").join(name);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("missing asset: {}", path.display()));
        let parsed: serde_json::Value = serde_json::from_str(&content)
            .unwrap_or_else(|_| panic!("invalid JSON in: {}", path.display()));
        assert_eq!(parsed["$schema"], "https://json-schema.org/draft/2020-12/schema");
        assert!(parsed["title"].is_string());
    }
}

#[test]
fn grammars_files_exist_and_have_root_rule() {
    let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for name in [
        "analyzer_v1.gbnf",
        "reformulator_v1.gbnf",
        "compressor_v1.gbnf",
        "verifier_v1.gbnf",
    ] {
        let path = crate_dir.join("assets/grammars").join(name);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("missing grammar: {}", path.display()));
        assert!(
            content.contains("root"),
            "grammar {name} must define a 'root' rule"
        );
    }
}

#[test]
fn prompts_files_exist_and_nonempty() {
    let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for name in ["analyzer.txt", "reformulator.txt", "compressor.txt", "verifier.txt"] {
        let path = crate_dir.join("assets/prompts").join(name);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("missing prompt: {}", path.display()));
        assert!(content.len() > 100, "prompt {name} too short: {}", content.len());
        assert!(content.contains("STRICT JSON"), "prompt {name} should enforce strict output");
    }
}
