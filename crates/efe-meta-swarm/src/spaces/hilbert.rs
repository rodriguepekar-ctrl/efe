//! Abstract Hilbert-space trait — kept minimal in Phase 1.
//!
//! The contract is intentionally narrow : a Hilbert space, for our
//! purposes, is just (a) a notion of dimension and (b) a way to produce
//! the zero element.  Concrete arithmetic is delegated to
//! [`crate::spaces::tensor::Tensor`].

use crate::Tensor;

/// Abstract description of a real Hilbert space $\mathcal{H} = \mathbb{R}^d$.
pub trait HilbertSpace {
    /// Dimension of the space.
    fn dim(&self) -> usize;

    /// Produce the zero element of the space.
    fn zero(&self) -> Tensor {
        Tensor::zeros(self.dim())
    }
}

/// Trivial implementation : a dimension is enough to describe a space.
#[derive(Debug, Clone, Copy)]
pub struct EuclideanSpace {
    /// Dimension `d` of `R^d`.
    pub dim: usize,
}

impl EuclideanSpace {
    /// Build a new Euclidean space of dimension `d`.
    #[must_use]
    pub const fn new(d: usize) -> Self { Self { dim: d } }
}

impl HilbertSpace for EuclideanSpace {
    fn dim(&self) -> usize { self.dim }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_has_correct_dim() {
        let s = EuclideanSpace::new(384);
        let z = s.zero();
        assert_eq!(z.dim(), 384);
    }
}
