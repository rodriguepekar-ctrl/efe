//! All public types for the M50 nano agent swarm SDK.
//!
//! Organized into submodules :
//! - [`identifier`] — [`NanoAgentId`]
//! - [`version`] — [`NanoAgentVersion`], [`VersionReq`]
//! - [`tenant`] — [`TenantId`]
//! - [`limits`] — [`ResourceLimits`]
//! - [`compliance`] — [`AuditPolicy`], [`ComplianceTags`]
//! - [`catalog`] — [`NanoAgent`] + supporting enums
//! - [`swarm`] — [`SwarmRequest`], [`Operation`], [`SwarmResponse`]

pub mod catalog;
pub mod compliance;
/// **Sprint 8 — N14** : Swarm envelope payload type.
pub mod envelope;
/// **Sprint 11 — N19** : Gold-test gate TIER-1 SDK types.
pub mod gold;
pub mod identifier;
pub mod limits;
/// **Sprint 9 — N17** : LLM provenance — forensic record of every
/// Mode C call.
pub mod llm_provenance;
/// **Sprint 10 — N18** : Publisher identity (Ed25519 signatures,
/// KYC ladder, revocation cascade).
pub mod publisher;
/// **Sprint 9 — N16** : Region taxonomy + data residency policy.
pub mod region;
/// **Sprint 9 — N16** : Operator-facing replay types.
pub mod replay;
pub mod rollout;
/// **Sprint 8 — N15** : Fairness scheduling types — [`TenantTier`],
/// [`QuotaPolicy`], [`BackpressureError`], [`BackpressureKind`].
pub mod scheduling;
pub mod swarm;
/// **Sprint 13 — N21** : Swarm orchestration + agentic bridge SDK types.
pub mod swarm_dag;
pub mod tenant;
pub mod version;

// Re-export the most commonly used types at the `types::` level for
// ergonomic imports : `use efe_nano_sdk::types::*`.

pub use catalog::{
    Author, Category, DeterminismLevel, ExecutionMode, IdempotencyPolicy, JsonSchema,
    McpToolMetadata, NanoAgent, NanoAgentRef, StabilityTier,
};
pub use compliance::{AuditPolicy, ComplianceTags, PiiStrategy, PromptInjectionStrategy};
pub use envelope::{EnvelopeKind, OpStatusCounts, SwarmEnvelopePayload};
pub use gold::{
    BypassCertificate, GoldAssertionStrategy, GoldCase, GoldCaseEntry, GoldCaseId,
    GoldCaseRun, GoldCaseVerdict, GoldErrorKind, GoldExpectedOutput, GoldFieldDiff,
    GoldGateError, GoldOverallVerdict, GoldReport, GoldRunId, GoldSet, GoldSetError,
    GoldSetId, GoldSetRef, GoldSetStatus, Provenance, BYPASS_MAX_LIFETIME_DAYS,
    BYPASS_RATIONALE_MIN_CHARS, GOLD_CASE_DEFAULT_TIMEOUT_SECONDS,
    GOLD_CASE_MAX_TIMEOUT_SECONDS, GOLD_SET_DEFAULT_COST_CAP_USD, GOLD_SET_MIN_APPROVERS,
    GOLD_SET_MIN_CASES, GOLD_SET_TIER_2_COST_CAP_MAX_USD, GOLD_SET_TIER_3_COST_CAP_MAX_USD,
};
pub use identifier::NanoAgentId;
pub use limits::ResourceLimits;
pub use llm_provenance::{
    AbortReason, Blake3Hash, CacheProvenance, Completeness, ComputeAttestation, CoSignatureScheme,
    CostAccounting, EndpointIdentity, FinishReason, LlmProvenanceRecord, ModelIdentity,
    NodeCoSignature, ProvenanceError, ProvenanceId, RetryOutcome, SafetyCategory, SafetyFlag,
    SamplingParams, TokenUsage, ToolCallRecord, ToolCallResultKind,
};
pub use publisher::{
    CandidateError, CoPublishingError, CoPublishingPolicy, HistoricalKey, Iso3166, IsoError,
    KeyId, KycTier, NewPublisherCandidate, OperatorSignature, PublisherError, PublisherId,
    PublisherIdentity, PublisherKey, PublisherSignature, PublisherStatus, RevocationCertificate,
    RevocationReason, Sha256Hash, SignatureAlgorithm,
};
pub use region::{RegionCode, ResidencyPolicy, ResidencyViolation};
pub use replay::{
    MatchOutcome, MatchPolicy, ReplayAggregate, ReplayError, ReplayJobId, ReplayJobStatus,
    ReplayMode, ReplayOpDiff, ReplayOverallOutcome, ReplayRequest, ReplayResult,
    ReplayToleranceMap, DEFAULT_REPLAY_TIMEOUT_MS, MAX_REPLAY_TIMEOUT_MS, REPLAY_SCHEMA_VERSION,
};
pub use rollout::{
    ComparisonStrategy, CostAttribution, DriftStats, GateOutcome, PromotionGates, RolloutPolicy,
    ShadowOutcome,
};
pub use scheduling::{BackpressureError, BackpressureKind, QuotaPolicy, TenantTier};
pub use swarm::{
    OpStatus, Operation, OperationInput, OperationResult, RetryPolicy, SwarmRequest,
    SwarmResponse, SwarmStatus,
};
pub use swarm_dag::{
    AgentId, AgentIdError, Classification, ContextBundle, ContextBundleMetadata, Entity,
    Redaction, SwarmId, MAX_CONTEXT_BUNDLE_BYTES,
};
pub use tenant::TenantId;
pub use version::{NanoAgentVersion, VersionReq};
