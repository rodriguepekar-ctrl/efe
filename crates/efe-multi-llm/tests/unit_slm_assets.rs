// SPDX-License-Identifier: UNLICENSED
//! **Fil B.1** — validation croisée OFFLINE des assets SLM multi-llm.
//!
//! Le contrat inter-crates : les exemples embarqués dans
//! `efe-nano-app-coordinator/assets/schemas/reformulator_multillm_v1.json`
//! doivent désérialiser (1) comme enveloppe `ReformulatorOutput` du crate nano,
//! et (2) champ `canonical` compris, dans le type `CanonicalX` d'`efe-multi-llm`
//! correspondant au `schema_id`. Si l'un des deux côtés dérive (renommage de
//! champ serde, variant, casse), CE test casse — sans llama-server.
//!
//! La grammaire et le prompt sont vérifiés structurellement (présence des
//! règles/tags attendus). La qualité de génération réelle du SLM n'est
//! vérifiée que par les tests live `#[ignore]` (`live_slm_reformulator.rs`).

use std::path::PathBuf;

use efe_multi_llm::canonical::{
    CanonicalArgument, CanonicalClaim, CanonicalCode, CanonicalPlan, CanonicalSynthesis,
};
use efe_nano_app_coordinator::types::{ReformulatorOutput, SchemaId};

fn nano_assets_dir() -> PathBuf {
    // efe-multi-llm/ → ../efe-nano-app-coordinator/assets
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/ parent")
        .join("efe-nano-app-coordinator/assets")
}

