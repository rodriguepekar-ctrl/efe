// SPDX-License-Identifier: UNLICENSED
//! **Sprint 9 — N16** : Persistent storage for replay jobs, results,
//! leases, and per-op diffs.
//!
//! Architecture § 7.7 + § 8.4.
//!
//! # Trait surface
//!
//! The [`ReplayStorage`] trait is the abstraction.  Two impls ship :
//! - [`InMemoryReplayStorage`] — Sprint 9 (tests + dev).
//! - `PostgresReplayStorage` — Sprint 10 (production).
//!
//! # Multi-tenancy (architecture I13, G9)
//!
//! Every read / write takes a `tenant_id` and enforces tenant
//! isolation at the trait layer.  Cross-tenant read returns
//! [`StorageError::CrossTenant`].

use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use dashmap::DashMap;
use thiserror::Error;
use uuid::Uuid;

use efe_nano_sdk::{
    RegionCode, ReplayJobId, ReplayJobStatus, ReplayMode, ReplayOpDiff, ReplayResult,
    ReplayToleranceMap, TenantId,
};

// ─────────────────────────────────────────────────────────────────────
//  Domain types
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : a queued / running replay job row.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayJob {
    pub job_id: ReplayJobId,
    pub tenant_id: TenantId,
    pub swarm_request_id: Uuid,
    pub origin_region: RegionCode,
    pub target_region: RegionCode,
    pub status: ReplayJobStatus,
    pub actor: String,
    pub ticket_id: String,
    pub reason: String,
    pub mode: ReplayMode,
    pub timeout_ms: u32,
    pub tolerance_override: Option<ReplayToleranceMap>,
    pub operator_id: Uuid,
    pub operator_jwt_fp: String,
    pub submitted_at: SystemTime,
    pub started_at: Option<SystemTime>,
    pub completed_at: Option<SystemTime>,
}

/// **Sprint 9 — N16** : a concurrent-replay lease.  Held by exactly
/// one in-flight job per `(tenant_id, swarm_request_id)` (G15).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReplayLease {
    pub tenant_id: TenantId,
    pub swarm_request_id: Uuid,
    pub job_id: ReplayJobId,
    pub leased_at: SystemTime,
}

impl ReplayLease {
    /// **N16** : Default lease lifetime (30 minutes per architecture
    /// § 7.7 lease validity).
    pub const DEFAULT_TTL: Duration = Duration::from_secs(30 * 60);

