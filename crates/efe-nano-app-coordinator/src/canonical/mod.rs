// SPDX-License-Identifier: UNLICENSED
//! Schémas canoniques cibles du Reformulateur.
//!
//! Le robot Reformulateur produit un `ReformulatorOutput { canonical, schema_id }`
//! où `canonical` est un `serde_json::Value` non-typé.  Ce module fournit la
//! validation typée par dispatch sur `schema_id`.
//!
//! ## Schémas supportés (Phase 1, frozen v1.0.0)
//!
//! | schema_id | Type Rust | Use case |
//! |---|---|---|
//! | `citation_v1` | [`Citation`] | Référence sourcée par un LLM |
//! | `fact_v1` | [`Fact`] | Triplet sujet/prédicat/objet sourcé |
//! | `tool_call_v1` | [`ToolCall`] | Appel d'outil structuré |
//! | `claim_v1` | [`Claim`] | Affirmation à vérifier avec evidence |
//! | `source_ref_v1` | [`SourceRef`] | Référence bibliographique complète |
//!
//! ## Usage
//!
//! ```no_run
//! # use efe_nano_app_coordinator::types::{ReformulatorOutput, SchemaId};
//! # use efe_nano_app_coordinator::canonical::{validate_canonical, CanonicalValue};
//! # let out: ReformulatorOutput = todo!();
//! match validate_canonical(&out) {
//!     Ok(CanonicalValue::Citation(c)) => println!("source={}, quote={}", c.source_id, c.quote),
//!     Ok(CanonicalValue::Fact(f)) => println!("{} {} {}", f.subject, f.predicate, f.object),
//!     Ok(_) => {},
//!     Err(e) => eprintln!("canonical invalid: {e}"),
//! }
//! ```

use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::types::{ReformulatorOutput, SchemaId};

/// Une citation sourcée (schema_id = `citation_v1`).
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct Citation {
    /// Identifiant de la source (ex: `src-fed-2026-03-20`).
    #[validate(length(min = 1, max = 64))]
    pub source_id: String,

    /// Citation littérale extraite.
    #[validate(length(min = 1, max = 2000))]
    pub quote: String,

    /// Confiance ∈ [0, 1] dans l'exactitude de l'extraction.
    #[validate(range(min = 0.0, max = 1.0))]
    pub confidence: f32,
}

/// Un fait sous forme de triplet sourcé (schema_id = `fact_v1`).
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct Fact {
    /// Sujet du fait (ex: "Federal Reserve").
    #[validate(length(min = 1, max = 200))]
    pub subject: String,

    /// Prédicat (ex: "maintains rate at").
    #[validate(length(min = 1, max = 200))]
    pub predicate: String,

    /// Objet (ex: "5.25-5.50%").
    #[validate(length(min = 1, max = 500))]
    pub object: String,

    /// IDs des sources qui supportent ce fait (≥ 1).
    #[validate(length(min = 1, max = 10))]
    pub source_ids: Vec<String>,
}

/// Un appel d'outil structuré (schema_id = `tool_call_v1`).
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct ToolCall {
    /// Nom de l'outil (ex: `efe_classify`, `web_search`).
    #[validate(length(min = 1, max = 128))]
    pub tool_name: String,

    /// Arguments de l'outil (JSON arbitraire validé séparément).
    pub arguments: serde_json::Value,

    /// Schéma attendu pour la sortie (référence ou identifiant).
    #[validate(length(min = 1, max = 256))]
    pub expected_output_schema: String,
}

/// Une affirmation avec evidence (schema_id = `claim_v1`).
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct Claim {
    /// Texte de l'affirmation.
    #[validate(length(min = 1, max = 1000))]
    pub claim_text: String,

    /// Evidence supportant l'affirmation (ex: extraits de sources).
    #[validate(length(max = 20), nested)]
    pub supporting_evidence: Vec<Evidence>,

    /// Confiance globale ∈ [0, 1].
    #[validate(range(min = 0.0, max = 1.0))]
    pub confidence: f32,
}

