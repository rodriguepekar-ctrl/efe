// SPDX-License-Identifier: UNLICENSED
//! **Sprint 11 — N19** : HTTP query / mutation API — § 9.1.
//!
//! Transport-agnostic handler abstraction.  Sprint 12 wires axum.

use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use efe_nano_sdk::{
    BypassCertificate, GoldReport, GoldRunId, GoldSet, GoldSetId, GoldSetStatus, NanoAgent,
    NanoAgentId, NanoAgentVersion, OperatorSignature,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::gate::{GoldGate, GoldGateConfig};
use crate::ledger::{GoldRunLedger, GoldRunLedgerEntry, GoldRunLedgerRow};
use crate::registry::{GoldSetRegistry, GoldSetRegistryError};

#[derive(Debug, Clone, PartialEq, Error)]
pub enum GoldApiError {
    #[error("not found")]
    NotFound,

    #[error("bad request : {reason}")]
    BadRequest { reason: String },

    #[error("conflict : {reason}")]
    Conflict { reason: String },

    #[error("forbidden : {reason}")]
    Forbidden { reason: String },

    #[error("backend : {message}")]
    Backend { message: String },
}

impl From<GoldSetRegistryError> for GoldApiError {
    fn from(e: GoldSetRegistryError) -> Self {
        match e {
            GoldSetRegistryError::NotFound { .. } => Self::NotFound,
            GoldSetRegistryError::InvalidTransition { .. } => Self::Conflict {
                reason: format!("{e}"),
            },
            GoldSetRegistryError::InsufficientApprovers { got, need } => Self::Forbidden {
                reason: format!("insufficient approvers : got {got}, need {need}"),
            },
            GoldSetRegistryError::DuplicateApprover { operator_id } => Self::BadRequest {
                reason: format!("duplicate approver {operator_id}"),
            },
            GoldSetRegistryError::Validation(v) => Self::BadRequest {
                reason: format!("{v}"),
            },
            GoldSetRegistryError::Backend { message } => Self::Backend { message },
        }
    }
}

impl From<efe_nano_sdk::GoldGateError> for GoldApiError {
    fn from(e: efe_nano_sdk::GoldGateError) -> Self {
        use efe_nano_sdk::GoldGateError::*;
        match e {
            GoldSetMissing | GoldSetNotApproved => Self::BadRequest {
                reason: format!("{e}"),
            },
            RunAlreadyInProgress { .. } => Self::Conflict {
                reason: format!("{e}"),
            },
            BypassInsufficientApprovers { .. }
            | BypassInvalidTicket { .. }
            | BypassRationaleTooShort { .. }
            | BypassSignatureInvalid { .. }
            | BypassInvalidLifetime
            | BypassPolicyRejected => Self::Forbidden {
                reason: format!("{e}"),
            },
            other => Self::Backend {
                message: format!("{other}"),
            },
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Request / response DTOs
// ─────────────────────────────────────────────────────────────────────

/// POST `/v1/gold-sets` body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegisterGoldSetRequest {
    pub draft: GoldSet,
    /// Caller-resolved : is the target agent's compliance tagged
    /// PII ? (the API can't introspect the catalog directly).
    pub pii_required_by_compliance: bool,
}

/// POST `/v1/gold-sets/{id}/approve` body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApproveGoldSetRequest {
    pub approvers: Vec<OperatorSignature>,
}

/// POST `/v1/gold-runs` body.
///
/// `NanoAgent` does not implement `PartialEq` (Sprint 1 design — too
/// many fields with non-PartialEq inner types) so this DTO is
/// `Clone` only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRequest {
    pub agent: NanoAgent,
    pub gold_set_id: GoldSetId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bypass: Option<BypassCertificate>,
}

/// POST `/v1/gold-runs/drift-sweep` body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriftSweepRequest {
    /// Agents to evaluate ; if `None`, sweeps all Tier 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_ids: Option<Vec<NanoAgentId>>,
    /// Drift window in days (default 30 per ADR-4).
    #[serde(default = "default_drift_window_days")]
    pub max_age_days: u64,
}

fn default_drift_window_days() -> u64 {
    30
}

