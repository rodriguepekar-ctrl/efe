//! PR #18 AD29 — test-only in-memory span exporter.
//!
//! Captures every span emitted by the OTel SDK into a `Mutex<Vec>`
//! for assertion in integration tests.  Production binaries never
//! install this exporter ; only the test harness does, via
//! `tracing_opentelemetry::layer().with_tracer(in_mem_tracer)`.
//!
//! Stores a lightweight projection of the span (name, attributes,
//! status, parent) rather than the full SDK `SpanData` — keeps tests
//! decoupled from SDK internals.

use std::sync::{Arc, Mutex};

use opentelemetry::trace::Status;
use opentelemetry::KeyValue;

/// Lightweight projection of an exported span — what tests typically
/// assert on.
#[derive(Debug, Clone)]
pub struct InMemorySpan {
    /// Span name (e.g. `"efe.meta_swarm.iterate"`).
    pub name: String,
    /// Wall-clock-microsecond duration ; `None` when the span is
    /// still open.
    pub duration_micros: Option<u128>,
    /// Span kind (`SERVER`, `CLIENT`, `INTERNAL`, `PRODUCER`, `CONSUMER`).
    pub kind: String,
    /// Final span status (`Ok`, `Error{description}`, `Unset`).
    pub status: SpanStatus,
    /// Attributes set on the span at close time.
    pub attributes: Vec<KeyValue>,
    /// Trace ID hex string (32 chars).
    pub trace_id: String,
    /// Span ID hex string (16 chars).
    pub span_id: String,
    /// Parent span ID hex (empty for root spans).
    pub parent_span_id: String,
}

/// Test-friendly equivalent of `opentelemetry::trace::Status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpanStatus {
    /// `Status::Unset` (sometimes called "ok by default").
    Unset,
    /// `Status::Ok`.
    Ok,
    /// `Status::Error { description }`.
    Error {
        /// Human-readable error message.
        description: String,
    },
}

impl From<&Status> for SpanStatus {
    fn from(s: &Status) -> Self {
        match s {
            Status::Unset => Self::Unset,
            Status::Ok => Self::Ok,
            Status::Error { description } => Self::Error {
                description: description.to_string(),
            },
        }
    }
}

/// Shared, thread-safe in-memory span store.  Cloned across the
/// exporter + test assertions ; the inner `Mutex<Vec>` is unlocked
/// briefly per span append.
#[derive(Debug, Clone, Default)]
pub struct InMemoryExporter {
    spans: Arc<Mutex<Vec<InMemorySpan>>>,
}

impl InMemoryExporter {
    /// Construct an empty exporter.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Test API : append a span.  Used by the SDK-side adapter when
    /// the BatchSpanProcessor flushes.
    pub fn append(&self, span: InMemorySpan) {
        if let Ok(mut g) = self.spans.lock() {
            g.push(span);
        }
    }

    /// Snapshot every captured span.  Returns owned copies so test
    /// assertions don't hold the lock.
    #[must_use]
    pub fn snapshot(&self) -> Vec<InMemorySpan> {
        self.spans.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Drain : return + clear.
    pub fn drain(&self) -> Vec<InMemorySpan> {
        if let Ok(mut g) = self.spans.lock() {
            std::mem::take(&mut *g)
        } else {
            Vec::new()
        }
    }

    /// Convenience : find the first span with the given name.
    #[must_use]
    pub fn find_first(&self, name: &str) -> Option<InMemorySpan> {
        self.snapshot().into_iter().find(|s| s.name == name)
    }

    /// Convenience : count spans by name.
    #[must_use]
    pub fn count_by_name(&self, name: &str) -> usize {
        self.snapshot().iter().filter(|s| s.name == name).count()
    }

    /// Convenience : count spans whose status is `Error`.
    #[must_use]
    pub fn count_errors(&self) -> usize {
        self.snapshot()
            .iter()
            .filter(|s| matches!(s.status, SpanStatus::Error { .. }))
            .count()
    }

    /// Convenience : test that spans form a tree rooted at the given
    /// trace_id.  Returns the (root, children) tuple.
    #[must_use]
    pub fn collect_trace(&self, trace_id: &str) -> Vec<InMemorySpan> {
        self.snapshot()
            .into_iter()
            .filter(|s| s.trace_id == trace_id)
            .collect()
    }

    /// Test convenience : true if the exporter is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.snapshot().is_empty()
    }

