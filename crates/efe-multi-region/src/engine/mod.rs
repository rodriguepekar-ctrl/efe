// SPDX-License-Identifier: UNLICENSED
//! **Sprint 9 — N16** : Multi-region replay execution engine.
//!
//! Architecture § 7.4.  Orchestrates the chain-shipment flow + per-op
//! comparison ; defers full re-dispatch to Sprint 10 (when the target
//! region has the production catalog loaded).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use efe_nano_sdk::{
    MatchOutcome, MatchPolicy, NanoAgentId, NanoAgentVersion, OpStatus, RegionCode, ReplayAggregate,
    ReplayError, ReplayMode, ReplayOpDiff, ReplayResult, ReplayToleranceMap, TenantId,
};
use uuid::Uuid;

use crate::metrics::N16Metrics;
use crate::shipment::{ChainShipmentClient, ShippedChain};
use crate::storage::{ReplayJob, ReplayStorage};

// ─────────────────────────────────────────────────────────────────────
//  ReplayCallContext
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : per-call replay context.  Pinned to the
/// recorded swarm so re-dispatch is reproducible.
#[derive(Debug, Clone)]
pub struct ReplayCallContext {
    pub tenant_id: TenantId,
    pub origin_region: RegionCode,
    pub target_region: RegionCode,
    pub swarm_request_id: Uuid,
    pub envelope_trace_id: Uuid,
    pub mode: ReplayMode,
    pub tolerance: ReplayToleranceMap,
    /// Virtual clock — agents requiring `now()` must read from here
    /// (architecture ADR-4 + G13).
    pub virtual_clock: SystemTime,
    /// Agent versions pinned from the swarm envelope.
    pub agent_versions: HashMap<NanoAgentId, NanoAgentVersion>,
    pub operator_id: Uuid,
    pub ticket_id: String,
    /// Per-swarm hard timeout (architecture I16).
    pub timeout: Duration,
}

impl ReplayCallContext {
    /// **N16** : Convenience for tests.
    pub fn for_job(job: &ReplayJob, virtual_clock: SystemTime) -> Self {
        Self {
            tenant_id: job.tenant_id,
            origin_region: job.origin_region,
            target_region: job.target_region,
            swarm_request_id: job.swarm_request_id,
            envelope_trace_id: Uuid::new_v4(),
            mode: job.mode,
            tolerance: job.tolerance_override.clone().unwrap_or_default(),
            virtual_clock,
            agent_versions: HashMap::new(),
            operator_id: job.operator_id,
            ticket_id: job.ticket_id.clone(),
            timeout: Duration::from_millis(u64::from(job.timeout_ms)),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  MultiRegionReplayEngine
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : the engine.  In Sprint 9 ship, it orchestrates
/// the shipment + comparison flow ; the actual re-dispatch step is
/// stubbed (returns the original outputs as the replay outputs).
/// Sprint 10 wires it to the real Sprint 6 `ReplayEngine` once
/// Postgres-backed chain storage is operational in the target region.
pub struct MultiRegionReplayEngine {
    shipment: Arc<dyn ChainShipmentClient>,
    storage: Arc<dyn ReplayStorage>,
    metrics: Option<Arc<N16Metrics>>,
    region: RegionCode,
}

impl std::fmt::Debug for MultiRegionReplayEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiRegionReplayEngine")
            .field("region", &self.region)
            .field("has_metrics", &self.metrics.is_some())
            .finish()
    }
}

impl MultiRegionReplayEngine {
    /// **N16** : Construct an engine.
    pub fn new(
        shipment: Arc<dyn ChainShipmentClient>,
        storage: Arc<dyn ReplayStorage>,
        region: RegionCode,
    ) -> Arc<Self> {
        Arc::new(Self {
            shipment,
            storage,
            metrics: None,
            region,
        })
    }

    /// **N16** : Construct an engine with metrics.
    pub fn with_metrics(
        shipment: Arc<dyn ChainShipmentClient>,
        storage: Arc<dyn ReplayStorage>,
        region: RegionCode,
        metrics: Arc<N16Metrics>,
    ) -> Arc<Self> {
        Arc::new(Self {
            shipment,
            storage,
            metrics: Some(metrics),
            region,
        })
    }

    /// **N16** : Region this engine operates in.
    pub fn region(&self) -> RegionCode {
        self.region
    }

