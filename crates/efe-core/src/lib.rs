//! EFE Core Runtime.
//!
//! This crate provides the fundamental building blocks of the Ephemeral
//! Fluid Swarm system:
//!
//! - [`types`] - shared types (identifiers, configurations, budgets)
//! - [`errors`] - unified error type [`EfeError`]
//! - [`config`] - configuration loader
//! - [`swarm`] - swarm orchestration (controller, arbiter, strategies)
//! - [`memory`] - HBM arena management (stub in MVP, real in M2)
//! - [`els`] - Epistemic Latent Space (stub in MVP, real in M4)
//! - [`fusion`] - tensor fusion data types (stats struct)
//! - [`consensus`] - convergence detection
//! - [`genesis`] - swarm spawning
//! - [`dissolution`] - centroid extraction and decoding
//! - [`alignment`] - constitution and projection
//! - [`telemetry`] - observability
//! - [`compute`] - device-agnostic tensor backend (CPU today, CUDA in Phase 3)
//!
//! # Stability
//!
//! Version 0.0.1. All public APIs may change without notice.
//! Stability guarantees start at 1.0.

#![warn(missing_docs)]
#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::should_implement_trait)]
#![allow(clippy::type_complexity)]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::panic_in_result_fn))]

/// Sprint **M1 extension D33** — nano-agent infrastructure module
/// (per-template behavioural descriptors). Cf.
/// `docs/EFE_CORE_EXTENSION_ARCHITECTURE.md` § 9.
pub mod agents;
pub mod alignment;
pub mod collapse;
pub mod compute;
pub mod config;
pub mod consensus;
pub mod dissolution;
pub mod els;
pub mod errors;
pub mod fusion;
pub mod genesis;
pub mod memory;
pub mod streams;
pub mod swarm;
pub mod telemetry;
pub mod types;

// Re-export the most commonly used items at crate root.
pub use alignment::{
    AlignmentRegistry, BreachContext, BreachFilter, BreachReport, BreachReporter, CanonicalCone,
    ConeEvolutionOp, ConeHandle, ConeKey, ConeMetadata, InMemoryRegistry, PolicyConfig,
    PolicyEngine, RegistryConfig, RegistryEvent,
};
pub use config::Config;
pub use errors::{EfeError, ErrorSeverity, Result};
pub use swarm::{
    mix_coupled, AggregationError, AggregationMetadata, AggregationOutput, AlignmentProjector,
    ArbiterSession, ConsensusDetector, ControllerConfig, DecodeMode, DecodedOutput, Dissolver,
    DynExecuteBackend, DynamicAnchor, EnergyMeter, ExecuteBackend, FusionStrategy,
    GenesisStrategy, LlmRouter,
    LlmRouterError, MeanAggregator, MedianAggregator, MockRouter, Mode2Cache, Mode2Controller,
    Mode2Verdict, Mode3Aggregator, Mode3Controller, Mode3Verdict, NullEnergyMeter,
    NullTelemetryCollector, Request, Response, RouterMetadata, RouterOutput, SubVerdictData,
    SwarmArbiter, SwarmController, SwarmControllerBuilder, SwarmState, TelemetryCollector,
    TelemetryGuard, Verdict, WeightedAggregator,
};
pub use fusion::{CanaryConfig, CanaryHybridFusion, FusionStats, HybridFusionReport};
pub use agents::templates::{
    CentroidInit, FusionKernelId, NaCausal, NaConsist, NaCrit, NaLog, NaQual, NaSafety, NaSem,
    NaTemp, NanoAgentTemplate,
};
pub use types::{
    AlignmentCone, ConvergenceThresholds, LatentDim, NanoAgentId, PopulationMix, RequestId,
    ResourceBudget, SwarmId, SwarmSpec, SwarmSpecBuilder, TemplateCode,
};

/// Current crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
