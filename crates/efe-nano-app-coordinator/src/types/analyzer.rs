// SPDX-License-Identifier: UNLICENSED
//! Types I/O du robot **Analyseur** (SmolLM2-360M Q5).
//!
//! Schema v1.0.0 — frozen Phase 1.

use serde::{Deserialize, Serialize};
use validator::{Validate, ValidationError};

/// Entrée du robot Analyseur.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct AnalyzerInput {
    /// Texte à analyser (peut être humain ou sortie d'un LLM).
    #[validate(length(min = 1, max = 8000))]
    pub text: String,

    /// Nombre max d'entités à retourner (défaut 20, max 20).
    #[serde(default = "default_max_entities")]
    #[validate(range(min = 1, max = 20))]
    pub max_entities: u32,

    /// Identifiant optionnel de l'agent source (`claude_opus_4`, `gpt4`, ...)
    /// — utilisé pour la provenance et le distillation downstream.
    #[serde(default)]
    pub source_agent: Option<String>,
}

const fn default_max_entities() -> u32 {
    20
}

/// Sortie *garantie* du robot Analyseur (la grammar GBNF empêche tout
/// autre format de sortir de `llama-server`).
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct AnalyzerOutput {
    /// Entités nommées extraites du texte (≤ 20).
    #[validate(length(max = 20), nested)]
    pub entities: Vec<Entity>,

    /// Intention dominante du texte (enum fermé).
    pub intent: IntentClass,

    /// Sentiment global ∈ [-1.0, 1.0] (-1 = très négatif, +1 = très positif).
    #[validate(range(min = -1.0, max = 1.0))]
    pub sentiment: f32,

    /// Code langue ISO 639-1 (ou `und` si indéterminé).
    pub language: IsoLang,

    /// Résumé en une ligne (≤ 200 chars, idéalement ≤ 20 mots).
    #[validate(length(min = 1, max = 200))]
    pub one_line_summary: String,

    /// Confiance globale ∈ [0.0, 1.0].
    #[validate(range(min = 0.0, max = 1.0))]
    pub confidence: f32,
}

impl AnalyzerOutput {
    /// Validation cohérence inter-champs au-delà des contraintes
    /// per-field — appelée après `validate()` classique.
    pub fn validate_semantics(&self) -> Result<(), String> {
        for (i, e) in self.entities.iter().enumerate() {
            if e.start_char >= e.end_char {
                return Err(format!(
                    "entity[{i}]: start_char ({}) >= end_char ({})",
                    e.start_char, e.end_char
                ));
            }
            if e.text.is_empty() {
                return Err(format!("entity[{i}]: text must not be empty"));
            }
        }
        Ok(())
    }
}

/// Une entité nommée détectée dans le texte.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct Entity {
    /// Texte exact tel qu'apparu (verbatim).
    #[validate(length(min = 1, max = 200))]
    pub text: String,

    /// Catégorie (enum 9 valeurs).
    pub kind: EntityKind,

    /// Offset caractère de début (UTF-8, inclus).
    pub start_char: u32,

    /// Offset caractère de fin (UTF-8, exclu).
    #[validate(custom(function = "validate_end_after_start"))]
    pub end_char: u32,

    /// Confiance ∈ [0.0, 1.0] pour cette entité spécifique.
    #[validate(range(min = 0.0, max = 1.0))]
    pub confidence: f32,
}

// Note : ne peut pas vraiment valider end > start car validator ne donne
// pas accès aux autres champs.  La validation est faite dans `validate_semantics`.
fn validate_end_after_start(_end: u32) -> Result<(), ValidationError> {
    Ok(())
}

/// Catégorie d'entité (enum fermé — strictement enforced par GBNF).
#[allow(missing_docs)] // each variant is self-explanatory
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum EntityKind {
    Person,
    Org,
    Location,
    Money,
    Date,
    Drug,
    Event,
    Product,
    Other,
}

/// Intention dominante d'un texte (enum fermé).
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum IntentClass {
    /// Constat de fait ("Apple revenue is $90B").
    Factual,
    /// Analyse / interprétation ("This suggests growth slowdown").
    Analytical,
    /// Comparaison ("X vs Y").
    Comparative,
    /// Temporel ("After the announcement", "In 2026").
    Temporal,
    /// Opinion / jugement ("I think this is great").
    Opinion,
    /// Instructionnel ("How do I configure X?").
    Instructional,
}

