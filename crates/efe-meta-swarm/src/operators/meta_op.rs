//! MetaOperator trait : aggregator of participant votes into a single
//! meta-consensus vector.
//!
//! Mathematically, $\Psi : \mathcal{H}_{\text{meta}}^M \to
//! \mathcal{H}_{\text{meta}}$.  When the weights are state-independent
//! (i.e. when $\Psi$ is a fixed convex combination), Lemma 3.1 of A5
//! guarantees that $\Psi$ is non-expansive.  Attention-weighted variants
//! are gated behind the `attention-weighted` feature for the reasons
//! detailed in Section 6 of the A5 paper.

use crate::{Result, Tensor};

/// Operator $\Psi$ aggregating $M$ votes into the meta-consensus.
pub trait MetaOperator: Send + Sync {
    /// Dimension of the meta-space.
    fn meta_dim(&self) -> usize;

    /// Aggregate a slice of votes into a single tensor of dimension
    /// `meta_dim()`.  All votes must share this dimension.
    fn aggregate(&self, votes: &[Tensor]) -> Result<Tensor>;

    /// Best-known empirical Lipschitz bound for the aggregator with
    /// respect to its (votes-stacked) input in $\ell^2$ norm.
    /// Returns `1.0` for uniform-weighted aggregators (= Lemma 3.1 of A5).
    fn lipschitz_bound(&self) -> f64;

    /// Human-readable name.
    fn name(&self) -> &str;

    /// Convenience method.
    #[must_use]
    fn boxed(self) -> Box<dyn MetaOperator>
    where Self: Sized + 'static
    {
        Box::new(self)
    }
}
