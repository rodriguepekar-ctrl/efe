//! Swarm orchestration (M1).
//!
//! This module exposes the [`controller::SwarmController`] plus all
//! supporting types (request/response, verdicts, strategies, arbiter).
//!
//! # Quick start
//!
//! ```no_run
//! use std::sync::Arc;
//! use efe_core::swarm::{
//!     SwarmController, ControllerConfig,
//! };
//!
//! // Strategies are provided by other modules (M3, M5, M7, M9, M11).
//! # async fn run<G, F, D, A>(
//! #     genesis: Arc<G>, fusion: Arc<F>, dissolver: Arc<D>, alignment: Arc<A>,
//! # ) where
//! #     G: efe_core::GenesisStrategy + 'static,
//! #     F: efe_core::FusionStrategy + 'static,
//! #     D: efe_core::Dissolver + 'static,
//! #     A: efe_core::AlignmentProjector + 'static,
//! # {
//! let controller = SwarmController::builder()
//!     .genesis(genesis)
//!     .fusion(fusion)
//!     .dissolver(dissolver)
//!     .alignment(alignment)
//!     .consensus_factory(|| unimplemented!("provide a detector"))
//!     .config(ControllerConfig::default())
//!     .build()
//!     .expect("valid controller");
//! # }
//! ```

pub mod arbiter;
/// **Sprint M1 extension D34** — `ExecuteBackend` trait abstraction.
/// Cf. `docs/EFE_CORE_EXTENSION_ARCHITECTURE.md` § 8.
pub mod backend;
pub mod controller;
/// **Sprint A1 Phase 1** — coupled-iteration primitive for the
/// hierarchical meta-swarm.  Purely additive surface ;  the existing
/// `SwarmController::execute` path is untouched.  See `coupled.rs` for
/// the pure mix and `controller.rs` for the new
/// `SwarmController::step_with_pull` async method.
pub mod coupled;
/// **Sprint M1 extension D34** — Mode-2 backend + LlmRouter trait +
/// MockRouter. Cf. § 10 of the M1 extension architecture.
pub mod mode2;
/// **Sprint M1 extension D37/D38** — Mode-3 backend + Mode3Aggregator
/// trait + 3 impls + parallel runner. Cf. § 11 + § 12.
pub mod mode3;
pub mod request;
pub mod state;
pub mod strategies;
pub mod verdict;

pub use arbiter::{ArbiterSession, EnergyMeter, NullEnergyMeter, SwarmArbiter};
pub use backend::{DynExecuteBackend, ExecuteBackend};
pub use controller::{
    CollapseRecoveryPolicy, ControllerConfig, SwarmController, SwarmControllerBuilder,
};
pub use coupled::mix_coupled;
pub use mode2::{
    DynamicAnchor, LlmRouter, LlmRouterError, MockRouter, Mode2Cache, Mode2Controller,
    Mode2Verdict, RouterMetadata, RouterOutput,
};
pub use mode3::{
    AggregationError, AggregationMetadata, AggregationOutput, MeanAggregator,
    MedianAggregator, Mode3Aggregator, Mode3Controller, Mode3Verdict, SubVerdictData,
    WeightedAggregator,
};
pub use request::{DecodeMode, DecodedOutput, Request, Response, TraceContext};
pub use state::SwarmState;
pub use strategies::{
    AlignmentProjector, ConsensusDetector, Dissolver, FusionStrategy, GenesisStrategy,
    NullTelemetryCollector, NullTelemetryGuard, TelemetryCollector, TelemetryGuard,
};
pub use verdict::Verdict;
