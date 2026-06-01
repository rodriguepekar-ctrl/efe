//! Sprint **D16** — ONNX Runtime adapter stub.
//!
//! D16 ships the bundle format + signature + verify + loader
//! infrastructure. The real ONNX Runtime adapter (using the `ort`
//! crate) is deferred to D16-bis to avoid pulling a 200 MiB+ shared
//! library dependency into the v0.1.16 build.
//!
//! This module exists so the runtime trait surface stays stable
//! across the D16 → D16-bis transition. When D16-bis lands, this
//! file is replaced by `onnx.rs` with the same `OnnxSession` symbol
//! and the [`super::instantiate`] dispatch flips from
//! [`super::mock::MockSession`] to `OnnxSession` for
//! `Framework::Onnx` bundles.
//!
//! # Why not a feature flag in D16
//!
//! Feature flags multiply the test matrix and surface cargo-build
//! permutations in CI. D16's hot path is "load + verify + sign" —
//! none of those touch ORT. We keep ORT out of the v0.1.16 BOM and
//! re-introduce it cleanly in D16-bis.

#![allow(missing_docs)]
#![allow(dead_code)]

/// Marker type for the future ONNX session. Constructed by the
/// `instantiate` function in D16-bis ; today it's unreachable
/// because the dispatch always returns [`super::mock::MockSession`].
pub struct OnnxSessionStub;
