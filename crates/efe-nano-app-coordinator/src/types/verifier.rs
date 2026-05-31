// SPDX-License-Identifier: UNLICENSED
//! Types I/O du robot **Vérificateur** (Phi-4-mini Q4).
//!
//! Fait du fact-checking d'un claim contre des sources candidates.
//! Tourne *en parallèle* du streaming du gros LLM pour annoter en temps réel.

use serde::{Deserialize, Serialize};
use validator::Validate;

/// Entrée du robot Vérificateur.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct VerifierInput {
    /// Affirmation à vérifier.
    #[validate(length(min = 1, max = 2000))]
    pub claim: String,

    /// Sources candidates parmi lesquelles chercher l'évidence (≤ 50).
    #[validate(length(min = 1, max = 50), nested)]
    pub candidate_sources: Vec<CandidateSource>,

    /// Niveau de rigueur exigé.
    #[serde(default = "default_strictness")]
    pub strictness: VerifierStrictness,
}

fn default_strictness() -> VerifierStrictness {
    VerifierStrictness::Standard
}

/// Une source candidate (extrait sourcé + identifiant).
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct CandidateSource {
    /// Identifiant unique de la source (ex: `src-fed-2026-03-20`).
    #[validate(length(min = 1, max = 64))]
    pub id: String,

    /// Texte sourçant l'éventuel claim.
    #[validate(length(min = 1, max = 8000))]
    pub text: String,
}

/// Sortie *garantie* du Vérificateur.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct VerifierOutput {
    /// Verdict principal.
    pub verdict: Verdict,

    /// Source la plus pertinente (None si UNSUPPORTED).
    #[serde(default)]
    #[validate(length(max = 64))]
    pub best_source_id: Option<String>,

    /// Mesure 0..1 de la similarité au meilleur match.
    #[validate(range(min = 0.0, max = 1.0))]
    pub similarity: f32,

    /// IDs des sources qui *contredisent* le claim (≤ 5).
    #[serde(default)]
    #[validate(length(max = 5))]
    pub contradictions: Vec<String>,

    /// Sévérité pour le caller (UI surligne en jaune si WARN, rouge si ERROR).
    pub severity: VerifierSeverity,

    /// Explication courte du verdict (≤ 500 chars).
    #[serde(default)]
    #[validate(length(max = 500))]
    pub explanation: Option<String>,
}

impl VerifierOutput {
    /// Cohérence : si SUPPORTED alors `best_source_id` doit être présent.
    pub fn validate_consistency(&self) -> Result<(), String> {
        match self.verdict {
            Verdict::Supported if self.best_source_id.is_none() => {
                Err("SUPPORTED requires a best_source_id".into())
            }
            Verdict::Unsupported if self.best_source_id.is_some() => {
                Err("UNSUPPORTED should not have a best_source_id".into())
            }
            _ => Ok(()),
        }
    }
}

/// Verdict possible (enum fermé strictement enforced par GBNF).
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    Supported,
    Unsupported,
    Contradicted,
    InsufficientEvidence,
}

/// Niveau de sévérité d'un verdict (pour UI + alerting).
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum VerifierSeverity {
    Info,
    Warn,
    Error,
}

/// Niveau de rigueur demandé au vérificateur.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VerifierStrictness {
    /// Tolère les paraphrases proches.
    Lenient,
    /// Demande une correspondance sémantique nette.
    Standard,
    /// Demande une correspondance presque verbatim.
    Strict,
    /// Mode audit-grade — verbatim only + trace.
    Audit,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_serializes_screaming() {
        let s = serde_json::to_string(&Verdict::InsufficientEvidence).unwrap();
        assert_eq!(s, "\"INSUFFICIENT_EVIDENCE\"");
    }

    #[test]
    fn supported_without_source_fails_consistency() {
        let out = VerifierOutput {
            verdict: Verdict::Supported,
            best_source_id: None, // illegal
            similarity: 0.95,
            contradictions: vec![],
            severity: VerifierSeverity::Info,
            explanation: None,
        };
        assert!(out.validate_consistency().is_err());
    }

    #[test]
    fn unsupported_with_source_fails_consistency() {
        let out = VerifierOutput {
            verdict: Verdict::Unsupported,
            best_source_id: Some("src-a1".into()), // illegal
            similarity: 0.1,
            contradictions: vec![],
            severity: VerifierSeverity::Warn,
            explanation: None,
        };
        assert!(out.validate_consistency().is_err());
    }

    #[test]
    fn full_supported_output_validates() {
        let json = r#"{
          "verdict": "SUPPORTED",
          "best_source_id": "src-fed-2026-03-20",
          "similarity": 0.96,
          "contradictions": [],
          "severity": "INFO",
          "explanation": "Source explicitly states rate range"
        }"#;
        let out: VerifierOutput = serde_json::from_str(json).unwrap();
        assert!(out.validate().is_ok());
        assert!(out.validate_consistency().is_ok());
    }

    #[test]
    fn too_many_contradictions_fails() {
        let out = VerifierOutput {
            verdict: Verdict::Contradicted,
            best_source_id: Some("src-a1".into()),
            similarity: 0.5,
            contradictions: vec!["c".to_string(); 10], // > 5
            severity: VerifierSeverity::Error,
            explanation: None,
        };
        assert!(out.validate().is_err());
    }
}
