// SPDX-License-Identifier: UNLICENSED
//! **Sprint 9 — N16** : Replay coordinator — operator front door.
//!
//! Architecture § 7.2.  The coordinator validates → enqueues → and
//! later (via the worker pump) executes a replay job.

use std::sync::Arc;
use std::time::SystemTime;

use efe_nano_sdk::{
    RegionCode, ReplayError, ReplayJobId, ReplayJobStatus, ReplayRequest, ReplayResult,
};

use crate::auth::{JwtValidator, OperatorClaims, ReplayScope};
use crate::engine::MultiRegionReplayEngine;
use crate::kms::{KmsError, KmsKeyResolver};
use crate::metrics::N16Metrics;
use crate::residency::ResidencyPolicyLookup;
use crate::shipment::ChainShipmentClient;
use crate::storage::{ReplayJob, ReplayLease, ReplayStorage, StorageError};

// ─────────────────────────────────────────────────────────────────────
//  ReplayCoordinator
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : Front door of the multi-region replay
/// subsystem.  Validates submissions, manages job lifecycle, and
/// dispatches work to the [`MultiRegionReplayEngine`].
pub struct ReplayCoordinator {
    jwt_validator: Arc<dyn JwtValidator>,
    residency: Arc<dyn ResidencyPolicyLookup>,
    kms: Arc<dyn KmsKeyResolver>,
    /// **N16** : Held by the coordinator for the worker's eventual
    /// per-job chain shipment.  Currently passed to the engine ; the
    /// coordinator's direct use lands in Sprint 10 (bulk shipment
    /// orchestration).  Field retained for forward compatibility.
    #[allow(dead_code)]
    shipment: Arc<dyn ChainShipmentClient>,
    storage: Arc<dyn ReplayStorage>,
    engine: Arc<MultiRegionReplayEngine>,
    metrics: Option<Arc<N16Metrics>>,
    /// The region this coordinator is running in (target region).
    region: RegionCode,
}

impl std::fmt::Debug for ReplayCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplayCoordinator")
            .field("region", &self.region)
            .field("has_metrics", &self.metrics.is_some())
            .finish()
    }
}

impl ReplayCoordinator {
    /// **N16** : Start a builder.
    pub fn builder() -> ReplayCoordinatorBuilder {
        ReplayCoordinatorBuilder::default()
    }

    /// **N16** : The region this coordinator is running in.
    pub fn region(&self) -> RegionCode {
        self.region
    }

    /// **N16** : Access the storage layer (used by tests + worker).
    pub fn storage(&self) -> &Arc<dyn ReplayStorage> {
        &self.storage
    }

    /// **N16** : Access the metrics surface (if installed).
    pub fn metrics(&self) -> Option<&Arc<N16Metrics>> {
        self.metrics.as_ref()
    }

    /// **N16** : Access the engine (for direct test invocation).
    pub fn engine(&self) -> &Arc<MultiRegionReplayEngine> {
        &self.engine
    }
}

// ─────────────────────────────────────────────────────────────────────
//  submit / status / result / cancel
// ─────────────────────────────────────────────────────────────────────

