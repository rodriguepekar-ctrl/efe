// SPDX-License-Identifier: UNLICENSED
//! # `efe-gold-test-gate` — M50 Sprint 11 N19 Gold-test gate TIER-1.
//!
//! See `docs/architecture/EFE_M50_N19_GOLD_TEST_GATE_TIER_1_ARCHITECTURE.md`.
//!
//! # Module map
//!
//! | Module | Purpose |
//! |---|---|
//! | [`registry`] | `GoldSetRegistry` trait + in-memory impl. |
//! | [`runner`] | `GoldRunner` trait + in-memory impl. |
//! | [`embedding`] | `EmbeddingProvider` trait + `StubEmbeddingProvider`. |
//! | [`assertion`] | `AssertionEngine` + 3 strategy implementations. |
//! | [`ledger`] | `GoldRunLedger` HMAC-chained ledger of runs + bypasses. |
//! | [`gate`] | `GoldGate` orchestrator. |
//! | [`api`] | HTTP query / mutation handler abstraction. |
//! | [`metrics`] | N19 Prometheus surface (12 surfaces). |

#![deny(unsafe_code)]
#![warn(rust_2018_idioms)]

pub mod api;
pub mod assertion;
/// **Sprint 12 — N20** : Cron-driven 24h drift sweep.
pub mod cron;
/// **Sprint 12 — N20** : Cryptographic bypass-signature verification.
pub mod crypto;
pub mod embedding;
pub mod gate;
pub mod ledger;
pub mod metrics;
pub mod registry;
pub mod runner;

pub use api::{DefaultGoldApi, GoldApi, GoldApiError};
pub use assertion::{AssertionEngine, AssertionError, DefaultAssertionEngine};
pub use cron::{
    DriftSweepCron, DriftSweepCronConfig, DriftSweepState, DriftSweepStateEntry,
    InMemoryDriftSweepState,
};
pub use crypto::{
    bypass_verifier::{
        BypassSignatureVerifier, InMemoryOperatorKeyResolver, OperatorKey,
        OperatorKeyResolutionError, OperatorKeyResolver,
    },
    canonical_bypass_message,
};
pub use embedding::{
    circuit_breaker::{CircuitBreaker, CircuitBreakerState},
    cohere::CohereEmbeddingProvider,
    multi_provider::{MultiProviderEmbeddingClient, NamedProvider},
    openai::OpenAiEmbeddingProvider,
    EmbeddingProvider, EmbeddingProviderError, StubEmbeddingProvider,
};
pub use gate::{GoldGate, GoldGateConfig};
pub use ledger::{
    ChainVerifyReport, GoldRunLedger, GoldRunLedgerEntry, GoldRunLedgerError,
    GoldRunLedgerRow, InMemoryGoldRunLedger,
};
pub use metrics::N19Metrics;
pub use registry::{
    GoldSetRegistry, GoldSetRegistryError, InMemoryGoldSetRegistry,
};
pub use runner::{GoldRunner, GoldRunnerError, InMemoryGoldRunner, RunnerInput};

/// **N19** : Crate semver.
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");
