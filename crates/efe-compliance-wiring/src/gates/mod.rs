// SPDX-License-Identifier: UNLICENSED
//! **Sprint 16 — N24 + ADR-3** : the three wired gates.
//!
//! Each gate is independently constructible (no Arc-of-runner
//! upfront) and `Send + Sync + 'static` so the N23 adapter's
//! `Arc<dyn ...>` slots accept them directly.

pub mod gold;
pub mod signature;
pub mod status;

pub use gold::WiredGoldBindingGate;
pub use signature::WiredSignatureGate;
pub use status::WiredPublisherStatusGate;
