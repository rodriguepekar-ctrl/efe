// SPDX-License-Identifier: UNLICENSED
//! # `efe-nano-swarm-adapter` — M50 Sprint 15 N23.
//!
//! Bidirectional adapter between the M50 `NanoAgent` catalogue
//! (data : signed, gold-bound, schema-defined, idempotent, cost-
//! estimated) and the N21 `SwarmAgent` runtime (behaviour : DAG-
//! native, blackboard + event-bus aware).
//!
//! ## Why two ecosystems exist
//!
//! - **`NanoAgent`** (`efe-nano-sdk`) is a *catalogue entry* — the
//!   signed, versioned, schema-bound descriptor of a unit of work.
//!   It does not run on its own ; a `Runtime` implementation (Mode
//!   A pure-Rust / Mode B local model / Mode C LLM-augmented)
//!   takes the catalogue entry + input and produces a
//!   `NanoAgentResponse`.
//!
//! - **`SwarmAgent`** (`efe-swarm-orchestrator`) is *runnable
//!   behaviour* inside a DAG.  It receives a `SwarmAgentContext`
//!   that hands it a shared `Blackboard` + `EventBus` and returns
//!   an `AgentOutcome`.
//!
//! The two diverged when N21 introduced `SwarmAgent` without back-
//! filling the N18-N20 compliance commitments (signature, gold
//! binding, idempotency cache) onto the swarm dispatch path.  The
//! six built-in pre-analysis agents currently bypass the entire
//! M50 control plane.  N23 fixes that.
//!
//! ## The two adapters
//!
//! - [`NanoToSwarm`] (forward, primary) — wraps a
//!   `(Arc<NanoAgent>, Arc<dyn NanoRunner>)` pair and implements
//!   `SwarmAgent`.  Around the wrapped runner the adapter
//!   performs : signature verification (G2), gold binding lookup
//!   (G4), idempotency cache lookup (G5), deterministic seed
//!   forwarding (G7), cost translation (G6), and `nano_envelope_v1`
//!   audit emission (G8).
//!
//! - [`SwarmToNano`] (reverse, ergonomic) — wraps a single
//!   `Arc<dyn SwarmAgent>` and exposes it through the
//!   `efe_nano_sdk::traits::runtime::Runtime` contract.  Used by
//!   CLI / FFI / MCP layers that speak `Runtime::execute(agent,
//!   input)` and want to invoke a swarm-native agent on the side.
//!
//! ## Catalogue back-fill
//!
//! The companion crate `efe-nano-preanalysis` ships six
//! pre-analysis agents (NER, language, schema, PII, sentiment,
//! summary).  Their `nano_descriptor()` + `nano_runner()` factory
//! functions return a fully-formed `NanoAgent` + `NanoRunner`
//! pair so the gateway's DAG registry wires them through this
//! adapter instead of as bare `SwarmAgent` impls.

#![deny(unsafe_code)]
#![warn(rust_2018_idioms)]
#![warn(missing_docs)]

pub mod cost;
pub mod determinism;
pub mod envelope;
pub mod error;
pub mod forward;
pub mod policy;
pub mod reverse;
pub mod runner;

// ─────────────────────────────────────────────────────────────────────
//  Public re-exports — the user-facing surface (I18).
// ─────────────────────────────────────────────────────────────────────

pub use cost::{cents_to_microcents, microcents_to_cents};
pub use determinism::{current_seed, SeedGuard};
pub use envelope::{NanoEnvelopeOutcome, NanoEnvelopeV1};
pub use error::AdapterError;
pub use forward::{NanoToSwarm, DEFAULT_INPUT_POINTER, NANO_OUTPUT_PREFIX};
pub use policy::{
    GoldBindingGate, IdempotencyCache, InMemoryIdempotencyCache, NanoToSwarmPolicy,
    PublisherStatusGate, SignatureGate,
};
pub use reverse::SwarmToNano;
pub use runner::{NanoRunner, NanoRunnerOutcome};

/// **N23** : crate semver.
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// **N23** : event bus topic for `nano.envelope.v1` events.
pub const ENVELOPE_TOPIC: &str = "nano.envelope.v1";