    /// **N16** : Run a replay for the given job.
    pub async fn replay(&self, job: &ReplayJob) -> Result<ReplayResult, ReplayError> {
        let started_at = SystemTime::now();

        let chain = self
            .fetch_chain(job.origin_region, job.target_region, job.tenant_id, job.swarm_request_id)
            .await?;

        if let Some(m) = &self.metrics {
            let bytes: u64 = chain
                .trace_rows
                .iter()
                .map(|r| r.canonical_row_body.len() as u64)
                .sum::<u64>()
                + chain
                    .body_blobs
                    .iter()
                    .map(|b| b.ciphertext.len() as u64)
                    .sum::<u64>();
            m.record_shipment_bytes(chain.source_region, chain.target_region, bytes);
        }

        let tolerance = job.tolerance_override.clone().unwrap_or_default();
        let mut aggregate = ReplayAggregate::default();
        let mut per_op = HashMap::new();
        for row in &chain.trace_rows {
            let op_id = format!("op_{}", row.trace_id);
            let diff = compare_row_stub(&op_id, &tolerance);
            aggregate.record(diff.match_outcome);
            if let Some(m) = &self.metrics {
                m.tick_match_outcome(diff.match_outcome);
            }
            let _ = self
                .storage
                .insert_diff(job.job_id, job.tenant_id, diff.clone())
                .await;
            per_op.insert(op_id, diff);
        }
        aggregate.overall_outcome = aggregate.compute_outcome();

        let completed_at = SystemTime::now();
        Ok(ReplayResult {
            job_id: job.job_id,
            tenant_id: job.tenant_id,
            origin_region: job.origin_region,
            target_region: job.target_region,
            swarm_request_id: job.swarm_request_id,
            envelope_trace_id: chain.envelope_trace_id,
            mode: job.mode,
            per_op,
            aggregate,
            agent_versions: HashMap::new(),
            operator_id: job.operator_id,
            ticket_id: job.ticket_id.clone(),
            replay_started_at: started_at,
            replay_completed_at: completed_at,
            total_cost_cents: 0.0,
        })
    }

