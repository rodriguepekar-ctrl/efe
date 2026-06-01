// SPDX-License-Identifier: UNLICENSED
//! # `efe-publisher-identity` — M50 Sprint 10 N18 publisher identity.
//!
//! See `docs/architecture/EFE_M50_N18_PUBLISHER_IDENTITY_ARCHITECTURE.md`.
//!
//! # Module map
//!
//! | Module | Purpose |
//! |---|---|
//! | [`canonical`] | Canonical body hashing (SHA-256 over CBOR-like). |
//! | [`verifier`] | Ed25519 signature verification (constant-time). |
//! | [`registry`] | `PublisherRegistry` trait + in-memory impl. |
//! | [`ledger`] | `PublishLedger` HMAC-chained audit log. |
//! | [`kyc`] | `KycProvider` trait + stub. |
//! | [`sanctions`] | `SanctionsScreener` trait + stub. |
//! | [`cooldown`] | Cooldown enforcement for Tier 2+ promotion. |
//! | [`keystore`] | `KeyStore` private-key custody trait (ADR-2). |
//! | [`api`] | HTTP query / mutation handlers (architecture § 9.1). |
//! | [`metrics`] | N18 Prometheus surface. |

#![deny(unsafe_code)]
#![warn(rust_2018_idioms)]

pub mod api;
pub mod canonical;
pub mod cooldown;
pub mod keystore;
pub mod kyc;
pub mod ledger;
pub mod metrics;
pub mod registry;
pub mod sanctions;
pub mod verifier;

pub use api::{
    DefaultPublisherApi, HistoryQuery, HistoryResponse, ListQuery, PromoteRequest,
    PublisherApi, PublisherApiError, RegisterRequest, RegisterResponse, RevokeRequest,
    RotateKeyRequest, RotateKeyResponse,
};
pub use canonical::{canonical_body_hash, CanonicalBody};
pub use cooldown::{CooldownEnforcer, CooldownError};
pub use keystore::{InMemoryKeyStore, KeyStore, KeyStoreError};
pub use kyc::{KycError, KycProvider, KycVerdict, StubKycProvider};
pub use ledger::{
    ChainVerifyReport, InMemoryPublishLedger, LedgerError, LedgerRow, PublishLedger,
    PublishLedgerEntry,
};
pub use metrics::N18Metrics;
pub use registry::{
    InMemoryPublisherRegistry, PublisherRegistry, RegistryError, RevocationCascade,
    RevokedVersion,
};
pub use sanctions::{
    SanctionsError, SanctionsList, SanctionsScreener, SanctionsVerdict, StubSanctionsScreener,
};
pub use verifier::{Ed25519Signer, Ed25519Verifier, SignatureError, SignatureVerifier};

/// **N18** : Crate semver.
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");
