// SPDX-License-Identifier: UNLICENSED
//! # `efe-swarm-topology` — Phase 2 : couche TOPOLOGIE.
//!
//! Compositions NOMMÉES du substrat d'essaim (`efe-blackboard`, WS1-WS6) en
//! topologies 1ʳᵉ classe, **génériques** sur des traits ([`Generator`],
//! [`Verifier`], [`Stage`]) — les générateurs/vérificateurs concrets (Mode A pur
//! Rust ou Mode C SLM via `LlmClient`) vivent chez l'appelant / dans les exemples.
//!
//! Le gate **Lot S2** (ADR-SWARM-002) a validé :
//! - [`GenerateVerify`] (**T1**) : K générateurs EN PARALLÈLE + sélection
//!   a-posteriori par le vérificateur. Le winner sur les tâches vérifiables.
//! - [`Pipeline`] (**T2**) : étages chaînés en STREAMING (sur
//!   `efe_blackboard::StreamingSwarm`, sans barrière, terminaison par quiescence)
//!   — décompose une tâche dure pour la ramener dans l'enveloppe de capacité.
//!
//! ```ignore
//! use efe_swarm_topology::{GenerateVerify, Generator, Verifier};
//! let t1 = GenerateVerify::new(7, 4);          // 7 générateurs, concurrence 4
//! let r = t1.run(&task, generator, &verifier).await;   // r.selected = candidat retenu
//! ```

#![deny(unsafe_code)]
#![warn(rust_2018_idioms)]

pub mod catalog;
pub mod consensus;
pub mod consensus_topology;
pub mod fleet;
pub mod generate_verify;
pub mod hierarchical;
pub mod iterative;
pub mod pipeline;
pub mod topology;
pub mod traits;

pub use consensus::{
    aggregator_for, agreement, AggOutcome, Aggregator, Ballot, BordaConsensus, CondorcetConsensus,
    Judge, MajorityVote, Pattern, PatternSelector,
};
pub use consensus_topology::{AggregatorPolicy, ConsensusTopology};
pub use iterative::{
    builtin_arguers, builtin_refiner, ConvergenceMetric, DebateTopology, GossipTopology, Refiner,
    RoundsReport, StageRefiner,
};
pub use fleet::AgentFleet;
pub use generate_verify::{GenerateVerify, GvResult};
pub use hierarchical::{FnSolver, GvSolver, HierResult, HierSolver, Hierarchical};
pub use pipeline::Pipeline;
pub use topology::{
    GvTopology, HierTopology, PipelineTopology, QualitySignals, SwarmContext, Topology,
    TopologyOutput,
};
pub use traits::{Decomposer, Generator, Merger, Solver, Stage, Verifier};

/// Version du crate (semver).
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");
