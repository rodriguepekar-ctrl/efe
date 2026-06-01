//! Certificate types — Phase-1 stubs.
//!
//! Three certificate kinds will be supported across phases :
//!
//! * Phase 1 : [`BundleCertificate::Stub`] — always valid, no real check.
//! * Phase 3 : [`BundleCertificate::LocalSignature`] — Ed25519 over the
//!   bundle hash + the empirical Lipschitz constant from `efe-prove`.
//! * Phase 5 : [`BundleCertificate::SigstoreBundle`] — keyless / Sigstore
//!   transparency-logged certificate (cf. M19 Sigstore architecture).

use serde::{Deserialize, Serialize};

/// Certificate that a bundle satisfies the hypotheses needed by the
/// meta-swarm (H1, plus H-π and H-ρ for its lift / pull).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BundleCertificate {
    /// Phase-1 placeholder.  Always valid.
    Stub,
    /// Phase-3 — Ed25519 signature.
    /// Byte arrays use `Vec<u8>` rather than `[u8; N]` to keep Serde happy
    /// with stable derives ;  semantically a 32-byte hash + 64-byte sig.
    LocalSignature {
        /// SHA-256 of the bundle (32 bytes).
        bundle_sha256: Vec<u8>,
        /// Empirical Lipschitz of $\Phi$.
        l_phi: f64,
        /// Empirical Lipschitz of $\pi$.
        l_pi: f64,
        /// Empirical Lipschitz of $\rho$.
        l_rho: f64,
        /// Ed25519 signature (64 bytes).
        signature: Vec<u8>,
    },
    /// Phase-5 — Sigstore Bundle v0.3 (opaque payload, parsed by
    /// `efe-prove` against the Rekor log).
    SigstoreBundle {
        /// Raw bytes of the bundle.
        bytes: Vec<u8>,
    },
}

impl BundleCertificate {
    /// Phase-1 acceptance rule : `Stub` is valid, signed variants check
    /// their empirical bounds (<= 1.0).
    #[must_use]
    pub fn is_valid(&self) -> bool {
        match self {
            Self::Stub => true,
            Self::LocalSignature { l_phi, l_pi, l_rho, .. } =>
                *l_phi < 1.0 && *l_pi < 1.0 && *l_rho < 1.0,
            Self::SigstoreBundle { bytes } => !bytes.is_empty(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_is_valid() {
        assert!(BundleCertificate::Stub.is_valid());
    }

    #[test]
    fn local_signature_with_high_l_is_invalid() {
        let cert = BundleCertificate::LocalSignature {
            bundle_sha256: vec![0; 32],
            l_phi: 1.5, l_pi: 0.5, l_rho: 0.5,
            signature: vec![0; 64],
        };
        assert!(!cert.is_valid());
    }

    #[test]
    fn local_signature_with_low_l_is_valid() {
        let cert = BundleCertificate::LocalSignature {
            bundle_sha256: vec![0; 32],
            l_phi: 0.8, l_pi: 0.5, l_rho: 0.5,
            signature: vec![0; 64],
        };
        assert!(cert.is_valid());
    }
}
