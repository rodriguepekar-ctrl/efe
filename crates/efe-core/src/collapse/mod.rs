//! Collapse / Decoherence Engine (M7).
//!
//! M7 is EFE's recovery mechanism. When M6 emits a non-convergent
//! verdict (`Diverged`, `Anomaly`, or an unacceptable `Stalled`), M1
//! hands the failure to M7's [`CollapseEngine`], which picks a
//! remediation action from an ordered escalation ladder:
//!
//! 1. **`SoftRollback`** — restore the latent from M5's replay buffer.
//! 2. **`NoiseInjection`** — perturb the latent to escape a plateau.
//! 3. **`CentroidProjection`** — project every row onto the centroid.
//! 4. **`PartialRespawn`** — deactivate outlier agents.
//! 5. **`FullAbort`** — declare the swarm unrecoverable.
//!
//! # Design pillars
//!
//! - **Transactional outcome** — each `handle()` call returns a
//!   [`CollapseOutcome`]; M1 inspects `is_recoverable()` / `is_terminal()`.
//! - **Circuit breaker** — `max_collapses_per_swarm` caps the total
//!   recovery attempts per swarm; prevents infinite loops.
//! - **Deterministic** — with `deterministic_seed = true`, the same
//!   trigger on the same swarm produces the same action.
//! - **Observable** — every call emits a [`CollapseEvent`] for M11.
//!
//! # Example
//!
//! ```ignore
//! use efe_core::collapse::{
//!     CollapseConfig, CollapseEngine, CollapsePolicy,
//! };
//!
//! let mut engine = CollapseEngine::new(
//!     CollapsePolicy::default_mvp(),
//!     CollapseConfig::default(),
//! )?;
//!
//! let outcome = engine.handle(&verdict, &mut els, &arena, fusion.clone(), round).await?;
//! if outcome.is_recoverable() {
//!     // Continue fusion
//! } else if outcome.is_terminal() {
//!     // Respawn via M3 or abort
//! }
//! ```

pub mod config;
pub mod context;
pub mod engine;
pub mod event;
pub(crate) mod helpers;
pub mod outcome;
pub mod policy;
pub mod strategies;
pub mod strategy;
pub mod trigger;
pub mod validator;

pub use config::CollapseConfig;
pub use context::CollapseContext;
pub use engine::CollapseEngine;
pub use event::CollapseEvent;
pub use outcome::{CollapseAction, CollapseOutcome, CollapseWarning};
pub use policy::CollapsePolicy;
pub use strategies::{
    CentroidProjectionStrategy, FullAbortStrategy, NoiseInjectionStrategy, PartialRespawnStrategy,
    SoftRollbackStrategy, StubBehavior, StubStrategy,
};
pub use strategy::CollapseStrategy;
pub use trigger::CollapseTrigger;
pub use validator::{validate_post_collapse, PostValidation};
