//! [`CallContext`] — per-invocation runtime context.

use efe_nano_sdk::{TenantId, TraceId};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::time::Instant;
use uuid::Uuid;

use crate::audit::AuditWriter;

/// **Sprint 7 — N7** : Shared swarm-wide cost tracker.
///
/// Tracks cumulative `cost_cents × 10_000` (fixed-point, 4 decimals)
/// across all completed ops of a swarm.  Used by the dispatcher to
/// enforce `SwarmRequest::cost_cap_cents`.
///
/// We use fixed-point integer atomic ops (not `AtomicU64` over `f64`)
/// because `f64::to_bits` round-trips aren't lockless-safe under
/// fetch_add semantics.  `0.0001 cent` precision = $0.000001 per call —
/// far below any real billing granularity.
#[derive(Debug, Default)]
pub struct SwarmCostTracker {
    /// `cost_cents × 10_000`, summed across completed ops.
    accumulated_micro_cents: AtomicU64,
    /// `cap × 10_000`, OR `u64::MAX` for "no cap".
    cap_micro_cents: u64,
}

impl SwarmCostTracker {
    /// Construct with an explicit cap in cents.  `None` = no cap.
    pub fn new(cap_cents: Option<f64>) -> Self {
        let cap_micro = match cap_cents {
            None => u64::MAX,
            Some(c) if c < 0.0 || !c.is_finite() => u64::MAX,
            Some(c) => (c * 10_000.0).round() as u64,
        };
        Self {
            accumulated_micro_cents: AtomicU64::new(0),
            cap_micro_cents: cap_micro,
        }
    }

    /// Add `cost_cents` to the accumulator.  Returns the new total
    /// **in cents** (rounded to 4 decimals).  Non-finite or negative
    /// values are ignored (defensive — runtimes should never emit them).
    pub fn add(&self, cost_cents: f64) -> f64 {
        if !cost_cents.is_finite() || cost_cents <= 0.0 {
            return self.total_cents();
        }
        let inc = (cost_cents * 10_000.0).round() as u64;
        let prev = self.accumulated_micro_cents.fetch_add(inc, Ordering::Relaxed);
        ((prev + inc) as f64) / 10_000.0
    }

    /// Current accumulated total in cents.
    pub fn total_cents(&self) -> f64 {
        (self.accumulated_micro_cents.load(Ordering::Relaxed) as f64) / 10_000.0
    }

    /// True if the accumulated total has reached or exceeded the cap.
    /// `u64::MAX` cap ⇒ always false.
    pub fn cap_reached(&self) -> bool {
        self.cap_micro_cents != u64::MAX
            && self.accumulated_micro_cents.load(Ordering::Relaxed) >= self.cap_micro_cents
    }

    /// Cap in cents (`None` ≡ no cap).
    pub fn cap_cents(&self) -> Option<f64> {
        if self.cap_micro_cents == u64::MAX {
            None
        } else {
            Some(self.cap_micro_cents as f64 / 10_000.0)
        }
    }
}

/// Per-invocation context passed to the runtime executor.
///
/// Contains :
/// - Tenant + trace identifiers
/// - Deadline (for timeout enforcement)
/// - Audit writer (where to record the trace)
/// - Budget checker (where to charge cost)
/// - Idempotency cache (Sprint 2)
///
/// **Sprint 7 changes** :
/// - **N1** : `parent_trace_id: Option<TraceId>` →
///   `parent_trace_ids: Vec<TraceId>` so `Merge` ops with multiple
///   upstream refs record all ancestors.
/// - **N2** : add `caller_agent_id: Option<String>` so traces capture
///   "which Claude session caused this op".
/// - **N7** : add `cost_tracker: Option<Arc<SwarmCostTracker>>` for
///   swarm-wide cost cap enforcement.
pub struct CallContext {
    pub tenant_id: TenantId,
    pub trace_id: TraceId,

    /// **Sprint 7 — N1** : DAG parents (one per `Ref` / `Merge` source).
    /// Empty `Vec` ≡ layer-0 op.
    pub parent_trace_ids: Vec<TraceId>,

