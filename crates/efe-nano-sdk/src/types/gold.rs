// SPDX-License-Identifier: UNLICENSED
//! **Sprint 11 — N19** : Gold-test gate TIER-1 — SDK types.
//!
//! Architecture § 7.2.  All types are wire-frozen Sprint 11 ; Sprint
//! 12 Postgres backing won't change the surface.
//!
//! # Module surface
//!
//! - [`GoldSetId`] / [`GoldCaseId`] / [`GoldRunId`] — opaque ids.
//! - [`GoldCase`] — single curated test case.
//! - [`GoldExpectedOutput`] — inline value or N16 replay anchor.
//! - [`GoldAssertionStrategy`] — `StrictEquality` /
//!   `NumericTolerance` / `SemanticSimilarity` / `SchemaValidationOnly`.
//! - [`GoldSet`] — operator-approved bundle of cases.
//! - [`GoldSetStatus`] / [`Provenance`] — lifecycle metadata.
//! - [`GoldReport`] / [`GoldCaseEntry`] / [`GoldCaseRun`] /
//!   [`GoldCaseVerdict`] / [`GoldFieldDiff`] / [`GoldOverallVerdict`].
//! - [`BypassCertificate`] — 2-of-3 EFE eng emergency override.
//! - [`GoldSetError`] / [`GoldGateError`] — closed taxonomies.
//!
//! # Wire compatibility
//!
//! - All enum variants use `#[serde(tag = "kind")]` for stable wire
//!   shape.  Adding a new variant in a future sprint is backward-
//!   compatible (consumers see `kind: "unknown"` via a fallback
//!   pattern at the caller).
//! - Hashes use the SDK's [`Sha256Hash`] (hex string on wire).
//! - All `Option<_>` fields are `#[serde(default,
//!   skip_serializing_if = "Option::is_none")]` so pre-N19
//!   producers stay compatible.

use std::collections::BTreeMap;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::types::llm_provenance::ModelIdentity;
use crate::types::{NanoAgentId, NanoAgentVersion, OperatorSignature, Sha256Hash};

// ─────────────────────────────────────────────────────────────────────
//  GoldSetId / GoldCaseId / GoldRunId
// ─────────────────────────────────────────────────────────────────────

/// **N19** : opaque gold set identifier.  Immutable per § 7.2 + I1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GoldSetId(pub Uuid);

impl GoldSetId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    pub fn from_uuid(u: Uuid) -> Self {
        Self(u)
    }
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for GoldSetId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for GoldSetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// **N19** : opaque gold case identifier.  Stable join key (I2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GoldCaseId(pub Uuid);

impl GoldCaseId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    pub fn from_uuid(u: Uuid) -> Self {
        Self(u)
    }
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for GoldCaseId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for GoldCaseId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// **N19** : opaque gold run identifier.  Referenced by N18 publish
/// ledger + N6 audit chain + N17 provenance (I5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GoldRunId(pub Uuid);

impl GoldRunId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    pub fn from_uuid(u: Uuid) -> Self {
        Self(u)
    }
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for GoldRunId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for GoldRunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  GoldAssertionStrategy — § 7.2 + ADR-1
// ─────────────────────────────────────────────────────────────────────

/// **N19** : how a gold case verifies the captured output.
///
/// Three first-class strategies + one fixture-only fallback (ADR-1) :
///
/// - [`Self::StrictEquality`] — byte-by-byte deterministic equality.
///   Mode A.
/// - [`Self::NumericTolerance`] — per-JSON-pointer ε comparison.
///   Mode B.
/// - [`Self::SemanticSimilarity`] — cosine similarity via a pinned
///   embedding model.  Mode C.
/// - [`Self::SchemaValidationOnly`] — fixture / placeholder
///   strategy ; the gold set validator REJECTS this as the sole
///   strategy on a real GoldSet (F11 — see [`GoldSet::validate`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GoldAssertionStrategy {
    /// Byte-by-byte deterministic equality.  Mode A.
    StrictEquality,

    /// Per-JSON-pointer epsilon.  Pointers not listed must match
    /// exactly.  Empty `fields` map = same as `StrictEquality` (F11
    /// validator detects + rejects).
    NumericTolerance {
        fields: BTreeMap<String, f64>,
    },

    /// Cosine similarity via a pinned embedding model.  Mode C.
    ///
    /// `cosine_threshold` is in `[0.0, 1.0]` ; a `field_pointer` of
    /// `None` embeds the entire serialised output (whitespace-
    /// normalised).
    SemanticSimilarity {
        embedding_model: ModelIdentity,
        cosine_threshold: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        field_pointer: Option<String>,
    },

    /// Fixture / placeholder only.  Sprint 11 stub.  Rejected on
    /// `Approved` gold sets (every approved set must use one of the
    /// first three strategies).
    SchemaValidationOnly,
}

