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
    /// `{claim_text, supporting_evidence[], confidence}`
    ClaimV1,
    /// `{ref_id, title, url, accessed_date}`
    SourceRefV1,
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
}