impl ReplayCoordinator {
    /// **N16** : Submit a replay request.  Returns a `ReplayJobId`
    /// on acceptance.  Validation order matches architecture I15.
    ///
    /// Validation steps :
    /// 1. JWT validation (`JwtValidator::validate`).
    /// 2. Scope check (`replay:execute`).
    /// 3. Tenant match (operator's JWT tenant_id == request.tenant_id).
    /// 4. Attribution check (`request.validate()`).
    /// 5. Residency check (target region permitted).
    /// 6. KMS access check (best-effort — Sprint 9 sample of common ARNs).
    /// 7. Lease acquisition (concurrency guard).
    /// 8. Insert job into storage.
    pub async fn submit(
        self: &Arc<Self>,
        request: ReplayRequest,
        jwt: &str,
    ) -> Result<ReplayJobId, ReplayError> {
        // (1) JWT validation.
        let claims = match self.jwt_validator.validate(jwt).await {
            Ok(c) => c,
            Err(e) => {
                // **N16 hardening D** : tick the rejection metric
                // even for unauthorized attempts so ops dashboards
                // surface auth-failure spikes.
                self.tick_request(request.target_region, false);
                return Err(ReplayError::Unauthorized {
                    reason: e.to_string(),
                });
            }
        };

        // (2) Scope check.
        if !claims.has_scope(ReplayScope::Execute) {
            self.tick_request(request.target_region, false);
            return Err(ReplayError::Unauthorized {
                reason: "missing scope replay:execute".into(),
            });
        }

        // (3) Tenant match.
        if claims.tenant_id != request.tenant_id {
            self.tick_request(request.target_region, false);
            return Err(ReplayError::CrossTenantForbidden);
        }

        // (4) Attribution + timeout validation.
        if let Err(err) = request.validate() {
            self.tick_request(request.target_region, false);
            return Err(err);
        }

        // (5) Residency check.
        let policy = self
            .residency
            .policy(request.tenant_id)
            .await
            .map_err(|e| ReplayError::Internal {
                message: format!("residency lookup error : {e}"),
            })?;
        if let Err(violation) = policy.validate(request.target_region) {
            if let Some(m) = &self.metrics {
                m.tick_residency_violation(request.target_region);
            }
            self.tick_request(request.target_region, false);
            return Err(violation.into());
        }

        // (6) KMS access check — best effort with no chain in hand
        // yet.  The full per-ARN check happens in the worker once the
        // chain is staged.  Here we just confirm the target region
        // can talk to KMS at all (defensive ; the `permissive`
        // implementation always passes).
        if let Err(err) = self
            .kms
            .check_access(request.target_region, "arn:probe")
            .await
        {
            // A probe failure is NOT a hard rejection at submit ;
            // we only reject if the underlying error indicates
            // backend down.  InsufficientPermissions on a probe ARN
            // is expected.
            if matches!(err, KmsError::Backend(_)) {
                self.tick_request(request.target_region, false);
                return Err(ReplayError::Internal {
                    message: format!("kms backend unavailable : {err}"),
                });
            }
        }

        // (7) Lease acquisition.
        let job_id = ReplayJobId::new();
        let lease_res = self
            .storage
            .acquire_lease(request.tenant_id, request.swarm_request_id, job_id)
            .await;
        if let Err(StorageError::LeaseConflict(existing)) = &lease_res {
            if let Some(m) = &self.metrics {
                m.tick_concurrent_collision();
            }
            self.tick_request(request.target_region, false);
            return Err(ReplayError::ConcurrentReplayInFlight {
                existing_job_id: *existing,
            });
        }
        let lease = lease_res.map_err(|e| ReplayError::Storage {
            message: e.to_string(),
        })?;

        // (8) Insert job.  **N16 hardening** : if the insert fails
        // after the lease was acquired, we MUST release the lease ;
        // otherwise the swarm becomes un-replayable for the full
        // lease TTL (30 minutes).
        let job = ReplayJob::from_request(
            &request,
            job_id,
            claims.operator_id,
            claims.jwt_fingerprint(),
        );
        if let Err(e) = self.storage.insert_job(job).await {
            let _ = self.storage.release_lease(lease).await;
            self.tick_request(request.target_region, false);
            return Err(ReplayError::Storage {
                message: e.to_string(),
            });
        }

        if let Some(m) = &self.metrics {
            m.tick_request(request.target_region, true);
            m.adjust_inflight(request.target_region, 1);
        }
        Ok(job_id)
    }

    fn tick_request(&self, target_region: RegionCode, accepted: bool) {
        if let Some(m) = &self.metrics {
            m.tick_request(target_region, accepted);
        }
    }

    /// **N16** : Read the current status of a job.  Enforces tenant
    /// isolation via the operator's JWT.
    pub async fn status(
        &self,
        job_id: ReplayJobId,
        jwt: &str,
    ) -> Result<ReplayJobStatus, ReplayError> {
        let claims = self.authorize_read(jwt).await?;
        let job = self
            .storage
            .get_job(job_id, claims.tenant_id)
            .await
            .map_err(map_storage_error)?;
        Ok(job.status)
    }

    /// **N16** : Fetch the final result.
    pub async fn result(
        &self,
        job_id: ReplayJobId,
        jwt: &str,
    ) -> Result<ReplayResult, ReplayError> {
        let claims = self.authorize_read(jwt).await?;
        self.storage
            .get_result(job_id, claims.tenant_id)
            .await
            .map_err(map_storage_error)
    }

    /// **N16** : Cancel an in-flight job (marks status = Cancelled).
    pub async fn cancel(&self, job_id: ReplayJobId, jwt: &str) -> Result<(), ReplayError> {
        let claims = self.authorize_execute(jwt).await?;
        let job = self
            .storage
            .get_job(job_id, claims.tenant_id)
            .await
            .map_err(map_storage_error)?;
        if job.status.is_terminal() {
            return Err(ReplayError::ConcurrentReplayInFlight {
                existing_job_id: job_id,
            });
        }
        self.storage
            .update_status(job_id, claims.tenant_id, ReplayJobStatus::Cancelled)
            .await
            .map_err(map_storage_error)?;
        // Release the lease so a future replay of the same swarm can proceed.
        let lease = ReplayLease {
            tenant_id: claims.tenant_id,
            swarm_request_id: job.swarm_request_id,
            job_id,
            leased_at: SystemTime::now(),
        };
        let _ = self.storage.release_lease(lease).await;
        if let Some(m) = &self.metrics {
            m.adjust_inflight(job.target_region, -1);
        }
        Ok(())
    }