impl GoldAssertionStrategy {
    /// **N19** : Stable label for metrics + UI.
    pub fn label(&self) -> &'static str {
        match self {
            Self::StrictEquality => "strict_equality",
            Self::NumericTolerance { .. } => "numeric_tolerance",
            Self::SemanticSimilarity { .. } => "semantic_similarity",
            Self::SchemaValidationOnly => "schema_validation_only",
        }
    }

    /// **N19** : True if this strategy is acceptable on an approved
    /// gold set (i.e. not `SchemaValidationOnly`).
    pub fn is_production_grade(&self) -> bool {
        !matches!(self, Self::SchemaValidationOnly)
    }

    /// **N19** : Validate strategy-internal parameters.
    pub fn validate(&self) -> Result<(), GoldSetError> {
        match self {
            Self::StrictEquality => Ok(()),
            Self::NumericTolerance { fields } => {
                for (p, eps) in fields {
                    if !p.starts_with('/') && !p.is_empty() {
                        return Err(GoldSetError::InvalidAssertionParam {
                            reason: format!(
                                "numeric tolerance JSON pointer must start with '/' (got {p:?})"
                            ),
                        });
                    }
                    if !eps.is_finite() || *eps < 0.0 {
                        return Err(GoldSetError::InvalidAssertionParam {
                            reason: format!(
                                "numeric tolerance epsilon must be finite + ≥ 0.0 (got {eps})"
                            ),
                        });
                    }
                }
                Ok(())
            }
            Self::SemanticSimilarity {
                cosine_threshold,
                field_pointer,
                ..
            } => {
                if !cosine_threshold.is_finite()
                    || *cosine_threshold < 0.0
                    || *cosine_threshold > 1.0
                {
                    return Err(GoldSetError::InvalidAssertionParam {
                        reason: format!(
                            "cosine threshold must be in [0.0, 1.0] (got {cosine_threshold})"
                        ),
                    });
                }
                if let Some(p) = field_pointer {
                    if !p.starts_with('/') && !p.is_empty() {
                        return Err(GoldSetError::InvalidAssertionParam {
                            reason: format!(
                                "semantic similarity field pointer must start with '/' (got {p:?})"
                            ),
                        });
                    }
                }
                Ok(())
            }
            Self::SchemaValidationOnly => Ok(()),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  GoldExpectedOutput — ADR-9
// ─────────────────────────────────────────────────────────────────────

/// **N19** : how the expected output is sourced for a gold case.
///
/// - [`Self::Inline`] — author hand-wrote the expected value.
/// - [`Self::ReplayAnchor`] — reference an N16 trace ; the gold
///   runner re-plays via N16 `ReplayEngine` and compares against
///   the re-execution result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GoldExpectedOutput {
    Inline {
        value: serde_json::Value,
    },
    ReplayAnchor {
        trace_id: Uuid,
        span_id: Uuid,
    },
}

impl GoldExpectedOutput {
    /// **N19** : Stable label for metrics + UI.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Inline { .. } => "inline",
            Self::ReplayAnchor { .. } => "replay_anchor",
        }
    }

    /// **N19** : True if the runtime path must talk to N16.
    pub fn is_replay_anchored(&self) -> bool {
        matches!(self, Self::ReplayAnchor { .. })
    }
}

// ─────────────────────────────────────────────────────────────────────
//  GoldCase — § 7.2
// ─────────────────────────────────────────────────────────────────────

/// **N19** : a single golden test case.
///
/// `timeout_seconds` is capped at 300 (I14).  `tags` are free-form
/// strings used by the operator + drift-sweep selector to slice
/// subsets (e.g. `"pii"`, `"happy_path"`, `"regression_anchor:CUS-1234"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldCase {
    pub gold_case_id: GoldCaseId,
    pub name: String,
    pub input: serde_json::Value,
    pub expected: GoldExpectedOutput,
    pub assertion: GoldAssertionStrategy,
    pub timeout_seconds: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// **N19** : maximum `timeout_seconds` accepted on a `GoldCase`.
/// Per I14.
pub const GOLD_CASE_MAX_TIMEOUT_SECONDS: u32 = 300;

/// **N19** : default `timeout_seconds` if author leaves it at 0.
pub const GOLD_CASE_DEFAULT_TIMEOUT_SECONDS: u32 = 30;

impl GoldCase {
    /// **N19** : Validate the case.  Called transitively by
    /// `GoldSet::validate`.
    pub fn validate(&self) -> Result<(), GoldSetError> {
        if self.name.trim().is_empty() {
            return Err(GoldSetError::InvalidCase {
                reason: "case name must not be empty".into(),
            });
        }
        if self.timeout_seconds == 0 {
            return Err(GoldSetError::InvalidCase {
                reason: "timeout_seconds must be ≥ 1 (use default 30s if unsure)".into(),
            });
        }
        if self.timeout_seconds > GOLD_CASE_MAX_TIMEOUT_SECONDS {
            return Err(GoldSetError::InvalidCase {
                reason: format!(
                    "timeout_seconds {} exceeds maximum {}",
                    self.timeout_seconds, GOLD_CASE_MAX_TIMEOUT_SECONDS
                ),
            });
        }
        self.assertion.validate()?;
        Ok(())
    }

    /// **N19** : True if this case is tagged `"pii"` (used by the
    /// PII-coverage validator — I8).
    pub fn covers_pii(&self) -> bool {
        self.tags.iter().any(|t| t == "pii")
    }
}

// ─────────────────────────────────────────────────────────────────────
//  GoldSetStatus + Provenance
// ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoldSetStatus {
    /// Author submitted ; not yet approved.
    Draft,
    /// Approved by ≥ 2 EFE engineers ; usable as a gate.
    Approved,
    /// Superseded by a later set ; preserved for forensic queries.
    Archived,
}

impl GoldSetStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Approved => "approved",
            Self::Archived => "archived",
        }
    }
}

/// **N19** : where the gold cases came from.  Sprint 11 only ships
/// `Manual` ; Sprint 13 adds the other variants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Provenance {
    Manual,
    /// Sprint 13 : N13 shadow-capture proposed the cases.
    ShadowCapture { run_id: Uuid },
    /// Sprint 13 : property-based mutation derived from a parent
    /// set.
    Mutation { parent_set_id: GoldSetId },
}

impl Provenance {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::ShadowCapture { .. } => "shadow_capture",
            Self::Mutation { .. } => "mutation",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  GoldSet — § 7.2
// ─────────────────────────────────────────────────────────────────────

/// **N19** : the operator-approved bundle of cases that gates Tier 1
/// promotion.
///
/// Immutability (I1) is structural : `Edit → new GoldSetId + new
/// content hash`.  The registry refuses to mutate an `Approved`
/// gold set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldSet {
    pub gold_set_id: GoldSetId,
    pub agent_id: NanoAgentId,
    pub agent_version: NanoAgentVersion,
    pub agent_input_schema_hash: Sha256Hash,
    pub agent_output_schema_hash: Sha256Hash,
    pub cases: Vec<GoldCase>,
    pub cost_cap_usd: f64,
    pub provenance: Provenance,
    pub status: GoldSetStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub approvers: Vec<OperatorSignature>,
    pub created_at: SystemTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<SystemTime>,
    pub gold_set_hash: Sha256Hash,
}

/// **N19** : minimum cases per gold set (I6).
pub const GOLD_SET_MIN_CASES: usize = 5;

