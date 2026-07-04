//! # `efe-meta-swarm` — hierarchical multi-swarm coordination for EFE
//!
//! This crate implements the meta-swarm primitive of the EFE platform : the
//! coordination layer that lets multiple independent EFE swarms converge
//! together on a shared latent consensus *without* either swarm needing to
//! know the internal state of the other.
//!
//! ## Quick start
//!
//! ```no_run
//! use efe_meta_swarm::prelude::*;
//!
//! # fn run() -> efe_meta_swarm::Result<()> {
//! let meta = MetaSwarm::builder()
//!     .id("demo")
//!     .meta_space_dim(64)
//!     .add_participant(MockParticipant::with_dim("a", 128).boxed())
//!     .add_participant(MockParticipant::with_dim("b", 128).boxed())
//!     .lift_for("a", LinearLift::identity(128, 64).map_err(|e| e)?.boxed())
//!     .lift_for("b", LinearLift::identity(128, 64).map_err(|e| e)?.boxed())
//!     .pull_for("a", LinearPull::identity(64, 128).map_err(|e| e)?.boxed())
//!     .pull_for("b", LinearPull::identity(64, 128).map_err(|e| e)?.boxed())
//!     .meta_operator(UniformMetaOp::new(64).boxed())
//!     .meta_safety(BoxSafety::unit_ball(64).boxed())
//!     .alpha_default(0.5)
//!     .coupling_gamma(0.2)
//!     .tolerance(1e-4)
//!     .max_iters(500)
//!     .build()?;
//!
//! let result = meta.iterate()?;
//! println!("converged in {} iters", result.iterations);
//! # Ok(())
//! # }
//! ```
//!
//! ## Crate layout (Phase 1)
//!
//! * [`spaces`]       — tensors, product spaces, Hilbert-space abstraction.
//! * [`participants`] — the `SwarmParticipant` trait + `MockParticipant`.
//!   The real `LocalParticipant<S: efe_core::Swarm>` adapter is **deferred to
//!   the workspace integration** (Phase 1, week 3 — see § 3 of the Phase 1
//!   architecture document) ; we keep this crate self-contained for now so
//!   it can be built and tested in isolation.
//! * [`operators`]    — Lift / Pull / MetaOperator traits and implementations.
//! * [`safety`]       — admissible safety sets and projection operators.
//! * [`iteration`]    — the coupled iteration loop and its trace.
//! * [`meta_swarm`]   — the top-level `MetaSwarm` struct.
//! * [`builder`]      — the `MetaSwarmBuilder` fluent API.
//! * [`config`]       — Serde-deserialisable configuration types.
//! * [`observability`] — tracer trait + `NoopTracer` default.
//! * [`certification`] — stub `BundleCertificate` for Phase-3 wiring.
//! * [`prelude`]      — convenient re-exports for downstream code.
//!
//! ## Activation model (forward-looking)
//!
//! At Phase 1, "activation" means the user (= EFE team) explicitly runs
//! `cargo run --example ...` or `cargo test`.  No code in this crate is
//! linked into `efe-server`.  Later phases will introduce :
//!
//! * an env-var gate `EFE_META_SWARM_ENABLED` (Phase 2),
//! * a per-tenant feature flag `tenant.features_jsonb['meta_swarm']`
//!   (Phase 3),
//! * SDK opt-in via the `meta` module (Phase 4),
//! * billing & metering integration (Phase 5).
//!
//! The Phase-1 scaffold already provides forward-compat hooks
//! ([`certification::BundleCertificate`], [`observability::MetaSwarmTracer`],
//! a `tenant_id: Option<TenantId>` field on configurations) so that
//! the later phases plug in without refactoring.
//!
//! ## Non-regression contract
//!
//! Building this crate does **not** modify any other crate in the EFE
//! workspace, and does **not** change the SHA-256 of the `efe-server`
//! production binary.  See `EFE_A1_PHASE1_ARCHITECTURE.md` § 11 for the
//! master-gate verification.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::all)]

// -----------------------------------------------------------------------------
// Public module re-exports.
// -----------------------------------------------------------------------------

pub mod error;
pub mod spaces;
pub mod participants;
pub mod operators;
pub mod safety;
pub mod iteration;
pub mod meta_swarm;
pub mod builder;
pub mod config;
pub mod observability;
pub mod certification;
pub mod prelude;

// -----------------------------------------------------------------------------
// Crate-wide aliases.
// -----------------------------------------------------------------------------

pub use error::{MetaSwarmError, Result};
pub use iteration::{ObservedStep, StepObserver};
pub use meta_swarm::{MetaSwarm, MetaSwarmResult};
pub use builder::MetaSwarmBuilder;
pub use spaces::Tensor;

/// Câblage de la provenance dans le moteur (feature `provenance`, off par défaut) : scelle chaque pas
/// de consensus dans une chaîne HMAC re-prouvable via [`StepObserver`], sans toucher au numérique.
#[cfg(feature = "provenance")]
pub mod provenance;

/// Identifier of a participant within a meta-swarm. Always unique within one
/// `MetaSwarm` instance.
pub type SwarmId = String;

/// Identifier of an EFE tenant. Used by forward-compat hooks from Phase 3 on.
/// In Phase 1 this is always `None`.
pub type TenantId = String;
