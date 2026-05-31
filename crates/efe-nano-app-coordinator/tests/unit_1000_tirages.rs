// SPDX-License-Identifier: UNLICENSED
//! Tests à grande échelle (1000 tirages par robot, sans llama-server).
//!
//! Vérifie que sur **mille variantes générées**, chaque robot :
//! 1. Sérialise tous ses inputs valides
//! 2. Désérialise correctement toutes ses sorties simulées
//! 3. Les enums round-trip sans perte
//! 4. Les validators rejettent systématiquement les inputs hors-spec
//!
//! Utilise `proptest` pour la génération aléatoire ciblée + asserts massifs.
//!
//! ⚠️  Ces tests ne valident PAS qu'un vrai modèle SLM produit du JSON valide
//! sous grammar ; pour ça il faut `scripts/validate-grammars.py` qui frappe
//! un vrai llama-server.  Cette suite vérifie le **contrat côté Rust**.

use efe_nano_app_coordinator::types::*;
use proptest::prelude::*;
use validator::Validate as _;

// ─── Stratégies proptest ─────────────────────────────────────────────────

fn intent_strategy() -> impl Strategy<Value = IntentClass> {
    prop_oneof![
        Just(IntentClass::Factual),
        Just(IntentClass::Analytical),
        Just(IntentClass::Comparative),
        Just(IntentClass::Temporal),
        Just(IntentClass::Opinion),
        Just(IntentClass::Instructional),
    ]
}

fn entity_kind_strategy() -> impl Strategy<Value = EntityKind> {
    prop_oneof![
        Just(EntityKind::Person),
        Just(EntityKind::Org),
        Just(EntityKind::Location),
        Just(EntityKind::Money),
        Just(EntityKind::Date),
        Just(EntityKind::Drug),
        Just(EntityKind::Event),
        Just(EntityKind::Product),
        Just(EntityKind::Other),
    ]
}

fn lang_strategy() -> impl Strategy<Value = IsoLang> {
    prop_oneof![
        Just(IsoLang::En), Just(IsoLang::Fr), Just(IsoLang::De), Just(IsoLang::Es),
        Just(IsoLang::It), Just(IsoLang::Pt), Just(IsoLang::Ja), Just(IsoLang::Zh),
        Just(IsoLang::Ru), Just(IsoLang::Ar), Just(IsoLang::Und),
    ]
}

fn entity_strategy() -> impl Strategy<Value = Entity> {
    (
        "[a-zA-Z0-9 ]{1,50}",
        entity_kind_strategy(),
        0u32..1000,
        0u32..1000,
        0.0f32..=1.0f32,
    )
        .prop_map(|(text, kind, mut start, mut end, conf)| {
            // ensure end > start
            if end <= start {
                std::mem::swap(&mut start, &mut end);
                end = end.saturating_add(1);
            }
            Entity {
                text,
                kind,
                start_char: start,
                end_char: end,
                confidence: conf,
            }
        })
}

fn analyzer_output_strategy() -> impl Strategy<Value = AnalyzerOutput> {
    (
        prop::collection::vec(entity_strategy(), 0..=20),
        intent_strategy(),
        -1.0f32..=1.0f32,
        lang_strategy(),
        "[a-zA-Z0-9 .]{1,100}",
        0.0f32..=1.0f32,
    )
        .prop_map(|(entities, intent, sentiment, language, summary, confidence)| {
            AnalyzerOutput {
                entities,
                intent,
                sentiment,
                language,
                one_line_summary: summary,
                confidence,
            }
        })
}

