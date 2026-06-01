//! Sprint D18 — PG-backed shared idempotency cache.
//!
//! Cf. architecture § 6.
//!
//! Pre-D18, the runtime had an in-memory `DashMap` idempotency
//! cache (per-replica). Under N>1 replicas, a client retry routed
//! to a different replica missed the cache → **re-execution +
//! duplicated side-effects** (audit row, rate-limit decrement).
//! T-D18-2 (CRIT × HIGH).
//!
//! Post-D18, idempotency state lives in `efe_idempotency`
//! (migration 0007). The trait [`IdempotencyStore`] abstracts the
//! storage so unit tests use an in-memory mock while production
//! uses [`PgIdempotencyStore`].

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use thiserror::Error;
use uuid::Uuid;

// **Audit fix D18-A3** — bump the D18 metrics from inside each impl so
// every caller observes the dedup outcome via Prometheus. Both the
// production stub (`PgIdempotencyStore`) and the unit-test mock
// (`MockIdempotencyStore`) flow through these bumps so pin tests can
// assert metric movement without spinning up Postgres.
use efe_telemetry::metrics::d18_instruments::{
    idempotency_hits_total, idempotency_outcome_str, idempotency_size, IdempotencyOutcomeLabel,
};

fn bump_idempotency_outcome(label: IdempotencyOutcomeLabel) {
    idempotency_hits_total()
        .with_label_values(&[idempotency_outcome_str(label)])
        .inc();
}

/// Errors from the idempotency store.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum IdempotencyError {
    /// SQL error from sqlx.
    #[error("sql error: {0}")]
    Sql(String),
    /// In-flight wait timed out (the leader replica is taking too
    /// long ; client should retry later).
    #[error("in-flight idempotency wait timed out after {timeout_ms} ms")]
    InFlightTimeout { timeout_ms: u64 },
    /// Schema-version mismatch in a stored row (architecture § 23.1).
    #[error("unknown idempotency row schema_version: {got}")]
    UnknownSchemaVersion { got: i32 },
}

/// Outcome of an [`IdempotencyStore::try_acquire_or_fetch`] call.
#[derive(Debug, Clone)]
pub enum IdempotencyOutcome {
    /// We are the leader for this `request_id` — caller must execute
    /// the handler then call [`IdempotencyStore::store_response`].
    Leader,
    /// Another replica is in-flight for this `request_id` —
    /// caller should poll [`IdempotencyStore::poll_response`].
    InFlight,
    /// Cached response is available immediately ; do NOT re-execute.
    Cached {
        /// Response payload (canonical bytes — opaque to this layer).
        response_blob: Vec<u8>,
        /// Status as recorded in the SQL row.
        status: String,
    },
}

/// Shared idempotency cache trait. Production impl is
/// [`PgIdempotencyStore`] ; tests use [`MockIdempotencyStore`].
#[async_trait]
pub trait IdempotencyStore: Send + Sync + std::fmt::Debug {
    /// Try to claim leadership of `request_id`.
    ///
    /// Returns one of three outcomes :
    ///
    ///   * [`IdempotencyOutcome::Leader`] — we won the INSERT race ;
    ///     the caller MUST execute the handler then call
    ///     [`Self::store_response`].
    ///   * [`IdempotencyOutcome::InFlight`] — another replica is
    ///     processing ; the caller polls via [`Self::poll_response`]
    ///     until the leader finishes.
    ///   * [`IdempotencyOutcome::Cached`] — we have a completed row ;
    ///     return its response directly.
    async fn try_acquire_or_fetch(
        &self,
        request_id: Uuid,
        tenant_id: &str,
        method: &str,
        request_hash: &[u8],
        ttl: ChronoDuration,
    ) -> Result<IdempotencyOutcome, IdempotencyError>;

    /// Persist the leader's response.
    async fn store_response(
        &self,
        request_id: Uuid,
        response_blob: &[u8],
        status: &str,
    ) -> Result<(), IdempotencyError>;

