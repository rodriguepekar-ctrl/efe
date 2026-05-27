// SPDX-License-Identifier: UNLICENSED
//! Types I/O des 6 nano-robots — frozen Schema v1.0.0.
//!
//! Chaque sous-module définit `<Robot>Input` et `<Robot>Output` avec
//! `serde` + `validator` pour la validation post-parse côté Rust
//! (validation que la grammar GBNF ne couvre pas : ranges numériques,
//! cohérences inter-champs, etc.).
//!
//! ## SemVer discipline
//!
//! - **MAJOR** (ex: v2.0.0) — change cassant.  Nouveau module `analyzer_v2`.
//! - **MINOR** (ex: v1.1.0) — champ optionnel ajouté.  Même module.
//! - **PATCH** (ex: v1.0.1) — correctif doc.  Même module.
//!
//! Le numéro de version est exposé via [`crate::SCHEMA_VERSION`] et
//! propagé dans chaque audit entry.

pub mod analyzer;
pub mod compressor;
pub mod filter;
pub mod reformulator;
pub mod router;
pub mod scoring;
pub mod verifier;

// Re-exports plats pour confort.
pub use analyzer::{
    AnalyzerInput, AnalyzerOutput, Entity, EntityKind, IntentClass, IsoLang,
};
pub use compressor::{CompressionStyle, CompressorInput, CompressorOutput};
pub use filter::{
    AgentRef, CapabilityVector, FilterInput, FilterOutput, RoutingDecision,
};
pub use reformulator::{
    LlmDialect, ReformulatorInput, ReformulatorOutput, SchemaId,
};
pub use router::{
    Budget, LlmMetadata, LlmRef, RouterInput, RouterOutput, TaskClass, TaskDescription,
};
pub use scoring::{LlmStaticMetadata, ScoringWeights, TaskClassScoringTable};
pub use verifier::{
    CandidateSource, VerifierInput, VerifierOutput, VerifierSeverity, VerifierStrictness, Verdict,
};
