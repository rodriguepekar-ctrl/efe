//! Safety sets : admissible regions of the latent space.
//!
//! A safety set $C \subseteq \mathcal{H}$ is closed, convex, and non-empty.
//! The metric projection $P_C : \mathcal{H} \to C$ is firmly non-expansive
//! (Lemma 3.3 of A5) and is applied at every iteration step to guarantee
//! that the iterate stays in $C$ (Theorem 5.1 of A5).
//!
//! Phase 1 ships three concrete implementations :
//!
//! * [`BoxSafety`]       — $\ell^\infty$ box, trivial projection.
//! * [`PolyhedronSafety`] — $\{x : Ax \le b\}$, projection via a single
//!   iteration of alternating projections.  See module documentation for
//!   the Phase-1 simplifications.
//! * [`LearnedSafety`]   — projection learned by a small MLP, with bounded
//!   approximation error $\varepsilon$ (cf. Theorem 5.2 of A5).

pub mod box_safety;
pub mod polyhedron;
pub mod learned;

pub use box_safety::BoxSafety;
pub use polyhedron::PolyhedronSafety;
pub use learned::LearnedSafety;

use crate::{Result, Tensor};

/// Admissible safety set : nonempty, closed, convex.
pub trait SafetySet: Send + Sync {
    /// Ambient dimension.
    fn dim(&self) -> usize;

    /// Project a point onto the set.  Returns a point in `C` regardless of
    /// where `x` started.
    fn project(&self, x: &Tensor) -> Result<Tensor>;

    /// Test membership up to a numerical tolerance.
    fn contains(&self, x: &Tensor, tolerance: f64) -> bool;

    /// Human-readable name.
    fn name(&self) -> &str;

    /// Cryptographic hash of the underlying constraints.
    ///
    /// Used in Phase 3+ to bind safety sets to certificates.  In Phase 1
    /// the default impl returns `[0; 32]` ; concrete implementations
    /// override this with a real SHA-256 over their parameters.
    fn hash(&self) -> [u8; 32] { [0; 32] }

    /// Convenience method.
    #[must_use]
    fn boxed(self) -> Box<dyn SafetySet>
    where Self: Sized + 'static
    {
        Box::new(self)
    }
}
