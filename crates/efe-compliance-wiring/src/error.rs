// SPDX-License-Identifier: UNLICENSED
//! **Sprint 16 — N24** : closed error taxonomy for the wiring crate.
//!
//! Every wiring failure produces a `WiringError` ; the N23 gates
//! convert it to a `String` for the `SignatureGate` / `GoldBindingGate`
//! / `PublisherStatusGate` return surfaces.  The adapter then maps
//! the string back into `AdapterError::SignatureInvalid` /
//! `GoldBindingMissing` / `DispatchFailed` per its existing
//! taxonomy.

use std::time::SystemTime;

use efe_nano_sdk::{KeyId, PublisherId};
use thiserror::Error;

/// **N24** : closed taxonomy of wiring failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WiringError {
    /// The catalogue entry referenced a publisher_id that the
    /// registry does not know.
    #[error("publisher {0} not found")]
    PublisherNotFound(PublisherId),

    /// The publisher is registered but blocked from dispatch
    /// (`Suspended` or `Revoked`).
    #[error("publisher {publisher_id} is blocked from dispatch ({reason})")]
    PublisherBlocked {
        /// The blocked publisher.
        publisher_id: PublisherId,
        /// `"suspended"` or `"revoked"`.
        reason: String,
    },

    /// The publisher registry lookup exceeded the configured
    /// timeout (F10).
    #[error("publisher registry lookup timed out after {ms} ms")]
    RegistryTimeout {
        /// Configured timeout in milliseconds.
        ms: u32,
    },

    /// A backend error from the publisher registry.
    #[error("publisher registry backend error : {0}")]
    RegistryBackend(String),

    /// No key in `(active_key + historical_keys)` matches the
    /// `signature.key_id` AND none was active at
    /// `signature.signed_at`.
    #[error("no key matches signature.key_id {} at {signed_at:?}", key_id.as_uuid())]
    KeyNotFound {
        /// Signature's referenced key id.
        key_id: KeyId,
        /// When the signature was produced.
        signed_at: SystemTime,
    },

    /// `nano.author.publisher_id != signature.publisher_id` (G8).
    #[error("author.publisher_id mismatch ; nano says {nano_pub_id}, signature says {sig_pub_id}")]
    AuthorMismatch {
        /// Publisher id declared by `Author::Publisher`.
        nano_pub_id: PublisherId,
        /// Publisher id carried by the signature.
        sig_pub_id: PublisherId,
    },

    /// The crypto-level signature verification failed.
    #[error("signature verification failed : {0}")]
    SignatureFailed(String),

    /// `GoldApi::lookup_gold_set` returned an error.
    #[error("gold set lookup failed : {0}")]
    GoldLookupFailed(String),

    /// The catalogue's `gold_set_ref.gold_set_hash` does not
    /// match the registry's stored hash (F7).
    #[error("gold set hash drift ; nano expects {expected}, registry has {actual}")]
    GoldHashDrift {
        /// Expected hash from the catalogue.
        expected: String,
        /// Actual hash from the registry.
        actual: String,
    },

    /// The gold set is not in `Approved` status.
    #[error("gold set status is {0}, expected Approved")]
    GoldNotApproved(String),

    /// The catalogue entry has no `gold_set_ref` but the gate
    /// demands one.
    #[error("nano has no gold_set_ref")]
    GoldRefMissing,

    /// The catalogue entry has no `signature` but the gate
    /// demands one.
    #[error("nano has no signature")]
    SignatureMissing,

    /// The catalogue entry author is not a registered publisher,
    /// so a publisher-status check is not applicable ; the gate
    /// returns Ok in this case.  This variant exists for
    /// completeness ; not actually raised as an error.
    #[error("nano author is not a publisher")]
    NotAPublisher,

    /// The grace window for an unsigned tenant catalogue has
    /// expired ; dispatch is now refused.
    #[error("grace window expired for tenant {0}")]
    GraceExpired(uuid::Uuid),
}

impl WiringError {
    /// **N24** : stable label for metrics + log search.
    pub fn label(&self) -> &'static str {
        match self {
            Self::PublisherNotFound(_) => "publisher_not_found",
            Self::PublisherBlocked { .. } => "publisher_blocked",
            Self::RegistryTimeout { .. } => "registry_timeout",
            Self::RegistryBackend(_) => "registry_backend",
            Self::KeyNotFound { .. } => "key_not_found",
            Self::AuthorMismatch { .. } => "author_mismatch",
            Self::SignatureFailed(_) => "signature_failed",
            Self::GoldLookupFailed(_) => "gold_lookup_failed",
            Self::GoldHashDrift { .. } => "gold_hash_drift",
            Self::GoldNotApproved(_) => "gold_not_approved",
            Self::GoldRefMissing => "gold_ref_missing",
            Self::SignatureMissing => "signature_missing",
            Self::NotAPublisher => "not_a_publisher",
            Self::GraceExpired(_) => "grace_expired",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn labels_are_stable_strings() {
        let cases = [
            (WiringError::PublisherNotFound(PublisherId::new()), "publisher_not_found"),
            (
                WiringError::PublisherBlocked {
                    publisher_id: PublisherId::new(),
                    reason: "revoked".into(),
                },
                "publisher_blocked",
            ),
            (WiringError::RegistryTimeout { ms: 500 }, "registry_timeout"),
            (WiringError::RegistryBackend("x".into()), "registry_backend"),
            (
                WiringError::KeyNotFound {
                    key_id: KeyId::new(),
                    signed_at: SystemTime::now(),
                },
                "key_not_found",
            ),
            (
                WiringError::AuthorMismatch {
                    nano_pub_id: PublisherId::new(),
                    sig_pub_id: PublisherId::new(),
                },
                "author_mismatch",
            ),
            (WiringError::SignatureFailed("x".into()), "signature_failed"),
            (WiringError::GoldLookupFailed("x".into()), "gold_lookup_failed"),
            (
                WiringError::GoldHashDrift {
                    expected: "a".into(),
                    actual: "b".into(),
                },
                "gold_hash_drift",
            ),
            (WiringError::GoldNotApproved("draft".into()), "gold_not_approved"),
            (WiringError::GoldRefMissing, "gold_ref_missing"),
            (WiringError::SignatureMissing, "signature_missing"),
            (WiringError::NotAPublisher, "not_a_publisher"),
            (WiringError::GraceExpired(Uuid::nil()), "grace_expired"),
        ];
        for (err, expected) in cases {
            assert_eq!(err.label(), expected);
        }
    }

    #[test]
    fn display_includes_context() {
        let e = WiringError::AuthorMismatch {
            nano_pub_id: PublisherId::new(),
            sig_pub_id: PublisherId::new(),
        };
        let s = e.to_string();
        assert!(s.contains("author.publisher_id mismatch"));
    }

    #[test]
    fn registry_timeout_display_includes_ms() {
        let e = WiringError::RegistryTimeout { ms: 500 };
        assert!(e.to_string().contains("500 ms"));
    }
}