    pub swarm_request_id: Uuid,
    pub op_id: String,

    /// **Sprint 7 — N2** : Identity of the external agent that issued
    /// the swarm (e.g. `"claude-sonnet-4-5"`).  Propagated from
    /// `SwarmRequest.caller_agent_id` into every trace of the swarm.
    pub caller_agent_id: Option<String>,

    pub deadline: Instant,
    pub audit: Arc<dyn AuditWriter>,

    /// **Sprint 7 — N7** : Shared swarm cost tracker.  `None` for
    /// stand-alone (non-swarm) invocations.  When `Some`, runtimes
    /// MUST call `cost_tracker.add(cost_cents)` after a successful
    /// execution so the dispatcher can detect cap breaches.
    pub cost_tracker: Option<Arc<SwarmCostTracker>>,

    /// **Sprint 7 — N6** : optional override for `input_hash_source` in
    /// the emitted trace.  The dispatcher computes this from the raw
    /// `OperationInput` AST and passes it down ; runtimes that see
    /// `Some(h)` use it instead of recomputing the source hash from
    /// the resolved value.
    ///
    /// `None` ⇒ stand-alone invocation, runtime uses
    /// `input_hash_source = input_hash_resolved` (the resolved hash).
    pub input_hash_source_override: Option<[u8; 32]>,

    /// **Sprint 8 — N14** : Swarm envelope anchor.  Set by the
    /// dispatcher at swarm dispatch time ; propagated to every emitted
    /// trace so the auditor can find all rows of a swarm via a single
    /// indexed query.
    ///
    /// `None` ⇒ stand-alone (non-swarm) invocation — no envelope.
    pub swarm_envelope_trace_id: Option<TraceId>,
}

impl CallContext {
    /// Construct a CallContext for a stand-alone (non-swarm) invocation.
    /// Used by unit tests and direct runtime calls.
    pub fn stand_alone(
        tenant_id: TenantId,
        audit: Arc<dyn AuditWriter>,
        wall_clock_budget_ms: u32,
    ) -> Self {
        Self {
            tenant_id,
            trace_id: TraceId::new(),
            parent_trace_ids: vec![],
            swarm_request_id: Uuid::new_v4(),
            op_id: "stand_alone".to_string(),
            caller_agent_id: None,
            deadline: Instant::now()
                + tokio::time::Duration::from_millis(wall_clock_budget_ms as u64),
            audit,
            cost_tracker: None,
            input_hash_source_override: None,
            swarm_envelope_trace_id: None,
        }
    }

    /// Construct a CallContext as part of a swarm.
    ///
    /// **Sprint 7** : signature extended with `parent_trace_ids`,
    /// `caller_agent_id`, and `cost_tracker`.
    #[allow(clippy::too_many_arguments)]
    pub fn from_swarm(
        tenant_id: TenantId,
        swarm_request_id: Uuid,
        op_id: impl Into<String>,
        parent_trace_ids: Vec<TraceId>,
        caller_agent_id: Option<String>,
        audit: Arc<dyn AuditWriter>,
        wall_clock_budget_ms: u32,
        cost_tracker: Option<Arc<SwarmCostTracker>>,
    ) -> Self {
        Self {
            tenant_id,
            trace_id: TraceId::new(),
            parent_trace_ids,
            swarm_request_id,
            op_id: op_id.into(),
            caller_agent_id,
            deadline: Instant::now()
                + tokio::time::Duration::from_millis(wall_clock_budget_ms as u64),
            audit,
            cost_tracker,
            input_hash_source_override: None,
            swarm_envelope_trace_id: None,
        }
    }

    /// Returns remaining time before deadline (None if past deadline).
    pub fn remaining_time(&self) -> Option<tokio::time::Duration> {
        self.deadline.checked_duration_since(Instant::now())
    }

