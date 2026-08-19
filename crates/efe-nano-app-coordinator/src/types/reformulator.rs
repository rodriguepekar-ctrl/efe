// SPDX-License-Identifier: UNLICENSED
//! Types I/O du robot **Reformulateur** (Qwen3-0.6B Q5).
//!
//! Converti la sortie d'un LLM dans son dialecte propre
//! (Claude XML / GPT JSON / Gemini Markdown / ...) vers un schéma canonique.

use serde::{Deserialize, Serialize};
use validator::Validate;

/// Entrée du robot Reformulateur.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct ReformulatorInput {
    /// Texte brut produit par un LLM source.
    #[validate(length(min = 1, max = 16_000))]
    pub raw_text: String,

    /// Dialecte source détecté ou annoncé.
    pub source_dialect: LlmDialect,

    /// Schéma canonique cible.
    pub target_schema_id: SchemaId,

    /// **Fil B.1 (additif)** — indice de position pour `argument_v1`
    /// (`"PRO"` / `"CONTRA"` / `"NEUTRAL"` / `"REBUTTAL"`). Ignoré par les
    /// autres schémas. `None` = absent du JSON sérialisé (rétro-compatible :
    /// les anciens payloads sans ce champ désérialisent inchangés).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[validate(length(max = 24))]
    pub stance_hint: Option<String>,
}

/// Sortie du Reformulateur (la structure de `canonical` dépend de `schema_id`).
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct ReformulatorOutput {
    /// La donnée structurée conforme au schema_id ciblé.
    /// On ne type pas plus strictement ici : c'est la responsabilité
    /// du caller de dispatcher selon `schema_id`.
    pub canonical: serde_json::Value,

    /// Echo du schéma utilisé pour la sortie.
    pub schema_id: SchemaId,

    /// Avertissements d'extraction (ambiguïtés, données manquantes, ...).
    #[validate(length(max = 10))]
    pub extraction_warnings: Vec<String>,
}

/// Dialectes LLM supportés en Phase 1 (extensible Phase 2+).
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LlmDialect {
    /// Claude utilise des balises XML pour les citations.
    ClaudeXml,
    /// GPT-4 préfère du JSON inline.
    Gpt4Json,
    /// Gemini utilise des footnotes Markdown.
    GeminiMarkdown,
    /// Mistral mixe parfois.
    MistralMixed,
    /// Format inconnu — best effort.
    Unknown,
}

