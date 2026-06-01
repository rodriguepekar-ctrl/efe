// SPDX-License-Identifier: UNLICENSED
//! Reformulator étendu Phase 3 — convertit dialecte LLM vers canonical.

mod canonical;
pub use canonical::CanonicalReformulator;

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