    /// **N16** : List jobs for the operator's tenant.
    pub async fn list_jobs(
        &self,
        jwt: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ReplayJob>, ReplayError> {
        let claims = self.authorize_read(jwt).await?;
        self.storage
            .list_jobs_for_tenant(claims.tenant_id, limit, offset)
            .await
            .map_err(map_storage_error)
    }

    async fn authorize_read(&self, jwt: &str) -> Result<OperatorClaims, ReplayError> {
        let claims = self
            .jwt_validator
            .validate(jwt)
            .await
            .map_err(|e| ReplayError::Unauthorized {
                reason: e.to_string(),
            })?;
        if !claims.has_scope(ReplayScope::Read) && !claims.has_scope(ReplayScope::Execute) {
            return Err(ReplayError::Unauthorized {
                reason: "missing scope replay:read or replay:execute".into(),
            });
        }
        Ok(claims)
    }

    async fn authorize_execute(&self, jwt: &str) -> Result<OperatorClaims, ReplayError> {
        let claims = self
            .jwt_validator
            .validate(jwt)
            .await
            .map_err(|e| ReplayError::Unauthorized {
                reason: e.to_string(),
            })?;
        if !claims.has_scope(ReplayScope::Execute) {
            return Err(ReplayError::Unauthorized {
                reason: "missing scope replay:execute".into(),
            });
        }
        Ok(claims)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Worker — runs pending jobs (single-tick API for tests)
// ─────────────────────────────────────────────────────────────────────

impl ReplayCoordinator {
    /// **N16** : Run a single worker pump cycle.  Fetches up to `limit`
    /// pending jobs and runs them sequentially.  Returns the number
    /// of jobs processed.
    ///
    /// Production deployments run this in a background `tokio::task`
    /// in a loop with a sleep.  Tests call it directly for
    /// determinism.
    pub async fn pump_pending(self: &Arc<Self>, limit: usize) -> usize {
        let pending = match self.storage.fetch_pending(limit).await {
            Ok(jobs) => jobs,
            Err(_) => return 0,
        };
        let mut processed = 0;
        for job in pending {
            if self.run_one(job).await.is_ok() {
                processed += 1;
            }
        }
        processed
    }

    async fn run_one(self: &Arc<Self>, job: ReplayJob) -> Result<(), ReplayError> {
        let tenant = job.tenant_id;
        let job_id = job.job_id;
        let target_region = job.target_region;
        let lease = ReplayLease {
            tenant_id: tenant,
            swarm_request_id: job.swarm_request_id,
            job_id,
            leased_at: job.submitted_at,
        };

        // **N16 hardening B** : the lease MUST be released regardless
        // of how the rest of this function exits.  We model that with
        // an explicit cleanup at every exit point below ; a Drop
        // guard would need to block on async release which is not
        // possible inside Drop.

        // Mark in-progress.  Failure here is non-fatal — we still
        // attempt the replay ; status might be stale.
        let _ = self
            .storage
            .update_status(
                job_id,
                tenant,
                ReplayJobStatus::InProgress { progress_pct: 0 },
            )
            .await;

        let engine_start = SystemTime::now();
        let result = self.engine.replay(&job).await;

        // **N16 hardening C** : before overwriting the status, check
        // whether an operator cancellation slipped in while the
        // engine was running.  If the status is `Cancelled`, respect
        // it ; do not clobber.  (TOCTOU window remains microseconds ;
        // a true compare-and-swap update lands in Sprint 10.)
        let cancelled_meanwhile = matches!(
            self.storage.get_job(job_id, tenant).await,
            Ok(j) if matches!(j.status, ReplayJobStatus::Cancelled)
        );

        let outcome: Result<(), ReplayError> = match result {
            Ok(replay_result) => {
                let mismatches = replay_result.aggregate.mismatch;
                let new_status = if mismatches == 0 {
                    ReplayJobStatus::Completed {
                        result_id: replay_result.envelope_trace_id,
                    }
                } else {
                    ReplayJobStatus::CompletedWithRegression {
                        result_id: replay_result.envelope_trace_id,
                        mismatch_count: mismatches,
                    }
                };
                // Always persist the result row — diffs are useful
                // even if the operator cancelled mid-flight.
                let store_res = self.storage.insert_result(replay_result.clone()).await;
                if !cancelled_meanwhile {
                    let _ = self.storage.update_status(job_id, tenant, new_status).await;
                }
                if let Some(m) = &self.metrics {
                    if let Ok(d) = SystemTime::now().duration_since(engine_start) {
                        m.observe_replay_duration(job.mode, d.as_secs_f64());
                    }
                    m.record_operator_cost(replay_result.total_cost_cents);
                }
                store_res.map(|_| ()).map_err(|e| ReplayError::Storage {
                    message: e.to_string(),
                })
            }
            Err(err) => {
                if !cancelled_meanwhile {
                    let _ = self
                        .storage
                        .update_status(
                            job_id,
                            tenant,
                            ReplayJobStatus::Failed {
                                error_label: err.label().to_string(),
                            },
                        )
                        .await;
                }
                Err(err)
            }
        };

        // **N16 hardening B (cont.)** : decrement inflight gauge +
        // release the lease, unconditionally.  Both succeed and fail
        // paths funnel here.
        if let Some(m) = &self.metrics {
            m.adjust_inflight(target_region, -1);
        }
        let _ = self.storage.release_lease(lease).await;

        outcome
    }
}

// ─────────────────────────────────────────────────────────────────────
//  ReplayCoordinatorBuilder
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : Builder for [`ReplayCoordinator`].
#[derive(Default)]
pub struct ReplayCoordinatorBuilder {
    jwt_validator: Option<Arc<dyn JwtValidator>>,
    residency: Option<Arc<dyn ResidencyPolicyLookup>>,
    kms: Option<Arc<dyn KmsKeyResolver>>,
    shipment: Option<Arc<dyn ChainShipmentClient>>,
    storage: Option<Arc<dyn ReplayStorage>>,
    engine: Option<Arc<MultiRegionReplayEngine>>,
    metrics: Option<Arc<N16Metrics>>,
    region: Option<RegionCode>,
}

impl ReplayCoordinatorBuilder {
    pub fn with_jwt_validator(mut self, v: Arc<dyn JwtValidator>) -> Self {
        self.jwt_validator = Some(v);
        self
    }
    pub fn with_residency(mut self, r: Arc<dyn ResidencyPolicyLookup>) -> Self {
        self.residency = Some(r);
        self
    }
    pub fn with_kms(mut self, k: Arc<dyn KmsKeyResolver>) -> Self {
        self.kms = Some(k);
        self
    }
    pub fn with_shipment(mut self, s: Arc<dyn ChainShipmentClient>) -> Self {
        self.shipment = Some(s);
        self
    }
    pub fn with_storage(mut self, s: Arc<dyn ReplayStorage>) -> Self {
        self.storage = Some(s);
        self
    }
    pub fn with_engine(mut self, e: Arc<MultiRegionReplayEngine>) -> Self {
        self.engine = Some(e);
        self
    }
    pub fn with_metrics(mut self, m: Arc<N16Metrics>) -> Self {
        self.metrics = Some(m);
        self
    }
    pub fn with_region(mut self, r: RegionCode) -> Self {
        self.region = Some(r);
        self
    }

    /// **N16** : Finalize.  Panics if any required component is missing.
    pub fn build(self) -> Arc<ReplayCoordinator> {
        Arc::new(ReplayCoordinator {
            jwt_validator: self.jwt_validator.expect("jwt_validator required"),
            residency: self.residency.expect("residency required"),
            kms: self.kms.expect("kms required"),
            shipment: self.shipment.expect("shipment required"),
            storage: self.storage.expect("storage required"),
            engine: self.engine.expect("engine required"),
            metrics: self.metrics,
            region: self.region.unwrap_or(RegionCode::UsEast1),
        })
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Error helpers
// ─────────────────────────────────────────────────────────────────────

fn map_storage_error(err: StorageError) -> ReplayError {
    match err {
        StorageError::JobNotFound(id) => ReplayError::JobNotFound { job_id: id },
        StorageError::ResultNotFound(id) => ReplayError::JobNotFound { job_id: id },
        StorageError::CrossTenant { .. } => ReplayError::CrossTenantForbidden,
        other => ReplayError::Storage {
            message: other.to_string(),
        },
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests — submit + status + result + worker pump
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{Hs256JwtValidator, OperatorClaims};
    use crate::kms::InMemoryKmsKeyResolver;
    use crate::residency::InMemoryResidencyPolicyLookup;
    use crate::shipment::{InMemoryChainShipmentClient, ShippedBodyBlob, ShippedTraceRow};
    use crate::storage::InMemoryReplayStorage;
    use efe_nano_sdk::{
        ReplayMode, ReplayRequest, ResidencyPolicy, TenantId, DEFAULT_REPLAY_TIMEOUT_MS,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    fn now_seconds() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    /// Wire up a complete coordinator + dependencies for E2E tests.
    fn build_coord(
        tenant: TenantId,
        target: RegionCode,
        published_chain: Option<crate::shipment::ShippedChain>,
    ) -> (Arc<ReplayCoordinator>, Hs256JwtValidator, OperatorClaims, Arc<InMemoryReplayStorage>) {
        let secret = [9u8; 32];
        let validator = Hs256JwtValidator::new(secret).with_expected_issuer("efe-operator-idp");

        let residency = Arc::new(InMemoryResidencyPolicyLookup::default());
        let kms = Arc::new(InMemoryKmsKeyResolver::permissive());
        let shipment = InMemoryChainShipmentClient::new();
        if let Some(ch) = published_chain {
            shipment.publish(ch);
        }
        let storage = Arc::new(InMemoryReplayStorage::new());
        let engine = MultiRegionReplayEngine::new(
            shipment.clone() as Arc<dyn ChainShipmentClient>,
            storage.clone() as Arc<dyn ReplayStorage>,
            target,
        );
        let coord = ReplayCoordinator::builder()
            .with_jwt_validator(Arc::new(
                Hs256JwtValidator::new(secret).with_expected_issuer("efe-operator-idp"),
            ) as Arc<dyn JwtValidator>)
            .with_residency(residency.clone() as Arc<dyn ResidencyPolicyLookup>)
            .with_kms(kms.clone() as Arc<dyn KmsKeyResolver>)
            .with_shipment(shipment.clone() as Arc<dyn ChainShipmentClient>)
            .with_storage(storage.clone() as Arc<dyn ReplayStorage>)
            .with_engine(engine)
            .with_metrics(N16Metrics::new())
            .with_region(target)
            .build();
        let claims = OperatorClaims {
            iss: "efe-operator-idp".into(),
            sub: "alice@efe.com".into(),
            operator_id: Uuid::new_v4(),
            tenant_id: tenant,
            scope: vec!["replay:execute".into(), "replay:read".into()],
            ticket_id: "JIRA-123".into(),
            iat: now_seconds() - 60,
            exp: now_seconds() + 3_600,
            jti: "test-jti".into(),
        };
        (coord, validator, claims, storage)
    }

    fn build_chain(source: RegionCode, target: RegionCode, tenant: TenantId, swarm: Uuid) -> crate::shipment::ShippedChain {
        let env = Uuid::new_v4();
        let rows = vec![ShippedTraceRow {
            trace_id: Uuid::new_v4(),
            canonical_row_body: b"row".to_vec(),
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

    fn build_request(tenant: TenantId, swarm: Uuid, target: RegionCode) -> ReplayRequest {
        ReplayRequest {
            tenant_id: tenant,
            swarm_request_id: swarm,
            origin_region: target,
            target_region: target,
            actor: "alice".into(),
            ticket_id: "JIRA-123".into(),
            reason: "audit".into(),
            timeout_ms: DEFAULT_REPLAY_TIMEOUT_MS,
            mode: ReplayMode::DispatchOnly,
            tolerance_override: None,
        }
    }

    use uuid::Uuid;

    #[tokio::test]
    async fn submit_happy_path_returns_job_id() {
        let tenant = TenantId::new();
        let swarm = Uuid::new_v4();
        let chain = build_chain(RegionCode::EuWest1, RegionCode::EuWest1, tenant, swarm);
        let (coord, v, claims, _storage) =
            build_coord(tenant, RegionCode::EuWest1, Some(chain));
        let token = v.sign(&claims).unwrap();
        let req = build_request(tenant, swarm, RegionCode::EuWest1);
        let job_id = coord.submit(req, &token).await.unwrap();
        let status = coord.status(job_id, &token).await.unwrap();
        assert!(matches!(status, ReplayJobStatus::Pending));
    }

    #[tokio::test]
    async fn submit_rejects_missing_attribution() {
        let tenant = TenantId::new();
        let (coord, v, claims, _) = build_coord(tenant, RegionCode::EuWest1, None);
        let token = v.sign(&claims).unwrap();
        let mut req = build_request(tenant, Uuid::new_v4(), RegionCode::EuWest1);
        req.actor = "".into();
        let err = coord.submit(req, &token).await.unwrap_err();
        assert!(matches!(err, ReplayError::AttributionInvalid));
    }

    #[tokio::test]
    async fn submit_rejects_residency_violation() {
        let tenant = TenantId::new();
        let (coord, _v, claims, _) = build_coord(tenant, RegionCode::UsEast1, None);
        // Pin EU-only on the tenant.
        let res = coord
            .residency
            .clone();
        let _ = res; // residency is Arc<dyn> ; we need a concrete handle
        // Re-build with explicit residency.
        let residency = Arc::new(InMemoryResidencyPolicyLookup::default());
        residency.set_policy(tenant, ResidencyPolicy::EuOnly);
        let secret = [9u8; 32];
        let shipment = InMemoryChainShipmentClient::new();
        let storage = Arc::new(InMemoryReplayStorage::new());
        let engine = MultiRegionReplayEngine::new(
            shipment.clone() as Arc<dyn ChainShipmentClient>,
            storage.clone() as Arc<dyn ReplayStorage>,
            RegionCode::UsEast1,
        );
        let coord2 = ReplayCoordinator::builder()
            .with_jwt_validator(Arc::new(
                Hs256JwtValidator::new(secret).with_expected_issuer("efe-operator-idp"),
            ) as Arc<dyn JwtValidator>)
            .with_residency(residency as Arc<dyn ResidencyPolicyLookup>)
            .with_kms(Arc::new(InMemoryKmsKeyResolver::permissive())
                as Arc<dyn KmsKeyResolver>)
            .with_shipment(shipment as Arc<dyn ChainShipmentClient>)
            .with_storage(storage as Arc<dyn ReplayStorage>)
            .with_engine(engine)
            .with_metrics(N16Metrics::new())
            .with_region(RegionCode::UsEast1)
            .build();
        let token = Hs256JwtValidator::new(secret).sign(&claims).unwrap();
        let req = build_request(tenant, Uuid::new_v4(), RegionCode::UsEast1);
        let err = coord2.submit(req, &token).await.unwrap_err();
        assert!(matches!(err, ReplayError::ResidencyViolation { .. }));
    }

    #[tokio::test]
    async fn submit_rejects_invalid_jwt() {
        let tenant = TenantId::new();
        let (coord, _v, _claims, _) = build_coord(tenant, RegionCode::EuWest1, None);
        let req = build_request(tenant, Uuid::new_v4(), RegionCode::EuWest1);
        let err = coord.submit(req, "not-a-jwt").await.unwrap_err();
        assert!(matches!(err, ReplayError::Unauthorized { .. }));
    }

    #[tokio::test]
    async fn submit_rejects_cross_tenant() {
        let tenant_a = TenantId::new();
        let tenant_b = TenantId::new();
        let (coord, v, mut claims, _) = build_coord(tenant_a, RegionCode::EuWest1, None);
        claims.tenant_id = tenant_a;
        let token = v.sign(&claims).unwrap();
        // Operator JWT says tenant_a, request says tenant_b.
        let req = build_request(tenant_b, Uuid::new_v4(), RegionCode::EuWest1);
        let err = coord.submit(req, &token).await.unwrap_err();
        assert!(matches!(err, ReplayError::CrossTenantForbidden));
    }

    #[tokio::test]
    async fn concurrent_submit_returns_409_conflict() {
        let tenant = TenantId::new();
        let swarm = Uuid::new_v4();
        let chain = build_chain(RegionCode::EuWest1, RegionCode::EuWest1, tenant, swarm);
        let (coord, v, claims, _) =
            build_coord(tenant, RegionCode::EuWest1, Some(chain));
        let token = v.sign(&claims).unwrap();
        let req = build_request(tenant, swarm, RegionCode::EuWest1);
        let id1 = coord.submit(req.clone(), &token).await.unwrap();
        let err = coord.submit(req, &token).await.unwrap_err();
        assert!(matches!(
            err,
            ReplayError::ConcurrentReplayInFlight { existing_job_id } if existing_job_id == id1
        ));
    }

    #[tokio::test]
    async fn pump_pending_runs_one_job_to_completion() {
        let tenant = TenantId::new();
        let swarm = Uuid::new_v4();
        let chain = build_chain(RegionCode::EuWest1, RegionCode::EuWest1, tenant, swarm);
        let (coord, v, claims, _) =
            build_coord(tenant, RegionCode::EuWest1, Some(chain));
        let token = v.sign(&claims).unwrap();
        let req = build_request(tenant, swarm, RegionCode::EuWest1);
        let id = coord.submit(req, &token).await.unwrap();
        let n = coord.pump_pending(10).await;
        assert_eq!(n, 1);
        let status = coord.status(id, &token).await.unwrap();
        assert!(matches!(status, ReplayJobStatus::Completed { .. }));
        let result = coord.result(id, &token).await.unwrap();
        assert_eq!(result.aggregate.exact, 1);
    }

    #[tokio::test]
    async fn cancel_marks_job_cancelled_and_releases_lease() {
        let tenant = TenantId::new();
        let swarm = Uuid::new_v4();
        let chain = build_chain(RegionCode::EuWest1, RegionCode::EuWest1, tenant, swarm);
        let (coord, v, claims, _) =
            build_coord(tenant, RegionCode::EuWest1, Some(chain.clone()));
        let token = v.sign(&claims).unwrap();
        let req = build_request(tenant, swarm, RegionCode::EuWest1);
        let id = coord.submit(req.clone(), &token).await.unwrap();
        coord.cancel(id, &token).await.unwrap();
        let status = coord.status(id, &token).await.unwrap();
        assert!(matches!(status, ReplayJobStatus::Cancelled));
        // Re-submit should succeed (lease released).
        let id2 = coord.submit(req, &token).await.unwrap();
        assert_ne!(id, id2);
    }

    #[tokio::test]
    async fn status_cross_tenant_forbidden() {
        let tenant_a = TenantId::new();
        let tenant_b = TenantId::new();
        let swarm = Uuid::new_v4();
        let chain = build_chain(RegionCode::EuWest1, RegionCode::EuWest1, tenant_a, swarm);
        let (coord, v, claims_a, _) =
            build_coord(tenant_a, RegionCode::EuWest1, Some(chain));
        let token_a = v.sign(&claims_a).unwrap();
        let req = build_request(tenant_a, swarm, RegionCode::EuWest1);
        let id = coord.submit(req, &token_a).await.unwrap();
        // Now build a JWT for tenant_b but try to read tenant_a's job.
        let claims_b = OperatorClaims {
            iss: "efe-operator-idp".into(),
            sub: "mallory".into(),
            operator_id: Uuid::new_v4(),
            tenant_id: tenant_b,
            scope: vec!["replay:read".into()],
            ticket_id: "JIRA-2".into(),
            iat: now_seconds() - 30,
            exp: now_seconds() + 3_600,
            jti: "mallory-jti".into(),
        };
        let token_b = v.sign(&claims_b).unwrap();
        let err = coord.status(id, &token_b).await.unwrap_err();
        assert!(matches!(err, ReplayError::CrossTenantForbidden));
    }

    #[tokio::test]
    async fn submit_without_execute_scope_rejected() {
        let tenant = TenantId::new();
        let (coord, v, mut claims, _) = build_coord(tenant, RegionCode::EuWest1, None);
        claims.scope = vec!["replay:read".into()]; // no execute
        let token = v.sign(&claims).unwrap();
        let req = build_request(tenant, Uuid::new_v4(), RegionCode::EuWest1);
        let err = coord.submit(req, &token).await.unwrap_err();
        assert!(matches!(err, ReplayError::Unauthorized { .. }));
    }

    #[tokio::test]
    async fn list_jobs_filters_to_tenant() {
        let tenant = TenantId::new();
        let swarm = Uuid::new_v4();
        let chain = build_chain(RegionCode::EuWest1, RegionCode::EuWest1, tenant, swarm);
        let (coord, v, claims, _) =
            build_coord(tenant, RegionCode::EuWest1, Some(chain));
        let token = v.sign(&claims).unwrap();
        let req = build_request(tenant, swarm, RegionCode::EuWest1);
        coord.submit(req, &token).await.unwrap();
        let jobs = coord.list_jobs(&token, 50, 0).await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].tenant_id, tenant);
    }

    #[tokio::test]
    async fn debug_format_does_not_panic() {
        let tenant = TenantId::new();
        let (coord, _, _, _) = build_coord(tenant, RegionCode::EuWest1, None);
        let _ = format!("{coord:?}");
    }

    // ─────────────────────────────────────────────────────────────────
    //  Post-N16-audit hardening tests
    // ─────────────────────────────────────────────────────────────────

    /// **Bug B regression test** : when `engine.replay` fails the
    /// lease MUST be released so the operator can re-submit a
    /// corrected request.  Pre-fix, the lease leaked for 30 min.
    #[tokio::test]
    async fn engine_replay_failure_releases_lease() {
        let tenant = TenantId::new();
        // Don't publish any chain → engine.replay returns SwarmNotFound.
        let (coord, v, claims, _) = build_coord(tenant, RegionCode::EuWest1, None);
        let token = v.sign(&claims).unwrap();
        let swarm = Uuid::new_v4();
        let req = build_request(tenant, swarm, RegionCode::EuWest1);
        let id1 = coord.submit(req.clone(), &token).await.unwrap();
        let processed = coord.pump_pending(10).await;
        // Pump returns processed = jobs that returned Ok ; SwarmNotFound returns Err.
        assert_eq!(processed, 0);
        // Status should be Failed.
        let status = coord.status(id1, &token).await.unwrap();
        assert!(matches!(status, ReplayJobStatus::Failed { .. }));
        // **Critical** : a second submit for the SAME swarm must succeed
        // because the lease was released despite the failure.
        let id2 = coord.submit(req, &token).await.unwrap();
        assert_ne!(id1, id2);
    }

    /// **Bug C regression test** : when the operator cancels while
    /// the engine is running, the final update MUST NOT overwrite
    /// the Cancelled status with Completed.
    #[tokio::test]
    async fn cancel_status_not_overwritten_by_completion() {
        let tenant = TenantId::new();
        let swarm = Uuid::new_v4();
        let chain = build_chain(RegionCode::EuWest1, RegionCode::EuWest1, tenant, swarm);
        let (coord, v, claims, storage) =
            build_coord(tenant, RegionCode::EuWest1, Some(chain));
        let token = v.sign(&claims).unwrap();
        let req = build_request(tenant, swarm, RegionCode::EuWest1);
        let id = coord.submit(req, &token).await.unwrap();

        // Simulate cancel happening WHILE replay is mid-flight by
        // pre-setting the status to Cancelled in storage.
        storage
            .update_status(id, tenant, ReplayJobStatus::Cancelled)
            .await
            .unwrap();

        // Pump — engine will succeed (chain is published), but the
        // run_one final-update step must respect the Cancelled
        // status set by `cancel()`.
        let _ = coord.pump_pending(10).await;

        let status = coord.status(id, &token).await.unwrap();
        assert!(matches!(status, ReplayJobStatus::Cancelled));
    }

    /// **Bug A regression test** : if `insert_job` fails after
    /// `acquire_lease`, the lease must be released.
    ///
    /// We simulate this via a storage decorator that fails
    /// `insert_job` once.
    #[tokio::test]
    async fn insert_job_failure_releases_lease() {
        use crate::storage::{ReplayJob, StorageError};

        use std::sync::atomic::{AtomicBool, Ordering};
        struct OneShotFailStorage {
            inner: Arc<InMemoryReplayStorage>,
            fail_next: AtomicBool,
        }
        #[async_trait::async_trait]
        impl ReplayStorage for OneShotFailStorage {
            async fn insert_job(&self, job: ReplayJob) -> Result<(), StorageError> {
                if self.fail_next.swap(false, Ordering::SeqCst) {
                    return Err(StorageError::Backend("simulated".into()));
                }
                self.inner.insert_job(job).await
            }
            async fn get_job(
                &self,
                job_id: ReplayJobId,
                tenant: TenantId,
            ) -> Result<ReplayJob, StorageError> {
                self.inner.get_job(job_id, tenant).await
            }
            async fn update_status(
                &self,
                job_id: ReplayJobId,
                tenant: TenantId,
                status: ReplayJobStatus,
            ) -> Result<(), StorageError> {
                self.inner.update_status(job_id, tenant, status).await
            }
            async fn fetch_pending(
                &self,
                limit: usize,
            ) -> Result<Vec<ReplayJob>, StorageError> {
                self.inner.fetch_pending(limit).await
            }
            async fn insert_result(&self, result: ReplayResult) -> Result<(), StorageError> {
                self.inner.insert_result(result).await
            }
            async fn get_result(
                &self,
                job_id: ReplayJobId,
                tenant: TenantId,
            ) -> Result<ReplayResult, StorageError> {
                self.inner.get_result(job_id, tenant).await
            }
            async fn insert_diff(
                &self,
                job_id: ReplayJobId,
                tenant: TenantId,
                diff: efe_nano_sdk::ReplayOpDiff,
            ) -> Result<(), StorageError> {
                self.inner.insert_diff(job_id, tenant, diff).await
            }
            async fn acquire_lease(
                &self,
                tenant: TenantId,
                swarm: Uuid,
                job_id: ReplayJobId,
            ) -> Result<ReplayLease, StorageError> {
                self.inner.acquire_lease(tenant, swarm, job_id).await
            }
            async fn release_lease(&self, lease: ReplayLease) -> Result<(), StorageError> {
                self.inner.release_lease(lease).await
            }
            async fn list_jobs_for_tenant(
                &self,
                tenant: TenantId,
                limit: usize,
                offset: usize,
            ) -> Result<Vec<ReplayJob>, StorageError> {
                self.inner.list_jobs_for_tenant(tenant, limit, offset).await
            }
        }

        let tenant = TenantId::new();
        let inner_storage = Arc::new(InMemoryReplayStorage::new());
        let storage: Arc<dyn ReplayStorage> = Arc::new(OneShotFailStorage {
            inner: inner_storage.clone(),
            fail_next: AtomicBool::new(true),
        });
        let secret = [9u8; 32];
        let shipment = InMemoryChainShipmentClient::new();
        let chain = build_chain(RegionCode::EuWest1, RegionCode::EuWest1, tenant, Uuid::new_v4());
        let swarm = chain.swarm_request_id;
        shipment.publish(chain);
        let engine = MultiRegionReplayEngine::new(
            shipment.clone() as Arc<dyn ChainShipmentClient>,
            storage.clone(),
            RegionCode::EuWest1,
        );
        let coord = ReplayCoordinator::builder()
            .with_jwt_validator(Arc::new(
                Hs256JwtValidator::new(secret).with_expected_issuer("efe-operator-idp"),
            ) as Arc<dyn JwtValidator>)
            .with_residency(Arc::new(InMemoryResidencyPolicyLookup::default())
                as Arc<dyn ResidencyPolicyLookup>)
            .with_kms(Arc::new(InMemoryKmsKeyResolver::permissive())
                as Arc<dyn KmsKeyResolver>)
            .with_shipment(shipment as Arc<dyn ChainShipmentClient>)
            .with_storage(storage)
            .with_engine(engine)
            .with_metrics(N16Metrics::new())
            .with_region(RegionCode::EuWest1)
            .build();
        let claims = OperatorClaims {
            iss: "efe-operator-idp".into(),
            sub: "alice@efe.com".into(),
            operator_id: Uuid::new_v4(),
            tenant_id: tenant,
            scope: vec!["replay:execute".into(), "replay:read".into()],
            ticket_id: "JIRA-1".into(),
            iat: now_seconds() - 60,
            exp: now_seconds() + 3_600,
            jti: "test".into(),
        };
        let token = Hs256JwtValidator::new(secret)
            .with_expected_issuer("efe-operator-idp")
            .sign(&claims)
            .unwrap();

        let req = build_request(tenant, swarm, RegionCode::EuWest1);
        // First submit : insert_job fails (one-shot).  Lease should
        // be released.
        let err1 = coord.submit(req.clone(), &token).await.unwrap_err();
        assert!(matches!(err1, ReplayError::Storage { .. }));
        // The lease MUST be free now — second submit should succeed.
        let id2 = coord.submit(req, &token).await.unwrap();
        let status = coord.status(id2, &token).await.unwrap();
        assert!(matches!(status, ReplayJobStatus::Pending));
        // Verify the inner storage shows no leftover lease for the failed attempt.
        assert_eq!(inner_storage.lease_count(), 1);
    }

    /// **Defensive** : pump_pending followed by pump_pending again
    /// must not re-process the same job.  Covers the InProgress
    /// idempotency.
    #[tokio::test]
    async fn pump_pending_does_not_double_process() {
        let tenant = TenantId::new();
        let swarm = Uuid::new_v4();
        let chain = build_chain(RegionCode::EuWest1, RegionCode::EuWest1, tenant, swarm);
        let (coord, v, claims, _) =
            build_coord(tenant, RegionCode::EuWest1, Some(chain));
        let token = v.sign(&claims).unwrap();
        let req = build_request(tenant, swarm, RegionCode::EuWest1);
        coord.submit(req, &token).await.unwrap();
        let n1 = coord.pump_pending(10).await;
        let n2 = coord.pump_pending(10).await;
        assert_eq!(n1, 1);
        assert_eq!(n2, 0); // job already terminal → not re-processed
    }

    /// **Defensive** : after a successful replay, the lease is
    /// released so the same swarm can be replayed again.
    #[tokio::test]
    async fn lease_released_after_successful_replay() {
        let tenant = TenantId::new();
        let swarm = Uuid::new_v4();
        let chain = build_chain(RegionCode::EuWest1, RegionCode::EuWest1, tenant, swarm);
        let (coord, v, claims, _) =
            build_coord(tenant, RegionCode::EuWest1, Some(chain));
        let token = v.sign(&claims).unwrap();
        let req = build_request(tenant, swarm, RegionCode::EuWest1);
        let _id1 = coord.submit(req.clone(), &token).await.unwrap();
        coord.pump_pending(10).await;
        // Re-submit the same swarm — lease was released, must succeed.
        let id2 = coord.submit(req, &token).await.unwrap();
        let status = coord.status(id2, &token).await.unwrap();
        assert!(matches!(status, ReplayJobStatus::Pending));
    }

    /// **Defensive** : metrics inflight gauge returns to zero after
    /// pump completes (no leaks even on engine failure).
    #[tokio::test]
    async fn inflight_gauge_returns_to_zero_on_failure() {
        let tenant = TenantId::new();
        let (coord, v, claims, _) = build_coord(tenant, RegionCode::EuWest1, None);
        let token = v.sign(&claims).unwrap();
        let req = build_request(tenant, Uuid::new_v4(), RegionCode::EuWest1);
        coord.submit(req, &token).await.unwrap();
        // After submit, inflight = 1.
        let metrics = coord.metrics().unwrap().clone();
        assert_eq!(metrics.inflight_for(RegionCode::EuWest1), 1);
        let _ = coord.pump_pending(10).await;
        // After failed replay, inflight must be back to 0.
        assert_eq!(metrics.inflight_for(RegionCode::EuWest1), 0);
    }
}

