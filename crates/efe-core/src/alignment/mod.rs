//! Alignment cone governance — M9.
//!
//! This module centralises the lifecycle of [`crate::types::AlignmentCone`]
//! instances: versioned storage, policy-enforced registration, structured
//! evolution, composition algebra, breach reporting, and an event stream
//! for M11.
//!
//! See `M9_ARCHITECTURE.md` at the workspace root for the full design
//! rationale, invariants, and sprint plan.
//!
//! # Quick tour
//!
//! - [`ConeKey`] — stable identifier `tenant/task@vN`.
//! - [`CanonicalCone`] — `AlignmentCone` + [`ConeMetadata`].
//! - [`ConeHandle`] — `(ConeKey, Arc<CanonicalCone>)` reference counted.
//! - [`AlignmentRegistry`] — async trait, main contract.
//! - [`InMemoryRegistry`] — MVP implementation (thread-safe, in-memory).
//! - [`PolicyEngine`] — validates the 7 invariants (I1–I7).
//! - [`ConeEvolutionOp`] — `AppendAnchor`, `RemoveAnchor`, …, `UnionWith`.
//! - [`union_cones`], [`blend_cones`], [`intersect_cones`] — algebra.
//! - [`BreachReport`], [`BreachLog`], [`BreachFilter`] — audit trail.
//! - [`RegistryEvent`] — stream for observability.
//! - [`crate::types::SwarmSpecBuilder::from_registry`] — opt-in builder
//!   extension exposed via the [`builder`] submodule.
//!
//! # Safety-critical
//!
//! Any change requires CODEOWNER review by `@efe-ai/safety-lead`.

pub mod breach;
pub mod builder;
pub mod canonical;
pub mod compose;
pub mod events;
pub mod evolution;
pub mod key;
pub mod policy;
pub mod registry;
pub mod validator;

// Re-exports: the curated public surface of M9.

pub use breach::{BreachContext, BreachFilter, BreachLog, BreachReport, BREACH_REASON_MAX_LEN};
pub use canonical::{CanonicalCone, ConeHandle, ConeMetadata};
pub use compose::{blend_cones, intersect_cones, union_cones};
pub use events::RegistryEvent;
pub use evolution::{
    apply_append, apply_remove, apply_replace, apply_scale, apply_update_radius, ConeEvolutionOp,
};
pub use key::ConeKey;
pub use policy::{PolicyConfig, PolicyEngine, RegistryConfig};
pub use registry::{AlignmentRegistry, BreachReporter, InMemoryRegistry};
pub use validator::validate_cone;
