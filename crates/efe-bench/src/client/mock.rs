//! Sprint D18 — `MockBenchClient` (instant in-process stub).
//!
//! Sprint **D29 (Φ-D)** — moved from `client.rs` into the
//! `client/` sub-module to make room for the real `tonic_client`
//! sibling. The implementation is **byte-identical** to D18
//! pre-split (no behavioural change ; all existing tests still
//! pass) plus a fresh [`BenchClient::kind`] override returning
//! [`ClientKind::Mock`].

use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;

use crate::client::{BenchClient, CallOutcome, ClientKind};
use crate::error::BenchError;
use crate::workload::WorkloadOp;

/// Test-only mock that records every call + returns a configurable
/// canned outcome. Default : 5 ms latency, success, no error.
#[derive(Debug)]
pub struct MockBenchClient {
    inner: Mutex<MockState>,
}

#[derive(Debug, Default)]
struct MockState {
    calls: Vec<(WorkloadOp, String)>,
    canned_latency: Option<Duration>,
    canned_error: Option<String>,
    fail_after: Option<u32>,    // fail every Nth call
    call_counter: u32,
}

impl Default for MockBenchClient {
    fn default() -> Self {
        Self::new()
    }
}

impl MockBenchClient {
    /// Construct a fresh mock with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(MockState::default()),
        }
    }

    /// Override the latency returned by every call.
    pub fn with_canned_latency(self, d: Duration) -> Self {
        if let Ok(mut g) = self.inner.lock() {
            g.canned_latency = Some(d);
        }
        self
    }

    /// Cause every call to fail with the given error label.
    pub fn with_canned_error(self, label: impl Into<String>) -> Self {
        if let Ok(mut g) = self.inner.lock() {
            g.canned_error = Some(label.into());
        }
        self
    }

    /// Cause every Nth call to fail (round-robin).
    pub fn with_fail_every(self, n: u32) -> Self {
        if let Ok(mut g) = self.inner.lock() {
            g.fail_after = Some(n);
        }
        self
    }

    /// Recorded calls.
    pub fn calls(&self) -> Vec<(WorkloadOp, String)> {
        self.inner.lock().map(|g| g.calls.clone()).unwrap_or_default()
    }

    /// Total recorded call count.
    pub fn call_count(&self) -> usize {
        self.inner.lock().map(|g| g.calls.len()).unwrap_or(0)
    }
}

#[async_trait]
impl BenchClient for MockBenchClient {
    async fn call(
        &self,
        op: WorkloadOp,
        tenant: &str,
        worker_id: u64,
    ) -> Result<CallOutcome, BenchError> {
        let (latency, error, current) = {
            let mut g = self.inner.lock().map_err(|_| {
                BenchError::config("mock client mutex poisoned")
            })?;
            g.calls.push((op, tenant.to_string()));
            g.call_counter = g.call_counter.saturating_add(1);
            let canned_latency =
                g.canned_latency.unwrap_or(Duration::from_millis(5));
            let mut error = g.canned_error.clone();
            if let Some(every) = g.fail_after {
                if every > 0 && g.call_counter % every == 0 {
                    error = error.or(Some("ScheduledFailure".into()));
                }
            }
            (canned_latency, error, g.call_counter)
        };

        // Simulate work via tokio::time::sleep so the outer runner
        // observes a realistic latency.
        tokio::time::sleep(Duration::from_micros(latency.as_micros() as u64))
            .await;

        match error {
            Some(label) => Ok(CallOutcome {
                op,
                latency,
                ok: false,
                error_label: Some(label),
            }),
            None => Ok(CallOutcome {
                op,
                latency,
                ok: true,
                error_label: None,
            }),
        }
        .map(|outcome| {
            tracing::trace!(
                target: "efe.bench.client",
                op = op.as_str(),
                tenant = tenant,
                worker_id = worker_id,
                call_idx = current,
                ok = outcome.ok,
                "mock client call"
            );
            outcome
        })
    }

    fn kind(&self) -> ClientKind {
        ClientKind::Mock
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_default_returns_ok() {
        let client = MockBenchClient::new();
        let out = client
            .call(WorkloadOp::Execute, "tenant-1", 0)
            .await
            .unwrap();
        assert!(out.ok);
        assert_eq!(out.op, WorkloadOp::Execute);
        assert!(out.error_label.is_none());
    }

    #[tokio::test]
    async fn mock_with_canned_error_marks_failed() {
        let client = MockBenchClient::new().with_canned_error("Unavailable");
        let out = client
            .call(WorkloadOp::Execute, "tenant", 0)
            .await
            .unwrap();
        assert!(!out.ok);
        assert_eq!(out.error_label.as_deref(), Some("Unavailable"));
    }

    #[tokio::test]
    async fn mock_with_fail_every_cycles() {
        let client = MockBenchClient::new().with_fail_every(3);
        let mut errors = 0;
        for _ in 0..9 {
            let o = client.call(WorkloadOp::Execute, "t", 0).await.unwrap();
            if !o.ok {
                errors += 1;
            }
        }
        // 3 failures expected (calls 3, 6, 9).
        assert_eq!(errors, 3);
    }

    #[tokio::test]
    async fn mock_records_all_calls() {
        let client = MockBenchClient::new();
        client.call(WorkloadOp::Execute, "a", 0).await.unwrap();
        client
            .call(WorkloadOp::ResolveCatalog, "b", 1)
            .await
            .unwrap();
        let calls = client.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], (WorkloadOp::Execute, "a".into()));
        assert_eq!(calls[1], (WorkloadOp::ResolveCatalog, "b".into()));
    }

    #[tokio::test]
    async fn mock_call_count() {
        let client = MockBenchClient::new();
        for i in 0..5u64 {
            client.call(WorkloadOp::Execute, "t", i).await.unwrap();
        }
        assert_eq!(client.call_count(), 5);
    }

    #[tokio::test]
    async fn mock_kind_returns_mock() {
        // **Sprint D29 (Φ-D)** — the report uses `client.kind()`
        // to populate the JSON `client_kind` field. Pin the value.
        let c = MockBenchClient::new();
        assert_eq!(c.kind(), ClientKind::Mock);
    }

    #[tokio::test]
    async fn mock_records_mode3_op() {
        // **Sprint D38 (Φ-F)** — Mock must accept the new
        // `Mode3Execute` variant transparently (the harness needs
        // to be able to bench Mode-3 workloads in Mock mode for
        // self-tests, even though Mock can't issue real Mode-3
        // RPCs).
        let client = MockBenchClient::new();
        let out = client
            .call(WorkloadOp::Mode3Execute { k_swarm: 5 }, "tenant", 0)
            .await
            .unwrap();
        assert!(out.ok);
        assert_eq!(out.op, WorkloadOp::Mode3Execute { k_swarm: 5 });
    }

    #[tokio::test]
    async fn mock_ignores_worker_id_for_outcome() {
        // **Audit fix M6-A1 (Φ-F)** — Mock has no network metadata
        // to set, so `worker_id` is recorded only via tracing
        // (no observable side effect on the outcome). Pin that the
        // value is accepted across the full u64 range without
        // changing the call's success or latency.
        let client = MockBenchClient::new();
        let out_low = client.call(WorkloadOp::Execute, "t", 0).await.unwrap();
        let out_high = client
            .call(WorkloadOp::Execute, "t", u64::MAX)
            .await
            .unwrap();
        assert!(out_low.ok);
        assert!(out_high.ok);
        // Both calls land in `calls()` regardless of worker_id.
        assert_eq!(client.call_count(), 2);
    }
}