// ─── 1000+ tirages ──────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 1000,
        max_shrink_iters: 100,
        .. ProptestConfig::default()
    })]

    /// Tout output AnalyzerOutput généré par notre strategy valide.
    #[test]
    fn analyzer_generated_output_always_valid(out in analyzer_output_strategy()) {
        prop_assert!(out.validate().is_ok());
        prop_assert!(out.validate_semantics().is_ok());
    }

    /// Round-trip JSON : sérialiser puis désérialiser → identique.
    #[test]
    fn analyzer_output_json_roundtrip(out in analyzer_output_strategy()) {
        let json = serde_json::to_string(&out).unwrap();
        let parsed: AnalyzerOutput = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(parsed.entities.len(), out.entities.len());
        prop_assert_eq!(parsed.intent, out.intent);
        prop_assert_eq!(parsed.language, out.language);
        prop_assert!((parsed.sentiment - out.sentiment).abs() < 1e-5);
    }

    /// Sentiment hors [-1,1] est TOUJOURS rejeté.
    #[test]
    fn analyzer_sentiment_out_of_range_always_fails(s in -100.0f32..=-1.01f32) {
        let out = AnalyzerOutput {
            entities: vec![],
            intent: IntentClass::Factual,
            sentiment: s,
            language: IsoLang::En,
            one_line_summary: "x".into(),
            confidence: 0.5,
        };
        prop_assert!(out.validate().is_err());
    }

    #[test]
    fn analyzer_sentiment_above_one_always_fails(s in 1.01f32..=100.0f32) {
        let out = AnalyzerOutput {
            entities: vec![],
            intent: IntentClass::Factual,
            sentiment: s,
            language: IsoLang::En,
            one_line_summary: "x".into(),
            confidence: 0.5,
        };
        prop_assert!(out.validate().is_err());
    }

    /// > 20 entités est TOUJOURS rejeté.
    #[test]
    fn analyzer_too_many_entities_always_fails(n in 21u32..=200u32) {
        let entities: Vec<Entity> = (0..n).map(|i| Entity {
            text: format!("e{i}"),
            kind: EntityKind::Other,
            start_char: i,
            end_char: i + 1,
            confidence: 0.5,
        }).collect();
        let out = AnalyzerOutput {
            entities,
            intent: IntentClass::Factual,
            sentiment: 0.0,
            language: IsoLang::En,
            one_line_summary: "x".into(),
            confidence: 0.5,
        };
        prop_assert!(out.validate().is_err());
    }

    /// EntityKind enum roundtrip (sur 1000 tirages).
    #[test]
    fn entity_kind_serde_roundtrip(k in entity_kind_strategy()) {
        let s = serde_json::to_string(&k).unwrap();
        let parsed: EntityKind = serde_json::from_str(&s).unwrap();
        prop_assert_eq!(parsed, k);
    }

    /// IntentClass enum roundtrip.
    #[test]
    fn intent_class_serde_roundtrip(i in intent_strategy()) {
        let s = serde_json::to_string(&i).unwrap();
        let parsed: IntentClass = serde_json::from_str(&s).unwrap();
        prop_assert_eq!(parsed, i);
    }

    /// IsoLang enum roundtrip.
    #[test]
    fn iso_lang_serde_roundtrip(l in lang_strategy()) {
        let s = serde_json::to_string(&l).unwrap();
        let parsed: IsoLang = serde_json::from_str(&s).unwrap();
        prop_assert_eq!(parsed, l);
    }

    /// AnalyzerInput avec texte vide TOUJOURS rejeté.
    #[test]
    fn analyzer_input_empty_text_always_fails(max_e in 1u32..=20u32) {
        let input = AnalyzerInput {
            text: String::new(),
            max_entities: max_e,
            source_agent: None,
        };
        prop_assert!(input.validate().is_err());
    }

    /// AnalyzerInput max_entities > 20 TOUJOURS rejeté.
    #[test]
    fn analyzer_input_too_many_entities_always_fails(max_e in 21u32..=100u32) {
        let input = AnalyzerInput {
            text: "x".into(),
            max_entities: max_e,
            source_agent: None,
        };
        prop_assert!(input.validate().is_err());
    }

    /// Verifier verdict + severity round-trip sur 1000 tirages.
    #[test]
    fn verifier_verdict_severity_roundtrip(
        v in prop_oneof![
            Just(Verdict::Supported),
            Just(Verdict::Unsupported),
            Just(Verdict::Contradicted),
            Just(Verdict::InsufficientEvidence),
        ],
        sev in prop_oneof![
            Just(VerifierSeverity::Info),
            Just(VerifierSeverity::Warn),
            Just(VerifierSeverity::Error),
        ],
    ) {
        let sv = serde_json::to_string(&v).unwrap();
        let parsed_v: Verdict = serde_json::from_str(&sv).unwrap();
        prop_assert_eq!(parsed_v, v);
        let ss = serde_json::to_string(&sev).unwrap();
        let parsed_s: VerifierSeverity = serde_json::from_str(&ss).unwrap();
        prop_assert_eq!(parsed_s, sev);
    }

    /// CompressorInput must_preserve > 50 TOUJOURS rejeté.
    #[test]
    fn compressor_too_many_preserve_always_fails(n in 51usize..=200usize) {
        let input = CompressorInput {
            text: "x".repeat(100),
            target_words: 50,
            must_preserve: vec!["x".to_string(); n],
            style: CompressionStyle::Extractive,
        };
        prop_assert!(input.validate().is_err());
    }

    /// FilterInput avec aucun agent TOUJOURS rejeté.
    #[test]
    fn filter_no_agents_always_fails(text in "[a-zA-Z]{1,200}") {
        let input = FilterInput {
            message_summary: text,
            available_agents: vec![],
            agent_capabilities: std::collections::HashMap::new(),
            context_budgets: std::collections::HashMap::new(),
        };
        prop_assert!(input.validate().is_err());
    }
}

// ─── Stress tests directs (sans proptest) ───────────────────────────────

#[test]
fn analyzer_thousand_synthetic_outputs_all_validate() {
    let mut errors = 0;
    for i in 0..1000 {
        let out = AnalyzerOutput {
            entities: vec![Entity {
                text: format!("entity_{i}"),
                kind: EntityKind::Other,
                start_char: 0,
                end_char: 10,
                confidence: (i as f32 / 1000.0).clamp(0.0, 1.0),
            }],
            intent: IntentClass::Factual,
            sentiment: ((i as f32 / 500.0) - 1.0).clamp(-1.0, 1.0),
            language: IsoLang::En,
            one_line_summary: format!("summary {i}"),
            confidence: 0.5,
        };
        if out.validate().is_err() || out.validate_semantics().is_err() {
            errors += 1;
        }
    }
    assert_eq!(errors, 0, "1000 synthetic outputs should all validate");
}

#[test]
fn verifier_thousand_synthetic_consistent() {
    let mut errors = 0;
    for i in 0..1000 {
        let verdict = match i % 4 {
            0 => Verdict::Supported,
            1 => Verdict::Unsupported,
            2 => Verdict::Contradicted,
            _ => Verdict::InsufficientEvidence,
        };
        let best_source_id = if verdict == Verdict::Unsupported {
            None
        } else {
            Some(format!("src-{i}"))
        };
        let out = VerifierOutput {
            verdict,
            best_source_id,
            similarity: (i as f32 / 1000.0).clamp(0.0, 1.0),
            contradictions: vec![],
            severity: VerifierSeverity::Info,
            explanation: None,
        };
        if out.validate_consistency().is_err() {
            errors += 1;
        }
    }
    assert_eq!(errors, 0);
}