/// **N19** : per-agent drift verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriftSweepEntry {
    pub agent_id: NanoAgentId,
    pub agent_version: NanoAgentVersion,
    pub last_run_id: Option<GoldRunId>,
    pub last_run_age_days: Option<u64>,
    pub outcome: DriftSweepOutcome,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DriftSweepOutcome {
    Fresh,
    Stale { age_days: u64 },
    Demoted { reason: String },
    Errored { message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriftSweepReport {
    pub batch_id: uuid::Uuid,
    pub max_age_days: u64,
    pub started_at: SystemTime,
    pub completed_at: SystemTime,
    pub entries: Vec<DriftSweepEntry>,
}

// ─────────────────────────────────────────────────────────────────────
//  GoldApi trait
// ─────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait GoldApi: Send + Sync + 'static {
    async fn register_gold_set(
        &self,
        req: RegisterGoldSetRequest,
    ) -> Result<GoldSetId, GoldApiError>;

    async fn approve_gold_set(
        &self,
        gold_set_id: GoldSetId,
        req: ApproveGoldSetRequest,
    ) -> Result<GoldSet, GoldApiError>;

    async fn lookup_gold_set(
        &self,
        gold_set_id: GoldSetId,
    ) -> Result<GoldSet, GoldApiError>;

    async fn list_gold_sets_for_agent(
        &self,
        agent_id: &NanoAgentId,
    ) -> Result<Vec<GoldSet>, GoldApiError>;

    async fn run_gate(&self, req: RunRequest) -> Result<GoldReport, GoldApiError>;

    async fn lookup_gold_run(
        &self,
        gold_run_id: GoldRunId,
    ) -> Result<Option<GoldRunLedgerRow>, GoldApiError>;

    async fn drift_sweep(
        &self,
        req: DriftSweepRequest,
    ) -> Result<DriftSweepReport, GoldApiError>;
}

// ─────────────────────────────────────────────────────────────────────
//  DefaultGoldApi
// ─────────────────────────────────────────────────────────────────────

pub struct DefaultGoldApi {
    pub registry: Arc<dyn GoldSetRegistry>,
    pub gate: Arc<GoldGate>,
    pub ledger: Arc<dyn GoldRunLedger>,
}

impl std::fmt::Debug for DefaultGoldApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DefaultGoldApi").finish_non_exhaustive()
    }
}

impl DefaultGoldApi {
    pub fn new(
        registry: Arc<dyn GoldSetRegistry>,
        gate: Arc<GoldGate>,
        ledger: Arc<dyn GoldRunLedger>,
    ) -> Self {
        Self {
            registry,
            gate,
            ledger,
        }
    }
}

#[async_trait]
impl GoldApi for DefaultGoldApi {
    async fn register_gold_set(
        &self,
        req: RegisterGoldSetRequest,
    ) -> Result<GoldSetId, GoldApiError> {
        let id = self
            .registry
            .register(req.draft, req.pii_required_by_compliance)
            .await?;
        Ok(id)
    }

    async fn approve_gold_set(
        &self,
        gold_set_id: GoldSetId,
        req: ApproveGoldSetRequest,
    ) -> Result<GoldSet, GoldApiError> {
        self.registry
            .approve(gold_set_id, req.approvers.clone())
            .await?;
        let set = self.registry.lookup(gold_set_id).await?;
        // Audit row : log the approval ceremony in the ledger.
        let agent_id = set.agent_id.clone();
        let approvers = set.approvers.clone();
        let _ = self
            .ledger
            .append(GoldRunLedgerEntry::GoldSetApproved {
                gold_set_id,
                agent_id,
                approvers,
                approved_at: set.approved_at.unwrap_or_else(SystemTime::now),
            })
            .await
            .map_err(|e| GoldApiError::Backend {
                message: format!("ledger : {e}"),
            })?;
        Ok(set)
    }

    async fn lookup_gold_set(
        &self,
        gold_set_id: GoldSetId,
    ) -> Result<GoldSet, GoldApiError> {
        Ok(self.registry.lookup(gold_set_id).await?)
    }

    async fn list_gold_sets_for_agent(
        &self,
        agent_id: &NanoAgentId,
    ) -> Result<Vec<GoldSet>, GoldApiError> {
        Ok(self.registry.list_for_agent(agent_id).await?)
    }

    async fn run_gate(&self, req: RunRequest) -> Result<GoldReport, GoldApiError> {
        let set = self.registry.lookup(req.gold_set_id).await?;
        if !matches!(set.status, GoldSetStatus::Approved) && req.bypass.is_none() {
            return Err(GoldApiError::BadRequest {
                reason: "gold set is not approved (and no bypass provided)".into(),
            });
        }
        let agent = Arc::new(req.agent);
        let cfg = GoldGateConfig {
            agent_tier_label: stability_tier_label(&agent.stability_tier),
            ..Default::default()
        };
        let report = self
            .gate
            .run(agent, set, req.bypass, &cfg)
            .await?;
        Ok(report)
    }

