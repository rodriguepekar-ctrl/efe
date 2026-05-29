// SPDX-License-Identifier: UNLICENSED
//! R1 — Vrais providers LLM (Anthropic Claude / Google Gemini / OpenAI GPT).
//!
//! Branchement des vrais endpoints HTTP au trait `LlmProvider` Phase 2.
//! Voir `EFE_PROJET/03_a_construire/PHASE_3_R1_REAL_LLM_PROVIDERS.pdf`
//! pour l'architecture détaillée.
//!
//! ## Activation
//!
//! Features `Cargo.toml` :
//! - `anthropic` → `ClaudeProvider` (modèle défaut : `claude-sonnet-4-6`)
//! - `gemini`    → `GeminiProvider` (modèle défaut : `gemini-2.5-flash`)
//! - `openai`    → `GptProvider`    (modèle défaut : `gpt-4o`)
//! - `all-providers` → tous les 3 (raccourci R1b complet)
//!
//! ## Auth
//!
//! Variables d'environnement attendues :
//! - `ANTHROPIC_API_KEY` (commence par `sk-ant-`)
//! - `GOOGLE_AI_KEY` (commence par `AIza`)
//! - `OPENAI_API_KEY` (commence par `sk-` — `sk-proj-` pour les nouvelles clés)
//!
//! ## Tests live
//!
//! ```bash
//! export ANTHROPIC_API_KEY="sk-ant-..."
//! export GOOGLE_AI_KEY="AIza..."
//! export OPENAI_API_KEY="sk-proj-..."
//! EFE_LIVE_LLM=1 cargo test -p efe-coordinator --features all-providers --test live_providers
//! ```

pub mod auth;
pub mod circuit;
pub mod common;
pub mod pricing;
pub mod retry;
pub mod streaming;

#[cfg(feature = "anthropic")]
pub mod anthropic;
#[cfg(feature = "gemini")]
pub mod gemini;
#[cfg(feature = "openai")]
pub mod openai;
// R3.S4 — Drafter SLM local (llama-server texte-libre).
#[cfg(feature = "local-slm")]
pub mod local_slm;

#[cfg(feature = "anthropic")]
pub use anthropic::ClaudeProvider;
#[cfg(feature = "gemini")]
pub use gemini::GeminiProvider;
#[cfg(feature = "openai")]
pub use openai::GptProvider;
#[cfg(feature = "local-slm")]
pub use local_slm::LocalSlmProvider;

pub use auth::ApiKey;
pub use circuit::CircuitBreaker;
pub use common::{strip_thinking_tags, PiiRedactor};
pub use pricing::PricingTable;
pub use retry::RetryPolicy;

#[cfg(feature = "anthropic")]
pub use anthropic::ClaudeRateLimits;
#[cfg(feature = "gemini")]
pub use gemini::GeminiObservability;
#[cfg(feature = "openai")]
pub use openai::OpenAiRateLimits;

/// Modèle Claude par défaut (Phase 3 R1a — Q1 utilisateur).
pub const DEFAULT_CLAUDE_MODEL: &str = "claude-sonnet-4-6";

/// Modèle Gemini par défaut (Phase 3 R1a — Q2 utilisateur).
pub const DEFAULT_GEMINI_MODEL: &str = "gemini-2.5-flash";

/// Modèle GPT par défaut (Phase 3 R1b — choix : balance qualité/coût stable).
pub const DEFAULT_GPT_MODEL: &str = "gpt-4o";

/// **R3.S4** — Modèle SLM local drafter par défaut (équilibre
/// qualité/vitesse/énergie pour FactualQa sur Metal). Cf. architecture
/// annexe C (Qwen 0.5B trop bruité, Llama 3B + lent).
pub const DEFAULT_LOCAL_SLM_MODEL: &str = "gemma-2-1.5b";