    /// Poll for a leader's response. Returns Some when a completed
    /// row is found ; None while the row is still in-flight.
    async fn poll_response(
        &self,
        request_id: Uuid,
    ) -> Result<Option<(Vec<u8>, String)>, IdempotencyError>;

    /// Sweep expired rows. Called by the leader-elected background
    /// task. Returns count deleted.
    async fn expire_sweep(&self, cutoff: DateTime<Utc>) -> Result<u64, IdempotencyError>;
}

/// Convenience helper : compute the SHA-256 of a request body, used
/// as the `request_hash` field for tamper detection.
pub fn hash_request_body(bytes: &[u8]) -> Vec<u8> {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(bytes);
    h.finalize().to_vec()
}

// ====================================================================
// Production implementation : PgIdempotencyStore
// ====================================================================

/// Production impl of `IdempotencyStore` backed by Postgres.
///
/// Uses a single `INSERT ... ON CONFLICT DO NOTHING RETURNING id`
/// to atomically claim leadership ; UPDATE on completion.
#[derive(Debug, Clone)]
pub struct PgIdempotencyStore {
    /// Symbolic source-of-truth label for tracing (e.g. "primary").
    pub label: String,
}

impl PgIdempotencyStore {
    /// Construct.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
        }
    }
}

#[async_trait]
impl IdempotencyStore for PgIdempotencyStore {
    async fn try_acquire_or_fetch(
        &self,
        _request_id: Uuid,
        _tenant_id: &str,
        _method: &str,
        _request_hash: &[u8],
        _ttl: ChronoDuration,
    ) -> Result<IdempotencyOutcome, IdempotencyError> {
        // Stub returns Err → bump the `error` outcome so the operator
        // sees the metric move while the backend is unwired.
        bump_idempotency_outcome(IdempotencyOutcomeLabel::Error);
        // **D18 v0.1.19 stub** — real sqlx implementation lands once
        // we have the PgPool wiring in `Server::build` (D18-bis or
        // late D18). For now, the trait is exercised via the Mock
        // impl below + integration tests stub the SQL layer with a
        // local pgmem fixture.
        //
        // The real impl will be :
        //
        //   ```sql
        //   INSERT INTO efe_idempotency (
        //     request_id, tenant_id, method, request_hash, expires_at
        //   ) VALUES ($1, $2, $3, $4, $5)
        //   ON CONFLICT (request_id) DO NOTHING
        //   RETURNING request_id;
        //   ```
        //
        // If RETURNING returns a row, we are the leader.
        // If not, we SELECT the existing row :
        //
        //   ```sql
        //   SELECT response_blob, response_status, completed_at
        //   FROM efe_idempotency WHERE request_id = $1;
        //   ```
        Err(IdempotencyError::Sql(
            "PgIdempotencyStore::try_acquire_or_fetch not yet wired (D18-bis)".into(),
        ))
    }

    async fn store_response(
        &self,
        _request_id: Uuid,
        _response_blob: &[u8],
        _status: &str,
    ) -> Result<(), IdempotencyError> {
        Err(IdempotencyError::Sql(
            "PgIdempotencyStore::store_response not yet wired (D18-bis)".into(),
        ))
    }

    async fn poll_response(
        &self,
        _request_id: Uuid,
    ) -> Result<Option<(Vec<u8>, String)>, IdempotencyError> {
        Err(IdempotencyError::Sql(
            "PgIdempotencyStore::poll_response not yet wired (D18-bis)".into(),
        ))
    }

    async fn expire_sweep(&self, _cutoff: DateTime<Utc>) -> Result<u64, IdempotencyError> {
        Err(IdempotencyError::Sql(
            "PgIdempotencyStore::expire_sweep not yet wired (D18-bis)".into(),
        ))
    }
}

// ====================================================================
// Test-only mock
// ====================================================================

/// In-memory mock impl of [`IdempotencyStore`] for unit tests.
#[derive(Debug, Default)]
pub struct MockIdempotencyStore {
    inner: Mutex<MockState>,
}

