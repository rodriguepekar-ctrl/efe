//! Sprint **D12** — server-side catalog admin scaffolding.
//!
//! This module provides the storage abstraction for versioned
//! anchor catalogs (the `efe.v1.catalog.CatalogService` surface
//! defined in `protos/efe/v1/catalog.proto`).
//!
//! # Status (v0.1.12)
//!
//! **Scaffolding only.** The trait + in-memory impl + policy + audit
//! emission are present and unit-tested ; the gRPC handler that
//! wires `tonic::transport::Server` to this module lands in
//! **D12-bis** (next sprint). Until then, calling
//! `CatalogService::*` from a client returns
//! `Code::Unimplemented` because the registry is not bound to
//! `runtime::Server`.
//!
//! # Architecture
//!
//! See `docs/bench/d12_catalog_admin_versioning_architecture.md` for
//! the full design. Summary :
//!
//!   * `registry::CatalogRegistry` — async trait, the only abstraction.
//!   * `memory::InMemoryRegistry`  — `tokio::sync::RwLock`-backed
//!     impl ; sufficient for single-replica deployments. Multi-replica
//!     deployments (Tier 3 in the Helm chart) need a postgres-backed
//!     impl, deferred to D13.
//!   * `policy::CatalogPolicy`     — quotas, schema invariants,
//!     RBAC tenant assertions.
//!   * `audit::AuditEmitter`       — emits a structured tracing event
//!     per mutation (`tracing::info!(target = "audit", ...)`).
//!
//! # Concurrency model
//!
//! Register / evolve are serialized **per `(tenant, name)`** : two
//! concurrent register calls on the same name go through the same
//! `RwLock<HashMap<...>>` write critical section. Different
//! `(tenant, name)` tuples proceed in parallel.
//!
//! For an MVP at our load profile (~10 admin RPCs/sec across a
//! tenant fleet), the global write lock is fine. The registry trait
//! is shaped so a postgres impl can refine the lock granularity
//! without changing the call sites.

#![allow(dead_code)]
// Module is scaffolding ; gRPC wire-up lands in D12-bis.
// Scaffolding modules — every public surface is documented at the
// module level + at the trait level. Field-by-field docs are
// deferred to D12-bis when these types become the gRPC handler's
// canonical shape.
#![allow(missing_docs)]

pub mod audit;
/// Sprint **D14-bis** — HMAC-chained tamper-evident audit sink.
/// Active when `EFE_AUDIT_SIGNING_ENABLED=true`. Cf.
/// `docs/bench/d14bis_hmac_audit_chain_architecture.md`.
pub mod audit_hmac;
/// Sprint **D14-bis** — loader for the HMAC keys file.
pub mod audit_keys;
pub mod memory;
pub mod policy;
/// Sprint **D14** — Postgres-backed [`CatalogRegistry`] implementation.
/// Active when `EFE_CATALOG_BACKEND=postgres`. Cf.
/// `docs/bench/d14_postgres_catalog_backend_architecture.md`.
pub mod postgres;
pub mod registry;

pub use audit::{AuditAction, AuditEmitter, AuditEvent, AuditSink, TracingAuditSink};
pub use audit_hmac::{
    canonical_audit_payload, compute_hmac, hex_lower, periodic_self_verify, verify_chain,
    HmacChainedAuditSink, SignableRecord, VerifyError, VerifyReport, GENESIS_PREV_HMAC,
    SCHEMA_VERSION,
};
pub use audit_keys::{key_id_of, load_keys_file, HmacKey, LoadKeysError, LoadedKeys};
pub use memory::InMemoryRegistry;
pub use policy::{CatalogPolicy, PolicyError};
pub use postgres::PostgresCatalogRegistry;
pub use registry::{
    CatalogClass, CatalogContent, CatalogError, CatalogKey, CatalogMetadata, CatalogPage,
    CatalogRefSummary, CatalogRegistry, CompiledClass, ConePolicy, ContentDigest, EmbedderSpec,
    EvolveOp, RegisteredCatalog, ThresholdPolicy,
};