/// Un élément de preuve (utilisé par `Claim`).
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct Evidence {
    /// Identifiant de la source.
    #[validate(length(min = 1, max = 64))]
    pub source_id: String,

    /// Extrait littéral.
    #[validate(length(min = 1, max = 2000))]
    pub excerpt: String,

    /// Force de l'evidence ∈ [0, 1].
    #[validate(range(min = 0.0, max = 1.0))]
    pub strength: f32,
}

/// Une référence bibliographique (schema_id = `source_ref_v1`).
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct SourceRef {
    /// Identifiant de la référence (canonique au sein du tenant).
    #[validate(length(min = 1, max = 64))]
    pub ref_id: String,

    /// Titre du document source.
    #[validate(length(min = 1, max = 500))]
    pub title: String,

    /// URL canonique (peut être vide pour sources offline).
    #[validate(length(max = 2000))]
    pub url: String,

    /// Date d'accès au format ISO 8601 `YYYY-MM-DD`.
    #[validate(length(min = 10, max = 10))]
    pub accessed_date: String,
}

/// Énumération des valeurs canoniques typées.
///
/// Retournée par [`validate_canonical`] après dispatch sur `schema_id`.
#[derive(Debug, Clone)]
pub enum CanonicalValue {
    /// `citation_v1` — référence sourcée.
    Citation(Citation),
    /// `fact_v1` — triplet sourcé.
    Fact(Fact),
    /// `tool_call_v1` — appel d'outil.
    ToolCall(ToolCall),
    /// `claim_v1` — affirmation + evidence.
    Claim(Claim),
    /// `source_ref_v1` — référence bibliographique.
    SourceRef(SourceRef),
}

/// Erreurs de validation canonique.
#[derive(Debug, thiserror::Error)]
pub enum CanonicalError {
    /// Le JSON ne matche pas le sub-schema attendu.
    #[error("schema {schema_id:?} parse failed: {detail}")]
    ParseFailed {
        /// Le schema_id attendu.
        schema_id: SchemaId,
        /// Détail de l'erreur de parsing.
        detail: String,
    },

    /// Le sub-schema a parsé mais une contrainte sémantique a échoué.
    #[error("schema {schema_id:?} semantic violation: {detail}")]
    SemanticViolation {
        /// Le schema_id attendu.
        schema_id: SchemaId,
        /// Détail de la violation.
        detail: String,
    },
}

/// Valide le champ `canonical` d'une `ReformulatorOutput` selon son
/// `schema_id` et retourne la valeur typée.
///
/// # Errors
///
/// Retourne `CanonicalError::ParseFailed` si le JSON ne matche pas le
/// sub-schema, ou `CanonicalError::SemanticViolation` si les contraintes
/// numériques/longueurs échouent.
pub fn validate_canonical(out: &ReformulatorOutput) -> Result<CanonicalValue, CanonicalError> {
    let parsed = match out.schema_id {
        SchemaId::CitationV1 => {
            let c: Citation = serde_json::from_value(out.canonical.clone()).map_err(|e| {
                CanonicalError::ParseFailed {
                    schema_id: out.schema_id,
                    detail: e.to_string(),
                }
            })?;
            c.validate().map_err(|e| CanonicalError::SemanticViolation {
                schema_id: out.schema_id,
                detail: e.to_string(),
            })?;
            CanonicalValue::Citation(c)
        }
        SchemaId::FactV1 => {
            let f: Fact = serde_json::from_value(out.canonical.clone()).map_err(|e| {
                CanonicalError::ParseFailed {
                    schema_id: out.schema_id,
                    detail: e.to_string(),
                }
            })?;
            f.validate().map_err(|e| CanonicalError::SemanticViolation {
                schema_id: out.schema_id,
                detail: e.to_string(),
            })?;
            CanonicalValue::Fact(f)
        }
        SchemaId::ToolCallV1 => {
            let t: ToolCall = serde_json::from_value(out.canonical.clone()).map_err(|e| {
                CanonicalError::ParseFailed {
                    schema_id: out.schema_id,
                    detail: e.to_string(),
                }
            })?;
            t.validate().map_err(|e| CanonicalError::SemanticViolation {
                schema_id: out.schema_id,
                detail: e.to_string(),
            })?;
            CanonicalValue::ToolCall(t)
        }
        SchemaId::ClaimV1 => {
            let c: Claim = serde_json::from_value(out.canonical.clone()).map_err(|e| {
                CanonicalError::ParseFailed {
                    schema_id: out.schema_id,
                    detail: e.to_string(),
                }
            })?;
            c.validate().map_err(|e| CanonicalError::SemanticViolation {
                schema_id: out.schema_id,
                detail: e.to_string(),
            })?;
            CanonicalValue::Claim(c)
        }
        SchemaId::SourceRefV1 => {
            let s: SourceRef = serde_json::from_value(out.canonical.clone()).map_err(|e| {
                CanonicalError::ParseFailed {
                    schema_id: out.schema_id,
                    detail: e.to_string(),
                }
            })?;
            s.validate().map_err(|e| CanonicalError::SemanticViolation {
                schema_id: out.schema_id,
                detail: e.to_string(),
            })?;
            // Validation manuelle de la forme ISO 8601 (validator ne le fait pas natif).
            validate_iso_date(&s.accessed_date).map_err(|d| {
                CanonicalError::SemanticViolation {
                    schema_id: out.schema_id,
                    detail: d,
                }
            })?;
            CanonicalValue::SourceRef(s)
        }
    };
    Ok(parsed)
}