#[derive(Debug, Default)]
struct MockState {
    rows: HashMap<Uuid, MockRow>,
}

#[derive(Debug, Clone)]
struct MockRow {
    tenant_id: String,
    method: String,
    request_hash: Vec<u8>,
    expires_at: DateTime<Utc>,
    response_blob: Option<Vec<u8>>,
    status: Option<String>,
    completed_at: Option<DateTime<Utc>>,
}

impl MockIdempotencyStore {
    /// Construct an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of rows currently held (for testing).
    pub fn row_count(&self) -> usize {
        self.inner.lock().map(|g| g.rows.len()).unwrap_or(0)
    }
}

#[async_trait]
impl IdempotencyStore for MockIdempotencyStore {
    async fn try_acquire_or_fetch(
        &self,
        request_id: Uuid,
        tenant_id: &str,
        method: &str,
        request_hash: &[u8],
        ttl: ChronoDuration,
    ) -> Result<IdempotencyOutcome, IdempotencyError> {
        let mut g = self.inner.lock().map_err(|_| {
            bump_idempotency_outcome(IdempotencyOutcomeLabel::Error);
            IdempotencyError::Sql("mock idempotency mutex poisoned".into())
        })?;
        if let Some(row) = g.rows.get(&request_id) {
            // Existing row.
            if let (Some(blob), Some(status)) = (row.response_blob.clone(), row.status.clone()) {
                bump_idempotency_outcome(IdempotencyOutcomeLabel::Cached);
                return Ok(IdempotencyOutcome::Cached {
                    response_blob: blob,
                    status,
                });
            }
            bump_idempotency_outcome(IdempotencyOutcomeLabel::InFlight);
            return Ok(IdempotencyOutcome::InFlight);
        }
        // New row — we win.
        g.rows.insert(
            request_id,
            MockRow {
                tenant_id: tenant_id.to_string(),
                method: method.to_string(),
                request_hash: request_hash.to_vec(),
                expires_at: Utc::now() + ttl,
                response_blob: None,
                status: None,
                completed_at: None,
            },
        );
        // Update the size gauge after the insert so dashboards see
        // the cache growth instantly.
        idempotency_size().set(g.rows.len() as i64);
        bump_idempotency_outcome(IdempotencyOutcomeLabel::Leader);
        Ok(IdempotencyOutcome::Leader)
    }