    /// Test convenience : total span count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.snapshot().len()
    }
}

// ─────────────────────────────────────────────────────────────────────
// Tests.
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_span(name: &str, trace_id: &str, parent: &str) -> InMemorySpan {
        InMemorySpan {
            name: name.into(),
            duration_micros: Some(100),
            kind: "INTERNAL".into(),
            status: SpanStatus::Ok,
            attributes: vec![],
            trace_id: trace_id.into(),
            span_id: format!("{name}-span"),
            parent_span_id: parent.into(),
        }
    }

    #[test]
    fn empty_exporter() {
        let e = InMemoryExporter::new();
        assert!(e.is_empty());
        assert_eq!(e.len(), 0);
        assert_eq!(e.count_by_name("anything"), 0);
        assert_eq!(e.count_errors(), 0);
    }

    #[test]
    fn append_and_snapshot() {
        let e = InMemoryExporter::new();
        e.append(sample_span("efe.meta_swarm.iterate", "abc123", ""));
        e.append(sample_span("efe.meta_swarm.step", "abc123", "iterate-span"));
        let snap = e.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].name, "efe.meta_swarm.iterate");
        assert_eq!(snap[1].name, "efe.meta_swarm.step");
    }

    #[test]
    fn drain_empties() {
        let e = InMemoryExporter::new();
        e.append(sample_span("x", "t", ""));
        let drained = e.drain();
        assert_eq!(drained.len(), 1);
        assert!(e.is_empty());
    }

    #[test]
    fn count_by_name() {
        let e = InMemoryExporter::new();
        e.append(sample_span("efe.meta_swarm.step", "t1", ""));
        e.append(sample_span("efe.meta_swarm.step", "t2", ""));
        e.append(sample_span("efe.meta_swarm.iterate", "t1", ""));
        assert_eq!(e.count_by_name("efe.meta_swarm.step"), 2);
        assert_eq!(e.count_by_name("efe.meta_swarm.iterate"), 1);
        assert_eq!(e.count_by_name("nope"), 0);
    }

    #[test]
    fn count_errors() {
        let e = InMemoryExporter::new();
        let mut s = sample_span("x", "t", "");
        e.append(s.clone());
        s.status = SpanStatus::Error {
            description: "boom".into(),
        };
        e.append(s);
        assert_eq!(e.count_errors(), 1);
    }

    #[test]
    fn collect_trace_filters_by_trace_id() {
        let e = InMemoryExporter::new();
        e.append(sample_span("a", "trace1", ""));
        e.append(sample_span("b", "trace2", ""));
        e.append(sample_span("c", "trace1", "a-span"));
        let t1 = e.collect_trace("trace1");
        assert_eq!(t1.len(), 2);
        let t2 = e.collect_trace("trace2");
        assert_eq!(t2.len(), 1);
    }

    #[test]
    fn span_status_from_otel_status() {
        assert_eq!(SpanStatus::from(&Status::Unset), SpanStatus::Unset);
        assert_eq!(SpanStatus::from(&Status::Ok), SpanStatus::Ok);
        let err_status = Status::error("dim mismatch");
        match SpanStatus::from(&err_status) {
            SpanStatus::Error { description } => assert_eq!(description, "dim mismatch"),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn shared_handle_can_be_cloned_and_observes_same_state() {
        let e = InMemoryExporter::new();
        let e2 = e.clone();
        e.append(sample_span("x", "t", ""));
        assert_eq!(e2.len(), 1);
    }
}
