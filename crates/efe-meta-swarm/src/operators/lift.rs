//! Lift trait : projection from a participant's space into the meta-space.
//!
//! Mathematically, a lift is an operator $\pi : \mathcal{H}_{\text{from}}
//! \to \mathcal{H}_{\text{to}}$ that satisfies, in the well-behaved regime
//! of A5,
//!
//! $$ \forall x, y \in \mathcal{H}_{\text{from}},\quad
//!    \|\pi(x) - \pi(y)\|_{\mathcal{H}_{\text{to}}} \leq \|x - y\|_{\mathcal{H}_{\text{from}}} $$
//!
//! (hypothesis H-π).  Concrete implementations must report their
//! empirical [Lipschitz bound](Lift::lipschitz_bound) to the meta-swarm at
//! construction time so the [`crate::builder::MetaSwarmBuilder::build`]
//! step can warn on bundles that may violate H-π.

use crate::{Result, Tensor};

/// Operator $\pi : \mathcal{H}_{\text{from}} \to \mathcal{H}_{\text{to}}$.
pub trait Lift: Send + Sync {
    /// Dimension of the source space.
    fn from_dim(&self) -> usize;

    /// Dimension of the target (meta) space.
    fn to_dim(&self) -> usize;

    /// Apply the lift to a vector of the source space.  Returns a vector
    /// of the target space.  Returns an error if the input dimension
    /// does not match `from_dim()`.
    fn apply(&self, x: &Tensor) -> Result<Tensor>;

    /// Best-known empirical Lipschitz bound, i.e., an upper bound on
    /// $\sup_{x \neq y} \|\pi(x) - \pi(y)\| / \|x - y\|$.
    ///
    /// Returns `≤ 1.0` if H-π is empirically satisfied.  Concrete
    /// implementations should compute this via spectral norm (linear
    /// case) or via empirical estimation on a held-out sample
    /// (non-linear case).
    fn lipschitz_bound(&self) -> f64;

    /// Human-readable name of the implementation (for logs and traces).
    fn name(&self) -> &str;

    /// Convenience method : box this lift for storage in a `MetaSwarm`.
    #[must_use]
    fn boxed(self) -> Box<dyn Lift>
    where Self: Sized + 'static
    {
        Box::new(self)
    }
}