    async fn lookup_gold_run(
        &self,
        gold_run_id: GoldRunId,
    ) -> Result<Option<GoldRunLedgerRow>, GoldApiError> {
        self.ledger
            .find_by_gold_run_id(gold_run_id)
            .await
            .map_err(|e| GoldApiError::Backend {
                message: format!("ledger : {e}"),
            })
    }

    async fn drift_sweep(
        &self,
        req: DriftSweepRequest,
    ) -> Result<DriftSweepReport, GoldApiError> {
        let started_at = SystemTime::now();
        let entries = req.agent_ids.unwrap_or_default()
            .into_iter()
            .map(|agent_id| DriftSweepEntry {
                agent_id,
                // Sprint 11 stub : the entries are placeholders.
                // Sprint 12 wires actual replay of the gold set
                // through `GoldGate::run` per agent and emits a
                // `GoldDriftDetected` ledger row on failure.
                agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
                last_run_id: None,
                last_run_age_days: None,
                outcome: DriftSweepOutcome::Fresh,
            })
            .collect();
        let completed_at = SystemTime::now();
        Ok(DriftSweepReport {
            batch_id: uuid::Uuid::new_v4(),
            max_age_days: req.max_age_days,
            started_at,
            completed_at,
            entries,
        })
    }
}