/// **N19** : minimum approvers for an approved gold set (G2 + I3).
pub const GOLD_SET_MIN_APPROVERS: usize = 2;

/// **N19** : default cost cap in USD if the author doesn't set one.
pub const GOLD_SET_DEFAULT_COST_CAP_USD: f64 = 100.0;

/// **N19** : maximum cost cap a Tier 2 publisher can self-set.
pub const GOLD_SET_TIER_2_COST_CAP_MAX_USD: f64 = 250.0;

/// **N19** : maximum cost cap with Tier 3 + operator override.
pub const GOLD_SET_TIER_3_COST_CAP_MAX_USD: f64 = 1_000.0;

impl GoldSet {
    /// **N19** : Validate a draft gold set (called at registration).
    ///
    /// Enforced floors (F11 + F12) :
    /// - `cases.len() >= GOLD_SET_MIN_CASES`,
    /// - every case validates (`GoldCase::validate`),
    /// - no `SchemaValidationOnly` strategy on production sets,
    /// - if `pii_required_by_compliance` is true, ≥ 1 case tagged
    ///   `"pii"`,
    /// - `cost_cap_usd` is finite + positive.
    ///
    /// The compliance check is parameterised because the SDK type
    /// doesn't know the agent's compliance tags ; the caller (the
    /// runner) passes the resolved value.
    pub fn validate(&self, pii_required_by_compliance: bool) -> Result<(), GoldSetError> {
        if self.cases.len() < GOLD_SET_MIN_CASES {
            return Err(GoldSetError::TooFewCases {
                got: self.cases.len(),
                need: GOLD_SET_MIN_CASES,
            });
        }
        for case in &self.cases {
            case.validate()?;
            if !case.assertion.is_production_grade() {
                return Err(GoldSetError::InvalidAssertionParam {
                    reason: format!(
                        "case {:?} uses SchemaValidationOnly which is not production-grade",
                        case.name
                    ),
                });
            }
        }
        if pii_required_by_compliance && !self.cases.iter().any(|c| c.covers_pii()) {
            return Err(GoldSetError::MissingPiiAssertionCase);
        }
        if !self.cost_cap_usd.is_finite() || self.cost_cap_usd <= 0.0 {
            return Err(GoldSetError::InvalidCostCap {
                value_usd: self.cost_cap_usd,
            });
        }
        // Collision check : every gold case id must be unique within
        // the set (F12).
        let mut ids: Vec<GoldCaseId> = self.cases.iter().map(|c| c.gold_case_id).collect();
        ids.sort_by_key(|id| id.as_uuid());
        let pre = ids.len();
        ids.dedup();
        if ids.len() != pre {
            return Err(GoldSetError::CaseIdCollision);
        }
        Ok(())
    }

    /// **N19** : True if this set is ready to gate Tier 1 promotion.
    pub fn is_executable(&self) -> bool {
        matches!(self.status, GoldSetStatus::Approved)
            && self.approvers.len() >= GOLD_SET_MIN_APPROVERS
    }

    /// **N19** : Number of cases (handy for metrics).
    pub fn case_count(&self) -> usize {
        self.cases.len()
    }

    /// **N19 + I4 + F6** : Compute the canonical content hash of this
    /// gold set.  The hash binds the immutable fields :
    /// `(agent_id, agent_version, input_schema_hash, output_schema_hash,
    /// cases, cost_cap_usd, provenance)`.  Mutable lifecycle fields
    /// (`status`, `approvers`, `created_at`, `approved_at`,
    /// `gold_set_hash`) are EXCLUDED — they change as the set moves
    /// through `Draft → Approved → Archived` without changing the
    /// *content* of the gold cases.
    ///
    /// This method is referenced by `GoldGate::run` to defend against
    /// silent registry tampering (F6).
    pub fn compute_canonical_hash(&self) -> Sha256Hash {
        use sha2::{Digest, Sha256};
        // Build a deterministic JSON representation : sorted keys ;
        // mutable fields excluded.
        let body = serde_json::json!({
            "agent_id": self.agent_id.as_str(),
            "agent_input_schema_hash": self.agent_input_schema_hash.to_hex(),
            "agent_output_schema_hash": self.agent_output_schema_hash.to_hex(),
            "agent_version": self.agent_version.as_str(),
            "cases": &self.cases,
            "cost_cap_usd": self.cost_cap_usd,
            "gold_set_id": self.gold_set_id.as_uuid().to_string(),
            "provenance": &self.provenance,
        });
        let bytes = serde_json::to_vec(&body).expect("canonical body serialises");
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let digest = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        Sha256Hash::from_bytes(out)
    }

    /// **N19 + F6** : True iff `gold_set_hash` equals the freshly
    /// computed canonical hash.
    pub fn verify_canonical_hash(&self) -> bool {
        self.compute_canonical_hash() == self.gold_set_hash
    }

    /// **N19 + ergonomic** : produce a copy of this gold set with the
    /// `gold_set_hash` field populated from the canonical hash of the
    /// current content.  Authors call this once before submission so
    /// the registry can verify on lookup.
    pub fn with_computed_hash(mut self) -> Self {
        self.gold_set_hash = self.compute_canonical_hash();
        self
    }

    /// **N19 + ergonomic** : in-place refresh of `gold_set_hash`.
    /// Used by tests / authoring tools that mutate fields after
    /// construction.
    pub fn refresh_hash(&mut self) {
        self.gold_set_hash = self.compute_canonical_hash();
    }
}

// ─────────────────────────────────────────────────────────────────────
//  GoldCaseRun + GoldCaseVerdict + GoldFieldDiff
// ─────────────────────────────────────────────────────────────────────

/// **N19** : execution outcome of a single case (pre-assertion).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldCaseRun {
    pub case_id: GoldCaseId,
    pub trace_id: Uuid,
    pub captured_output: serde_json::Value,
    pub duration_ms: u32,
    pub cost_usd: f64,
    /// `LlmProvenanceRecord.provenance_id` of every LLM call this
    /// case triggered.  Used by N17 to attribute calls back to the
    /// gold-case that drove them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_calls: Vec<Uuid>,
    /// Set if the runner produced an error (NOT an assertion
    /// failure).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub errored: Option<String>,
}

