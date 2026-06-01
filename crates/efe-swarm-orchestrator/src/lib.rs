// SPDX-License-Identifier: UNLICENSED
//! # `efe-swarm-orchestrator` — M50 Sprint 13 N21.
//!
//! See `docs/architecture/EFE_M50_N21_SWARM_ORCHESTRATION_AGENTIC_BRIDGE_ARCHITECTURE.md`.

#![deny(unsafe_code)]
#![warn(rust_2018_idioms)]

pub mod agent;
pub mod compliance;
pub mod dag;
pub mod metrics;
pub mod report;
pub mod runner;

pub use agent::{
    AgentErrorKind, AgentNode, AgentOutcome, SwarmAgent, SwarmAgentContext, SwarmAgentError,
};
pub use compliance::{Compliance, ComplianceRunner};
pub use dag::{
    ConditionalEdge, DagEdge, SwarmDag, SwarmDagBuilder, SwarmDagError,
};
pub use metrics::N21Metrics;
pub use report::{AgentReport, SwarmOverallVerdict, SwarmReport};
pub use runner::{SwarmOrchestrator, SwarmRunConfig, SwarmRunError};

/// **N21** : Crate semver.
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");
