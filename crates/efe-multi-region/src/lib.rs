// SPDX-License-Identifier: UNLICENSED
//! # `efe-multi-region` — M50 Sprint 9 N16 multi-region replay.
//!
//! Multi-region replay coordinator, chain shipment, JWT auth, and the
//! multi-region replay engine.  Consolidates the three Sprint 9 N16
//! logical components (`efe-replay-coordinator`, `efe-chain-shipment`,
//! `efe-replay-auth`) into a single crate ; modular split can happen
//! later without breaking the public API.
//!
//! See `docs/architecture/EFE_M50_N16_MULTI_REGION_REPLAY_ARCHITECTURE.md`.
//!
//! # Module map
//!
//! | Module | Purpose |
//! |---|---|
//! | [`auth`] | Operator JWT validation (HS256 ; RS256 in Sprint 10). |
//! | [`storage`] | Persistent replay job + result + lease store (trait + in-memory impl). |
//! | [`residency`] | `ResidencyPolicyLookup` trait + in-memory impl. |
//! | [`kms`] | `KmsKeyResolver` trait + stub impl. |
//! | [`shipment`] | Cross-region chain shipment client + server (in-process for Sprint 9 ; gRPC follows in Sprint 10). |
//! | [`coordinator`] | `ReplayCoordinator` — the operator front door. |
//! | [`engine`] | `MultiRegionReplayEngine` — wraps Sprint 6 `ReplayEngine` with region context. |
//! | [`metrics`] | N16 Prometheus surface. |

#![deny(unsafe_code)]
#![warn(rust_2018_idioms)]

pub mod auth;
pub mod coordinator;
pub mod engine;
pub mod kms;
pub mod metrics;
pub mod residency;
pub mod shipment;
pub mod storage;

pub use auth::{Hs256JwtValidator, JwtError, JwtValidator, OperatorClaims, ReplayScope};
pub use coordinator::{ReplayCoordinator, ReplayCoordinatorBuilder};
pub use engine::{MultiRegionReplayEngine, ReplayCallContext};
pub use kms::{InMemoryKmsKeyResolver, KmsError, KmsKeyResolver};
pub use metrics::N16Metrics;
pub use residency::{InMemoryResidencyPolicyLookup, ResidencyError, ResidencyPolicyLookup};
pub use shipment::{
    ChainShipmentClient, ChainShipmentError, InMemoryChainShipmentClient, ShippedChain,
};
pub use storage::{
    InMemoryReplayStorage, ReplayJob, ReplayLease, ReplayStorage, StorageError,
};

/// **N16** : Crate semver.
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// **N16** : Maximum SDK schema version this build accepts on cross-
/// region chain shipment (architecture G10 + F16).
pub const MAX_SUPPORTED_SCHEMA_VERSION: u32 = efe_nano_sdk::REPLAY_SCHEMA_VERSION;
