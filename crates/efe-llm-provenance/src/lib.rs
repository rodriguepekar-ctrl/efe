// SPDX-License-Identifier: UNLICENSED
//! # `efe-llm-provenance` — M50 Sprint 9 N17 LLM provenance.
//!
//! Per-call forensic record for every Mode C invocation.
//!
//! See `docs/architecture/EFE_M50_N17_LLM_PROVENANCE_ARCHITECTURE.md`.
//!
//! # Module map
//!
//! | Module | Purpose |
//! |---|---|
//! | [`registry`] | `ModelRegistry` trait + in-memory impl (hash verif). |
//! | [`store`] | `LlmProvenanceStore` trait + in-memory + tenant queries. |
//! | [`collector`] | `LlmProvenanceCollector` — assembles + writes records. |
//! | [`hash`] | BLAKE3 helpers + prompt-hashing utilities. |
//! | [`metrics`] | N17 Prometheus surface (12 counters / gauges / histograms). |
//! | [`query`] | HTTP-shaped query handlers for the 5 endpoints. |

#![deny(unsafe_code)]
#![warn(rust_2018_idioms)]

pub mod api;
pub mod caller;
pub mod collector;
pub mod hash;
pub mod metrics;
pub mod query;
pub mod registry;
pub mod store;

pub use api::LlmProvenanceQueryApi;
pub use caller::{call_with_provenance, ProvenancedCall};
pub use collector::{
    CacheHitProvenance, DefaultLlmProvenanceCollector, LlmProvenanceCollector,
    PostCallProvenance, PreCallProvenance, ReplayProvenance, StreamingPostCallProvenance,
};
pub use hash::{hash_bytes, hash_str, prompt_hashes_for, PromptHashes};
pub use metrics::N17Metrics;
pub use query::{CostSummary, DateRange, ModelCostRow, ModelHashMismatchEvent};
pub use registry::{
    InMemoryModelRegistry, ModelRegistry, ModelRegistryError, RegisteredModel,
};
pub use store::{
    InMemoryLlmProvenanceStore, LlmProvenanceStore, ProvenanceStoreError,
};

/// **N17** : Crate semver.
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");
