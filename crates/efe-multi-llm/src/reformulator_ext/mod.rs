// SPDX-License-Identifier: UNLICENSED
//! Reformulator étendu Phase 3 — convertit dialecte LLM vers canonical.
//!
//! **Fil B.0** rend le reformulateur *pluggable* derrière le trait
//! [`Reformulator`] : deux moteurs interchangeables — le déterministe
//! [`CanonicalReformulator`] (parsing Rust, défaut/repli) et le vrai petit
//! modèle [`SlmReformulator`] (Qwen3-0.6B, dormant en B.0). Les missions
//! choisissent via [`reformulator_for`] ; chaque sortie porte le moteur effectif
//! ([`ReformulatorEngine`]) pour un reçu de provenance honnête.

mod canonical;
mod engine;
mod slm;

pub use canonical::CanonicalReformulator;
pub use engine::{Reformulated, ReformulatorEngine};
pub use slm::{reformulator_for, SlmReformulator};

use crate::canonical::{ArgumentStance, CanonicalSchemaId};
use crate::error::MultiLlmResult;

/// Dialectes sources reconnus par le reformulator.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceDialect {
    ClaudeXml,
    Gpt4Json,
    GeminiMarkdown,
    /// Format quelconque, on tente JSON puis fallback texte brut.
    Unknown,
}

impl SourceDialect {
    /// Tag string pour logs / SSE.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::ClaudeXml => "CLAUDE_XML",
            Self::Gpt4Json => "GPT4_JSON",
            Self::GeminiMarkdown => "GEMINI_MARKDOWN",
            Self::Unknown => "UNKNOWN",
        }
    }

    /// Devine le dialecte à partir d'un nom de provider (`claude_opus_4` →
    /// `ClaudeXml`).
    #[must_use]
    pub fn from_provider_id(id: &str) -> Self {
        let l = id.to_lowercase();
        if l.contains("claude") {
            Self::ClaudeXml
        } else if l.contains("gpt") || l.contains("openai") {
            Self::Gpt4Json
        } else if l.contains("gemini") || l.contains("google") {
            Self::GeminiMarkdown
        } else {
            Self::Unknown
        }
    }
}

/// **Fil B.0** — reformulateur *pluggable*.
///
/// Traduit la sortie d'un gros modèle (dialecte propre) vers une forme
/// canonique, en déterministe OU via un petit modèle. Deux méthodes distinctes
/// des méthodes inhérentes de [`CanonicalReformulator`] (`route`, `to_argument`)
/// — délibérément, pour que les impls puissent appeler l'inhérent sans
/// ambiguïté de résolution.
#[async_trait::async_trait]
pub trait Reformulator: Send + Sync {
    /// Traduit `raw` (dialecte `from`, produit par `llm_source`) vers la forme
    /// canonique `target`. Renvoie la valeur **et** le moteur qui l'a produite.
    async fn to_canonical(
        &self,
        raw: &str,
        from: SourceDialect,
        target: CanonicalSchemaId,
        llm_source: &str,
    ) -> MultiLlmResult<Reformulated>;

    /// Cas particulier de `debate` : reformule un argument en portant sa
    /// `stance` (que la forme canonique générique `to_canonical` ne transporte
    /// pas). Renvoie la valeur `CanonicalArgument` sérialisée + le moteur.
    async fn to_argument_value(
        &self,
        raw: &str,
        stance: ArgumentStance,
        from: SourceDialect,
    ) -> MultiLlmResult<Reformulated>;
}
