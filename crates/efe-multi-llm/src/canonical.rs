// SPDX-License-Identifier: UNLICENSED
//! Canonical schemas v1.0.0 — pivot multi-LLM.
//!
//! Tous les outputs LLM (Claude XML / GPT JSON / Gemini Markdown) sont
//! reformulés via [`crate::CanonicalReformulator`] vers l'un de ces 5
//! types canoniques.  Cela permet aux LLMs de "se comprendre" malgré
//! leurs dialectes différents.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Version frozen des 5 schemas canoniques.
pub const CANONICAL_SCHEMA_VERSION: &str = "1.0.0";

/// Identifiant d'un canonical schema — utilisé pour le routing du
/// reformulator.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CanonicalSchemaId {
    PlanV1,
    CodeV1,
    ClaimV1,
    ArgumentV1,
    SynthesisV1,
}

impl CanonicalSchemaId {
    /// Chemin asset (`canonical/<schema>.json`).
    #[must_use]
    pub fn asset_id(self) -> &'static str {
        match self {
            Self::PlanV1 => "canonical/plan_v1",
            Self::CodeV1 => "canonical/code_v1",
            Self::ClaimV1 => "canonical/claim_v1",
            Self::ArgumentV1 => "canonical/argument_v1",
            Self::SynthesisV1 => "canonical/synthesis_v1",
        }
    }

    /// Nom court (kebab) pour observabilité.
    #[must_use]
    pub fn short(self) -> &'static str {
        match self {
            Self::PlanV1 => "plan_v1",
            Self::CodeV1 => "code_v1",
            Self::ClaimV1 => "claim_v1",
            Self::ArgumentV1 => "argument_v1",
            Self::SynthesisV1 => "synthesis_v1",
        }
    }
}

// ─── plan_v1 ──────────────────────────────────────────────────────

/// Plan structuré — produit typique par Claude (raisonnement).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CanonicalPlan {
    /// UUID v7 du plan.
    pub plan_id: Uuid,
    /// Titre court.
    #[serde(default)]
    pub title: String,
    /// Liste ordonnée d'étapes.
    pub steps: Vec<PlanStep>,
    /// Justification du plan.
    #[serde(default)]
    pub rationale: String,
}

/// Une étape d'un plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanStep {
    /// Index 1-based.
    pub id: u32,
    /// Description verbale de l'action.
    pub action: String,
    /// Dépendances (IDs d'autres étapes du plan).
    #[serde(default)]
    pub deps: Vec<u32>,
    /// Estimation de durée en minutes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_duration_min: Option<u32>,
}

// ─── code_v1 ──────────────────────────────────────────────────────

/// Code produit — typiquement par GPT (capability CODE=0.94).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CanonicalCode {
    /// Langage (enum fermé).
    pub language: CodeLanguage,
    /// Code source principal.
    pub source: String,
    /// Tests unitaires associés.
    #[serde(default)]
    pub tests: Vec<CodeTest>,
    /// Dépendances externes (Cargo, npm, pip, etc.).
    #[serde(default)]
    pub deps: Vec<CodeDep>,
    /// Estimation complexité.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complexity_estimate: Option<CodeComplexity>,
}

/// Langages supportés en canonical.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodeLanguage {
    Rust,
    Python,
    Typescript,
    Go,
    Java,
    Cpp,
    Sql,
    Bash,
}

/// Un test unitaire associé au code.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeTest {
    /// Nom du test.
    pub name: String,
    /// Source du test.
    pub source: String,
}

/// Une dépendance externe.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeDep {
    /// Nom du package.
    pub name: String,
    /// Version (semver, range, …).
    pub version: String,
}

/// Complexité estimée.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CodeComplexity {
    Trivial,
    Simple,
    Moderate,
    Complex,
    Research,
}

// ─── claim_v1 ─────────────────────────────────────────────────────

/// Affirmation factuelle (utilisée pour Mission Vote).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CanonicalClaim {
    /// L'affirmation.
    pub claim: String,
    /// Évidences (≤ 10).
    #[serde(default)]
    pub evidence: Vec<ClaimEvidence>,
    /// Confiance 0..1.
    pub confidence: f32,
    /// Domaine (médical, finance, etc.).
    #[serde(default)]
    pub domain: String,
}

/// Une évidence supportant un claim.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClaimEvidence {
    /// Source (URL, doc id, etc.).
    pub source: String,
    /// Extrait verbatim.
    pub excerpt: String,
}

// ─── argument_v1 ──────────────────────────────────────────────────

