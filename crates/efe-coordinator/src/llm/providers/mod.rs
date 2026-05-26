// SPDX-License-Identifier: UNLICENSED
//! R1 — Vrais providers LLM (Anthropic Claude / Google Gemini).
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
//! - `all-providers` → tous (raccourci)
//!
//! ## Auth
//!
//! Variables d'environnement attendues :
//! - `ANTHROPIC_API_KEY` (commence par `sk-ant-`)
//! - `GOOGLE_AI_KEY` (commence par `AIza`)
//!
//! ## Tests live
//!
//! ```bash
//! export ANTHROPIC_API_KEY="sk-ant-..."
//! export GOOGLE_AI_KEY="AIza..."
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

#[cfg(feature = "anthropic")]
pub use anthropic::ClaudeProvider;
#[cfg(feature = "gemini")]
pub use gemini::GeminiProvider;

pub use auth::ApiKey;
pub use circuit::CircuitBreaker;
pub use common::{strip_thinking_tags, PiiRedactor};
pub use pricing::PricingTable;
pub use retry::RetryPolicy;

#[cfg(feature = "anthropic")]
pub use anthropic::ClaudeRateLimits;
#[cfg(feature = "gemini")]
pub use gemini::GeminiObservability;

/// Modèle Claude par défaut (Phase 3 R1a — Q1 utilisateur).
pub const DEFAULT_CLAUDE_MODEL: &str = "claude-sonnet-4-6";

/// Modèle Gemini par défaut (Phase 3 R1a — Q2 utilisateur).
pub const DEFAULT_GEMINI_MODEL: &str = "gemini-2.5-flash";
