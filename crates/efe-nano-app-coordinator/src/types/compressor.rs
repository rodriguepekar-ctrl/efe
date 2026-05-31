// SPDX-License-Identifier: UNLICENSED
//! Types I/O du robot **Compresseur** (Llama-3.2-1B Q5).
//!
//! Compresse un texte long en préservant des entités spécifiées et en
//! produisant un ratio mesurable (compression typique 8:1 à 12:1).

use serde::{Deserialize, Serialize};
use validator::Validate;

/// Entrée du robot Compresseur.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct CompressorInput {
    /// Texte source à compresser.
    #[validate(length(min = 50, max = 50_000))]
    pub text: String,

    /// Nombre de mots cible (±20 %).
    #[validate(range(min = 10, max = 500))]
    pub target_words: u32,

    /// Liste d'entités à préserver verbatim dans le résumé (≤ 50).
    #[validate(length(max = 50))]
    pub must_preserve: Vec<String>,

    /// Style de compression demandé.
    #[serde(default = "default_style")]
    pub style: CompressionStyle,
}

fn default_style() -> CompressionStyle {
    CompressionStyle::Extractive
}

/// Sortie *garantie* du Compresseur.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct CompressorOutput {
    /// Résumé compressé.
    #[validate(length(min = 1, max = 2000))]
    pub summary: String,

    /// Nombre de mots réel du résumé.
    #[validate(range(min = 1, max = 500))]
    pub word_count: u32,

    /// Liste des entités préservées (devrait être superset de `must_preserve`).
    #[validate(length(max = 50))]
    pub preserved: Vec<String>,

    /// Contenu omis pour brièveté (snippets courts, ≤ 100 chars chacun).
    #[validate(length(max = 20))]
    pub omitted_for_brevity: Vec<String>,

    /// Ratio source_words / summary_words.
    #[validate(range(min = 1.0, max = 50.0))]
    pub compression_ratio: f32,
}

impl CompressorOutput {
    /// Vérifie que tous les `must_preserve` sont effectivement dans `preserved`.
    pub fn validate_preservation(&self, must_preserve: &[String]) -> Result<(), String> {
        for entity in must_preserve {
            if !self.preserved.iter().any(|p| p == entity) {
                return Err(format!(
                    "must_preserve entity '{}' not found in preserved list",
                    entity
                ));
            }
        }
        Ok(())
    }
}

/// Style de compression (enum fermé).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CompressionStyle {
    /// Garde uniquement des phrases (ou fragments) tels quels du source.
    Extractive,
    /// Réécrit librement en restant fidèle au sens.
    Abstractive,
    /// Ne garde que les affirmations factuelles.
    ClaimsOnly,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_style_is_extractive() {
        let input: CompressorInput = serde_json::from_str(
            r#"{"text":"a".repeat(60), "target_words": 50, "must_preserve": []}"#
                .replace("\"a\".repeat(60)", "\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"")
                .as_str(),
        )
        .unwrap();
        assert_eq!(input.style, CompressionStyle::Extractive);
    }

    #[test]
    fn must_preserve_all_present_validates() {
        let out = CompressorOutput {
            summary: "Fed held rates at 5.25-5.50% per FOMC statement.".into(),
            word_count: 8,
            preserved: vec!["Fed".into(), "5.25".into(), "FOMC".into()],
            omitted_for_brevity: vec![],
            compression_ratio: 3.0,
        };
        let must = vec!["Fed".to_string(), "FOMC".to_string()];
        assert!(out.validate_preservation(&must).is_ok());
    }

    #[test]
    fn must_preserve_missing_fails() {
        let out = CompressorOutput {
            summary: "Fed held rates.".into(),
            word_count: 3,
            preserved: vec!["Fed".into()],
            omitted_for_brevity: vec![],
            compression_ratio: 4.0,
        };
        let must = vec!["FOMC".to_string()];
        assert!(out.validate_preservation(&must).is_err());
    }

    #[test]
    fn compression_ratio_out_of_range_fails() {
        let out = CompressorOutput {
            summary: "x".into(),
            word_count: 1,
            preserved: vec![],
            omitted_for_brevity: vec![],
            compression_ratio: 100.0, // > 50
        };
        assert!(out.validate().is_err());
    }
}