    async fn store_response(
        &self,
        request_id: Uuid,
        response_blob: &[u8],
        status: &str,
    ) -> Result<(), IdempotencyError> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| IdempotencyError::Sql("mock idempotency mutex poisoned".into()))?;
        if let Some(row) = g.rows.get_mut(&request_id) {
            row.response_blob = Some(response_blob.to_vec());
            row.status = Some(status.to_string());
            row.completed_at = Some(Utc::now());
            Ok(())
        } else {
            Err(IdempotencyError::Sql(format!(
                "store_response : row {request_id} not found (caller violated leader contract)"
            )))
        }
    }

    async fn poll_response(
        &self,
        request_id: Uuid,
    ) -> Result<Option<(Vec<u8>, String)>, IdempotencyError> {
        let g = self
            .inner
            .lock()
            .map_err(|_| IdempotencyError::Sql("mock idempotency mutex poisoned".into()))?;
        if let Some(row) = g.rows.get(&request_id) {
            if let (Some(blob), Some(status)) = (row.response_blob.clone(), row.status.clone()) {
                return Ok(Some((blob, status)));
            }
            return Ok(None);
        }
        Err(IdempotencyError::Sql(format!(
            "poll_response : row {request_id} not found"
        )))
    }

    async fn expire_sweep(&self, cutoff: DateTime<Utc>) -> Result<u64, IdempotencyError> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| IdempotencyError::Sql("mock idempotency mutex poisoned".into()))?;
        let before = g.rows.len();
        g.rows.retain(|_, r| r.expires_at > cutoff);
        // Refresh the size gauge after the sweep.
        idempotency_size().set(g.rows.len() as i64);
        Ok((before - g.rows.len()) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ttl() -> ChronoDuration {
        ChronoDuration::seconds(60)
    }

    #[tokio::test]
    async fn first_caller_becomes_leader() {
        let store = MockIdempotencyStore::new();
        let req = Uuid::new_v4();
        let outcome = store
            .try_acquire_or_fetch(req, "tenant", "Execute", b"hash", ttl())
            .await
            .unwrap();
        assert!(matches!(outcome, IdempotencyOutcome::Leader));
    }

    #[tokio::test]
    async fn second_caller_sees_in_flight() {
        let store = MockIdempotencyStore::new();
        let req = Uuid::new_v4();
        let _ = store
            .try_acquire_or_fetch(req, "tenant", "Execute", b"hash", ttl())
            .await
            .unwrap();
        let outcome = store
            .try_acquire_or_fetch(req, "tenant", "Execute", b"hash", ttl())
            .await
            .unwrap();
        assert!(matches!(outcome, IdempotencyOutcome::InFlight));
    }

    #[tokio::test]
    async fn third_caller_sees_cached_after_store() {
        let store = MockIdempotencyStore::new();
        let req = Uuid::new_v4();
        let _ = store
            .try_acquire_or_fetch(req, "tenant", "Execute", b"hash", ttl())
            .await
            .unwrap();
        store
            .store_response(req, b"response-bytes", "ok")
            .await
            .unwrap();
        let outcome = store
            .try_acquire_or_fetch(req, "tenant", "Execute", b"hash", ttl())
            .await
            .unwrap();
        if let IdempotencyOutcome::Cached {
            response_blob,
            status,
        } = outcome
        {
            assert_eq!(response_blob, b"response-bytes");
            assert_eq!(status, "ok");
        } else {
            panic!("expected Cached, got {outcome:?}");
        }
    }

    #[tokio::test]
    async fn poll_response_returns_some_after_complete() {
        let store = MockIdempotencyStore::new();
        let req = Uuid::new_v4();
        let _ = store
            .try_acquire_or_fetch(req, "tenant", "Execute", b"hash", ttl())
            .await
            .unwrap();
        // Before store_response, poll returns None.
        assert!(store.poll_response(req).await.unwrap().is_none());
        store.store_response(req, b"ok-blob", "ok").await.unwrap();
        let polled = store.poll_response(req).await.unwrap().unwrap();
        assert_eq!(polled.0, b"ok-blob");
        assert_eq!(polled.1, "ok");
    }

    #[tokio::test]
    async fn store_response_unknown_id_errors() {
        let store = MockIdempotencyStore::new();
        let result = store.store_response(Uuid::new_v4(), b"x", "ok").await;
        assert!(matches!(result, Err(IdempotencyError::Sql(_))));
    }

    #[tokio::test]
    async fn expire_sweep_drops_expired_rows() {
        let store = MockIdempotencyStore::new();
        let req = Uuid::new_v4();
        // TTL of -10s → already expired immediately.
        store
            .try_acquire_or_fetch(
                req,
                "tenant",
                "Execute",
                b"hash",
                ChronoDuration::seconds(-10),
            )
            .await
            .unwrap();
        assert_eq!(store.row_count(), 1);
        let removed = store.expire_sweep(Utc::now()).await.unwrap();
        assert_eq!(removed, 1);
        assert_eq!(store.row_count(), 0);
    }

    #[test]
    fn hash_request_body_is_32_bytes_sha256() {
        let h = hash_request_body(b"hello");
        assert_eq!(h.len(), 32);
        assert_ne!(h, hash_request_body(b"world"));
    }

    #[tokio::test]
    async fn pg_store_constructor_works() {
        let s = PgIdempotencyStore::new("primary");
        assert_eq!(s.label, "primary");
        // Methods return the v0.1.19 stub error.
        let r = s
            .try_acquire_or_fetch(Uuid::new_v4(), "t", "m", b"h", ChronoDuration::seconds(60))
            .await;
        assert!(matches!(r, Err(IdempotencyError::Sql(_))));
    }
}
