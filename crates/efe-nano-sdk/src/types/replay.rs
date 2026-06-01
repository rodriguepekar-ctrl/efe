//! **Sprint 9 — N16** : Operator-facing replay types.
//!
//! See `docs/architecture/EFE_M50_N16_MULTI_REGION_REPLAY_ARCHITECTURE.md`
//! (§ 8.2).
//!
//! # Module surface
//!
//! - [`ReplayRequest`] — operator submits a replay job.
//! - [`ReplayJobId`] — opaque identifier for a queued / running job.
//! - [`ReplayJobStatus`] — state machine of a job.
//! - [`ReplayResult`] — final per-swarm result with per-op diffs.
//! - [`ReplayOpDiff`] — per-op comparison result.
//! - [`MatchOutcome`] — comparison outcome enum.
//! - [`ReplayAggregate`] — aggregate counts.
//! - [`ReplayOverallOutcome`] — Clean / Regression / Failed.
//! - [`ReplayMode`] — Full vs DispatchOnly.
//! - [`ReplayToleranceMap`] / [`MatchPolicy`] — per-mode tolerance.
//! - [`ReplayError`] — typed error taxonomy.

use std::collections::HashMap;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::types::region::{RegionCode, ResidencyViolation};
use crate::types::{NanoAgentId, NanoAgentVersion, OpStatus, TenantId};

/// **Sprint 9 — N16** : SDK schema version embedded in cross-region
/// shipment headers.  Target regions reject payloads with a higher
/// schema (architecture G10, I19, F16).  Bump on every wire-incompatible
/// change to `ReplayRequest` / `ReplayResult` / shipment payloads.
pub const REPLAY_SCHEMA_VERSION: u32 = 1;

/// **Sprint 9 — N16** : Maximum per-swarm replay timeout (60 minutes).
/// `ReplayRequest::timeout_ms` MUST be `≤ MAX_REPLAY_TIMEOUT_MS`
/// (architecture I16).
pub const MAX_REPLAY_TIMEOUT_MS: u32 = 60 * 60 * 1_000;

/// **Sprint 9 — N16** : Default per-swarm replay timeout (10 minutes).
pub const DEFAULT_REPLAY_TIMEOUT_MS: u32 = 10 * 60 * 1_000;

// ─────────────────────────────────────────────────────────────────────
//  ReplayMode
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : how a replay re-executes Mode C ops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayMode {
    /// **N16** : Real re-dispatch.  Mode C agents make real LLM
    /// provider calls, billed to the operator's account
    /// (`CostAttribution::Operator`).
    Full,

    /// **N16** : Stub LLM returns recorded outputs for Mode C ops.
    /// Cheaper ; validates dispatch logic only, not LLM determinism.
    /// This is the default.
    DispatchOnly,
}

impl Default for ReplayMode {
    fn default() -> Self {
        Self::DispatchOnly
    }
}

impl ReplayMode {
    /// Stable string label for metric dimensions.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::DispatchOnly => "dispatch_only",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  MatchPolicy + ReplayToleranceMap
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : per-mode comparison policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "config")]
pub enum MatchPolicy {
    /// **N16** : Byte-for-byte equality.  Default for Mode A.
    Exact,
    /// **N16** : Structural JSON equality with floating-point
    /// epsilon.  Default for Mode B.
    Structural { float_epsilon: f64 },
    /// **N16** : N13 semantic comparison with a minimum similarity
    /// score.  Default for Mode C.
    Semantic { min_score: f64 },
}

impl MatchPolicy {
    /// **N16** : Default Mode A policy : bit-exact.
    pub fn default_mode_a() -> Self {
        Self::Exact
    }

    /// **N16** : Default Mode B policy : 1e-5 float tolerance.
    pub fn default_mode_b() -> Self {
        Self::Structural { float_epsilon: 1e-5 }
    }

    /// **N16** : Default Mode C policy : 95% semantic.
    pub fn default_mode_c() -> Self {
        Self::Semantic { min_score: 0.95 }
    }

    /// Static label for metric dimensions.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Structural { .. } => "structural",
            Self::Semantic { .. } => "semantic",
        }
    }
}

