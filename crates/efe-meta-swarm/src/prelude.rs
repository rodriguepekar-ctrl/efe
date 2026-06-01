//! One-stop import for downstream code :
//!
//! ```rust
//! use efe_meta_swarm::prelude::*;
//! ```
//!
//! brings in everything you need to wire up a meta-swarm.

pub use crate::{MetaSwarm, MetaSwarmResult, MetaSwarmBuilder, MetaSwarmError, Result, Tensor};
pub use crate::participants::{SwarmParticipant, MockParticipant, NonExpansiveCertificate};
pub use crate::operators::{Lift, Pull, MetaOperator, LinearLift, LinearPull, UniformMetaOp};
pub use crate::safety::{SafetySet, BoxSafety, PolyhedronSafety, LearnedSafety};
pub use crate::iteration::{ConvergenceCriterion, StopReason, IterationTrace};
pub use crate::observability::{MetaSwarmTracer, NoopTracer};
pub use crate::certification::BundleCertificate;
pub use crate::config::{MetaSwarmConfig, ParticipantConfig};

#[cfg(feature = "attention-weighted")]
pub use crate::operators::AttentionMetaOp;
