//! Genesis engine (M3).
//!
//! Transforms an abstract [`crate::types::SwarmSpec`] into a concrete
//! swarm population. The pipeline has four stages, each exposed as a
//! trait for testability:
//!
//! 1. [`MetaAnalyzer`] — classifies the problem embedding into a
//!    [`TaskDag`].
//! 2. [`TemplateRouter`] — selects a weighted mix of templates.
//! 3. [`CardinalityEstimator`] — picks the total agent count and
//!    per-template split.
//! 4. [`BatchSpawner`] — allocates HBM via M2, uploads priors/anchors,
//!    launches the `spawn_batch` kernel, produces
//!    [`NanoAgentBatch`] + [`EpistemicLatentSpace`].
//!
//! The [`GenesisEngine`] wires the four stages together and
//! implements [`crate::swarm::GenesisStrategy`].
//!
//! [`NanoAgentBatch`]: crate::memory::NanoAgentBatch
//! [`EpistemicLatentSpace`]: crate::els::EpistemicLatentSpace

pub mod cardinality;
pub mod context;
pub mod dag;
pub mod engine;
pub mod meta_analyzer;
pub mod registry;
pub mod router;
pub mod spawn_kernel;
pub mod spawner;

pub use cardinality::{Cardinality, CardinalityEstimator, HeuristicEstimator};
pub use context::GenesisContext;
pub use dag::{TaskClass, TaskDag, TaskMix};
pub use engine::{GenesisEngine, GenesisEngineBuilder};
pub use meta_analyzer::{HeuristicMetaAnalyzer, MetaAnalyzer};
pub use registry::{
    ActiveAnchorsSlot, AnchorAwareTemplateRegistry, StubTemplateRegistry, TemplateRegistry,
};
pub use router::{StaticRouter, TemplateDistribution, TemplateRouter};
pub use spawn_kernel::{launch_spawn_batch, spawn_batch_cpu, SpawnBatchArgs};
pub use spawner::{BatchSpawner, DefaultSpawner, SpawnerConfig};
