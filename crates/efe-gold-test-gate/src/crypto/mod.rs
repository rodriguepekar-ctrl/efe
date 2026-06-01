// SPDX-License-Identifier: UNLICENSED
//! **Sprint 12 — N20** : Cryptographic verification helpers — § 7.7
//! + G10 + ADR-5.
//!
//! Bypass certificates are signed by 2-of-3 EFE engineers ; N19
//! only checked SHAPE (count + ticket prefix + rationale length).
//! N20 adds real Ed25519 verification of each
//! `OperatorSignature.signature` against a canonical message.

pub mod bypass_verifier;

pub use bypass_verifier::{canonical_bypass_message, BypassSignatureVerifier};