fn stability_tier_label(t: &efe_nano_sdk::StabilityTier) -> String {
    t.as_str().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DefaultAssertionEngine, InMemoryGoldRunLedger, InMemoryGoldRunner,
        InMemoryGoldSetRegistry, StubEmbeddingProvider, N19Metrics,
    };
    use efe_nano_sdk::{
        Author, Category, DeterminismLevel, ExecutionMode, GoldAssertionStrategy,
        GoldCaseId, GoldExpectedOutput, GoldSet, IdempotencyPolicy, Provenance,
        ResourceLimits, Sha256Hash, StabilityTier,
    };

    fn api() -> DefaultGoldApi {
        let registry: Arc<dyn GoldSetRegistry> = Arc::new(InMemoryGoldSetRegistry::new());
        let ledger: Arc<dyn GoldRunLedger> = Arc::new(InMemoryGoldRunLedger::new([42u8; 32]));
        let gate = Arc::new(GoldGate::new(
            registry.clone(),
            Arc::new(InMemoryGoldRunner::echo()),
            Arc::new(DefaultAssertionEngine::new(Arc::new(StubEmbeddingProvider::new()))),
            ledger.clone(),
            N19Metrics::new(),
        ));
        DefaultGoldApi::new(registry, gate, ledger)
    }

    fn agent() -> NanoAgent {
        NanoAgent {
            id: NanoAgentId::new("x").unwrap(),
            version: NanoAgentVersion::new("1.0.0").unwrap(),
            description: "x".into(),
            author: Author::Efe,
            category: Category::Validation,
            stability_tier: StabilityTier::TierExperimental,
            execution_mode: ExecutionMode::PureRust {
                entry: "x::run".into(),
            },
            determinism: DeterminismLevel::FullDeterministic,
            idempotency: IdempotencyPolicy::auto_24h(),
            input_schema: serde_json::json!({}),
            output_schema: serde_json::json!({}),
            error_schema: None,
            limits: ResourceLimits::default_mode_a(),
            cost_estimate_cents: 0.01,
            latency_p99_ms_target: 50,
            audit: Default::default(),
            compliance: Default::default(),
            mcp_tool_metadata: None,
            signature: None,
            co_publishing: None,
            gold_set_ref: None,
            created_at: chrono::Utc::now(),
        }
    }

    fn draft(agent_id: &NanoAgentId) -> GoldSet {
        let cases: Vec<_> = (0..5)
            .map(|i| efe_nano_sdk::GoldCase {
                gold_case_id: GoldCaseId::new(),
                name: format!("c{i}"),
                input: serde_json::json!({}),
                expected: GoldExpectedOutput::Inline {
                    value: serde_json::json!({"i": i}),
                },
                assertion: GoldAssertionStrategy::StrictEquality,
                timeout_seconds: 30,
                tags: vec![],
            })
            .collect();
        GoldSet {
            gold_set_id: GoldSetId::new(),
            agent_id: agent_id.clone(),
            agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            agent_input_schema_hash: Sha256Hash::from_bytes([1u8; 32]),
            agent_output_schema_hash: Sha256Hash::from_bytes([2u8; 32]),
            cases,
            cost_cap_usd: 100.0,
            provenance: Provenance::Manual,
            status: GoldSetStatus::Draft,
            approvers: vec![],
            created_at: SystemTime::now(),
            approved_at: None,
            gold_set_hash: Sha256Hash::from_bytes([0u8; 32]),
        }
        .with_computed_hash()
    }

    fn op_sig() -> OperatorSignature {
        OperatorSignature {
            operator_id: uuid::Uuid::new_v4(),
            signed_at: SystemTime::now(),
            ticket_id: "GOLD-1".into(),
            signature: vec![0u8; 64],
        }
    }

    #[tokio::test]
    async fn register_then_lookup_round_trip() {
        let api = api();
        let a = agent();
        let id = api
            .register_gold_set(RegisterGoldSetRequest {
                draft: draft(&a.id),
                pii_required_by_compliance: false,
            })
            .await
            .unwrap();
        let got = api.lookup_gold_set(id).await.unwrap();
        assert_eq!(got.gold_set_id, id);
        assert!(matches!(got.status, GoldSetStatus::Draft));
    }

    #[tokio::test]
    async fn approve_endpoint_appends_ledger_row() {
        let api = api();
        let a = agent();
        let id = api
            .register_gold_set(RegisterGoldSetRequest {
                draft: draft(&a.id),
                pii_required_by_compliance: false,
            })
            .await
            .unwrap();
        let set = api
            .approve_gold_set(
                id,
                ApproveGoldSetRequest {
                    approvers: vec![op_sig(), op_sig()],
                },
            )
            .await
            .unwrap();
        assert!(matches!(set.status, GoldSetStatus::Approved));
        let history = api
            .ledger
            .history(&a.id, 10)
            .await
            .unwrap();
        assert!(history
            .iter()
            .any(|r| matches!(r.entry, GoldRunLedgerEntry::GoldSetApproved { .. })));
    }

    #[tokio::test]
    async fn run_endpoint_happy_path() {
        let api = api();
        let a = agent();
        let id = api
            .register_gold_set(RegisterGoldSetRequest {
                draft: draft(&a.id),
                pii_required_by_compliance: false,
            })
            .await
            .unwrap();
        let _ = api
            .approve_gold_set(
                id,
                ApproveGoldSetRequest {
                    approvers: vec![op_sig(), op_sig()],
                },
            )
            .await
            .unwrap();
        let report = api
            .run_gate(RunRequest {
                agent: a,
                gold_set_id: id,
                bypass: None,
            })
            .await
            .unwrap();
        assert!(report.permits_promote());
    }

    #[tokio::test]
    async fn run_endpoint_rejects_unapproved_set_without_bypass() {
        let api = api();
        let a = agent();
        let id = api
            .register_gold_set(RegisterGoldSetRequest {
                draft: draft(&a.id),
                pii_required_by_compliance: false,
            })
            .await
            .unwrap();
        let err = api
            .run_gate(RunRequest {
                agent: a,
                gold_set_id: id,
                bypass: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, GoldApiError::BadRequest { .. }));
    }

    #[tokio::test]
    async fn list_for_agent_returns_registered_sets() {
        let api = api();
        let a = agent();
        let _ = api
            .register_gold_set(RegisterGoldSetRequest {
                draft: draft(&a.id),
                pii_required_by_compliance: false,
            })
            .await
            .unwrap();
        let _ = api
            .register_gold_set(RegisterGoldSetRequest {
                draft: draft(&a.id),
                pii_required_by_compliance: false,
            })
            .await
            .unwrap();
        let all = api.list_gold_sets_for_agent(&a.id).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn lookup_unknown_gold_set_returns_not_found() {
        let api = api();
        let err = api.lookup_gold_set(GoldSetId::new()).await.unwrap_err();
        assert!(matches!(err, GoldApiError::NotFound));
    }

    #[tokio::test]
    async fn drift_sweep_returns_report() {
        let api = api();
        let rep = api
            .drift_sweep(DriftSweepRequest {
                agent_ids: Some(vec![NanoAgentId::new("a1").unwrap()]),
                max_age_days: 30,
            })
            .await
            .unwrap();
        assert_eq!(rep.entries.len(), 1);
    }

    #[test]
    fn debug_format_no_panic() {
        let api = api();
        let _ = format!("{api:?}");
    }

    #[test]
    fn registry_error_maps_to_api_error() {
        let e = GoldSetRegistryError::NotFound {
            gold_set_id: GoldSetId::new(),
        };
        assert!(matches!(GoldApiError::from(e), GoldApiError::NotFound));
    }
}