/// **N19** : verdict for a single case AFTER assertion.
///
/// Wire tag is `verdict` (not `kind`) because the `Errored` variant
/// already carries a `kind: GoldErrorKind` field — serde rejects a
/// tag name that collides with an internal struct field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum GoldCaseVerdict {
    Passed {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        similarity: Option<f64>,
    },
    Failed {
        reason: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        diff: Vec<GoldFieldDiff>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        similarity: Option<f64>,
    },
    Errored {
        kind: GoldErrorKind,
        message: String,
    },
    Skipped {
        reason: String,
    },
}

impl GoldCaseVerdict {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Passed { .. } => "passed",
            Self::Failed { .. } => "failed",
            Self::Errored { .. } => "errored",
            Self::Skipped { .. } => "skipped",
        }
    }

    pub fn is_passed(&self) -> bool {
        matches!(self, Self::Passed { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoldErrorKind {
    /// Runner panicked / microVM crashed.
    RunnerPanic,
    /// LLM provider returned an error.
    ProviderError,
    /// Case timed out.
    Timeout,
    /// Hermeticity probe fired (network leak detected).
    HermeticityBroken,
    /// N16 replay anchor not found.
    ReplayAnchorMissing,
    /// Embedding provider failed.
    EmbeddingProviderError,
    /// Output schema validation failed (G5).
    SchemaViolation,
    /// Other backend error.
    Backend,
}

impl GoldErrorKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::RunnerPanic => "runner_panic",
            Self::ProviderError => "provider_error",
            Self::Timeout => "timeout",
            Self::HermeticityBroken => "hermeticity_broken",
            Self::ReplayAnchorMissing => "replay_anchor_missing",
            Self::EmbeddingProviderError => "embedding_provider_error",
            Self::SchemaViolation => "schema_violation",
            Self::Backend => "backend",
        }
    }
}

/// **N19** : per-field diff entry on a `Failed` verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldFieldDiff {
    pub json_pointer: String,
    pub expected: serde_json::Value,
    pub actual: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epsilon_used: Option<f64>,
}

// ─────────────────────────────────────────────────────────────────────
//  GoldOverallVerdict + GoldReport
// ─────────────────────────────────────────────────────────────────────

/// **N19** : aggregate verdict for the whole gold run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GoldOverallVerdict {
    Passed {
        cases_passed: u32,
        cases_total: u32,
    },
    Failed {
        cases_passed: u32,
        cases_failed: u32,
        cases_total: u32,
    },
    AbortedCostCap {
        cost_actual_usd: f64,
        cost_cap_usd: f64,
    },
    AbortedHermeticityBroken {
        case_id: GoldCaseId,
    },
    Errored {
        error_count: u32,
        cases_total: u32,
    },
}

impl GoldOverallVerdict {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Passed { .. } => "passed",
            Self::Failed { .. } => "failed",
            Self::AbortedCostCap { .. } => "aborted_cost_cap",
            Self::AbortedHermeticityBroken { .. } => "aborted_hermeticity",
            Self::Errored { .. } => "errored",
        }
    }

    /// **N19 + G6** : True if this verdict permits a Tier 1 promote.
    pub fn permits_promote(&self) -> bool {
        matches!(self, Self::Passed { .. })
    }
}

/// **N19** : per-case row in a gold report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldCaseEntry {
    pub case: GoldCase,
    pub run: GoldCaseRun,
    pub verdict: GoldCaseVerdict,
}

/// **N19** : the comprehensive report of a single gold-gate run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldReport {
    pub gold_run_id: GoldRunId,
    pub gold_set_id: GoldSetId,
    pub agent_id: NanoAgentId,
    pub agent_version: NanoAgentVersion,
    pub gold_set_hash: Sha256Hash,
    pub started_at: SystemTime,
    pub completed_at: SystemTime,
    pub cases: Vec<GoldCaseEntry>,
    pub overall: GoldOverallVerdict,
    pub total_cost_usd: f64,
    pub cost_cap_usd: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bypass: Option<BypassCertificate>,
}

impl GoldReport {
    /// **N19** : True if this report came from a bypass (skipped
    /// case execution).
    pub fn is_bypassed(&self) -> bool {
        self.bypass.is_some()
    }

    /// **N19** : True if this report permits a Tier 1 promote.
    pub fn permits_promote(&self) -> bool {
        self.overall.permits_promote() || self.is_bypassed()
    }

    /// **N19** : Total wall-clock duration in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        self.completed_at
            .duration_since(self.started_at)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  BypassCertificate — § 7.7 + ADR-5 + G11
// ─────────────────────────────────────────────────────────────────────

/// **N19** : emergency operator override that lets a Tier 1 promote
/// skip the gold gate.
///
/// MUST have ≥ 2 approvers (ADR-5).  `ticket_id` must match
/// `^(INC|SEC|GOLD)-\d+$`.  `rationale` must be ≥ 64 chars.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BypassCertificate {
    pub approvers: Vec<OperatorSignature>,
    pub ticket_id: String,
    pub rationale: String,
    pub valid_until: SystemTime,
}

/// **N19** : minimum rationale length on a bypass certificate.
pub const BYPASS_RATIONALE_MIN_CHARS: usize = 64;

/// **N19** : maximum lifetime of a bypass certificate (operator-set
/// `valid_until` capped at now + 14 days).
pub const BYPASS_MAX_LIFETIME_DAYS: u64 = 14;

