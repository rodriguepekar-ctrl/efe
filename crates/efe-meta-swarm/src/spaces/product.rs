//! The product space $\mathcal{H}_1 \times \cdots \times \mathcal{H}_M$.
//!
//! Used by the convergence theorem (cf. Section 4.2 of the A1 architecture)
//! to express the joint state of all participants.  The product space
//! inherits a natural Hilbert structure from its components.

use crate::Tensor;

/// Joint state of $M$ swarms, each living in its own space.
#[derive(Debug, Clone)]
pub struct ProductSpace {
    dims: Vec<usize>,
}

impl ProductSpace {
    /// Build a product space from the list of component dimensions.
    #[must_use]
    pub fn new(dims: Vec<usize>) -> Self { Self { dims } }

    /// Number of components.
    #[must_use]
    pub fn m(&self) -> usize { self.dims.len() }

    /// Dimension of the `m`-th component.
    #[must_use]
    pub fn dim_of(&self, m: usize) -> usize { self.dims[m] }

    /// Total joint dimension $\sum_{m} d_m$.
    #[must_use]
    pub fn total_dim(&self) -> usize { self.dims.iter().sum() }

    /// Joint squared norm $\sum_m \|x_m\|^2$.
    #[must_use]
    pub fn joint_norm_squared(states: &[Tensor]) -> f64 {
        states.iter().map(Tensor::norm_squared).sum()
    }

    /// Joint distance between two joint states.
    /// Panics if shapes don't match.
    #[must_use]
    pub fn joint_distance(a: &[Tensor], b: &[Tensor]) -> f64 {
        assert_eq!(a.len(), b.len(), "joint state cardinality mismatch");
        let mut sq = 0.0_f64;
        for (xa, xb) in a.iter().zip(b) {
            // We tolerate component-level dimension mismatch by returning ∞ —
            // such a situation indicates a bug upstream, not a numerical
            // condition.
            sq += xa.distance_squared(xb).unwrap_or(f64::INFINITY);
        }
        sq.sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_dim_is_sum() {
        let ps = ProductSpace::new(vec![384, 512, 384]);
        assert_eq!(ps.total_dim(), 1280);
    }

    #[test]
    fn joint_norm_aggregates() {
        let states = vec![
            Tensor::from_vec(vec![3.0, 4.0]),  // ‖·‖² = 25
            Tensor::from_vec(vec![0.0, 5.0]),  // ‖·‖² = 25
        ];
        assert!((ProductSpace::joint_norm_squared(&states) - 50.0).abs() < 1e-6);
    }

    #[test]
    fn joint_distance_zero_to_self() {
        let s = vec![Tensor::pseudo_random(32, 1), Tensor::pseudo_random(64, 2)];
        assert!(ProductSpace::joint_distance(&s, &s) < 1e-9);
    }
}