    async fn fetch_chain(
        &self,
        source: RegionCode,
        target: RegionCode,
        tenant: TenantId,
        swarm: Uuid,
    ) -> Result<ShippedChain, ReplayError> {
        let started = SystemTime::now();
        let handle = self
            .shipment
            .fetch_and_stage(source, target, tenant, swarm)
            .await
            .map_err(|e| match e {
                crate::shipment::ChainShipmentError::ChainNotFound { swarm_request_id, .. } => {
                    ReplayError::SwarmNotFound { swarm_request_id }
                }
                crate::shipment::ChainShipmentError::SourceUnavailable { region } => {
                    ReplayError::SourceRegionUnavailable { region }
                }
                crate::shipment::ChainShipmentError::SchemaTooNew {
                    source_version,
                    target_max,
                } => ReplayError::SchemaTooNew {
                    source_version,
                    target_max,
                },
                other => ReplayError::Internal {
                    message: format!("shipment failed : {other}"),
                },
            })?;
        let chain = self
            .shipment
            .verify_and_commit(&handle)
            .await
            .map_err(|e| match e {
                crate::shipment::ChainShipmentError::HeadHashMismatch => {
                    ReplayError::ChainBroken {
                        trace_id: handle.staging_id,
                    }
                }
                other => ReplayError::Internal {
                    message: format!("commit failed : {other}"),
                },
            })?;
        if let Some(m) = &self.metrics {
            if let Ok(d) = SystemTime::now().duration_since(started) {
                m.observe_shipment_duration(source, target, d.as_secs_f64());
            }
        }
        Ok(chain)
    }
}

/// **Sprint 9 — N16** : per-row comparison stub.  Sprint 10 will read
/// the canonical body, identify mode, and apply per-mode tolerance.
fn compare_row_stub(op_id: &str, _tolerance: &ReplayToleranceMap) -> ReplayOpDiff {
    ReplayOpDiff {
        op_id: op_id.to_string(),
        mode_label: "pure_rust".to_string(),
        match_outcome: MatchOutcome::Exact,
        original_status: OpStatus::Success,
        replay_status: OpStatus::Success,
        structural_diff: None,
        semantic_score: None,
        latency_ms_original: 0,
        latency_ms_replay: 0,
        cost_cents_original: 0.0,
        cost_cents_replay: 0.0,
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Per-mode comparison helpers
// ─────────────────────────────────────────────────────────────────────

/// **N16** : Compare two JSON outputs per a [`MatchPolicy`].
pub fn compare_outputs(
    original: &serde_json::Value,
    replay: &serde_json::Value,
    policy: &MatchPolicy,
) -> MatchOutcome {
    match policy {
        MatchPolicy::Exact => {
            if original == replay {
                MatchOutcome::Exact
            } else {
                MatchOutcome::Mismatch
            }
        }
        MatchPolicy::Structural { float_epsilon } => {
            if structural_match(original, replay, *float_epsilon) {
                MatchOutcome::Exact
            } else {
                MatchOutcome::Mismatch
            }
        }
        MatchPolicy::Semantic { min_score } => {
            if original == replay {
                MatchOutcome::Exact
            } else {
                MatchOutcome::Tolerated(*min_score)
            }
        }
    }
}

/// **N16** : Structural JSON equality with float ε.
pub fn structural_match(
    a: &serde_json::Value,
    b: &serde_json::Value,
    epsilon: f64,
) -> bool {
    use serde_json::Value;
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => {
            if let (Some(xf), Some(yf)) = (x.as_f64(), y.as_f64()) {
                (xf - yf).abs() <= epsilon
            } else {
                x == y
            }
        }
        (Value::Array(xs), Value::Array(ys)) => {
            xs.len() == ys.len()
                && xs.iter().zip(ys).all(|(a, b)| structural_match(a, b, epsilon))
        }
        (Value::Object(xs), Value::Object(ys)) => {
            xs.len() == ys.len()
                && xs.iter().all(|(k, v)| {
                    ys.get(k)
                        .is_some_and(|w| structural_match(v, w, epsilon))
                })
        }
        _ => false,
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shipment::{InMemoryChainShipmentClient, ShippedBodyBlob, ShippedTraceRow};
    use crate::storage::InMemoryReplayStorage;
    use efe_nano_sdk::{ReplayJobId, ReplayJobStatus};
    use serde_json::json;

    fn build_chain(source: RegionCode, target: RegionCode, tenant: TenantId) -> ShippedChain {
        let swarm = Uuid::new_v4();
        let env = Uuid::new_v4();
        let rows = vec![ShippedTraceRow {
            trace_id: Uuid::new_v4(),
            canonical_row_body: b"row-a".to_vec(),
            hmac: vec![0u8; 32],
            kms_key_arns: vec![],
        }];
        let bodies = vec![ShippedBodyBlob {
            trace_id: rows[0].trace_id,
            field: "full_input".into(),
            ciphertext: b"enc".to_vec(),
            kms_key_arn: "arn:test".into(),
        }];
        InMemoryChainShipmentClient::build_test_chain(
            source, target, tenant, swarm, env, rows, bodies,
        )
    }

    fn build_job(chain: &ShippedChain) -> ReplayJob {
        ReplayJob {
            job_id: ReplayJobId::new(),
            tenant_id: chain.tenant_id,
            swarm_request_id: chain.swarm_request_id,
            origin_region: chain.source_region,
            target_region: chain.target_region,
            status: ReplayJobStatus::Pending,
            actor: "alice".into(),
            ticket_id: "JIRA-1".into(),
            reason: "audit".into(),
            mode: ReplayMode::DispatchOnly,
            timeout_ms: 60_000,
            tolerance_override: None,
            operator_id: Uuid::new_v4(),
            operator_jwt_fp: "fp".into(),
            submitted_at: SystemTime::now(),
            started_at: None,
            completed_at: None,
        }
    }

    #[tokio::test]
    async fn replay_happy_path_returns_clean_result() {
        let storage = Arc::new(InMemoryReplayStorage::new());
        let shipment = InMemoryChainShipmentClient::new();
        let tenant = TenantId::new();
        let chain = build_chain(RegionCode::EuWest1, RegionCode::EuWest1, tenant);
        shipment.publish(chain.clone());
        let engine = MultiRegionReplayEngine::new(
            shipment.clone() as Arc<dyn ChainShipmentClient>,
            storage.clone() as Arc<dyn ReplayStorage>,
            RegionCode::EuWest1,
        );

        let job = build_job(&chain);
        storage.insert_job(job.clone()).await.unwrap();
        let result = engine.replay(&job).await.unwrap();
        assert_eq!(result.aggregate.total_ops, 1);
        assert_eq!(result.aggregate.exact, 1);
        assert_eq!(result.aggregate.mismatch, 0);
    }

    #[tokio::test]
    async fn replay_swarm_not_found_returns_swarm_not_found() {
        let storage = Arc::new(InMemoryReplayStorage::new());
        let shipment = InMemoryChainShipmentClient::new();
        let engine = MultiRegionReplayEngine::new(
            shipment as Arc<dyn ChainShipmentClient>,
            storage.clone() as Arc<dyn ReplayStorage>,
            RegionCode::EuWest1,
        );
        let job = ReplayJob {
            job_id: ReplayJobId::new(),
            tenant_id: TenantId::new(),
            swarm_request_id: Uuid::new_v4(),
            origin_region: RegionCode::EuWest1,
            target_region: RegionCode::EuWest1,
            status: ReplayJobStatus::Pending,
            actor: "x".into(),
            ticket_id: "y".into(),
            reason: "z".into(),
            mode: ReplayMode::DispatchOnly,
            timeout_ms: 1_000,
            tolerance_override: None,
            operator_id: Uuid::new_v4(),
            operator_jwt_fp: "fp".into(),
            submitted_at: SystemTime::now(),
            started_at: None,
            completed_at: None,
        };
        storage.insert_job(job.clone()).await.unwrap();
        let err = engine.replay(&job).await.unwrap_err();
        assert!(matches!(err, ReplayError::SwarmNotFound { .. }));
    }

    #[tokio::test]
    async fn replay_corrupted_chain_returns_chain_broken() {
        let storage = Arc::new(InMemoryReplayStorage::new());
        let shipment = InMemoryChainShipmentClient::new();
        let tenant = TenantId::new();
        let mut chain = build_chain(RegionCode::EuWest1, RegionCode::EuWest1, tenant);
        chain.head_hash = vec![0xFFu8; 32];
        shipment.publish(chain.clone());
        let engine = MultiRegionReplayEngine::new(
            shipment as Arc<dyn ChainShipmentClient>,
            storage.clone() as Arc<dyn ReplayStorage>,
            RegionCode::EuWest1,
        );
        let job = build_job(&chain);
        storage.insert_job(job.clone()).await.unwrap();
        let err = engine.replay(&job).await.unwrap_err();
        assert!(matches!(err, ReplayError::ChainBroken { .. }));
    }

    #[test]
    fn compare_exact_match_when_equal() {
        let a = json!({"x": 1});
        let b = json!({"x": 1});
        assert_eq!(compare_outputs(&a, &b, &MatchPolicy::Exact), MatchOutcome::Exact);
    }

    #[test]
    fn compare_exact_mismatch_when_different() {
        let a = json!({"x": 1});
        let b = json!({"x": 2});
        assert_eq!(compare_outputs(&a, &b, &MatchPolicy::Exact), MatchOutcome::Mismatch);
    }

    #[test]
    fn compare_structural_tolerates_float_epsilon() {
        let a = json!({"score": 0.123_456});
        let b = json!({"score": 0.123_457});
        assert_eq!(
            compare_outputs(&a, &b, &MatchPolicy::Structural { float_epsilon: 1e-5 }),
            MatchOutcome::Exact
        );
    }

    #[test]
    fn compare_structural_rejects_beyond_epsilon() {
        let a = json!({"score": 0.1});
        let b = json!({"score": 0.2});
        assert_eq!(
            compare_outputs(&a, &b, &MatchPolicy::Structural { float_epsilon: 1e-5 }),
            MatchOutcome::Mismatch
        );
    }

    #[test]
    fn structural_match_recursive_arrays() {
        let a = json!([1.0, 2.0, 3.0]);
        let b = json!([1.000_001, 2.0, 3.0]);
        assert!(structural_match(&a, &b, 1e-3));
    }

    #[test]
    fn structural_match_arrays_different_length_fails() {
        let a = json!([1, 2, 3]);
        let b = json!([1, 2]);
        assert!(!structural_match(&a, &b, 1e-5));
    }

    #[test]
    fn structural_match_objects_missing_key_fails() {
        let a = json!({"a": 1, "b": 2});
        let b = json!({"a": 1});
        assert!(!structural_match(&a, &b, 1e-5));
    }

    #[test]
    fn structural_match_nulls_and_bools() {
        assert!(structural_match(&json!(null), &json!(null), 0.0));
        assert!(structural_match(&json!(true), &json!(true), 0.0));
        assert!(!structural_match(&json!(true), &json!(false), 0.0));
    }

    #[test]
    fn structural_match_strings() {
        assert!(structural_match(&json!("ok"), &json!("ok"), 0.0));
        assert!(!structural_match(&json!("ok"), &json!("nope"), 0.0));
    }

    #[test]
    fn compare_semantic_equal_returns_exact() {
        let a = json!({"x": 1});
        let b = json!({"x": 1});
        assert_eq!(
            compare_outputs(&a, &b, &MatchPolicy::Semantic { min_score: 0.95 }),
            MatchOutcome::Exact
        );
    }

    #[test]
    fn compare_semantic_different_returns_tolerated() {
        let a = json!({"x": 1});
        let b = json!({"x": 2});
        let outcome = compare_outputs(&a, &b, &MatchPolicy::Semantic { min_score: 0.95 });
        assert!(matches!(outcome, MatchOutcome::Tolerated(_)));
    }

    #[test]
    fn replay_call_context_for_job_carries_tenant_and_mode() {
        let tenant = TenantId::new();
        let chain = build_chain(RegionCode::EuWest1, RegionCode::EuWest1, tenant);
        let job = build_job(&chain);
        let ctx = ReplayCallContext::for_job(&job, SystemTime::now());
        assert_eq!(ctx.tenant_id, job.tenant_id);
        assert_eq!(ctx.mode, job.mode);
    }
}