    /// Returns true if the deadline has passed.
    pub fn is_past_deadline(&self) -> bool {
        Instant::now() >= self.deadline
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::stub::ConsoleAuditWriter;

    #[tokio::test]
    async fn stand_alone_context_has_correct_tenant() {
        let tenant = TenantId::new();
        let audit: Arc<dyn AuditWriter> = Arc::new(ConsoleAuditWriter::new());
        let ctx = CallContext::stand_alone(tenant, audit, 5000);
        assert_eq!(ctx.tenant_id, tenant);
        assert!(ctx.parent_trace_ids.is_empty());
        assert_eq!(ctx.op_id, "stand_alone");
        assert!(ctx.caller_agent_id.is_none());
        assert!(ctx.cost_tracker.is_none());
    }

    #[tokio::test]
    async fn deadline_in_future_by_default() {
        let audit: Arc<dyn AuditWriter> = Arc::new(ConsoleAuditWriter::new());
        let ctx = CallContext::stand_alone(TenantId::new(), audit, 5000);
        assert!(!ctx.is_past_deadline());
        assert!(ctx.remaining_time().is_some());
        assert!(ctx.remaining_time().unwrap().as_millis() <= 5000);
    }

    #[tokio::test]
    async fn from_swarm_propagates_parent_trace_ids() {
        let parent_a = TraceId::new();
        let parent_b = TraceId::new();
        let audit: Arc<dyn AuditWriter> = Arc::new(ConsoleAuditWriter::new());
        let ctx = CallContext::from_swarm(
            TenantId::new(),
            Uuid::new_v4(),
            "op-1",
            vec![parent_a, parent_b],
            Some("claude-sonnet-4-5".to_string()),
            audit,
            5000,
            None,
        );
        assert_eq!(ctx.parent_trace_ids, vec![parent_a, parent_b]);
        assert_eq!(ctx.op_id, "op-1");
        assert_eq!(ctx.caller_agent_id.as_deref(), Some("claude-sonnet-4-5"));
    }

    // ─────────────────────────────────────────────────────────────────
    //  Sprint 7 — N7 SwarmCostTracker
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn cost_tracker_uncapped_never_breaches() {
        let t = SwarmCostTracker::new(None);
        assert!(!t.cap_reached());
        t.add(1_000_000.0);
        assert!(!t.cap_reached());
        assert_eq!(t.cap_cents(), None);
    }

    #[test]
    fn cost_tracker_accumulates_correctly() {
        let t = SwarmCostTracker::new(Some(10.0));
        assert_eq!(t.add(2.5), 2.5);
        assert_eq!(t.add(3.0), 5.5);
        assert_eq!(t.total_cents(), 5.5);
        assert!(!t.cap_reached());
    }

    #[test]
    fn cost_tracker_breaches_cap_exact() {
        let t = SwarmCostTracker::new(Some(10.0));
        t.add(9.9999);
        assert!(!t.cap_reached());
        t.add(0.0001);
        assert!(t.cap_reached());
    }

    #[test]
    fn cost_tracker_ignores_invalid_input() {
        let t = SwarmCostTracker::new(Some(10.0));
        t.add(f64::NAN);
        t.add(f64::NEG_INFINITY);
        t.add(-5.0);
        assert_eq!(t.total_cents(), 0.0);
    }

    #[test]
    fn cost_tracker_rejects_invalid_cap() {
        let t = SwarmCostTracker::new(Some(f64::NAN));
        assert_eq!(t.cap_cents(), None);
        let t = SwarmCostTracker::new(Some(-1.0));
        assert_eq!(t.cap_cents(), None);
    }

    #[test]
    fn cost_tracker_concurrent_adds_are_safe() {
        use std::thread;
        let t = Arc::new(SwarmCostTracker::new(Some(10_000.0)));
        let mut handles = Vec::new();
        for _ in 0..16 {
            let t = Arc::clone(&t);
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    t.add(1.0);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // 16 × 100 × 1.0 = 1600.0
        assert!((t.total_cents() - 1_600.0).abs() < 0.0001);
    }
}
