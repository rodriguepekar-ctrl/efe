// SPDX-License-Identifier: UNLICENSED
//! # `efe-nano-app-coordinator` — Phase 1 — JSON Grammar enforcement
//!
//! Cette crate est la **fondation déterministe** de l'application Scout &
//! Synthesize (S&S).  Elle transforme les 4 nano-robots SLM (qui répondent
//! aujourd'hui en texte libre potentiellement bancal) en **APIs strictement
//! typées** dont la sortie est *mathématiquement garantie* conforme à un
//! JSON Schema, via une grammaire GBNF appliquée côté `llama-server`.
//!
//! ## Les 6 nano-robots
//!
//! | Robot | Modèle | Mode EFE | Crate-side |
//! |---|---|---|---|
//! | Analyseur | SmolLM2-360M Q5 | B | [`clients::analyzer::AnalyzerClient`] |
//! | Reformulateur | Qwen3-0.6B Q5 | C | [`clients::reformulator::ReformulatorClient`] |
//! | Compresseur | Llama-3.2-1B Q5 | C | [`clients::compressor::CompressorClient`] |
//! | Vérificateur | Phi-4-mini Q4 | C | [`clients::verifier::VerifierClient`] |
//! | Filtreur | bge-m3 (ONNX) | A | [`clients::filter::FilterClient`] |
//! | Routeur | bge-m3 + heuristics | A | [`clients::router::RouterClient`] |
//!
//! ## Vision
//!
//! Les nano-robots remplissent **deux missions** complémentaires :
//! 1. **Prémâcher** le texte avant qu'un gros LLM (Claude) le consomme.
//! 2. **Servir de substrat d'information** entre N agents LLM tournant en
//!    parallèle (Claude + GPT-4 + Gemini + Mistral…).
//!
//! Cette crate ne s'occupe que des **clients** (Phase 1).  L'orchestration
//! DAG, le blackboard partagé et le bus NATS arrivent en Phase 2.
//!
//! ## Architecture en couches
//!
//! ```text
//!     ┌─────────────────────────────────────────────────────────────┐
//!     │  USAGE (Phase 2 — futur coordinator)                         │
//!     │  let analyzer = AnalyzerClient::from_assets(":8090")?;       │
//!     │  let out = analyzer.invoke(input, &ctx).await?;              │
//!     └────────────────────────────┬────────────────────────────────┘
//!                                  ▼
//!     ┌─────────────────────────────────────────────────────────────┐
//!     │  TRAIT NanoRobotClient (clients::NanoRobotClient)            │
//!     │  invoke(Input) -> Output, audit chain, retry policy          │
//!     └────────────────────────────┬────────────────────────────────┘
//!                                  ▼
//!     ┌─────────────────────────────────────────────────────────────┐
//!     │  HELPER LlamaServerClient (clients::llama_server::*)         │
//!     │  HTTP /completion + grammar enforcement + audit              │
//!     └────────────────────────────┬────────────────────────────────┘
//!                                  ▼
//!     ┌─────────────────────────────────────────────────────────────┐
//!     │  llama-server (port 8090/8091/8092/8093) avec GBNF chargée   │
//!     └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Niveau de stabilité
//!
//! | Composant | Statut |
//! |---|---|
//! | Trait [`clients::NanoRobotClient`] | **Stable** (frozen Phase 1) |
//! | Types I/O des 6 robots | **Stable v1.0.0** (SemVer-protégés) |
//! | Helper [`clients::llama_server::LlamaServerClient`] | Stable |
//! | Stub [`context::InMemoryAuditWriter`] | Provisoire — remplacé Phase 2 par `efe-server::audit::hmac_chain` |

#![warn(
    missing_docs,
    rust_2018_idioms,
    unreachable_pub,
    elided_lifetimes_in_paths
)]
#![allow(clippy::module_name_repetitions)]

pub mod canonical;
pub mod clients;
pub mod context;
pub mod error;
pub mod types;

// Re-exports pratiques pour les consumers.
pub use canonical::{
    validate_canonical, CanonicalError, CanonicalValue, Citation, Claim, Evidence, Fact, SourceRef,
    ToolCall,
};
pub use clients::{
    AnalyzerClient, CompressorClient, FilterClient, NanoRobotClient, ReformulatorClient,
    RouterClient, VerifierClient,
    // R3.S2 (fix L2) : re-export crate-root pour cohérence avec les autres
    // clients (AnalyzerClient etc. sont accessibles directement à
    // `efe_nano_app_coordinator::SlmCircuitBreaker` au lieu du chemin
    // verbeux `efe_nano_app_coordinator::clients::SlmCircuitBreaker`).
    SlmCircuitBreaker, SlmCircuitState,
};
pub use context::{AuditEntry, AuditStatus, AuditWriter, CallContext, InMemoryAuditWriter};
pub use error::{RobotError, RobotResult};

/// Identifiants canoniques des 6 robots (kebab-case).
pub mod robot_ids {
    /// L0 — extraction NER + intent + sentiment + langue + résumé.
    pub const ANALYZER: &str = "analyzer";
    /// L1 — normalisation de dialecte LLM vers schéma canonique.
    pub const REFORMULATOR: &str = "reformulator";
    /// L3 — compression contrainte avec préservation d'entités.
    pub const COMPRESSOR: &str = "compressor";
    /// L5 — fact-check d'un claim contre des sources.
    pub const VERIFIER: &str = "verifier";
    /// (Filtreur) routage par pertinence (embedding bge-m3).
    pub const FILTER: &str = "filter";
    /// (Routeur) sélection Pareto coût/latence/qualité du gros LLM.
    pub const ROUTER: &str = "router";
}

/// Version de schéma I/O actuelle pour les 6 robots (alignée Phase 1).
pub const SCHEMA_VERSION: &str = "1.0.0";

/// Ports par défaut des `llama-server` (mappage robot → port).
///
/// Doit rester aligné avec `EFE_PROJET/03_a_construire/scripts/start-nano-robots.sh`.
pub mod default_ports {
    /// Analyseur — SmolLM2-360M.
    pub const ANALYZER: u16 = 8090;
    /// Reformulateur — Qwen3-0.6B.
    pub const REFORMULATOR: u16 = 8091;
    /// Compresseur — Llama-3.2-1B.
    pub const COMPRESSOR: u16 = 8092;
    /// Vérificateur — Phi-4-mini.
    pub const VERIFIER: u16 = 8093;
}
