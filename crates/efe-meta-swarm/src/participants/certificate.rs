//! Certificates of non-expansiveness — Phase-1 stub.
//!
//! In Phase 1 we have no real `efe-prove` integration ;  a participant
//! returns a `Stub` certificate that is always valid.  Phase 3 will
//! introduce signed certificates that bind the participant's bundle hash
//! to a real Lipschitz audit.

/// Certificate that a participant's internal operator satisfies H1
/// (non-expansiveness on the operational domain).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum NonExpansiveCertificate {
    /// Phase-1 placeholder — always valid, no real check performed.
    Stub,
    /// Phase-3 form — Ed25519 signature over the bundle hash + the
    /// empirical Lipschitz constant measured by `efe-prove`.
    LocalSignature {
        /// SHA-256 of the bundle whose operator was audited (32 bytes).
        bundle_sha256: Vec<u8>,
        /// Empirical Lipschitz bound, in $[0, 1)$ when H1 holds.
        empirical_l_phi: f64,
        /// Ed25519 signature over `(bundle_sha256 ++ empirical_l_phi)` (64 bytes).
        signature: Vec<u8>,
    },
}

impl NonExpansiveCertificate {
    /// Is this certificate considered valid by the meta-swarm builder ?
    ///
    /// Phase 1 :  always true.
    /// Phase 3+ : checks the signature and the empirical bound.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        match self {
            Self::Stub => true,
            Self::LocalSignature { empirical_l_phi, .. } => *empirical_l_phi < 1.0,
        }
    }
}
