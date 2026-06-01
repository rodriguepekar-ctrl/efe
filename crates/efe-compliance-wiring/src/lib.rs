// SPDX-License-Identifier: UNLICENSED
//! # `efe-compliance-wiring` — M50 Sprint 16 N24.
//!
//! Wires the real N18 `Ed25519Verifier` + `PublisherRegistry` and
//! the N19 `GoldApi` into the N23 `NanoToSwarm` adapter via three
//! concrete gates :
//!
//! - [`WiredSignatureGate`] — implements
//!   [`efe_nano_swarm_adapter::SignatureGate`] using a
//!   `(PublisherRegistry, SignatureVerifier)` pair.  Reconstructs
//!   the canonical body from a `NanoAgent` + `PublisherSignature`
//!   pair on the fly (catalogue does NOT persist the canonical
//!   body — annex A1).
//! - [`WiredGoldBindingGate`] — implements
//!   [`efe_nano_swarm_adapter::GoldBindingGate`] using a `GoldApi`.
//!   Verifies both status (`Approved`) and the catalogue's
//!   `gold_set_ref.gold_set_hash` against the registry value
//!   (defense against silent gold-set rewrites — F7).
//! - [`WiredPublisherStatusGate`] — implements the new
//!   `efe_nano_swarm_adapter::PublisherStatusGate` trait.
//!   Cheap-first check : runs BEFORE signature + gold (I12).
//!
//! All three are bundled by [`WiredComplianceRunner`], a factory
//! plus facade that owns the cache and metrics and exposes
//! pre-baked `NanoToSwarmPolicy` presets the gateway picks per
//! agent tier :
//!
//! - [`WiredComplianceRunner::tier1_strict_policy`] — all gates ON.
//! - [`WiredComplianceRunner::experimental_relaxed_policy`] —
//!   status only ; signature plus gold OFF (used for
//!   `TierExperimental` catalogue entries).

#![deny(unsafe_code)]
#![warn(rust_2018_idioms)]
#![warn(missing_docs)]

pub mod builtin;
pub mod cache;
pub mod canonical;
pub mod error;
pub mod gates;
pub mod grace;
pub mod metrics;
pub mod runner;

// ─────────────────────────────────────────────────────────────────────
//  Public re-exports — the user-facing surface (I20).
// ─────────────────────────────────────────────────────────────────────

pub use builtin::{
    sign_catalogue_with_builtin_publisher, EFE_BUILTIN_PUBLISHER_ID,
    EFE_BUILTIN_PUBLISHER_IDENTITY, EFE_BUILTIN_PUBLISHER_KEY_ID,
    EFE_BUILTIN_PUBLISHER_PEM, EFE_BUILTIN_PUBLISHER_SEED,
};
pub use cache::{PublisherIdentityCache, DEFAULT_CACHE_CAPACITY, DEFAULT_CACHE_TTL_SECS};
pub use canonical::{compute_agent_body_sha256, nano_to_canonical_body};
pub use error::WiringError;
pub use gates::{WiredGoldBindingGate, WiredPublisherStatusGate, WiredSignatureGate};
pub use grace::{GraceWindowResolver, InMemoryGraceWindow};
pub use metrics::N24ComplianceMetrics;
pub use runner::{
    ComplianceStatus, ComplianceStatusCounters, ComplianceStatusGates,
    WiredComplianceConfig, WiredComplianceRunner, DEFAULT_LOOKUP_TIMEOUT_MS,
};

/// **N24** : crate semver.
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");