    /// True if the lease has expired against `now`.
    pub fn is_expired(&self, now: SystemTime) -> bool {
        now.duration_since(self.leased_at)
            .map(|elapsed| elapsed >= Self::DEFAULT_TTL)
            .unwrap_or(false)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  StorageError
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : closed taxonomy of storage errors.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum StorageError {
    /// Job not present in storage.
    #[error("job {0} not found")]
    JobNotFound(ReplayJobId),

    /// Result not present.
    #[error("result for job {0} not found")]
    ResultNotFound(ReplayJobId),

    /// Concurrent-replay lease already held for this `(tenant, swarm)`.
    #[error("lease already held by job {0}")]
    LeaseConflict(ReplayJobId),

    /// Cross-tenant access attempt (architecture F14).
    #[error("cross-tenant access : query tenant {query_tenant} != stored {stored_tenant}")]
    CrossTenant {
        query_tenant: TenantId,
        stored_tenant: TenantId,
    },

    /// I/O or backend fault.
    #[error("backend error : {0}")]
    Backend(String),
}

// ─────────────────────────────────────────────────────────────────────
//  ReplayStorage trait
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : persistent storage abstraction for the
/// replay subsystem.  Every method asserts tenant isolation.
#[async_trait]
pub trait ReplayStorage: Send + Sync + 'static {
    /// **N16** : Insert a fresh job row (status = `Pending`).
    async fn insert_job(&self, job: ReplayJob) -> Result<(), StorageError>;

    /// **N16** : Fetch a job by id.  Enforces tenant scope.
    async fn get_job(
        &self,
        job_id: ReplayJobId,
        tenant: TenantId,
    ) -> Result<ReplayJob, StorageError>;

    /// **N16** : Update the status of an existing job.
    async fn update_status(
        &self,
        job_id: ReplayJobId,
        tenant: TenantId,
        status: ReplayJobStatus,
    ) -> Result<(), StorageError>;

    /// **N16** : Fetch up to `limit` pending jobs in submission order.
    async fn fetch_pending(&self, limit: usize) -> Result<Vec<ReplayJob>, StorageError>;

    /// **N16** : Store the final result.
    async fn insert_result(&self, result: ReplayResult) -> Result<(), StorageError>;

    /// **N16** : Fetch the final result.  Enforces tenant scope.
    async fn get_result(
        &self,
        job_id: ReplayJobId,
        tenant: TenantId,
    ) -> Result<ReplayResult, StorageError>;

    /// **N16** : Insert a per-op diff row.
    async fn insert_diff(
        &self,
        job_id: ReplayJobId,
        tenant: TenantId,
        diff: ReplayOpDiff,
    ) -> Result<(), StorageError>;

    /// **N16** : Acquire a concurrent-replay lease.  Fails with
    /// `LeaseConflict` if another job already holds it.
    async fn acquire_lease(
        &self,
        tenant: TenantId,
        swarm_request_id: Uuid,
        job_id: ReplayJobId,
    ) -> Result<ReplayLease, StorageError>;

    /// **N16** : Release a previously held lease (idempotent).
    async fn release_lease(&self, lease: ReplayLease) -> Result<(), StorageError>;

    /// **N16** : List all jobs for a tenant.  Used by the operator
    /// `GET /v1/replay` endpoint.
    async fn list_jobs_for_tenant(
        &self,
        tenant: TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ReplayJob>, StorageError>;
}

// ─────────────────────────────────────────────────────────────────────
//  InMemoryReplayStorage
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N16** : process-local replay storage backed by
/// concurrent maps.  Used for tests + the Sprint 9 demo binary.
pub struct InMemoryReplayStorage {
    jobs: DashMap<ReplayJobId, ReplayJob>,
    results: DashMap<ReplayJobId, ReplayResult>,
    diffs: DashMap<(ReplayJobId, String), ReplayOpDiff>,
    leases: DashMap<(TenantId, Uuid), ReplayLease>,
    /// Job ordering for `fetch_pending` — VecDeque protected by a
    /// short critical section.
    pending_order: Mutex<Vec<(SystemTime, ReplayJobId)>>,
}

impl Default for InMemoryReplayStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryReplayStorage {
    /// **N16** : Construct an empty store.
    pub fn new() -> Self {
        Self {
            jobs: DashMap::new(),
            results: DashMap::new(),
            diffs: DashMap::new(),
            leases: DashMap::new(),
            pending_order: Mutex::new(Vec::new()),
        }
    }

    /// **N16** : Number of jobs in the store.
    pub fn job_count(&self) -> usize {
        self.jobs.len()
    }

    /// **N16** : Number of held leases.
    pub fn lease_count(&self) -> usize {
        self.leases.len()
    }

    /// **N16** : Number of stored diffs.
    pub fn diff_count(&self) -> usize {
        self.diffs.len()
    }

    fn check_tenant(stored: TenantId, query: TenantId) -> Result<(), StorageError> {
        if stored == query {
            Ok(())
        } else {
            Err(StorageError::CrossTenant {
                query_tenant: query,
                stored_tenant: stored,
            })
        }
    }
}

#[async_trait]
impl ReplayStorage for InMemoryReplayStorage {
    async fn insert_job(&self, job: ReplayJob) -> Result<(), StorageError> {
        let id = job.job_id;
        let submitted = job.submitted_at;
        // Track pending order for FIFO scheduling.
        if matches!(job.status, ReplayJobStatus::Pending) {
            self.pending_order
                .lock()
                .map_err(|e| StorageError::Backend(e.to_string()))?
                .push((submitted, id));
        }
        self.jobs.insert(id, job);
        Ok(())
    }

    async fn get_job(
        &self,
        job_id: ReplayJobId,
        tenant: TenantId,
    ) -> Result<ReplayJob, StorageError> {
        let job = self
            .jobs
            .get(&job_id)
            .ok_or(StorageError::JobNotFound(job_id))?
            .clone();
        Self::check_tenant(job.tenant_id, tenant)?;
        Ok(job)
    }

    async fn update_status(
        &self,
        job_id: ReplayJobId,
        tenant: TenantId,
        status: ReplayJobStatus,
    ) -> Result<(), StorageError> {
        let mut entry = self
            .jobs
            .get_mut(&job_id)
            .ok_or(StorageError::JobNotFound(job_id))?;
        Self::check_tenant(entry.tenant_id, tenant)?;
        let now = SystemTime::now();
        match &status {
            ReplayJobStatus::InProgress { .. } => {
                if entry.started_at.is_none() {
                    entry.started_at = Some(now);
                }
            }
            ReplayJobStatus::Completed { .. }
            | ReplayJobStatus::CompletedWithRegression { .. }
            | ReplayJobStatus::Failed { .. }
            | ReplayJobStatus::Cancelled => {
                entry.completed_at = Some(now);
            }
            ReplayJobStatus::Pending => {}
        }
        entry.status = status;
        Ok(())
    }

    async fn fetch_pending(&self, limit: usize) -> Result<Vec<ReplayJob>, StorageError> {
        let order = self
            .pending_order
            .lock()
            .map_err(|e| StorageError::Backend(e.to_string()))?
            .clone();
        let mut out = Vec::new();
        for (_, id) in order {
            if out.len() >= limit {
                break;
            }
            if let Some(j) = self.jobs.get(&id) {
                if matches!(j.status, ReplayJobStatus::Pending) {
                    out.push(j.clone());
                }
            }
        }
        Ok(out)
    }

    async fn insert_result(&self, result: ReplayResult) -> Result<(), StorageError> {
        // **N16 hardening** : cross-check the result's tenant against
        // the parent job's tenant.  Catches buggy callers that fan
        // out work to the wrong tenant ; defense-in-depth on top of
        // the I13 multi-tenant invariant.
        if let Some(job) = self.jobs.get(&result.job_id) {
            Self::check_tenant(job.tenant_id, result.tenant_id)?;
        }
        self.results.insert(result.job_id, result);
        Ok(())
    }

    async fn get_result(
        &self,
        job_id: ReplayJobId,
        tenant: TenantId,
    ) -> Result<ReplayResult, StorageError> {
        let res = self
            .results
            .get(&job_id)
            .ok_or(StorageError::ResultNotFound(job_id))?
            .clone();
        Self::check_tenant(res.tenant_id, tenant)?;
        Ok(res)
    }

    async fn insert_diff(
        &self,
        job_id: ReplayJobId,
        tenant: TenantId,
        diff: ReplayOpDiff,
    ) -> Result<(), StorageError> {
        // Validate tenant by referring to the parent job (cross-tenant guard).
        if let Some(job) = self.jobs.get(&job_id) {
            Self::check_tenant(job.tenant_id, tenant)?;
        } else {
            return Err(StorageError::JobNotFound(job_id));
        }
        self.diffs.insert((job_id, diff.op_id.clone()), diff);
        Ok(())
    }

    async fn acquire_lease(
        &self,
        tenant: TenantId,
        swarm_request_id: Uuid,
        job_id: ReplayJobId,
    ) -> Result<ReplayLease, StorageError> {
        let now = SystemTime::now();
        let key = (tenant, swarm_request_id);
        // Lock-free CAS-style insert via DashMap::entry.
        let existing = self.leases.entry(key);
        match existing {
            dashmap::Entry::Occupied(occ) => {
                let lease = occ.get().clone();
                if lease.is_expired(now) {
                    // Replace expired lease.
                    let new_lease = ReplayLease {
                        tenant_id: tenant,
                        swarm_request_id,
                        job_id,
                        leased_at: now,
                    };
                    occ.replace_entry(new_lease.clone());
                    Ok(new_lease)
                } else {
                    Err(StorageError::LeaseConflict(lease.job_id))
                }
            }
            dashmap::Entry::Vacant(vac) => {
                let new_lease = ReplayLease {
                    tenant_id: tenant,
                    swarm_request_id,
                    job_id,
                    leased_at: now,
                };
                vac.insert(new_lease.clone());
                Ok(new_lease)
            }
        }
    }

    async fn release_lease(&self, lease: ReplayLease) -> Result<(), StorageError> {
        let key = (lease.tenant_id, lease.swarm_request_id);
        self.leases.remove_if(&key, |_, existing| existing.job_id == lease.job_id);
        Ok(())
    }

    async fn list_jobs_for_tenant(
        &self,
        tenant: TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ReplayJob>, StorageError> {
        let mut all: Vec<ReplayJob> = self
            .jobs
            .iter()
            .filter(|r| r.value().tenant_id == tenant)
            .map(|r| r.value().clone())
            .collect();
        all.sort_by_key(|j| j.submitted_at);
        let _ = offset;
        Ok(all.into_iter().skip(offset).take(limit).collect())
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Helper : build a default ReplayJob from ReplayRequest
// ─────────────────────────────────────────────────────────────────────

impl ReplayJob {
    /// **N16** : Convert a validated `ReplayRequest` + auth context
    /// into a queued `ReplayJob`.
    pub fn from_request(
        request: &efe_nano_sdk::ReplayRequest,
        job_id: ReplayJobId,
        operator_id: Uuid,
        operator_jwt_fp: String,
    ) -> Self {
        Self {
            job_id,
            tenant_id: request.tenant_id,
            swarm_request_id: request.swarm_request_id,
            origin_region: request.origin_region,
            target_region: request.target_region,
            status: ReplayJobStatus::Pending,
            actor: request.actor.clone(),
            ticket_id: request.ticket_id.clone(),
            reason: request.reason.clone(),
            mode: request.mode,
            timeout_ms: request.timeout_ms,
            tolerance_override: request.tolerance_override.clone(),
            operator_id,
            operator_jwt_fp,
            submitted_at: SystemTime::now(),
            started_at: None,
            completed_at: None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{MatchOutcome, OpStatus};
    use std::collections::HashMap;

    fn job() -> ReplayJob {
        ReplayJob {
            job_id: ReplayJobId::new(),
            tenant_id: TenantId::new(),
            swarm_request_id: Uuid::new_v4(),
            origin_region: RegionCode::EuWest1,
            target_region: RegionCode::EuWest1,
            status: ReplayJobStatus::Pending,
            actor: "alice".into(),
            ticket_id: "JIRA-1".into(),
            reason: "audit".into(),
            mode: ReplayMode::DispatchOnly,
            timeout_ms: 600_000,
            tolerance_override: None,
            operator_id: Uuid::new_v4(),
            operator_jwt_fp: "fp".into(),
            submitted_at: SystemTime::now(),
            started_at: None,
            completed_at: None,
        }
    }

    fn diff(op_id: &str) -> ReplayOpDiff {
        ReplayOpDiff {
            op_id: op_id.into(),
            mode_label: "pure_rust".into(),
            match_outcome: MatchOutcome::Exact,
            original_status: OpStatus::Success,
            replay_status: OpStatus::Success,
            structural_diff: None,
            semantic_score: None,
            latency_ms_original: 5,
            latency_ms_replay: 6,
            cost_cents_original: 0.01,
            cost_cents_replay: 0.0,
        }
    }

    fn result(job_id: ReplayJobId, tenant: TenantId) -> ReplayResult {
        ReplayResult {
            job_id,
            tenant_id: tenant,
            origin_region: RegionCode::EuWest1,
            target_region: RegionCode::EuWest1,
            swarm_request_id: Uuid::new_v4(),
            envelope_trace_id: Uuid::new_v4(),
            mode: ReplayMode::DispatchOnly,
            per_op: HashMap::new(),
            aggregate: Default::default(),
            agent_versions: HashMap::new(),
            operator_id: Uuid::new_v4(),
            ticket_id: "JIRA-1".into(),
            replay_started_at: SystemTime::now(),
            replay_completed_at: SystemTime::now(),
            total_cost_cents: 0.0,
        }
    }

    #[tokio::test]
    async fn insert_and_get_job() {
        let s = InMemoryReplayStorage::new();
        let j = job();
        let id = j.job_id;
        let tenant = j.tenant_id;
        s.insert_job(j.clone()).await.unwrap();
        let got = s.get_job(id, tenant).await.unwrap();
        assert_eq!(got, j);
    }

    #[tokio::test]
    async fn get_job_unknown_returns_not_found() {
        let s = InMemoryReplayStorage::new();
        let err = s.get_job(ReplayJobId::new(), TenantId::new()).await.unwrap_err();
        assert!(matches!(err, StorageError::JobNotFound(_)));
    }

    #[tokio::test]
    async fn cross_tenant_get_job_rejected() {
        let s = InMemoryReplayStorage::new();
        let j = job();
        s.insert_job(j.clone()).await.unwrap();
        let other = TenantId::new();
        let err = s.get_job(j.job_id, other).await.unwrap_err();
        assert!(matches!(err, StorageError::CrossTenant { .. }));
    }

    #[tokio::test]
    async fn fetch_pending_returns_in_order() {
        let s = InMemoryReplayStorage::new();
        let j1 = job();
        let j2 = job();
        s.insert_job(j1.clone()).await.unwrap();
        s.insert_job(j2.clone()).await.unwrap();
        let pending = s.fetch_pending(10).await.unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].job_id, j1.job_id);
        assert_eq!(pending[1].job_id, j2.job_id);
    }

    #[tokio::test]
    async fn update_status_marks_completed() {
        let s = InMemoryReplayStorage::new();
        let j = job();
        let id = j.job_id;
        let tenant = j.tenant_id;
        s.insert_job(j).await.unwrap();
        s.update_status(
            id,
            tenant,
            ReplayJobStatus::Completed {
                result_id: Uuid::new_v4(),
            },
        )
        .await
        .unwrap();
        let got = s.get_job(id, tenant).await.unwrap();
        assert!(got.completed_at.is_some());
        assert!(matches!(got.status, ReplayJobStatus::Completed { .. }));
    }

    #[tokio::test]
    async fn update_status_first_in_progress_marks_started_at() {
        let s = InMemoryReplayStorage::new();
        let j = job();
        let id = j.job_id;
        let tenant = j.tenant_id;
        s.insert_job(j).await.unwrap();
        s.update_status(id, tenant, ReplayJobStatus::InProgress { progress_pct: 10 })
            .await
            .unwrap();
        let got = s.get_job(id, tenant).await.unwrap();
        assert!(got.started_at.is_some());
    }

    #[tokio::test]
    async fn acquire_lease_then_conflict() {
        let s = InMemoryReplayStorage::new();
        let tenant = TenantId::new();
        let swarm = Uuid::new_v4();
        let id1 = ReplayJobId::new();
        let id2 = ReplayJobId::new();
        let lease = s.acquire_lease(tenant, swarm, id1).await.unwrap();
        assert_eq!(lease.job_id, id1);
        let err = s.acquire_lease(tenant, swarm, id2).await.unwrap_err();
        assert!(matches!(err, StorageError::LeaseConflict(j) if j == id1));
    }

    #[tokio::test]
    async fn release_lease_frees_slot() {
        let s = InMemoryReplayStorage::new();
        let tenant = TenantId::new();
        let swarm = Uuid::new_v4();
        let id1 = ReplayJobId::new();
        let id2 = ReplayJobId::new();
        let lease = s.acquire_lease(tenant, swarm, id1).await.unwrap();
        s.release_lease(lease).await.unwrap();
        // Now id2 can acquire.
        let l2 = s.acquire_lease(tenant, swarm, id2).await.unwrap();
        assert_eq!(l2.job_id, id2);
    }

    #[tokio::test]
    async fn release_with_wrong_job_id_is_noop() {
        let s = InMemoryReplayStorage::new();
        let tenant = TenantId::new();
        let swarm = Uuid::new_v4();
        let id1 = ReplayJobId::new();
        let _lease = s.acquire_lease(tenant, swarm, id1).await.unwrap();
        let fake_lease = ReplayLease {
            tenant_id: tenant,
            swarm_request_id: swarm,
            job_id: ReplayJobId::new(),
            leased_at: SystemTime::now(),
        };
        s.release_lease(fake_lease).await.unwrap();
        // Original lease still held.
        assert_eq!(s.lease_count(), 1);
    }

    #[tokio::test]
    async fn insert_diff_enforces_tenant() {
        let s = InMemoryReplayStorage::new();
        let j = job();
        let job_id = j.job_id;
        let tenant = j.tenant_id;
        s.insert_job(j).await.unwrap();
        s.insert_diff(job_id, tenant, diff("op1")).await.unwrap();
        assert_eq!(s.diff_count(), 1);
        let other = TenantId::new();
        let err = s.insert_diff(job_id, other, diff("op2")).await.unwrap_err();
        assert!(matches!(err, StorageError::CrossTenant { .. }));
    }

    #[tokio::test]
    async fn result_round_trip_with_tenant_isolation() {
        let s = InMemoryReplayStorage::new();
        let tenant = TenantId::new();
        let id = ReplayJobId::new();
        let r = result(id, tenant);
        s.insert_result(r.clone()).await.unwrap();
        let got = s.get_result(id, tenant).await.unwrap();
        assert_eq!(got, r);
        let other = TenantId::new();
        let err = s.get_result(id, other).await.unwrap_err();
        assert!(matches!(err, StorageError::CrossTenant { .. }));
    }

    #[tokio::test]
    async fn list_jobs_for_tenant_filters_correctly() {
        let s = InMemoryReplayStorage::new();
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        let mut j1 = job();
        j1.tenant_id = t1;
        let mut j2 = job();
        j2.tenant_id = t2;
        let mut j3 = job();
        j3.tenant_id = t1;
        s.insert_job(j1).await.unwrap();
        s.insert_job(j2).await.unwrap();
        s.insert_job(j3).await.unwrap();
        let t1_jobs = s.list_jobs_for_tenant(t1, 10, 0).await.unwrap();
        assert_eq!(t1_jobs.len(), 2);
        for j in t1_jobs {
            assert_eq!(j.tenant_id, t1);
        }
    }

    #[tokio::test]
    async fn insert_result_rejects_tenant_mismatch_with_parent_job() {
        let s = InMemoryReplayStorage::new();
        let job = job();
        let job_id = job.job_id;
        let real_tenant = job.tenant_id;
        s.insert_job(job).await.unwrap();
        // Forge a result whose tenant_id doesn't match the parent
        // job's tenant_id.
        let mut forged = result(job_id, TenantId::new());
        forged.tenant_id = TenantId::new(); // wrong tenant
        let err = s.insert_result(forged).await.unwrap_err();
        assert!(matches!(err, StorageError::CrossTenant { .. }));
        // The legitimate result inserts cleanly.
        let legit = result(job_id, real_tenant);
        s.insert_result(legit).await.unwrap();
    }

    #[test]
    fn lease_expiry_default_ttl_thirty_minutes() {
        assert_eq!(ReplayLease::DEFAULT_TTL, Duration::from_secs(30 * 60));
    }

    #[test]
    fn lease_is_expired_after_ttl() {
        let lease = ReplayLease {
            tenant_id: TenantId::new(),
            swarm_request_id: Uuid::new_v4(),
            job_id: ReplayJobId::new(),
            leased_at: SystemTime::now() - Duration::from_secs(35 * 60),
        };
        assert!(lease.is_expired(SystemTime::now()));
    }

    #[test]
    fn lease_not_expired_within_ttl() {
        let lease = ReplayLease {
            tenant_id: TenantId::new(),
            swarm_request_id: Uuid::new_v4(),
            job_id: ReplayJobId::new(),
            leased_at: SystemTime::now() - Duration::from_secs(60),
        };
        assert!(!lease.is_expired(SystemTime::now()));
    }

    #[tokio::test]
    async fn expired_lease_can_be_replaced() {
        let s = InMemoryReplayStorage::new();
        let tenant = TenantId::new();
        let swarm = Uuid::new_v4();
        let id1 = ReplayJobId::new();
        // Manually insert an expired lease.
        let expired = ReplayLease {
            tenant_id: tenant,
            swarm_request_id: swarm,
            job_id: id1,
            leased_at: SystemTime::now() - Duration::from_secs(31 * 60),
        };
        s.leases.insert((tenant, swarm), expired);
        // New acquire should replace the expired lease.
        let id2 = ReplayJobId::new();
        let l = s.acquire_lease(tenant, swarm, id2).await.unwrap();
        assert_eq!(l.job_id, id2);
    }

    #[test]
    fn replay_job_from_request() {
        let req = efe_nano_sdk::ReplayRequest {
            tenant_id: TenantId::new(),
            swarm_request_id: Uuid::new_v4(),
            origin_region: RegionCode::UsEast1,
            target_region: RegionCode::UsEast1,
            actor: "a".into(),
            ticket_id: "t".into(),
            reason: "r".into(),
            timeout_ms: 1_000,
            mode: ReplayMode::Full,
            tolerance_override: None,
        };
        let id = ReplayJobId::new();
        let op_id = Uuid::new_v4();
        let j = ReplayJob::from_request(&req, id, op_id, "fp".into());
        assert_eq!(j.job_id, id);
        assert_eq!(j.tenant_id, req.tenant_id);
        assert_eq!(j.mode, ReplayMode::Full);
        assert!(matches!(j.status, ReplayJobStatus::Pending));
    }
}
