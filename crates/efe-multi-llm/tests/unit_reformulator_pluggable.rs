// SPDX-License-Identifier: UNLICENSED
//! **Fil B.0** — invariant de non-régression du reformulateur *pluggable*.
//!
//! Prouve que faire passer le reformulateur derrière le trait
//! [`Reformulator`] ne change PAS la sortie déterministe : le chemin trait
//! (`to_canonical`) produit la même valeur canonique que la méthode inhérente
//! (`route`) et l'estampille [`ReformulatorEngine::Rule`]. C'est la garantie
//! centrale de B.0 : sans SLM câblé, tout est identique — seul le reçu gagne le
//! champ `engine`.

use efe_multi_llm::canonical::CanonicalSchemaId;
use efe_multi_llm::reformulator_ext::SourceDialect;
use efe_multi_llm::{CanonicalReformulator, Reformulator, ReformulatorEngine};

/// `ClaimV1` est **déterministe** (pas d'UUID) : la valeur produite par le trait
/// doit être bit-identique à celle de la méthode inhérente.
#[tokio::test]
async fn trait_claim_matches_inherent_and_stamps_rule() {
    let r = CanonicalReformulator::new();
    let raw = "The Fed held rates steady. confidence: 0.8";
    let dialect = SourceDialect::GeminiMarkdown;

    let inherent = r
        .route(raw, dialect, CanonicalSchemaId::ClaimV1, "gemini_x")
        .expect("inherent route ok");
    let via_trait = r
        .to_canonical(raw, dialect, CanonicalSchemaId::ClaimV1, "gemini_x", None)
        .await
        .expect("trait to_canonical ok");

    assert_eq!(via_trait.engine, ReformulatorEngine::Rule);
    assert_eq!(
        via_trait.value, inherent,
        "le chemin trait doit produire la même valeur canonique que l'inhérent"
    );
}

/// Idem pour `CodeV1` (déterministe, pas d'UUID).
#[tokio::test]
async fn trait_code_matches_inherent() {
    let r = CanonicalReformulator::new();
    let raw = "```rust\nfn main() { println!(\"hi\"); }\n```";
    let dialect = SourceDialect::GeminiMarkdown;

    let inherent = r
        .route(raw, dialect, CanonicalSchemaId::CodeV1, "gpt_x")
        .expect("inherent");
    let via_trait = r
        .to_canonical(raw, dialect, CanonicalSchemaId::CodeV1, "gpt_x", None)
        .await
        .expect("trait");

    assert_eq!(via_trait.engine, ReformulatorEngine::Rule);
    assert_eq!(via_trait.value, inherent);
}

/// Le chemin `to_argument_value` (debate) via le trait produit un
/// `CanonicalArgument` valide, estampillé `Rule`, égal au round-trip inhérent.
#[tokio::test]
async fn trait_argument_matches_inherent_and_stamps_rule() {
    use efe_multi_llm::canonical::{ArgumentStance, CanonicalArgument};

    let r = CanonicalReformulator::new();
    let raw = "<thesis>Rates should stay</thesis>\n- inflation is sticky\n- growth is fine";
    let dialect = SourceDialect::ClaudeXml;

    let inherent = r
        .to_argument(raw, ArgumentStance::Pro, dialect)
        .expect("inherent to_argument");
    let via_trait = r
        .to_argument_value(raw, ArgumentStance::Pro, dialect, None)
        .await
        .expect("trait to_argument_value");

    assert_eq!(via_trait.engine, ReformulatorEngine::Rule);
    let round: CanonicalArgument =
        serde_json::from_value(via_trait.value).expect("value is a valid CanonicalArgument");
    assert_eq!(round.thesis, inherent.thesis);
    assert_eq!(round.stance, inherent.stance);
}

/// Sanity : les 5 schémas passent par le trait sans erreur ni panique, tous
/// estampillés `Rule` (le déterministe est complet).
#[tokio::test]
async fn all_five_schemas_route_via_trait_as_rule() {
    let r = CanonicalReformulator::new();
    let cases = [
        (CanonicalSchemaId::PlanV1, "1. do a\n2. do b"),
        (CanonicalSchemaId::CodeV1, "```rust\nfn a(){}\n```"),
        (CanonicalSchemaId::ClaimV1, "The sky is blue."),
        (CanonicalSchemaId::SynthesisV1, "# Title\n\nA short synthesis paragraph."),
    ];
    for (schema, raw) in cases {
        let out = r
            .to_canonical(raw, SourceDialect::Unknown, schema, "x", None)
            .await
            .unwrap_or_else(|e| panic!("schema {schema:?} failed: {e}"));
        assert_eq!(out.engine, ReformulatorEngine::Rule, "{schema:?}");
    }
}
