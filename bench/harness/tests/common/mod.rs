//! Shared helpers for integration tests.
//!
//! Code in `tests/common/` is NOT compiled as its own test binary.
//! It is included by other integration tests via `mod common;`.
//! See : <https://doc.rust-lang.org/book/ch11-03-test-organization.html#submodules-in-integration-tests>
//!
//! Sub-modules :
//!   * `sot` — single-source-of-truth detector for parallel const lists
//!     (PR #25 § 4.4 — STABILITY v1.7 § 25.2).

pub mod sot;