/// Validation ISO 8601 stricte `YYYY-MM-DD` (sans dépendre de chrono).
fn validate_iso_date(date: &str) -> Result<(), String> {
    if date.len() != 10 {
        return Err(format!("expected YYYY-MM-DD (10 chars), got {} chars", date.len()));
    }
    let bytes = date.as_bytes();
    if bytes[4] != b'-' || bytes[7] != b'-' {
        return Err("expected dashes at positions 5 and 8".into());
    }
    for (i, &b) in bytes.iter().enumerate() {
        if i == 4 || i == 7 {
            continue;
        }
        if !b.is_ascii_digit() {
            return Err(format!("non-digit at position {i}"));
        }
    }
    // Mois 01-12, jour 01-31 (sans validation calendaire complète).
    let month: u8 = date[5..7].parse().map_err(|_| "invalid month")?;
    let day: u8 = date[8..10].parse().map_err(|_| "invalid day")?;
    if !(1..=12).contains(&month) {
        return Err(format!("month out of range: {month}"));
    }
    if !(1..=31).contains(&day) {
        return Err(format!("day out of range: {day}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ReformulatorOutput;

    fn make_output(schema_id: SchemaId, canonical: serde_json::Value) -> ReformulatorOutput {
        ReformulatorOutput {
            canonical,
            schema_id,
            extraction_warnings: vec![],
        }
    }

    #[test]
    fn citation_v1_valid_parses() {
        let out = make_output(
            SchemaId::CitationV1,
            serde_json::json!({
                "source_id": "src-fed-2026-03-20",
                "quote": "Federal Reserve maintained rates at 5.25-5.50%",
                "confidence": 0.95
            }),
        );
        let result = validate_canonical(&out).unwrap();
        match result {
            CanonicalValue::Citation(c) => {
                assert_eq!(c.source_id, "src-fed-2026-03-20");
                assert!((c.confidence - 0.95).abs() < 1e-6);
            }
            other => panic!("expected Citation, got {other:?}"),
        }
    }

    #[test]
    fn citation_v1_missing_source_id_fails_parse() {
        let out = make_output(
            SchemaId::CitationV1,
            serde_json::json!({
                "quote": "x",
                "confidence": 0.5
            }),
        );
        assert!(matches!(
            validate_canonical(&out),
            Err(CanonicalError::ParseFailed { .. })
        ));
    }

    #[test]
    fn citation_v1_confidence_out_of_range_fails_semantic() {
        let out = make_output(
            SchemaId::CitationV1,
            serde_json::json!({
                "source_id": "src-a1",
                "quote": "x",
                "confidence": 2.5
            }),
        );
        assert!(matches!(
            validate_canonical(&out),
            Err(CanonicalError::SemanticViolation { .. })
        ));
    }

    #[test]
    fn fact_v1_valid_parses() {
        let out = make_output(
            SchemaId::FactV1,
            serde_json::json!({
                "subject": "Federal Reserve",
                "predicate": "maintains rate at",
                "object": "5.25-5.50%",
                "source_ids": ["src-fed-2026-03-20"]
            }),
        );
        let result = validate_canonical(&out).unwrap();
        assert!(matches!(result, CanonicalValue::Fact(_)));
    }

    #[test]
    fn fact_v1_empty_sources_fails() {
        let out = make_output(
            SchemaId::FactV1,
            serde_json::json!({
                "subject": "x",
                "predicate": "y",
                "object": "z",
                "source_ids": []
            }),
        );
        assert!(matches!(
            validate_canonical(&out),
            Err(CanonicalError::SemanticViolation { .. })
        ));
    }

    #[test]
    fn tool_call_v1_valid_parses() {
        let out = make_output(
            SchemaId::ToolCallV1,
            serde_json::json!({
                "tool_name": "efe_classify",
                "arguments": {"text": "hello", "catalog": "support_triage"},
                "expected_output_schema": "ClassifyResult"
            }),
        );
        let result = validate_canonical(&out).unwrap();
        if let CanonicalValue::ToolCall(t) = result {
            assert_eq!(t.tool_name, "efe_classify");
            assert_eq!(t.arguments["text"], "hello");
        } else {
            panic!("expected ToolCall");
        }
    }

    #[test]
    fn claim_v1_with_evidence_parses() {
        let out = make_output(
            SchemaId::ClaimV1,
            serde_json::json!({
                "claim_text": "Fed held rates",
                "supporting_evidence": [
                    {"source_id": "src-a1", "excerpt": "FOMC maintained", "strength": 0.95}
                ],
                "confidence": 0.9
            }),
        );
        let result = validate_canonical(&out).unwrap();
        if let CanonicalValue::Claim(c) = result {
            assert_eq!(c.supporting_evidence.len(), 1);
            assert_eq!(c.supporting_evidence[0].source_id, "src-a1");
        } else {
            panic!("expected Claim");
        }
    }

    #[test]
    fn source_ref_v1_valid_iso_date_parses() {
        let out = make_output(
            SchemaId::SourceRefV1,
            serde_json::json!({
                "ref_id": "ref-001",
                "title": "FOMC Statement March 20 2026",
                "url": "https://federalreserve.gov/...",
                "accessed_date": "2026-05-25"
            }),
        );
        let result = validate_canonical(&out).unwrap();
        assert!(matches!(result, CanonicalValue::SourceRef(_)));
    }

    #[test]
    fn source_ref_v1_invalid_date_fails() {
        let out = make_output(
            SchemaId::SourceRefV1,
            serde_json::json!({
                "ref_id": "ref-001",
                "title": "x",
                "url": "https://x",
                "accessed_date": "2026/05/25"  // wrong separator
            }),
        );
        assert!(matches!(
            validate_canonical(&out),
            Err(CanonicalError::SemanticViolation { .. })
        ));
    }

    #[test]
    fn source_ref_v1_invalid_month_fails() {
        let out = make_output(
            SchemaId::SourceRefV1,
            serde_json::json!({
                "ref_id": "ref-001",
                "title": "x",
                "url": "https://x",
                "accessed_date": "2026-13-01"  // month 13
            }),
        );
        assert!(matches!(
            validate_canonical(&out),
            Err(CanonicalError::SemanticViolation { .. })
        ));
    }

    #[test]
    fn validate_iso_date_edge_cases() {
        assert!(validate_iso_date("2026-05-25").is_ok());
        assert!(validate_iso_date("2026-01-01").is_ok());
        assert!(validate_iso_date("2026-12-31").is_ok());
        assert!(validate_iso_date("202-05-25").is_err());
        assert!(validate_iso_date("2026-5-25").is_err());
        assert!(validate_iso_date("").is_err());
        assert!(validate_iso_date("2026-00-15").is_err());
        assert!(validate_iso_date("2026-13-15").is_err());
        assert!(validate_iso_date("2026-05-32").is_err());
    }
}
