//! Certificates of non-expansiveness (hypothesis **H1**, `L_Φ ≤ 1`).
//!
//! Three forms, from weakest to strongest evidence:
//!
//! * [`NonExpansiveCertificate::Stub`] — Phase-1 placeholder, **no check performed**.
//!   Accepted in *lenient* mode (so pure-`MockParticipant` demos keep working) but
//!   **rejected in strict mode** (an unverified participant must not silently pass the
//!   H1 gate). This is the Couche 3 M1.c change that revives the dead guard.
//! * [`NonExpansiveCertificate::Empirical`] — the M1.c **canary** verdict: a lower
//!   bound `L̂` on `L_Φ` from adversarial black-box sampling (see
//!   [`crate::participants::lipschitz`]). A **falsifier**, not a proof — it accepts iff
//!   `L̂ < 1 − margin` *and* the search was conclusive; it can never certify
//!   non-expansiveness, only detect its violation (fail-closed).
//! * [`NonExpansiveCertificate::LocalSignature`] — Phase-3 form, an Ed25519 signature
//!   binding the bundle hash to an audited bound.

/// Certificate that a participant's internal operator satisfies H1
/// (non-expansiveness on the operational domain).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum NonExpansiveCertificate {
    /// Phase-1 placeholder — no real check performed. Valid only in lenient mode.
    Stub,
    /// **Couche 3 M1.c** — the empirical Lipschitz-canary verdict. A property-tested
    /// (NOT certified) non-expansiveness result: `L̂` is a lower bound on `L_Φ`.
    Empirical {
        /// The measured lower bound `L̂` on the participant's `L_Φ`.
        l_hat: f64,
        /// Acceptance margin: valid iff `l_hat < 1 − margin` (and conclusive).
        margin: f64,
        /// Pairs actually scored (both endpoints in-domain).
        n_pairs_scored: usize,
        /// Sampled points where `Φ` was out-of-domain.
        n_domain_errors: usize,
        /// The search could not conclude: too few in-domain samples, the `Φ`-evaluation
        /// budget was exhausted, or `Φ` **blew up** (numerical explosion / divergence) at
        /// a probe point. **Fail-closed**: an inconclusive canary is never valid.
        inconclusive: bool,
    },
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
    /// Does this certificate pass the H1 gate, in the given strictness mode?
    ///
    /// * `strict = false` (lenient, Phase-1 default) — a [`Self::Stub`] is accepted
    ///   (unverified, but the mock demos rely on it); real certificates are checked.
    /// * `strict = true` — a [`Self::Stub`] is **rejected** (fail-closed on unverified
    ///   participants).
    ///
    /// In BOTH modes an [`Self::Empirical`] / [`Self::LocalSignature`] is held to its
    /// real bound, so a `LocalParticipant` whose canary found expansion (`L̂ ≥ 1 −
    /// margin`) fails the gate regardless of strictness.
    #[must_use]
    pub fn accepts(&self, strict: bool) -> bool {
        match self {
            Self::Stub => !strict,
            Self::Empirical { l_hat, margin, inconclusive, .. } => {
                // `margin.max(0.0)`: a negative margin must never lift the threshold above
                // 1.0 and admit a measured expansion (fail-closed invariant).
                !inconclusive && l_hat.is_finite() && *l_hat < 1.0 - margin.max(0.0)
            }
            Self::LocalSignature { empirical_l_phi, .. } => *empirical_l_phi < 1.0,
        }
    }

    /// Backward-compatible lenient validity check (`accepts(false)`).
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.accepts(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empirical(l_hat: f64, margin: f64, inconclusive: bool) -> NonExpansiveCertificate {
        NonExpansiveCertificate::Empirical {
            l_hat,
            margin,
            n_pairs_scored: 100,
            n_domain_errors: 0,
            inconclusive,
        }
    }

    #[test]
    fn stub_is_lenient_only() {
        let c = NonExpansiveCertificate::Stub;
        assert!(c.accepts(false), "Stub passes in lenient mode (Phase-1 mocks)");
        assert!(!c.accepts(true), "Stub is REJECTED in strict mode (unverified)");
        assert!(c.is_valid(), "is_valid == accepts(false)");
    }

    #[test]
    fn empirical_accepts_below_margin() {
        let c = empirical(0.17, 0.05, false); // 0.17 < 1 - 0.05 = 0.95
        assert!(c.accepts(false) && c.accepts(true), "a measured 0.17-contraction satisfies H1");
    }

    #[test]
    fn empirical_rejects_at_or_above_margin() {
        // Exactly on the boundary and above must both fail (strict inequality).
        assert!(!empirical(0.95, 0.05, false).accepts(false), "L̂ = 1 − margin must fail");
        assert!(!empirical(1.3, 0.05, false).accepts(false), "an expansive L̂ must fail");
        assert!(!empirical(1.0, 0.0, false).accepts(false), "L̂ = 1 at margin 0 must fail");
    }

    #[test]
    fn empirical_inconclusive_fails_closed() {
        // Even a tiny L̂ must be rejected if the search was inconclusive.
        let c = empirical(0.01, 0.05, true);
        assert!(!c.accepts(false), "inconclusive canary must never certify (fail-closed)");
        assert!(!c.accepts(true));
    }

    #[test]
    fn empirical_non_finite_fails_closed() {
        assert!(!empirical(f64::NAN, 0.05, false).accepts(false), "NaN L̂ must fail-closed");
        assert!(!empirical(f64::INFINITY, 0.05, false).accepts(false), "inf L̂ must fail-closed");
    }

    #[test]
    fn empirical_negative_margin_cannot_admit_expansion() {
        // A negative margin must be clamped at 0, so a measured expansion (L̂ ≥ 1) is
        // never admitted regardless of the (nonsensical) margin.
        assert!(!empirical(1.2, -0.5, false).accepts(false), "negative margin must not lift threshold above 1");
        assert!(!empirical(1.0, -0.5, false).accepts(true));
        // And a genuine contraction still passes under a negative margin (clamped to 0).
        assert!(empirical(0.5, -0.5, false).accepts(false));
    }
}
