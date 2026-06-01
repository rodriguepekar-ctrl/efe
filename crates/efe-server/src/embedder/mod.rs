//! Sprint **D16** — Embedder bundling subsystem.
//!
//! Mirror of `docs/bench/d16_embedder_bundling_architecture.md`.
//!
//! Public surface :
//!
//!   * [`Bundle`] — a fully-loaded, verified, ready-to-encode bundle.
//!   * [`load_bundle_dir`] — top-level orchestrator (parse → verify
//!     → sign → instantiate).
//!   * [`EmbedderRegistry`] — singleflight cache with one bundle
//!     per id.
//!   * [`Manifest`] / [`Component`] / [`RuntimeConfig`] — the JSON
//!     schema types.
//!   * [`TrustStore`] / [`load_trust_store_file`] — Ed25519 trusted
//!     keys.
//!   * [`compatibility_match`] / [`CompatMatch`] — D8 ↔ D16 catalog
//!     fingerprint anti-corruption layer.
//!
//! # Module map
//!
//! ```text
//! embedder/
//! ├── mod.rs          — public API
//! ├── manifest.rs     — manifest.json types + invariants
//! ├── trust.rs        — pubkeys.json loader
//! ├── signature.rs    — Ed25519 detached verify
//! ├── verify.rs       — per-component SHA-256 walk
//! ├── bundle.rs       — load_bundle_dir orchestrator + Bundle handle
//! ├── cache.rs        — EmbedderRegistry singleflight
//! ├── legacy.rs       — D8 fingerprint anti-corruption
//! └── runtime/
//!     ├── mod.rs      — EmbedderSession trait + instantiate()
//!     ├── mock.rs     — deterministic SHA-256-seeded mock
//!     └── onnx_stub.rs — placeholder for D16-bis ONNX adapter
//! ```

pub mod bundle;
pub mod cache;
pub mod legacy;
pub mod manifest;
/// Sprint **D27** (M4 § 11) — atomic bundle reload (Phase 1 verify,
/// Phase 2 atomic swap, Phase 3 background drain).
pub mod reload;
pub mod runtime;
pub mod signature;
pub mod trust;
pub mod verify;

// Re-exports — single place to consume the D16 API.

pub use bundle::{
    load_bundle_dir, Bundle, BundleFingerprint, BundleLoadError, LoadOutcome, LoadPolicy,
};
pub use cache::{BundleId, EmbedderRegistry};
pub use legacy::{
    compatibility_match, extract_legacy_fingerprint, warn_on_legacy_match, CompatMatch,
};
pub use manifest::{
    parse_bundle_id, BuildInfo, Component, Dtype, Framework, LicenseInfo, Manifest, ManifestError,
    ManifestParseError, ModelOrigin, ParsedBundleId, Pooling, RuntimeConfig, CANONICAL_DIMS,
    MAX_SEQ_LEN_HARD_CAP, SUPPORTED_MANIFEST_VERSION,
};
pub use reload::{
    ActiveBundleSlot, ReloadError, ReloadOutcome, DEFAULT_DRAIN_TIMEOUT, DRAIN_POLL_INTERVAL,
    SMOKE_FIXTURES, SMOKE_L2_TOLERANCE,
};
pub use runtime::{instantiate, mock::MockSession, EmbedderError, EmbedderSession};
pub use signature::{verify_manifest_signature, SigError, VerifiedSignature};
pub use trust::{
    load_trust_store_file, TrustStore, TrustStoreDocument, TrustStoreLoadError, TrustedKey,
    TrustedKeyDoc, SUPPORTED_TRUST_STORE_VERSION,
};
pub use verify::{verify_components, VerifyError};