/// **Sprint 9 — N16** : per-mode tolerance configuration.  Operators
/// may override per-request to relax or tighten the comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayToleranceMap {
    /// Tolerance applied to Mode A (`PureRust`) ops.
    pub mode_a: MatchPolicy,
    /// Tolerance applied to Mode B (`LocalModel`) ops.
    pub mode_b: MatchPolicy,
    /// Tolerance applied to Mode C (`LlmAugmented`) ops.
    pub mode_c: MatchPolicy,
}

impl Default for ReplayToleranceMap {
    fn default() -> Self {
        Self {
            mode_a: MatchPolicy::default_mode_a(),
            mode_b: MatchPolicy::default_mode_b(),
            mode_c: MatchPolicy::default_mode_c(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  ReplayJobId
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : opaque identifier for a queued / running job.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct ReplayJobId(pub Uuid);

impl ReplayJobId {
    /// **N16** : Generate a fresh random job ID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// **N16** : Construct from an existing UUID.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// **N16** : Extract the inner UUID.
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for ReplayJobId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ReplayJobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  ReplayRequest
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : a replay request submitted to the
/// `ReplayCoordinator`.
///
/// `actor`, `ticket_id`, and `reason` are mandatory (G14, I15.2) ;
/// coordinator rejects empty values with
/// [`ReplayError::AttributionInvalid`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayRequest {
    /// Tenant whose swarm is being replayed.
    pub tenant_id: TenantId,
    /// Original swarm to re-execute.
    pub swarm_request_id: Uuid,
    /// Region where the swarm originally ran (source of the chain).
    pub origin_region: RegionCode,
    /// Region where the replay should execute.  Validated against
    /// `ResidencyPolicy`.
    pub target_region: RegionCode,
    /// **Required** (G14) : operator username / email.
    pub actor: String,
    /// **Required** (G14) : forensic ticket reference (e.g. JIRA-12345).
    pub ticket_id: String,
    /// **Required** (G14) : human-readable reason for the replay.
    pub reason: String,
    /// Per-swarm timeout.  Capped at [`MAX_REPLAY_TIMEOUT_MS`].
    pub timeout_ms: u32,
    /// Replay mode (default : `DispatchOnly`).
    #[serde(default)]
    pub mode: ReplayMode,
    /// Optional per-mode tolerance override.  `None` → catalog default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerance_override: Option<ReplayToleranceMap>,
}

impl ReplayRequest {
    /// **N16** : Validate the attribution + timeout fields.  Used
    /// by the coordinator at submit time.
    pub fn validate(&self) -> Result<(), ReplayError> {
        if self.actor.trim().is_empty()
            || self.ticket_id.trim().is_empty()
            || self.reason.trim().is_empty()
        {
            return Err(ReplayError::AttributionInvalid);
        }
        if self.timeout_ms == 0 || self.timeout_ms > MAX_REPLAY_TIMEOUT_MS {
            return Err(ReplayError::InvalidTimeout {
                requested_ms: self.timeout_ms,
                max_ms: MAX_REPLAY_TIMEOUT_MS,
            });
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────
//  ReplayJobStatus
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : state machine of a replay job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum ReplayJobStatus {
    /// **N16** : enqueued but not yet picked up by a worker.
    Pending,

    /// **N16** : worker is fetching the chain and / or re-dispatching.
    InProgress {
        /// Approximate progress in percent (0-100).
        progress_pct: u8,
    },

    /// **N16** : finished successfully, all ops within tolerance.
    Completed { result_id: Uuid },

    /// **N16** : finished, but at least one op mismatched.
    CompletedWithRegression {
        result_id: Uuid,
        mismatch_count: u32,
    },

    /// **N16** : terminal failure (chain integrity, residency,
    /// timeout, etc.).
    Failed {
        #[serde(rename = "error")]
        error_label: String,
    },

    /// **N16** : operator-requested cancel.
    Cancelled,
}

impl ReplayJobStatus {
    /// True if this is a terminal state (no further transitions).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. }
                | Self::CompletedWithRegression { .. }
                | Self::Failed { .. }
                | Self::Cancelled
        )
    }

    /// Stable label for metrics.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress { .. } => "in_progress",
            Self::Completed { .. } => "completed",
            Self::CompletedWithRegression { .. } => "completed_with_regression",
            Self::Failed { .. } => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  MatchOutcome + ReplayOpDiff
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : per-op comparison outcome.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome", content = "score")]
pub enum MatchOutcome {
    /// **N16** : Output bit-exact or semantic-exact match.
    Exact,
    /// **N16** : Within configured tolerance ; score in `[0, 1]`.
    Tolerated(f64),
    /// **N16** : Beyond tolerance — possible regression.
    Mismatch,
    /// **N16** : Op skipped in either original or replay (no
    /// comparison performed).
    Skipped,
}

impl MatchOutcome {
    /// True if this outcome counts as a successful match for
    /// aggregate purposes.
    pub fn is_success(self) -> bool {
        matches!(self, Self::Exact | Self::Tolerated(_))
    }

    /// True if this outcome flags a potential regression.
    pub fn is_regression(self) -> bool {
        matches!(self, Self::Mismatch)
    }

    /// Stable label for metrics.
    pub fn label(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Tolerated(_) => "tolerated",
            Self::Mismatch => "mismatch",
            Self::Skipped => "skipped",
        }
    }
}

/// **Sprint 9 — N16** : per-op diff result, stored in
/// `replay.replay_diffs`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayOpDiff {
    /// Op id as in the original `SwarmRequest`.
    pub op_id: String,
    /// Execution mode string label (`pure_rust`, `local_model`,
    /// `llm_augmented`).  We store the string label instead of the
    /// full `ExecutionMode` enum so the diff row is mode-config
    /// independent.
    pub mode_label: String,
    /// Comparison outcome.
    pub match_outcome: MatchOutcome,
    /// Original op status.
    pub original_status: OpStatus,
    /// Replay op status.
    pub replay_status: OpStatus,
    /// Optional structural diff payload (for `Mismatch` outcomes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structural_diff: Option<serde_json::Value>,
    /// Optional semantic similarity score (Mode C).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_score: Option<f64>,
    /// Original op latency (ms).
    pub latency_ms_original: u32,
    /// Replay op latency (ms) — informational only ; not part of
    /// comparison.
    pub latency_ms_replay: u32,
    /// Original op cost (cents).
    pub cost_cents_original: f64,
    /// Replay op cost (cents) — attributed to operator, not customer.
    pub cost_cents_replay: f64,
}

// ─────────────────────────────────────────────────────────────────────
//  ReplayAggregate
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : aggregate counts across all ops in a swarm.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct ReplayAggregate {
    pub total_ops: u32,
    pub exact: u32,
    pub tolerated: u32,
    pub mismatch: u32,
    pub skipped: u32,
    pub overall_outcome: ReplayOverallOutcome,
}

impl ReplayAggregate {
    /// **N16** : Accumulate one op result.
    pub fn record(&mut self, outcome: MatchOutcome) {
        self.total_ops = self.total_ops.saturating_add(1);
        match outcome {
            MatchOutcome::Exact => self.exact = self.exact.saturating_add(1),
            MatchOutcome::Tolerated(_) => self.tolerated = self.tolerated.saturating_add(1),
            MatchOutcome::Mismatch => self.mismatch = self.mismatch.saturating_add(1),
            MatchOutcome::Skipped => self.skipped = self.skipped.saturating_add(1),
        }
        self.overall_outcome = self.compute_outcome();
    }

    /// **N16** : Re-compute the overall outcome from current counts.
    pub fn compute_outcome(&self) -> ReplayOverallOutcome {
        if self.mismatch > 0 {
            ReplayOverallOutcome::Regression
        } else {
            ReplayOverallOutcome::Clean
        }
    }
}

/// **Sprint 9 — N16** : top-level outcome of a replay job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReplayOverallOutcome {
    /// Every op matched within tolerance.
    #[default]
    Clean,
    /// One or more ops mismatched.
    Regression,
    /// Engine errored before completion.
    Failed,
}

impl ReplayOverallOutcome {
    /// Stable label for metrics.
    pub fn label(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Regression => "regression",
            Self::Failed => "failed",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  ReplayResult
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : final result of a replay job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayResult {
    pub job_id: ReplayJobId,
    pub tenant_id: TenantId,
    pub origin_region: RegionCode,
    pub target_region: RegionCode,
    pub swarm_request_id: Uuid,
    pub envelope_trace_id: Uuid,
    pub mode: ReplayMode,
    /// Per-op diffs, keyed by `op_id`.
    pub per_op: HashMap<String, ReplayOpDiff>,
    pub aggregate: ReplayAggregate,
    /// Pinned agent versions used during replay.
    pub agent_versions: HashMap<NanoAgentId, NanoAgentVersion>,
    /// Operator attribution (no customer billing for replay cost).
    pub operator_id: Uuid,
    pub ticket_id: String,
    pub replay_started_at: SystemTime,
    pub replay_completed_at: SystemTime,
    /// Total replay cost in cents, attributed to operator.
    pub total_cost_cents: f64,
}

// ─────────────────────────────────────────────────────────────────────
//  ReplayError
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : typed error taxonomy.
///
/// Maps to HTTP status codes :
/// - 400 : [`AttributionInvalid`], [`InvalidTimeout`]
/// - 401 : [`Unauthorized`]
/// - 403 : [`ResidencyViolation`], [`CrossTenantForbidden`]
/// - 404 : [`AgentVersionMissing`], [`JobNotFound`], [`SwarmNotFound`]
/// - 409 : [`ConcurrentReplayInFlight`], [`NestedReplayForbidden`]
/// - 422 : [`ChainBroken`], [`KmsKeyNotAvailable`]
/// - 425 : (early — status only)
/// - 426 : [`SchemaTooNew`]
/// - 503 : [`SourceRegionUnavailable`], [`TargetRegionBusy`]
/// - 500 : [`Internal`]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReplayError {
    /// **F5** : Data-residency policy forbids this target region.
    #[error("residency violation : {violation}")]
    ResidencyViolation { violation: ResidencyViolation },

    /// **F4** : Agent version missing in target region's catalog.
    #[error("agent {agent_id} v{version} not in target region {region}")]
    AgentVersionMissing {
        agent_id: NanoAgentId,
        version: NanoAgentVersion,
        region: RegionCode,
    },

    /// **F6** : One or more KMS keys not accessible in target region.
    #[error("KMS keys not accessible in {target_region} : {arns:?}")]
    KmsKeyNotAvailable {
        arns: Vec<String>,
        target_region: RegionCode,
    },

    /// **F12** : Source swarm is already a replay output.
    #[error("nested replays are forbidden")]
    NestedReplayForbidden,

    /// **F11** : Another replay of the same swarm is already in flight.
    #[error("concurrent replay already in flight : job {existing_job_id}")]
    ConcurrentReplayInFlight { existing_job_id: ReplayJobId },

    /// **F1** : Source region unavailable.
    #[error("source region {region} unavailable")]
    SourceRegionUnavailable { region: RegionCode },

    /// **F2** : Target region admission gate saturated.
    #[error("target region {region} busy ; retry later")]
    TargetRegionBusy { region: RegionCode },

    /// **F3** : Audit chain HMAC verification failed.
    #[error("chain integrity broken at trace {trace_id}")]
    ChainBroken { trace_id: Uuid },

    /// **F16** : SDK schema version too new for target.
    #[error("schema too new : source {source_version}, target max {target_max}")]
    SchemaTooNew {
        source_version: u32,
        target_max: u32,
    },

    /// **G14** : `actor`, `ticket_id`, or `reason` empty.
    #[error("operator attribution missing or invalid")]
    AttributionInvalid,

    /// **I16** : `timeout_ms` exceeds the hard ceiling.
    #[error(
        "invalid timeout : requested {requested_ms} ms, max {max_ms} ms"
    )]
    InvalidTimeout { requested_ms: u32, max_ms: u32 },

    /// **F10** : JWT validation failed.
    #[error("unauthorized : {reason}")]
    Unauthorized { reason: String },

    /// **F14** : Operator's JWT tenant_id != request.tenant_id.
    #[error("cross-tenant access forbidden")]
    CrossTenantForbidden,

    /// **F15** : Per-swarm replay timeout exceeded.
    #[error("replay timeout after {elapsed_ms} ms")]
    Timeout { elapsed_ms: u32 },

    /// Job ID not found in storage.
    #[error("replay job {job_id} not found")]
    JobNotFound { job_id: ReplayJobId },

    /// Source swarm envelope not found.
    #[error("swarm {swarm_request_id} envelope not found in chain")]
    SwarmNotFound { swarm_request_id: Uuid },

    /// **F9 + F8** : storage / queue layer fault.
    #[error("storage error : {message}")]
    Storage { message: String },

    /// Catch-all internal error.
    #[error("internal : {message}")]
    Internal { message: String },
}

impl ReplayError {
    /// **N16** : HTTP status code mapping.
    pub fn http_status(&self) -> u16 {
        match self {
            Self::AttributionInvalid | Self::InvalidTimeout { .. } => 400,
            Self::Unauthorized { .. } => 401,
            Self::ResidencyViolation { .. } | Self::CrossTenantForbidden => 403,
            Self::AgentVersionMissing { .. }
            | Self::JobNotFound { .. }
            | Self::SwarmNotFound { .. } => 404,
            Self::ConcurrentReplayInFlight { .. } | Self::NestedReplayForbidden => 409,
            Self::ChainBroken { .. } | Self::KmsKeyNotAvailable { .. } => 422,
            Self::SchemaTooNew { .. } => 426,
            Self::SourceRegionUnavailable { .. } | Self::TargetRegionBusy { .. } => 503,
            Self::Timeout { .. } | Self::Storage { .. } | Self::Internal { .. } => 500,
        }
    }

    /// **N16** : Stable label for metric dimensions.
    pub fn label(&self) -> &'static str {
        match self {
            Self::ResidencyViolation { .. } => "residency_violation",
            Self::AgentVersionMissing { .. } => "agent_version_missing",
            Self::KmsKeyNotAvailable { .. } => "kms_key_not_available",
            Self::NestedReplayForbidden => "nested_replay_forbidden",
            Self::ConcurrentReplayInFlight { .. } => "concurrent_replay_in_flight",
            Self::SourceRegionUnavailable { .. } => "source_region_unavailable",
            Self::TargetRegionBusy { .. } => "target_region_busy",
            Self::ChainBroken { .. } => "chain_broken",
            Self::SchemaTooNew { .. } => "schema_too_new",
            Self::AttributionInvalid => "attribution_invalid",
            Self::InvalidTimeout { .. } => "invalid_timeout",
            Self::Unauthorized { .. } => "unauthorized",
            Self::CrossTenantForbidden => "cross_tenant_forbidden",
            Self::Timeout { .. } => "timeout",
            Self::JobNotFound { .. } => "job_not_found",
            Self::SwarmNotFound { .. } => "swarm_not_found",
            Self::Storage { .. } => "storage",
            Self::Internal { .. } => "internal",
        }
    }
}

impl From<ResidencyViolation> for ReplayError {
    fn from(violation: ResidencyViolation) -> Self {
        Self::ResidencyViolation { violation }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::region::ResidencyPolicy;

    fn sample_request() -> ReplayRequest {
        ReplayRequest {
            tenant_id: TenantId::new(),
            swarm_request_id: Uuid::new_v4(),
            origin_region: RegionCode::EuWest1,
            target_region: RegionCode::EuWest1,
            actor: "alice@efe.com".into(),
            ticket_id: "JIRA-12345".into(),
            reason: "GDPR audit Q1 2026".into(),
            timeout_ms: DEFAULT_REPLAY_TIMEOUT_MS,
            mode: ReplayMode::DispatchOnly,
            tolerance_override: None,
        }
    }

    #[test]
    fn replay_mode_default_is_dispatch_only() {
        assert_eq!(ReplayMode::default(), ReplayMode::DispatchOnly);
    }

    #[test]
    fn replay_mode_as_str_stable() {
        assert_eq!(ReplayMode::Full.as_str(), "full");
        assert_eq!(ReplayMode::DispatchOnly.as_str(), "dispatch_only");
    }

    #[test]
    fn match_policy_defaults_are_mode_appropriate() {
        assert!(matches!(MatchPolicy::default_mode_a(), MatchPolicy::Exact));
        assert!(matches!(
            MatchPolicy::default_mode_b(),
            MatchPolicy::Structural { .. }
        ));
        assert!(matches!(
            MatchPolicy::default_mode_c(),
            MatchPolicy::Semantic { .. }
        ));
    }

    #[test]
    fn match_policy_label_stable() {
        assert_eq!(MatchPolicy::Exact.label(), "exact");
        assert_eq!(
            MatchPolicy::Structural { float_epsilon: 1e-5 }.label(),
            "structural"
        );
        assert_eq!(MatchPolicy::Semantic { min_score: 0.95 }.label(), "semantic");
    }

    #[test]
    fn replay_tolerance_map_default_matches_per_mode_defaults() {
        let map = ReplayToleranceMap::default();
        assert!(matches!(map.mode_a, MatchPolicy::Exact));
        assert!(matches!(map.mode_b, MatchPolicy::Structural { .. }));
        assert!(matches!(map.mode_c, MatchPolicy::Semantic { .. }));
    }

    #[test]
    fn replay_job_id_generation_is_unique() {
        let a = ReplayJobId::new();
        let b = ReplayJobId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn replay_job_id_round_trip() {
        let uuid = Uuid::new_v4();
        let id = ReplayJobId::from_uuid(uuid);
        assert_eq!(id.as_uuid(), uuid);
    }

    #[test]
    fn replay_request_validate_accepts_valid_request() {
        let req = sample_request();
        assert!(req.validate().is_ok());
    }

    #[test]
    fn replay_request_validate_rejects_empty_actor() {
        let mut req = sample_request();
        req.actor = "  ".into();
        assert!(matches!(req.validate(), Err(ReplayError::AttributionInvalid)));
    }

    #[test]
    fn replay_request_validate_rejects_empty_ticket() {
        let mut req = sample_request();
        req.ticket_id = "".into();
        assert!(matches!(req.validate(), Err(ReplayError::AttributionInvalid)));
    }

    #[test]
    fn replay_request_validate_rejects_empty_reason() {
        let mut req = sample_request();
        req.reason = "".into();
        assert!(matches!(req.validate(), Err(ReplayError::AttributionInvalid)));
    }

    #[test]
    fn replay_request_validate_rejects_zero_timeout() {
        let mut req = sample_request();
        req.timeout_ms = 0;
        assert!(matches!(req.validate(), Err(ReplayError::InvalidTimeout { .. })));
    }

    #[test]
    fn replay_request_validate_rejects_oversized_timeout() {
        let mut req = sample_request();
        req.timeout_ms = MAX_REPLAY_TIMEOUT_MS + 1;
        assert!(matches!(req.validate(), Err(ReplayError::InvalidTimeout { .. })));
    }

    #[test]
    fn replay_request_round_trip_serde() {
        let req = sample_request();
        let json = serde_json::to_string(&req).unwrap();
        let req2: ReplayRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, req2);
    }

    #[test]
    fn replay_job_status_is_terminal_logic() {
        assert!(!ReplayJobStatus::Pending.is_terminal());
        assert!(!ReplayJobStatus::InProgress { progress_pct: 30 }.is_terminal());
        assert!(ReplayJobStatus::Completed { result_id: Uuid::new_v4() }.is_terminal());
        assert!(ReplayJobStatus::Failed { error_label: "x".into() }.is_terminal());
        assert!(ReplayJobStatus::Cancelled.is_terminal());
    }

    #[test]
    fn match_outcome_is_success_classification() {
        assert!(MatchOutcome::Exact.is_success());
        assert!(MatchOutcome::Tolerated(0.97).is_success());
        assert!(!MatchOutcome::Mismatch.is_success());
        assert!(!MatchOutcome::Skipped.is_success());
    }

    #[test]
    fn match_outcome_regression_only_for_mismatch() {
        assert!(!MatchOutcome::Exact.is_regression());
        assert!(!MatchOutcome::Tolerated(0.5).is_regression());
        assert!(MatchOutcome::Mismatch.is_regression());
        assert!(!MatchOutcome::Skipped.is_regression());
    }

    #[test]
    fn replay_aggregate_record_updates_counts_and_outcome() {
        let mut agg = ReplayAggregate::default();
        agg.record(MatchOutcome::Exact);
        agg.record(MatchOutcome::Exact);
        agg.record(MatchOutcome::Tolerated(0.97));
        assert_eq!(agg.total_ops, 3);
        assert_eq!(agg.exact, 2);
        assert_eq!(agg.tolerated, 1);
        assert_eq!(agg.mismatch, 0);
        assert_eq!(agg.overall_outcome, ReplayOverallOutcome::Clean);

        agg.record(MatchOutcome::Mismatch);
        assert_eq!(agg.mismatch, 1);
        assert_eq!(agg.overall_outcome, ReplayOverallOutcome::Regression);
    }

    #[test]
    fn replay_aggregate_skipped_does_not_trigger_regression() {
        let mut agg = ReplayAggregate::default();
        agg.record(MatchOutcome::Skipped);
        agg.record(MatchOutcome::Exact);
        assert_eq!(agg.skipped, 1);
        assert_eq!(agg.overall_outcome, ReplayOverallOutcome::Clean);
    }

    #[test]
    fn replay_overall_outcome_label_stable() {
        assert_eq!(ReplayOverallOutcome::Clean.label(), "clean");
        assert_eq!(ReplayOverallOutcome::Regression.label(), "regression");
        assert_eq!(ReplayOverallOutcome::Failed.label(), "failed");
    }

    #[test]
    fn replay_error_http_status_correct_mapping() {
        let cases: &[(ReplayError, u16)] = &[
            (ReplayError::AttributionInvalid, 400),
            (
                ReplayError::InvalidTimeout {
                    requested_ms: 99,
                    max_ms: 100,
                },
                400,
            ),
            (
                ReplayError::Unauthorized {
                    reason: "expired".into(),
                },
                401,
            ),
            (ReplayError::CrossTenantForbidden, 403),
            (
                ReplayError::ResidencyViolation {
                    violation: ResidencyViolation {
                        policy: ResidencyPolicy::EuOnly,
                        target_region: RegionCode::UsEast1,
                    },
                },
                403,
            ),
            (
                ReplayError::NestedReplayForbidden,
                409,
            ),
            (
                ReplayError::ConcurrentReplayInFlight {
                    existing_job_id: ReplayJobId::new(),
                },
                409,
            ),
            (
                ReplayError::ChainBroken {
                    trace_id: Uuid::new_v4(),
                },
                422,
            ),
            (
                ReplayError::SchemaTooNew {
                    source_version: 5,
                    target_max: 4,
                },
                426,
            ),
            (
                ReplayError::SourceRegionUnavailable {
                    region: RegionCode::UsEast1,
                },
                503,
            ),
            (
                ReplayError::TargetRegionBusy {
                    region: RegionCode::EuWest1,
                },
                503,
            ),
            (ReplayError::Timeout { elapsed_ms: 5_000 }, 500),
            (ReplayError::JobNotFound { job_id: ReplayJobId::new() }, 404),
            (ReplayError::SwarmNotFound { swarm_request_id: Uuid::new_v4() }, 404),
        ];
        for (err, expected_status) in cases {
            assert_eq!(
                err.http_status(),
                *expected_status,
                "status mapping wrong for {err:?}"
            );
        }
    }

    #[test]
    fn replay_error_label_stable_for_metrics() {
        // Every variant should have a non-empty static label.
        let cases = [
            ReplayError::AttributionInvalid,
            ReplayError::NestedReplayForbidden,
            ReplayError::CrossTenantForbidden,
            ReplayError::Timeout { elapsed_ms: 1_000 },
        ];
        for e in cases {
            assert!(!e.label().is_empty());
        }
    }

    #[test]
    fn replay_error_from_residency_violation() {
        let v = ResidencyViolation {
            policy: ResidencyPolicy::EuOnly,
            target_region: RegionCode::UsEast1,
        };
        let err: ReplayError = v.into();
        assert!(matches!(err, ReplayError::ResidencyViolation { .. }));
    }

    #[test]
    fn replay_error_round_trip_serde() {
        let cases = vec![
            ReplayError::AttributionInvalid,
            ReplayError::NestedReplayForbidden,
            ReplayError::Timeout { elapsed_ms: 3_000 },
            ReplayError::SchemaTooNew {
                source_version: 9,
                target_max: 5,
            },
        ];
        for err in cases {
            let json = serde_json::to_string(&err).unwrap();
            let err2: ReplayError = serde_json::from_str(&json).unwrap();
            assert_eq!(err.label(), err2.label());
            assert_eq!(err.http_status(), err2.http_status());
        }
    }

    #[test]
    fn schema_version_constant_is_positive() {
        const _: () = assert!(REPLAY_SCHEMA_VERSION >= 1);
    }
}