#[test]
fn schema_examples_deserialize_into_multi_llm_canonical_types() {
    let path = nano_assets_dir().join("schemas/reformulator_multillm_v1.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("asset manquant {path:?}: {e}"));
    let doc: serde_json::Value = serde_json::from_str(&text).expect("JSON schema valide");

    let examples = doc["examples"].as_array().expect("champ examples[]");
    assert_eq!(examples.len(), 5, "un exemple par schéma multi-llm");

    let mut seen = Vec::new();
    for ex in examples {
        // (1) L'enveloppe entière est un ReformulatorOutput nano valide.
        let out: ReformulatorOutput = serde_json::from_value(ex.clone())
            .unwrap_or_else(|e| panic!("enveloppe invalide pour {ex}: {e}"));
        // (2) Le canonical désérialise dans le type multi-llm du schema_id.
        match out.schema_id {
            SchemaId::PlanV1 => {
                let p: CanonicalPlan = serde_json::from_value(out.canonical.clone())
                    .expect("plan_v1 → CanonicalPlan");
                assert!(!p.steps.is_empty());
            }
            SchemaId::CodeV1 => {
                let c: CanonicalCode = serde_json::from_value(out.canonical.clone())
                    .expect("code_v1 → CanonicalCode");
                assert!(!c.source.is_empty());
            }
            SchemaId::ClaimV1 => {
                let c: CanonicalClaim = serde_json::from_value(out.canonical.clone())
                    .expect("claim_v1 → CanonicalClaim");
                assert!(!c.claim.is_empty());
                assert!((0.0..=1.0).contains(&c.confidence));
            }
            SchemaId::ArgumentV1 => {
                let a: CanonicalArgument = serde_json::from_value(out.canonical.clone())
                    .expect("argument_v1 → CanonicalArgument");
                assert!(!a.reasoning.is_empty());
            }
            SchemaId::SynthesisV1 => {
                let s: CanonicalSynthesis = serde_json::from_value(out.canonical.clone())
                    .expect("synthesis_v1 → CanonicalSynthesis");
                assert!(!s.summary.is_empty());
            }
            other => panic!("schema_id hors profil multi-llm dans les exemples : {other:?}"),
        }
        seen.push(out.schema_id);
    }
    // Les 5 schémas sont couverts, chacun une fois.
    for want in [
        SchemaId::PlanV1,
        SchemaId::CodeV1,
        SchemaId::ClaimV1,
        SchemaId::ArgumentV1,
        SchemaId::SynthesisV1,
    ] {
        assert_eq!(seen.iter().filter(|s| **s == want).count(), 1, "{want:?}");
    }
}

#[test]
fn multillm_grammar_has_expected_structure() {
    let path = nano_assets_dir().join("grammars/reformulator_multillm_v1.gbnf");
    let g = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("grammaire manquante {path:?}: {e}"));
    assert!(g.contains("root ::="), "règle root");
    // L'enveloppe lie chaque schema_id à sa forme : les 5 tags sont présents.
    for tag in ["plan_v1", "code_v1", "claim_v1", "argument_v1", "synthesis_v1"] {
        assert!(g.contains(&format!("\\\"{tag}\\\"")), "tag {tag} absent de la grammaire");
    }
    // Les briques sensibles.
    for rule in ["uuid ::=", "stance ::=", "language ::=", "zero-one ::=", "warnings-arr ::="] {
        assert!(g.contains(rule), "règle {rule} absente");
    }
    // Les littéraux d'enum doivent matcher le serde multi-llm.
    for lit in ["\\\"PRO\\\"", "\\\"CONTRA\\\"", "\\\"NEUTRAL\\\"", "\\\"REBUTTAL\\\"", "\\\"rust\\\""] {
        assert!(g.contains(lit), "littéral {lit} absent");
    }
    // Chaque CLÉ de champ canonique requise doit être émise par la grammaire —
    // un typo de clé ou une dérive grammaire↔serde casse ce test (revue B.1 #8).
    for key in [
        // enveloppe
        "\\\"canonical\\\":", "\\\"schema_id\\\":", "\\\"extraction_warnings\\\":",
        // plan_v1
        "\\\"plan_id\\\":", "\\\"title\\\":", "\\\"steps\\\":", "\\\"rationale\\\":",
        "\\\"id\\\":", "\\\"action\\\":", "\\\"deps\\\":",
        // code_v1
        "\\\"language\\\":", "\\\"source\\\":",
        // claim_v1
        "\\\"claim\\\":", "\\\"evidence\\\":", "\\\"confidence\\\":", "\\\"domain\\\":",
        "\\\"excerpt\\\":",
        // argument_v1
        "\\\"thesis\\\":", "\\\"reasoning\\\":", "\\\"sources\\\":", "\\\"stance\\\":",
        // synthesis_v1
        "\\\"summary\\\":", "\\\"citations\\\":", "\\\"uncertainty\\\":",
        "\\\"llm_source\\\":", "\\\"snippet\\\":",
    ] {
        assert!(g.contains(key), "clé de champ {key} absente de la grammaire");
    }
    // La non-vacuité contractuelle (minLength 1) est bien câblée : la règle
    // json-string-1 existe et est utilisée pour claim/thesis/summary/action.
    assert!(g.contains("json-string-1 ::="), "règle json-string-1 absente");
    for bound in [
        "\\\"claim\\\":\" ws json-string-1",
        "\\\"thesis\\\":\" ws json-string-1",
        "\\\"summary\\\":\" ws json-string-1",
        "\\\"action\\\":\" ws json-string-1",
    ] {
        assert!(g.contains(bound), "liaison non-vide manquante : {bound}");
    }
}

#[test]
fn multillm_prompt_documents_the_contract() {
    let path = nano_assets_dir().join("prompts/reformulator_multillm.txt");
    let p = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("prompt manquant {path:?}: {e}"));
    for tag in ["plan_v1", "code_v1", "claim_v1", "argument_v1", "synthesis_v1"] {
        assert!(p.contains(tag), "prompt ne mentionne pas {tag}");
    }
    assert!(p.contains("stance_hint"), "prompt ne documente pas stance_hint");
    assert!(p.contains("STRICT JSON"), "prompt n'exige pas le JSON strict");
    assert!(
        p.contains("MUST equal target_schema_id"),
        "prompt n'impose pas l'écho de schéma"
    );
}

/// Le client multi-llm charge réellement ses assets (chemins, lecture disque)
/// — sans aucun appel réseau.
#[test]
fn from_assets_multillm_loads() {
    use efe_nano_app_coordinator::{ReformulatorClient, ReformulatorProfile};
    let c = ReformulatorClient::from_assets_multillm("http://127.0.0.1:1")
        .expect("les 2 assets multillm doivent se charger");
    assert_eq!(c.profile(), ReformulatorProfile::MultiLlmV1);
}