/// Argument structuré (Mission Debate).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CanonicalArgument {
    /// Thèse principale.
    pub thesis: String,
    /// Étapes de raisonnement.
    pub reasoning: Vec<String>,
    /// Sources citées.
    #[serde(default)]
    pub sources: Vec<String>,
    /// Si rebut, identifie l'argument adverse (verbatim ou ID).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addresses_counter: Option<String>,
    /// Position.
    pub stance: ArgumentStance,
}

/// Position d'un argument.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ArgumentStance {
    Pro,
    Contra,
    Neutral,
    Rebuttal,
}

// ─── synthesis_v1 ─────────────────────────────────────────────────

/// Synthèse multi-LLM (Mission Specialist, Debate jury, etc.).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CanonicalSynthesis {
    /// Résumé principal.
    pub summary: String,
    /// Citations des LLMs sources.
    #[serde(default)]
    pub citations: Vec<SynthesisCitation>,
    /// Notes d'incertitude.
    #[serde(default)]
    pub uncertainty: String,
    /// Sections optionnelles.
    #[serde(default)]
    pub sections: Vec<SynthesisSection>,
}

/// Citation dans une synthèse.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SynthesisCitation {
    /// Source LLM (`claude` | `gpt` | `gemini` | `drafter` | `unknown`).
    pub llm_source: String,
    /// Extrait cité.
    pub snippet: String,
}

/// Section de synthèse.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SynthesisSection {
    /// Titre.
    #[serde(default)]
    pub heading: String,
    /// Corps.
    #[serde(default)]
    pub body: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_version_frozen() {
        assert_eq!(CANONICAL_SCHEMA_VERSION, "1.0.0");
    }

    #[test]
    fn schema_id_asset_paths() {
        assert_eq!(CanonicalSchemaId::PlanV1.asset_id(), "canonical/plan_v1");
        assert_eq!(CanonicalSchemaId::CodeV1.asset_id(), "canonical/code_v1");
        assert_eq!(CanonicalSchemaId::ClaimV1.asset_id(), "canonical/claim_v1");
        assert_eq!(CanonicalSchemaId::ArgumentV1.asset_id(), "canonical/argument_v1");
        assert_eq!(CanonicalSchemaId::SynthesisV1.asset_id(), "canonical/synthesis_v1");
    }

    #[test]
    fn plan_roundtrips() {
        let p = CanonicalPlan {
            plan_id: Uuid::now_v7(),
            title: "test".into(),
            steps: vec![PlanStep {
                id: 1,
                action: "do it".into(),
                deps: vec![],
                estimated_duration_min: Some(5),
            }],
            rationale: "because".into(),
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: CanonicalPlan = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn code_language_serializes_lowercase() {
        let s = serde_json::to_string(&CodeLanguage::Rust).unwrap();
        assert_eq!(s, "\"rust\"");
    }

    #[test]
    fn code_complexity_screams() {
        let s = serde_json::to_string(&CodeComplexity::Research).unwrap();
        assert_eq!(s, "\"RESEARCH\"");
    }

    #[test]
    fn claim_with_evidence_roundtrips() {
        let c = CanonicalClaim {
            claim: "Paris is in France".into(),
            evidence: vec![ClaimEvidence {
                source: "wikipedia".into(),
                excerpt: "Paris is the capital of France.".into(),
            }],
            confidence: 0.99,
            domain: "geography".into(),
        };
        let s = serde_json::to_string(&c).unwrap();
        let back: CanonicalClaim = serde_json::from_str(&s).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn argument_stance_serializes() {
        let a = ArgumentStance::Pro;
        assert_eq!(serde_json::to_string(&a).unwrap(), "\"PRO\"");
        let r = ArgumentStance::Rebuttal;
        assert_eq!(serde_json::to_string(&r).unwrap(), "\"REBUTTAL\"");
    }

    #[test]
    fn synthesis_with_citations_roundtrips() {
        let syn = CanonicalSynthesis {
            summary: "Multi-LLM analysis".into(),
            citations: vec![SynthesisCitation {
                llm_source: "claude".into(),
                snippet: "...".into(),
            }],
            uncertainty: "moderate".into(),
            sections: vec![],
        };
        let s = serde_json::to_string(&syn).unwrap();
        let back: CanonicalSynthesis = serde_json::from_str(&s).unwrap();
        assert_eq!(back, syn);
    }

    #[test]
    fn schema_id_short_names_kebab_lowercase() {
        for s in [
            CanonicalSchemaId::PlanV1,
            CanonicalSchemaId::CodeV1,
            CanonicalSchemaId::ClaimV1,
            CanonicalSchemaId::ArgumentV1,
            CanonicalSchemaId::SynthesisV1,
        ] {
            assert!(s
                .short()
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit()));
        }
    }
}
