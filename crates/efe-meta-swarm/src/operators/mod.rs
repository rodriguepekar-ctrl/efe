//! Lift / Pull / Meta-operator traits and their Phase-1 implementations.
//!
//! The three traits together define what a meta-swarm *can compose* :
//!
//! * a [`Lift`] takes a participant's state into the meta-space
//!   ($\pi_m : \mathcal{H}_m \to \mathcal{H}_{\text{meta}}$),
//! * a [`Pull`] takes the meta-consensus back into a participant's space
//!   ($\rho_m : \mathcal{H}_{\text{meta}} \to \mathcal{H}_m$),
//! * a [`MetaOperator`] aggregates lifts into the consensus
//!   ($\Psi : \mathcal{H}_{\text{meta}}^M \to \mathcal{H}_{\text{meta}}$).
//!
//! Each trait is `Send + Sync` so that the iteration can fan out across
//! threads via `rayon::par_iter`.

pub mod lift;
pub mod pull;
pub mod meta_op;
pub mod linear;
pub mod uniform;

#[cfg(feature = "attention-weighted")]
pub mod attention;

pub use lift::Lift;
pub use pull::Pull;
pub use meta_op::MetaOperator;
pub use linear::{LinearLift, LinearPull};
pub use uniform::UniformMetaOp;

#[cfg(feature = "attention-weighted")]
pub use attention::AttentionMetaOp;
