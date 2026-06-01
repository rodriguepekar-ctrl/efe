//! Hilbert-space abstractions used throughout the crate.
//!
//! For Phase 1 we ship :
//!
//! * a concrete dynamic-dimension [`Tensor`] type with the linear-algebra
//!   primitives required by the iteration ;
//! * a [`HilbertSpace`] trait that is the formal contract every concrete
//!   space (= a tensor of given dimension) must satisfy ;
//! * a [`ProductSpace`] helper representing the joint state
//!   $\mathcal{H}_1 \times \cdots \times \mathcal{H}_M$ used by the
//!   theoretical proof of convergence.
//!
//! ## Why dynamic dimensions
//!
//! See DA-2 of the Phase-1 architecture doc.  We deliberately avoid const
//! generics to keep runtime flexibility (different bundles, different
//! latent dimensions) at the cost of compile-time dimension safety.
//! Validation happens in [`crate::builder::MetaSwarmBuilder::build`].

pub mod tensor;
pub mod hilbert;
pub mod product;

pub use tensor::Tensor;
pub use hilbert::HilbertSpace;
pub use product::ProductSpace;