impl BypassCertificate {
    /// **N19** : Validate the certificate shape (does NOT verify the
    /// signatures cryptographically — that's the verifier's job).
    pub fn validate_shape(&self) -> Result<(), GoldGateError> {
        if self.approvers.len() < GOLD_SET_MIN_APPROVERS {
            return Err(GoldGateError::BypassInsufficientApprovers {
                got: self.approvers.len() as u32,
                need: GOLD_SET_MIN_APPROVERS as u32,
            });
        }
        let ticket_ok = self.ticket_id.starts_with("INC-")
            || self.ticket_id.starts_with("SEC-")
            || self.ticket_id.starts_with("GOLD-");
        if !ticket_ok {
            return Err(GoldGateError::BypassInvalidTicket {
                ticket_id: self.ticket_id.clone(),
            });
        }
        if self.rationale.chars().count() < BYPASS_RATIONALE_MIN_CHARS {
            return Err(GoldGateError::BypassRationaleTooShort {
                got: self.rationale.chars().count() as u32,
                need: BYPASS_RATIONALE_MIN_CHARS as u32,
            });
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────
//  GoldSetError + GoldGateError — closed taxonomies
// ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Error, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GoldSetError {
    #[error("gold set {gold_set_id} not found")]
    NotFound { gold_set_id: GoldSetId },

    #[error("gold case {gold_case_id} not found in set {gold_set_id}")]
    CaseNotFound {
        gold_set_id: GoldSetId,
        gold_case_id: GoldCaseId,
    },

    #[error("too few cases — got {got}, need {need}")]
    TooFewCases { got: usize, need: usize },

    #[error("PII-sensitive agent requires ≥ 1 case tagged 'pii'")]
    MissingPiiAssertionCase,

    #[error("invalid case : {reason}")]
    InvalidCase { reason: String },

    #[error("invalid assertion parameter : {reason}")]
    InvalidAssertionParam { reason: String },

    #[error("invalid cost cap : {value_usd}")]
    InvalidCostCap { value_usd: f64 },

    #[error("gold case id collision within set")]
    CaseIdCollision,

    #[error("gold set already approved — edits forbidden (I1)")]
    AlreadyApproved,

    #[error("gold set hash mismatch : computed != stored")]
    HashMismatch,

    #[error("operator approval count {got} below required {need}")]
    InsufficientApprovers { got: u32, need: u32 },

    #[error("agent input/output schema hash mismatch (stale gold set)")]
    StaleGoldSet,

    #[error("backend : {message}")]
    Backend { message: String },
}

/// **N19** : closed taxonomy of gate-level errors (orchestration).
#[derive(Debug, Clone, PartialEq, Error, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GoldGateError {
    #[error("gold set missing : Tier 1 promote requires a gold set")]
    GoldSetMissing,

    #[error("gold set not approved")]
    GoldSetNotApproved,

    #[error("gold run already in progress : {gold_run_id}")]
    RunAlreadyInProgress { gold_run_id: GoldRunId },

    #[error("cost cap exceeded : actual ${cost_actual_usd}, cap ${cost_cap_usd}")]
    CostCapExceeded {
        cost_actual_usd: f64,
        cost_cap_usd: f64,
    },

    #[error("hermeticity violated for case {case_id}")]
    HermeticityBroken { case_id: GoldCaseId },

    #[error("bypass : insufficient approvers (got {got}, need {need})")]
    BypassInsufficientApprovers { got: u32, need: u32 },

    #[error("bypass : invalid ticket id {ticket_id:?}")]
    BypassInvalidTicket { ticket_id: String },

    #[error("bypass : rationale too short (got {got} chars, need {need})")]
    BypassRationaleTooShort { got: u32, need: u32 },

    #[error("bypass : signature verification failed for approver {approver_index}")]
    BypassSignatureInvalid { approver_index: u32 },

    #[error("bypass : valid_until in the past or > 14 d in the future")]
    BypassInvalidLifetime,

    #[error("bypass : ticket id does not match policy")]
    BypassPolicyRejected,

    #[error("gold runner : {message}")]
    Runner { message: String },

    #[error("assertion engine : {message}")]
    Assertion { message: String },

    #[error("ledger : {message}")]
    Ledger { message: String },

    #[error("backend : {message}")]
    Backend { message: String },
}

impl From<GoldSetError> for GoldGateError {
    fn from(e: GoldSetError) -> Self {
        match e {
            GoldSetError::NotFound { .. } => Self::GoldSetMissing,
            other => Self::Backend {
                message: other.to_string(),
            },
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  GoldSetRef — used by `NanoAgent.gold_set_ref`
// ─────────────────────────────────────────────────────────────────────

/// **N19** : compact reference attached to a `NanoAgent` indicating
/// which gold set gates its Tier 1 promotion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoldSetRef {
    pub gold_set_id: GoldSetId,
    pub gold_set_hash: Sha256Hash,
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Iso3166, OperatorSignature};
    use std::time::Duration;
    use uuid::Uuid;

    fn op_sig() -> OperatorSignature {
        OperatorSignature {
            operator_id: Uuid::new_v4(),
            signed_at: SystemTime::now(),
            ticket_id: "GOLD-1".into(),
            signature: vec![0u8; 64],
        }
    }

    fn case_strict(name: &str) -> GoldCase {
        GoldCase {
            gold_case_id: GoldCaseId::new(),
            name: name.into(),
            input: serde_json::json!({"a": 1}),
            expected: GoldExpectedOutput::Inline {
                value: serde_json::json!({"b": 2}),
            },
            assertion: GoldAssertionStrategy::StrictEquality,
            timeout_seconds: GOLD_CASE_DEFAULT_TIMEOUT_SECONDS,
            tags: vec![],
        }
    }

    fn case_tol(name: &str, eps: f64) -> GoldCase {
        let mut fields = BTreeMap::new();
        fields.insert("/score".into(), eps);
        GoldCase {
            gold_case_id: GoldCaseId::new(),
            name: name.into(),
            input: serde_json::json!({"x": 1}),
            expected: GoldExpectedOutput::Inline {
                value: serde_json::json!({"score": 0.5}),
            },
            assertion: GoldAssertionStrategy::NumericTolerance { fields },
            timeout_seconds: GOLD_CASE_DEFAULT_TIMEOUT_SECONDS,
            tags: vec![],
        }
    }

    fn case_sim(name: &str, threshold: f64) -> GoldCase {
        GoldCase {
            gold_case_id: GoldCaseId::new(),
            name: name.into(),
            input: serde_json::json!({"prompt": "summarise"}),
            expected: GoldExpectedOutput::Inline {
                value: serde_json::json!({"summary": "ok"}),
            },
            assertion: GoldAssertionStrategy::SemanticSimilarity {
                embedding_model: ModelIdentity::hosted("openai", "text-embedding-3-small", "1"),
                cosine_threshold: threshold,
                field_pointer: None,
            },
            timeout_seconds: GOLD_CASE_DEFAULT_TIMEOUT_SECONDS,
            tags: vec![],
        }
    }

    fn set_5_cases() -> GoldSet {
        GoldSet {
            gold_set_id: GoldSetId::new(),
            agent_id: NanoAgentId::new("x").unwrap(),
            agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            agent_input_schema_hash: Sha256Hash::from_bytes([1u8; 32]),
            agent_output_schema_hash: Sha256Hash::from_bytes([2u8; 32]),
            cases: vec![
                case_strict("happy_1"),
                case_strict("happy_2"),
                case_tol("numeric_1", 0.01),
                case_sim("semantic_1", 0.9),
                case_sim("semantic_2", 0.85),
            ],
            cost_cap_usd: 100.0,
            provenance: Provenance::Manual,
            status: GoldSetStatus::Draft,
            approvers: vec![],
            created_at: SystemTime::now(),
            approved_at: None,
            gold_set_hash: Sha256Hash::from_bytes([9u8; 32]),
        }
    }

    // ─── ID round-trips ──────────────────────────────────────────────

    #[test]
    fn gold_set_id_round_trip() {
        let id = GoldSetId::new();
        let json = serde_json::to_string(&id).unwrap();
        let back: GoldSetId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn gold_case_id_round_trip() {
        let id = GoldCaseId::new();
        let json = serde_json::to_string(&id).unwrap();
        let back: GoldCaseId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn gold_run_id_round_trip() {
        let id = GoldRunId::new();
        let json = serde_json::to_string(&id).unwrap();
        let back: GoldRunId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }

    // ─── GoldAssertionStrategy ──────────────────────────────────────

    #[test]
    fn strategy_label_stable() {
        assert_eq!(GoldAssertionStrategy::StrictEquality.label(), "strict_equality");
        let tol = GoldAssertionStrategy::NumericTolerance {
            fields: BTreeMap::new(),
        };
        assert_eq!(tol.label(), "numeric_tolerance");
        let sim = GoldAssertionStrategy::SemanticSimilarity {
            embedding_model: ModelIdentity::hosted("o", "m", "v"),
            cosine_threshold: 0.9,
            field_pointer: None,
        };
        assert_eq!(sim.label(), "semantic_similarity");
        assert_eq!(GoldAssertionStrategy::SchemaValidationOnly.label(), "schema_validation_only");
    }

    #[test]
    fn schema_validation_only_is_not_production_grade() {
        assert!(!GoldAssertionStrategy::SchemaValidationOnly.is_production_grade());
        assert!(GoldAssertionStrategy::StrictEquality.is_production_grade());
    }

    #[test]
    fn strategy_validate_strict_equality_ok() {
        assert!(GoldAssertionStrategy::StrictEquality.validate().is_ok());
    }

    #[test]
    fn strategy_validate_tolerance_rejects_negative_epsilon() {
        let mut fields = BTreeMap::new();
        fields.insert("/score".into(), -0.1);
        let s = GoldAssertionStrategy::NumericTolerance { fields };
        assert!(s.validate().is_err());
    }

    #[test]
    fn strategy_validate_tolerance_rejects_non_finite_epsilon() {
        let mut fields = BTreeMap::new();
        fields.insert("/score".into(), f64::NAN);
        let s = GoldAssertionStrategy::NumericTolerance { fields };
        assert!(s.validate().is_err());
    }

    #[test]
    fn strategy_validate_tolerance_rejects_non_pointer_key() {
        let mut fields = BTreeMap::new();
        fields.insert("score".into(), 0.1);
        let s = GoldAssertionStrategy::NumericTolerance { fields };
        assert!(s.validate().is_err());
    }

    #[test]
    fn strategy_validate_similarity_rejects_threshold_out_of_range() {
        let s = GoldAssertionStrategy::SemanticSimilarity {
            embedding_model: ModelIdentity::hosted("o", "m", "v"),
            cosine_threshold: 1.5,
            field_pointer: None,
        };
        assert!(s.validate().is_err());
        let s2 = GoldAssertionStrategy::SemanticSimilarity {
            embedding_model: ModelIdentity::hosted("o", "m", "v"),
            cosine_threshold: -0.1,
            field_pointer: None,
        };
        assert!(s2.validate().is_err());
    }

    #[test]
    fn strategy_serde_round_trip_all_variants() {
        let mut fields = BTreeMap::new();
        fields.insert("/score".into(), 0.01);
        let cases = vec![
            GoldAssertionStrategy::StrictEquality,
            GoldAssertionStrategy::NumericTolerance { fields },
            GoldAssertionStrategy::SemanticSimilarity {
                embedding_model: ModelIdentity::hosted("o", "m", "v"),
                cosine_threshold: 0.9,
                field_pointer: Some("/summary".into()),
            },
            GoldAssertionStrategy::SchemaValidationOnly,
        ];
        for s in cases {
            let json = serde_json::to_string(&s).unwrap();
            let back: GoldAssertionStrategy = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back);
        }
    }

    // ─── GoldExpectedOutput ─────────────────────────────────────────

    #[test]
    fn expected_output_serde_round_trip() {
        let cases = vec![
            GoldExpectedOutput::Inline {
                value: serde_json::json!({"a": 1}),
            },
            GoldExpectedOutput::ReplayAnchor {
                trace_id: Uuid::new_v4(),
                span_id: Uuid::new_v4(),
            },
        ];
        for c in cases {
            let json = serde_json::to_string(&c).unwrap();
            let back: GoldExpectedOutput = serde_json::from_str(&json).unwrap();
            assert_eq!(c, back);
        }
    }

    #[test]
    fn expected_output_is_replay_anchored() {
        let inline = GoldExpectedOutput::Inline {
            value: serde_json::json!({}),
        };
        assert!(!inline.is_replay_anchored());
        let anchor = GoldExpectedOutput::ReplayAnchor {
            trace_id: Uuid::nil(),
            span_id: Uuid::nil(),
        };
        assert!(anchor.is_replay_anchored());
    }

    // ─── GoldCase ──────────────────────────────────────────────────

    #[test]
    fn gold_case_validate_ok() {
        assert!(case_strict("ok").validate().is_ok());
        assert!(case_tol("ok", 0.01).validate().is_ok());
        assert!(case_sim("ok", 0.9).validate().is_ok());
    }

    #[test]
    fn gold_case_rejects_empty_name() {
        let mut c = case_strict("");
        c.name = "".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn gold_case_rejects_zero_timeout() {
        let mut c = case_strict("x");
        c.timeout_seconds = 0;
        assert!(c.validate().is_err());
    }

    #[test]
    fn gold_case_rejects_excessive_timeout() {
        let mut c = case_strict("x");
        c.timeout_seconds = GOLD_CASE_MAX_TIMEOUT_SECONDS + 1;
        assert!(c.validate().is_err());
    }

    #[test]
    fn gold_case_covers_pii_tag() {
        let mut c = case_strict("pii_test");
        c.tags.push("pii".into());
        assert!(c.covers_pii());
        let c2 = case_strict("non_pii");
        assert!(!c2.covers_pii());
    }

    // ─── GoldSet ───────────────────────────────────────────────────

    #[test]
    fn gold_set_validate_ok() {
        let s = set_5_cases();
        assert!(s.validate(false).is_ok());
    }

    #[test]
    fn gold_set_validate_rejects_too_few_cases() {
        let mut s = set_5_cases();
        s.cases.truncate(3);
        let err = s.validate(false).unwrap_err();
        assert!(matches!(err, GoldSetError::TooFewCases { .. }));
    }

    #[test]
    fn gold_set_validate_rejects_missing_pii_when_required() {
        let s = set_5_cases();
        let err = s.validate(true).unwrap_err();
        assert!(matches!(err, GoldSetError::MissingPiiAssertionCase));
    }

    #[test]
    fn gold_set_validate_passes_with_pii_case() {
        let mut s = set_5_cases();
        s.cases[0].tags.push("pii".into());
        assert!(s.validate(true).is_ok());
    }

    #[test]
    fn gold_set_validate_rejects_schema_only_strategy() {
        let mut s = set_5_cases();
        s.cases[0].assertion = GoldAssertionStrategy::SchemaValidationOnly;
        let err = s.validate(false).unwrap_err();
        assert!(matches!(err, GoldSetError::InvalidAssertionParam { .. }));
    }

    #[test]
    fn gold_set_validate_rejects_negative_cost_cap() {
        let mut s = set_5_cases();
        s.cost_cap_usd = -10.0;
        let err = s.validate(false).unwrap_err();
        assert!(matches!(err, GoldSetError::InvalidCostCap { .. }));
    }

    #[test]
    fn gold_set_validate_rejects_nan_cost_cap() {
        let mut s = set_5_cases();
        s.cost_cap_usd = f64::NAN;
        let err = s.validate(false).unwrap_err();
        assert!(matches!(err, GoldSetError::InvalidCostCap { .. }));
    }

    #[test]
    fn gold_set_validate_rejects_case_id_collision() {
        let mut s = set_5_cases();
        let dup_id = s.cases[0].gold_case_id;
        s.cases[1].gold_case_id = dup_id;
        let err = s.validate(false).unwrap_err();
        assert!(matches!(err, GoldSetError::CaseIdCollision));
    }

    #[test]
    fn gold_set_is_executable_requires_approved_and_two_approvers() {
        let mut s = set_5_cases();
        assert!(!s.is_executable());
        s.status = GoldSetStatus::Approved;
        assert!(!s.is_executable());
        s.approvers.push(op_sig());
        assert!(!s.is_executable());
        s.approvers.push(op_sig());
        assert!(s.is_executable());
    }

    #[test]
    fn gold_set_status_label_stable() {
        assert_eq!(GoldSetStatus::Draft.label(), "draft");
        assert_eq!(GoldSetStatus::Approved.label(), "approved");
        assert_eq!(GoldSetStatus::Archived.label(), "archived");
    }

    #[test]
    fn gold_set_serde_round_trip() {
        let s = set_5_cases();
        let json = serde_json::to_string(&s).unwrap();
        let back: GoldSet = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    // ─── Verdicts ──────────────────────────────────────────────────

    #[test]
    fn case_verdict_labels() {
        let v = GoldCaseVerdict::Passed { similarity: None };
        assert_eq!(v.label(), "passed");
        assert!(v.is_passed());
        let v2 = GoldCaseVerdict::Failed {
            reason: "bad".into(),
            diff: vec![],
            similarity: None,
        };
        assert_eq!(v2.label(), "failed");
        assert!(!v2.is_passed());
        let v3 = GoldCaseVerdict::Errored {
            kind: GoldErrorKind::Timeout,
            message: "x".into(),
        };
        assert_eq!(v3.label(), "errored");
        let v4 = GoldCaseVerdict::Skipped {
            reason: "cost cap".into(),
        };
        assert_eq!(v4.label(), "skipped");
    }

    #[test]
    fn overall_verdict_permits_promote_only_on_passed() {
        assert!(GoldOverallVerdict::Passed {
            cases_passed: 5,
            cases_total: 5
        }
        .permits_promote());
        assert!(!GoldOverallVerdict::Failed {
            cases_passed: 4,
            cases_failed: 1,
            cases_total: 5
        }
        .permits_promote());
        assert!(!GoldOverallVerdict::AbortedCostCap {
            cost_actual_usd: 101.0,
            cost_cap_usd: 100.0
        }
        .permits_promote());
    }

    #[test]
    fn gold_report_is_bypassed_only_when_certificate_present() {
        let report = GoldReport {
            gold_run_id: GoldRunId::new(),
            gold_set_id: GoldSetId::new(),
            agent_id: NanoAgentId::new("x").unwrap(),
            agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            gold_set_hash: Sha256Hash::from_bytes([0u8; 32]),
            started_at: SystemTime::now(),
            completed_at: SystemTime::now() + Duration::from_secs(1),
            cases: vec![],
            overall: GoldOverallVerdict::Passed {
                cases_passed: 5,
                cases_total: 5,
            },
            total_cost_usd: 0.1,
            cost_cap_usd: 100.0,
            bypass: None,
        };
        assert!(!report.is_bypassed());
        assert!(report.permits_promote());
    }

    // ─── BypassCertificate ─────────────────────────────────────────

    #[test]
    fn bypass_validate_shape_ok() {
        let b = BypassCertificate {
            approvers: vec![op_sig(), op_sig()],
            ticket_id: "INC-9117".into(),
            rationale: "x".repeat(BYPASS_RATIONALE_MIN_CHARS),
            valid_until: SystemTime::now() + Duration::from_secs(86_400),
        };
        assert!(b.validate_shape().is_ok());
    }

    #[test]
    fn bypass_rejects_short_rationale() {
        let b = BypassCertificate {
            approvers: vec![op_sig(), op_sig()],
            ticket_id: "INC-1".into(),
            rationale: "short".into(),
            valid_until: SystemTime::now() + Duration::from_secs(86_400),
        };
        let err = b.validate_shape().unwrap_err();
        assert!(matches!(err, GoldGateError::BypassRationaleTooShort { .. }));
    }

    #[test]
    fn bypass_rejects_invalid_ticket() {
        let b = BypassCertificate {
            approvers: vec![op_sig(), op_sig()],
            ticket_id: "FREE-FORM".into(),
            rationale: "x".repeat(BYPASS_RATIONALE_MIN_CHARS),
            valid_until: SystemTime::now() + Duration::from_secs(86_400),
        };
        let err = b.validate_shape().unwrap_err();
        assert!(matches!(err, GoldGateError::BypassInvalidTicket { .. }));
    }

    #[test]
    fn bypass_rejects_single_approver() {
        let b = BypassCertificate {
            approvers: vec![op_sig()],
            ticket_id: "INC-1".into(),
            rationale: "x".repeat(BYPASS_RATIONALE_MIN_CHARS),
            valid_until: SystemTime::now() + Duration::from_secs(86_400),
        };
        let err = b.validate_shape().unwrap_err();
        assert!(matches!(err, GoldGateError::BypassInsufficientApprovers { .. }));
    }

    #[test]
    fn bypass_serde_round_trip() {
        let b = BypassCertificate {
            approvers: vec![op_sig(), op_sig()],
            ticket_id: "GOLD-7".into(),
            rationale: "x".repeat(BYPASS_RATIONALE_MIN_CHARS),
            valid_until: SystemTime::now() + Duration::from_secs(86_400),
        };
        let json = serde_json::to_string(&b).unwrap();
        let back: BypassCertificate = serde_json::from_str(&json).unwrap();
        assert_eq!(b, back);
    }

    // ─── Provenance ────────────────────────────────────────────────

    #[test]
    fn provenance_labels() {
        assert_eq!(Provenance::Manual.label(), "manual");
        assert_eq!(
            Provenance::ShadowCapture {
                run_id: Uuid::nil()
            }
            .label(),
            "shadow_capture"
        );
        assert_eq!(
            Provenance::Mutation {
                parent_set_id: GoldSetId::new()
            }
            .label(),
            "mutation"
        );
    }

    // ─── GoldErrorKind ─────────────────────────────────────────────

    #[test]
    fn gold_error_kind_labels() {
        assert_eq!(GoldErrorKind::RunnerPanic.label(), "runner_panic");
        assert_eq!(GoldErrorKind::Timeout.label(), "timeout");
        assert_eq!(
            GoldErrorKind::HermeticityBroken.label(),
            "hermeticity_broken"
        );
        assert_eq!(GoldErrorKind::Backend.label(), "backend");
    }

    // ─── Errors ────────────────────────────────────────────────────

    #[test]
    fn gold_set_error_display_non_empty() {
        let e = GoldSetError::TooFewCases { got: 3, need: 5 };
        assert!(!format!("{e}").is_empty());
    }

    #[test]
    fn gold_gate_error_display_non_empty() {
        let e = GoldGateError::CostCapExceeded {
            cost_actual_usd: 101.0,
            cost_cap_usd: 100.0,
        };
        assert!(!format!("{e}").is_empty());
    }

    #[test]
    fn gold_set_error_serde_round_trip() {
        let cases = vec![
            GoldSetError::TooFewCases { got: 3, need: 5 },
            GoldSetError::MissingPiiAssertionCase,
            GoldSetError::CaseIdCollision,
            GoldSetError::AlreadyApproved,
            GoldSetError::HashMismatch,
            GoldSetError::StaleGoldSet,
            GoldSetError::InvalidCostCap { value_usd: -1.0 },
        ];
        for c in cases {
            let json = serde_json::to_string(&c).unwrap();
            let back: GoldSetError = serde_json::from_str(&json).unwrap();
            assert_eq!(c, back);
        }
    }

    #[test]
    fn gold_gate_error_serde_round_trip() {
        let cases = vec![
            GoldGateError::GoldSetMissing,
            GoldGateError::GoldSetNotApproved,
            GoldGateError::CostCapExceeded {
                cost_actual_usd: 101.0,
                cost_cap_usd: 100.0,
            },
            GoldGateError::BypassInsufficientApprovers { got: 1, need: 2 },
        ];
        for c in cases {
            let json = serde_json::to_string(&c).unwrap();
            let back: GoldGateError = serde_json::from_str(&json).unwrap();
            assert_eq!(c, back);
        }
    }

    // ─── GoldSetRef ────────────────────────────────────────────────

    #[test]
    fn gold_set_ref_serde_round_trip() {
        let r = GoldSetRef {
            gold_set_id: GoldSetId::new(),
            gold_set_hash: Sha256Hash::from_bytes([3u8; 32]),
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: GoldSetRef = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }

    // ─── Cross-cutting : Iso3166 import sanity ────────────────────

    #[test]
    fn iso3166_still_exported() {
        // Sanity that the SDK re-exports we depend on transitively
        // are still reachable.
        let _ = Iso3166::new("US").unwrap();
    }
}
