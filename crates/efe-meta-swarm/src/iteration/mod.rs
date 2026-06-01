//! The coupled meta-swarm iteration.
//!
//! Three submodules :
//!
//! * [`coupled`]      — the main loop, parallelised via `rayon`.
//! * [`convergence`]  — stop criteria.
//! * [`trace`]        — optional capture of the trajectory for debugging
//!   and benchmark visualisation.

pub mod coupled;
pub mod convergence;
pub mod trace;

pub use coupled::CoupledIteration;
pub use convergence::{ConvergenceCriterion, StopReason};
pub use trace::{IterationTrace, IterationStep};