/// Identifiants des schémas canoniques cibles supportés.
///
/// Deux familles, selon le **profil d'assets** du client (cf.
/// `clients::reformulator::ReformulatorProfile`) :
///
/// * **Provenance v1** (grammaire `reformulator_v1.gbnf`) : les 5 premiers
///   variants. La forme de `canonical` est celle documentée sur chaque variant.
/// * **Multi-LLM v1.1 (Fil B.1)** (grammaire `reformulator_multillm_v1.gbnf`) :
///   `ClaimV1` + les 4 derniers variants ; la forme de `canonical` est alors le
///   type `CanonicalX` d'`efe-multi-llm` (p. ex. `ClaimV1` ⟹
///   `{claim, evidence[{source, excerpt}], confidence, domain}` — et NON
///   `{claim_text, ...}` comme en provenance v1). La grammaire multi-llm LIE
///   `schema_id` à la forme émise ; la validation typée finale reste côté
///   appelant (`serde_json::from_value::<CanonicalX>`).
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaId {
    /// `{source_id, quote, confidence}`
    CitationV1,
    /// `{subject, predicate, object, source_ids[]}`
    FactV1,
    /// `{tool_name, arguments{}, expected_output_schema}`
    ToolCallV1,
    /// Provenance v1 : `{claim_text, supporting_evidence[], confidence}` ;
    /// profil multi-llm : `{claim, evidence[], confidence, domain}`.
    ClaimV1,
    /// `{ref_id, title, url, accessed_date}`
    SourceRefV1,
    /// **Fil B.1** — `{plan_id, title, steps[{id, action, deps[]}], rationale}`.
    PlanV1,
    /// **Fil B.1** — `{language, source}`.
    CodeV1,
    /// **Fil B.1** — `{thesis, reasoning[], sources[], stance}`.
    ArgumentV1,
    /// **Fil B.1** — `{summary, citations[{llm_source, snippet}], uncertainty}`.
    SynthesisV1,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialect_serializes_screaming_snake() {
        let s = serde_json::to_string(&LlmDialect::ClaudeXml).unwrap();
        assert_eq!(s, "\"CLAUDE_XML\"");
    }

    #[test]
    fn schema_id_serializes_snake() {
        let s = serde_json::to_string(&SchemaId::CitationV1).unwrap();
        assert_eq!(s, "\"citation_v1\"");
    }

    #[test]
    fn full_output_validation_passes() {
        let json = r#"{
          "canonical": {"source_id": "src-a1", "quote": "Fed held rates", "confidence": 0.95},
          "schema_id": "citation_v1",
          "extraction_warnings": []
        }"#;
        let out: ReformulatorOutput = serde_json::from_str(json).unwrap();
        assert!(out.validate().is_ok());
        assert_eq!(out.schema_id, SchemaId::CitationV1);
        assert_eq!(
            out.canonical["source_id"].as_str().unwrap(),
            "src-a1"
        );
    }

    #[test]
    fn too_many_warnings_fail() {
        let out = ReformulatorOutput {
            canonical: serde_json::json!({}),
            schema_id: SchemaId::CitationV1,
            extraction_warnings: vec!["w".to_string(); 20], // > 10
        };
        assert!(out.validate().is_err());
    }

    // ─── Fil B.1 ──────────────────────────────────────────────────

    #[test]
    fn multillm_schema_ids_serialize_snake() {
        for (id, tag) in [
            (SchemaId::PlanV1, "\"plan_v1\""),
            (SchemaId::CodeV1, "\"code_v1\""),
            (SchemaId::ArgumentV1, "\"argument_v1\""),
            (SchemaId::SynthesisV1, "\"synthesis_v1\""),
        ] {
            assert_eq!(serde_json::to_string(&id).unwrap(), tag);
            let back: SchemaId = serde_json::from_str(tag).unwrap();
            assert_eq!(back, id);
        }
    }

    #[test]
    fn input_without_stance_hint_deserializes_retrocompat() {
        // Un payload PRÉ-B.1 (sans le champ) doit désérialiser inchangé.
        let json = r#"{
          "raw_text": "some raw output",
          "source_dialect": "CLAUDE_XML",
          "target_schema_id": "claim_v1"
        }"#;
        let input: ReformulatorInput = serde_json::from_str(json).unwrap();
        assert!(input.stance_hint.is_none());
        assert!(input.validate().is_ok());
        // Et None est absent de la sérialisation (skip_serializing_if).
        let s = serde_json::to_string(&input).unwrap();
        assert!(!s.contains("stance_hint"));
    }

    #[test]
    fn input_with_stance_hint_roundtrips() {
        let input = ReformulatorInput {
            raw_text: "<thesis>rates hold</thesis>".into(),
            source_dialect: LlmDialect::ClaudeXml,
            target_schema_id: SchemaId::ArgumentV1,
            stance_hint: Some("PRO".into()),
        };
        assert!(input.validate().is_ok());
        let s = serde_json::to_string(&input).unwrap();
        assert!(s.contains("\"stance_hint\":\"PRO\""));
        let back: ReformulatorInput = serde_json::from_str(&s).unwrap();
        assert_eq!(back.stance_hint.as_deref(), Some("PRO"));
    }

    #[test]
    fn oversized_stance_hint_fails_validation() {
        let input = ReformulatorInput {
            raw_text: "x".into(),
            source_dialect: LlmDialect::Unknown,
            target_schema_id: SchemaId::ArgumentV1,
            stance_hint: Some("X".repeat(64)), // > 24
        };
        assert!(input.validate().is_err());
    }
}