/// Code langue ISO 639-1 (set fermé pour Phase 1, sera élargi V2).
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IsoLang {
    En,
    Fr,
    De,
    Es,
    It,
    Pt,
    Ja,
    Zh,
    Ru,
    Ar,
    /// Indéterminé.
    Und,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_max_entities_is_20() {
        let input: AnalyzerInput = serde_json::from_str(r#"{"text":"hello"}"#).unwrap();
        assert_eq!(input.max_entities, 20);
        assert!(input.validate().is_ok());
    }

    #[test]
    fn empty_text_fails_validation() {
        let input = AnalyzerInput {
            text: String::new(),
            max_entities: 20,
            source_agent: None,
        };
        assert!(input.validate().is_err());
    }

    #[test]
    fn entity_enum_roundtrip() {
        let kinds = [
            EntityKind::Person,
            EntityKind::Org,
            EntityKind::Money,
            EntityKind::Other,
        ];
        for k in kinds {
            let s = serde_json::to_string(&k).unwrap();
            let back: EntityKind = serde_json::from_str(&s).unwrap();
            assert_eq!(k, back);
        }
    }

    #[test]
    fn intent_serializes_uppercase() {
        let s = serde_json::to_string(&IntentClass::Factual).unwrap();
        assert_eq!(s, "\"FACTUAL\"");
    }

    #[test]
    fn lang_serializes_lowercase() {
        let s = serde_json::to_string(&IsoLang::Fr).unwrap();
        assert_eq!(s, "\"fr\"");
    }

    #[test]
    fn full_output_validation_passes() {
        let json = r#"{
          "entities": [
            {"text": "Apple", "kind": "ORG", "start_char": 0, "end_char": 5, "confidence": 0.97}
          ],
          "intent": "FACTUAL",
          "sentiment": 0.2,
          "language": "en",
          "one_line_summary": "Apple Q3 revenue",
          "confidence": 0.9
        }"#;
        let out: AnalyzerOutput = serde_json::from_str(json).unwrap();
        assert!(out.validate().is_ok());
        assert!(out.validate_semantics().is_ok());
        assert_eq!(out.entities.len(), 1);
        assert_eq!(out.entities[0].kind, EntityKind::Org);
    }

    /// C-B (right-size) — un AnalyzerOutput à **5 entités** (le nouveau cap imposé
    /// par la grammaire `analyzer_v1.gbnf` : `entity){0,4}`) valide de bout en bout
    /// (schéma + sémantique). Non-régression : le right-size produit une sortie
    /// pleinement valide, **non tronquée** — le cap 5 reste ≤ au validateur (20).
    #[test]
    fn cb_five_entity_output_is_valid() {
        let entities: Vec<Entity> = (0..5u32)
            .map(|i| Entity {
                text: format!("E{i}"),
                kind: EntityKind::Other,
                start_char: i * 2,
                end_char: i * 2 + 1,
                confidence: 0.9,
            })
            .collect();
        let out = AnalyzerOutput {
            entities,
            intent: IntentClass::Analytical,
            sentiment: 0.0,
            language: IsoLang::Fr,
            one_line_summary: "Résumé bref".into(),
            confidence: 0.8,
        };
        assert_eq!(out.entities.len(), 5);
        assert!(out.validate().is_ok(), "5 entités ≤ cap validateur (20) → valide");
        assert!(out.validate_semantics().is_ok(), "offsets cohérents → sémantique OK");
    }

    #[test]
    fn semantic_validation_catches_inverted_offsets() {
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
            one_line_summary: "X".into(),
            confidence: 0.9,
        };
        assert!(out.validate_semantics().is_err());
    }

    #[test]
    fn sentiment_out_of_range_fails() {
        let out = AnalyzerOutput {
            entities: vec![],
            intent: IntentClass::Factual,
            sentiment: 1.5, // > 1.0
            language: IsoLang::En,
            one_line_summary: "x".into(),
            confidence: 0.9,
        };
        assert!(out.validate().is_err());
    }
}
