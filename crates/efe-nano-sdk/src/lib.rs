// SPDX-License-Identifier: MIT
//! # `efe-nano-sdk` — M50 Nano Agent Swarm SDK
//!
//! Public SDK for authoring + invoking EFE nano agents.  This crate
//! is **customer-facing** and MIT-licensed.  It exposes :
//!
//! - **Shared types** : [`types::NanoAgent`], [`types::SwarmRequest`],
//!   [`types::SwarmResponse`], [`types::Operation`], error types.
//! - **Traits** : [`traits::Catalog`], [`traits::Runtime`] —
//!   the public contracts implemented by the runtime.
//! - **Errors** : [`error::OperationError`], [`error::NanoAgentIdError`].
//!
//! The runtime (catalog impl, dispatcher impl, audit chain, billing)
//! lives in the private [`efe-nano`] crate.  Customers consume only
//! this SDK.
//!
//! ## Status
//!
//! Phase 1 (Sprint 1-3) — Foundation :
//! - All public types implemented (this crate).
//! - In-memory catalog + Postgres catalog in `efe-nano`.
//! - Mode A runtime (pure Rust thread pool).
//! - Dispatcher v0 (DAG planner + parallel execution).
//! - 10 starter nano agents (all Mode A).
//!
//! ## See also
//!
//! - [Architecture spec](../../docs/architecture/EFE_M50_AGENTIC_ARCHITECTURE.md)
//! - [Phase 1 architecture](../../docs/architecture/EFE_M50_PHASE_1_ARCHITECTURE.md)
//! - [Roadmap](../../docs/roadmap/EFE_M50_ROADMAP.md)
//!
//! ## License
//!
//! MIT.

#![doc(html_root_url = "https://docs.rs/efe-nano-sdk/0.0.1")]
#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(rust_2018_idioms)]
#![allow(missing_docs)]  // TODO Sprint 1 Day 10 : enable strict missing_docs

pub mod error;
pub mod traits;
pub mod types;

// ─────────────────────────────────────────────────────────────────────
//  Re-exports — convenience aliases at crate root
// ─────────────────────────────────────────────────────────────────────

pub use error::{
    NanoAgentIdError, NanoAgentVersionError, OperationError, OperationErrorKind, VersionReqError,
};

pub use traits::{
    Catalog, CatalogResult, ListFilter, NanoAgentResponse, PoolHealth, ResolvedAgent, Runtime,
    RuntimeHealth, RuntimeResult, TraceId,
};

pub use types::{
    AbortReason, Author, AuditPolicy, BackpressureError, BackpressureKind, Blake3Hash,
    CacheProvenance, CandidateError, Category, CoPublishingError, CoPublishingPolicy,
    ComparisonStrategy,
    Completeness, ComplianceTags, CostAccounting, CostAttribution, DeterminismLevel, DriftStats,
    BypassCertificate, EnvelopeKind, ExecutionMode, FinishReason, GateOutcome,
    GoldAssertionStrategy, GoldCase, GoldCaseEntry, GoldCaseId, GoldCaseRun, GoldCaseVerdict,
    GoldErrorKind, GoldExpectedOutput, GoldFieldDiff, GoldGateError, GoldOverallVerdict,
    GoldReport, GoldRunId, GoldSet, GoldSetError, GoldSetId, GoldSetRef, GoldSetStatus,
    HistoricalKey, IdempotencyPolicy, Iso3166, IsoError, JsonSchema, KeyId, KycTier,
    LlmProvenanceRecord, MatchOutcome,
    MatchPolicy, McpToolMetadata, ModelIdentity, NanoAgent, NanoAgentId, NanoAgentRef,
    NanoAgentVersion, NewPublisherCandidate, OpStatus, OpStatusCounts, Operation,
    OperationInput, OperationResult, OperatorSignature, PiiStrategy, Provenance, PromotionGates,
    PromptInjectionStrategy, ProvenanceError, ProvenanceId, PublisherError, PublisherId,
    PublisherIdentity, PublisherKey, PublisherSignature, PublisherStatus, QuotaPolicy,
    RegionCode, ReplayAggregate, ReplayError, ReplayJobId, ReplayJobStatus, ReplayMode,
    ReplayOpDiff, ReplayOverallOutcome, ReplayRequest, ReplayResult, ReplayToleranceMap,
    ResidencyPolicy, ResidencyViolation, ResourceLimits, RetryOutcome, RetryPolicy, RevocationCertificate,
    RevocationReason, RolloutPolicy, SafetyCategory, SafetyFlag, SamplingParams, Sha256Hash,
    ShadowOutcome, SignatureAlgorithm, StabilityTier, SwarmEnvelopePayload, SwarmRequest,
    SwarmResponse, SwarmStatus, TenantId, TenantTier, TokenUsage, ToolCallRecord,
    ToolCallResultKind, VersionReq, BYPASS_MAX_LIFETIME_DAYS, BYPASS_RATIONALE_MIN_CHARS,
    DEFAULT_REPLAY_TIMEOUT_MS, GOLD_CASE_DEFAULT_TIMEOUT_SECONDS,
    GOLD_CASE_MAX_TIMEOUT_SECONDS, GOLD_SET_DEFAULT_COST_CAP_USD, GOLD_SET_MIN_APPROVERS,
    GOLD_SET_MIN_CASES, GOLD_SET_TIER_2_COST_CAP_MAX_USD, GOLD_SET_TIER_3_COST_CAP_MAX_USD,
    MAX_REPLAY_TIMEOUT_MS, REPLAY_SCHEMA_VERSION,
    // N21 SDK types
    AgentId, AgentIdError, Classification, ContextBundle, ContextBundleMetadata, Entity,
    Redaction, SwarmId, MAX_CONTEXT_BUNDLE_BYTES,
};

// ─────────────────────────────────────────────────────────────────────
//  Crate constants
// ─────────────────────────────────────────────────────────────────────

/// SDK semver.
pub const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");

/// EFE module identifier this SDK belongs to.
pub const MODULE_ID: &str = "M50";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdk_version_present() {
        assert!(SDK_VERSION.starts_with("0.0."));
    }

    #[test]
    fn module_id_constant() {
        assert_eq!(MODULE_ID, "M50");
    }

    #[test]
    fn re_exports_compile() {
        // Compile-time : ensure the re-export hierarchy is correct.
        let _: NanoAgentId = NanoAgentId::new("test_id").unwrap();
        let _: NanoAgentVersion = NanoAgentVersion::new("1.0.0").unwrap();
        let _: TenantId = TenantId::new();
        let _: VersionReq = VersionReq::any();
        let _: ResourceLimits = ResourceLimits::default_mode_a();
        let _: AuditPolicy = AuditPolicy::default();
        let _: ComplianceTags = ComplianceTags::default();
        let _: RetryPolicy = RetryPolicy::default();
        let _: ListFilter = ListFilter::include_all();
        let _: TraceId = TraceId::new();
    }
}
