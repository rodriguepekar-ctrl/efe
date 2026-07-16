// SPDX-License-Identifier: UNLICENSED
//! Trait `NanoRobotClient` + helper `LlamaServerClient` + 6 implémentations.
//!
//! ## Architecture
//!
//! ```text
//!  [LlamaServerClient]  ── HTTP /completion ──►  llama-server (b9297)
//!         ▲                                              ▲
//!         │ used by                                       │ grammar enforced
//!         │                                               │
//!  [AnalyzerClient]                              [analyzer_v1.gbnf]
//!  [ReformulatorClient]                          [reformulator_v1.gbnf]
//!  [CompressorClient]                            [compressor_v1.gbnf]
//!  [VerifierClient]                              [verifier_v1.gbnf]
//!
//!  [FilterClient]                                bge-m3 ONNX (fastembed-rs)
//!  [RouterClient]                                bge-m3 + heuristics Pareto
//! ```

pub mod analyzer;
pub mod compressor;
pub mod filter;
pub mod llama_server;
pub mod reformulator;
pub mod router;
pub mod slm_circuit;
pub mod verifier;

// Re-exports plats.
pub use analyzer::AnalyzerClient;
pub use compressor::CompressorClient;
pub use filter::{EmbeddingBackend, FilterClient, MockEmbedder};
pub use llama_server::{LlamaServerClient, RobotHealth};
pub use reformulator::{ReformulatorClient, ReformulatorProfile};
pub use router::RouterClient;
pub use slm_circuit::{SlmCircuitBreaker, SlmCircuitState};
pub use verifier::VerifierClient;

use crate::context::CallContext;
use crate::error::RobotResult;
use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};

/// Trait commun à tous les nano-robots (LLM-based ou ONNX-based).
///
/// Définit la surface minimale nécessaire pour qu'un coordinateur DAG
/// (Phase 2) puisse invoquer n'importe quel robot uniformément.
#[async_trait]
pub trait NanoRobotClient: Send + Sync + 'static {
    /// Type d'input attendu.
    type Input: Serialize + Send + Sync;

    /// Type d'output garanti.
    type Output: DeserializeOwned + Send + std::fmt::Debug;

    /// Identifiant canonique kebab-case (`"analyzer"`, ...).
    fn id(&self) -> &'static str;

    /// Version du schéma I/O (SemVer `"1.0.0"`).
    fn schema_version(&self) -> &'static str;

    /// Mode d'exécution EFE (A/B/C).
    fn run_mode(&self) -> RunMode;

    /// Exécute l'inférence en produisant exactement une entrée d'audit.
    async fn invoke(
        &self,
        input: Self::Input,
        ctx: &CallContext,
    ) -> RobotResult<Self::Output>;

    /// Health check rapide (utilisé au boot du coordinator).
    async fn health(&self) -> RobotResult<RobotHealth>;
}

/// Mode d'exécution canonique d'un nano-agent EFE.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum RunMode {
    /// Pure Rust, déterminisme mathématique.
    A,
    /// ML local seeded.
    B,
    /// LLM + JSON schema enforcement (cache-replayable).
    C,
}
