//! Phase 1 stub audit writers — console + in-memory.

use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::{AuditWriter, NanoAgentTrace};

/// Audit writer that logs traces to `tracing::info!`.
///
/// Useful for development + Sprint 1 dev environment.  Production
/// will use `PostgresAuditWriter` (Sprint 6).
#[derive(Debug, Default)]
pub struct ConsoleAuditWriter;

impl ConsoleAuditWriter {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl AuditWriter for ConsoleAuditWriter {
    async fn record(&self, trace: NanoAgentTrace) {
        tracing::info!(
            target: "efe_nano::audit",
            trace_id = %trace.trace_id.0,
            tenant = %trace.tenant_id,
            agent = %trace.nano_agent_id,
            version = %trace.nano_agent_version,
            mode = %trace.execution_mode,
            latency_ms = trace.latency_ms.unwrap_or(0),
            cost_cents = trace.cost_cents,
            error_kind = trace.error_kind.as_deref().unwrap_or("none"),
            "nano agent call"
        );
    }
}

/// Audit writer that stores traces in memory.  For tests.
#[derive(Debug, Default, Clone)]
pub struct InMemoryAuditWriter {
    traces: Arc<Mutex<Vec<NanoAgentTrace>>>,
}

impl InMemoryAuditWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a snapshot of all traces recorded.
    pub async fn traces(&self) -> Vec<NanoAgentTrace> {
        self.traces.lock().await.clone()
    }

    /// Number of traces recorded.
    pub async fn count(&self) -> usize {
        self.traces.lock().await.len()
    }

    /// **Sprint 8 — N14** : Number of traces excluding audit-metadata
    /// sentinels (CacheHit / ShadowRun / ShadowComparison /
    /// RolloutPolicyChanged / SwarmEnvelope).  This is the count of
    /// "real op invocations" that the test assertions historically
    /// expected before N12 / N13 / N14 added sentinel rows.
    pub async fn op_count(&self) -> usize {
        self.traces
            .lock()
            .await
            .iter()
            .filter(|t| {
                !matches!(
                    t.error_kind.as_deref(),
                    Some("CacheHit")
                        | Some("ShadowRun")
                        | Some("ShadowComparison")
                        | Some("RolloutPolicyChanged")
                        | Some("SwarmEnvelope")
                )
            })
            .count()
    }

    /// **Sprint 8 — N14** : Number of swarm envelope traces emitted.
    pub async fn envelope_count(&self) -> usize {
        self.traces
            .lock()
            .await
            .iter()
            .filter(|t| t.error_kind.as_deref() == Some("SwarmEnvelope"))
            .count()
    }
}

#[async_trait]
impl AuditWriter for InMemoryAuditWriter {
    async fn record(&self, trace: NanoAgentTrace) {
        self.traces.lock().await.push(trace);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::hash_json;
    use efe_nano_sdk::{NanoAgentId, NanoAgentVersion, TenantId, TraceId};
    use uuid::Uuid;

    fn sample_trace() -> NanoAgentTrace {
        let h = hash_json(&serde_json::json!({}));
        NanoAgentTrace {
            trace_id: TraceId::new(),
            parent_trace_ids: vec![],
            swarm_request_id: Uuid::new_v4(),
            operation_id: "stand_alone".to_string(),
            tenant_id: TenantId::new(),
            caller_agent_id: None,
            nano_agent_id: NanoAgentId::new("json_validate").unwrap(),
            nano_agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            execution_mode: "pure_rust".to_string(),
            input_hash_source: h,
            input_hash_resolved: h,
            output_hash: Some(hash_json(&serde_json::json!({}))),
            error_kind: None,
            started_at: chrono::Utc::now(),
            completed_at: Some(chrono::Utc::now()),
            latency_ms: Some(3),
            cost_cents: 0.0001,
            llm_tokens_used: None,
            full_input: None,
            full_output: None,
            tombstone: None,
            swarm_envelope_trace_id: None,
        }
    }

    #[tokio::test]
    async fn console_writer_does_not_panic() {
        let w = ConsoleAuditWriter::new();
        w.record(sample_trace()).await;
        // No assertion beyond "doesn't panic".
    }

    #[tokio::test]
    async fn in_memory_writer_stores_traces() {
        let w = InMemoryAuditWriter::new();
        assert_eq!(w.count().await, 0);
        w.record(sample_trace()).await;
        assert_eq!(w.count().await, 1);
        w.record(sample_trace()).await;
        assert_eq!(w.count().await, 2);
    }

    #[tokio::test]
    async fn in_memory_writer_returns_traces() {
        let w = InMemoryAuditWriter::new();
        w.record(sample_trace()).await;
        let traces = w.traces().await;
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].nano_agent_id.as_str(), "json_validate");
    }

    #[tokio::test]
    async fn in_memory_writer_clones_share_state() {
        let w = InMemoryAuditWriter::new();
        let w2 = w.clone();
        w.record(sample_trace()).await;
        // The clone sees the recorded trace because Arc<Mutex> is shared.
        assert_eq!(w2.count().await, 1);
    }
}
