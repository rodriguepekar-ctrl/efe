//! Pull trait : injection from the meta-space into a participant's space.
//!
//! Mathematically, a pull is an operator $\rho : \mathcal{H}_{\text{from}}
//! \to \mathcal{H}_{\text{to}}$ that satisfies hypothesis H-ρ :
//!
//! $$ \forall z, w \in \mathcal{H}_{\text{from}},\quad
//!    \|\rho(z) - \rho(w)\|_{\mathcal{H}_{\text{to}}} \leq \|z - w\|_{\mathcal{H}_{\text{from}}}. $$

use crate::{Result, Tensor};

/// Operator $\rho : \mathcal{H}_{\text{from}} \to \mathcal{H}_{\text{to}}$.
pub trait Pull: Send + Sync {
    /// Dimension of the source (meta) space.
    fn from_dim(&self) -> usize;

    /// Dimension of the target (participant's) space.
    fn to_dim(&self) -> usize;

    /// Apply the pull.  Returns a vector of the target space.
    fn apply(&self, z: &Tensor) -> Result<Tensor>;

    /// Best-known empirical Lipschitz bound.
    fn lipschitz_bound(&self) -> f64;

    /// Human-readable name.
    fn name(&self) -> &str;

    /// Convenience method.
    #[must_use]
    fn boxed(self) -> Box<dyn Pull>
    where Self: Sized + 'static
    {
        Box::new(self)
    }
}
