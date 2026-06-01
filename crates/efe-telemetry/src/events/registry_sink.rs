//! Consume M9 [`RegistryEvent`]s and fan them out to metrics, logs, and
//! the audit sink.
//!
//! The sink owns the consumer side of an unbounded mpsc channel and
//! runs inside a dedicated tokio task. It is decoupled from the
//! registry itself so it can be hot-swapped for testing or to add
//! replication in post-MVP.

use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::mpsc;

use efe_core::alignment::{BreachContext, BreachReport, RegistryEvent};

use crate::logs::audit::AuditSink;
use crate::metrics::instruments;

/// Consumes a `RegistryEvent` stream.
pub struct RegistrySink {
    audit: Option<Arc<dyn AuditSink>>,
}

impl std::fmt::Debug for RegistrySink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistrySink")
            .field("audit_attached", &self.audit.is_some())
            .finish()
    }
}

impl RegistrySink {
    /// Build a sink that writes audit entries into `audit` (optional).
    #[must_use]
    pub fn new(audit: Option<Arc<dyn AuditSink>>) -> Self {
        Self { audit }
    }

    /// Drain `rx` until the sender is dropped. Runs as a standalone task.
    ///
    /// Returns the number of events processed (handy for tests).
    pub async fn run(self, mut rx: mpsc::UnboundedReceiver<RegistryEvent>) -> usize {
        let mut processed = 0usize;
        while let Some(event) = rx.recv().await {
            self.handle(&event);
            processed += 1;
        }
        processed
    }

    /// Process a single event synchronously. Exposed for unit tests and
    /// for callers that want tighter control over scheduling.
    pub fn handle(&self, event: &RegistryEvent) {
        match event {
            RegistryEvent::Registered { key, at_nanos } => {
                instruments::cone_registrations_total()
                    .with_label_values(&["registered", &key.tenant])
                    .inc();
                tracing::info!(
                    event = "cone.registered",
                    tenant = %key.tenant,
                    task = %key.task,
                    version = key.version,
                    at_nanos = at_nanos,
                    "cone registered"
                );
                self.audit(json!({
                    "event": "cone.registered",
                    "tenant": key.tenant,
                    "task": key.task,
                    "version": key.version,
                    "at_nanos": at_nanos,
                }));
            }
            RegistryEvent::Evolved { from, to, op_kind } => {
                instruments::cone_registrations_total()
                    .with_label_values(&["evolved", &from.tenant])
                    .inc();
                tracing::info!(
                    event = "cone.evolved",
                    tenant = %from.tenant,
                    task = %from.task,
                    from_version = from.version,
                    to_version = to.version,
                    op_kind = op_kind.as_str(),
                    "cone evolved"
                );
                self.audit(json!({
                    "event": "cone.evolved",
                    "tenant": from.tenant,
                    "task": from.task,
                    "from": from.to_string(),
                    "to": to.to_string(),
                    "op_kind": op_kind,
                }));
            }
            RegistryEvent::Deleted { key } => {
                instruments::cone_registrations_total()
                    .with_label_values(&["deleted", &key.tenant])
                    .inc();
                tracing::warn!(
                    event = "cone.deleted",
                    tenant = %key.tenant,
                    task = %key.task,
                    version = key.version,
                    "cone deleted"
                );
                self.audit(json!({
                    "event": "cone.deleted",
                    "tenant": key.tenant,
                    "task": key.task,
                    "version": key.version,
                }));
            }
            RegistryEvent::BreachReported { report } => {
                let ctx_label = breach_context_label(&report.context);
                instruments::breaches_total()
                    .with_label_values(&[&report.cone_key.tenant, &report.cone_key.task, ctx_label])
                    .inc();
                instruments::breach_magnitude().observe(f64::from(breach_magnitude(report)));
                tracing::warn!(
                    event = "alignment.breach",
                    tenant = %report.cone_key.tenant,
                    task = %report.cone_key.task,
                    swarm_id = %report.swarm_id,
                    observed_delta = report.observed_delta,
                    cone_radius = report.cone_radius,
                    context = ctx_label,
                    "alignment breach reported"
                );
                self.audit(breach_to_json(report));
            }
            RegistryEvent::PolicyRejected {
                attempted_key,
                reason,
            } => {
                let code = extract_invariant_code(reason);
                instruments::policy_rejections_total()
                    .with_label_values(&[code])
                    .inc();
                tracing::warn!(
                    event = "policy.rejected",
                    code = code,
                    reason = %reason,
                    key = ?attempted_key.as_ref().map(std::string::ToString::to_string),
                    "policy rejection"
                );
                self.audit(json!({
                    "event": "policy.rejected",
                    "code": code,
                    "reason": reason,
                    "attempted_key": attempted_key.as_ref().map(std::string::ToString::to_string),
                }));
            }
            // `RegistryEvent` is `#[non_exhaustive]` cross-crate — future
            // variants must not silently break the sink.
            _ => {
                tracing::debug!("registry sink received unknown RegistryEvent variant; ignored");
            }
        }
    }

    fn audit(&self, entry: Value) {
        if let Some(sink) = &self.audit {
            sink.write_entry(entry);
        }
    }
}

/// Extract the invariant code (`I1`..`I7`) from the reason string, or
/// return a canonical label if none matches.
///
/// The M9 `PolicyEngine` prefixes every policy message with
/// `"policy violation: <code>: ..."`. We split on the first `":"` after
/// `policy violation` and compare exactly, so `"I10"` (a hypothetical
/// future invariant) no longer collides with `"I1"` via substring
/// matching (audit finding M11-#1).
fn extract_invariant_code(reason: &str) -> &'static str {
    // Most policy violations arrive in the form
    // `"policy violation: IN: <details>"`. We extract the token between
    // the first and second colons and compare it exactly.
    let trimmed = reason.trim_start();
    let after_prefix = trimmed.strip_prefix("policy violation:").unwrap_or(trimmed);
    let after_prefix = after_prefix.trim_start();
    // Grab the next token (up to ':' or whitespace).
    let code_candidate: String = after_prefix
        .chars()
        .take_while(|c| *c != ':' && !c.is_whitespace())
        .collect();
    match code_candidate.as_str() {
        "I1" => "I1",
        "I2" => "I2",
        "I3" => "I3",
        "I4" => "I4",
        "I5" => "I5",
        "I6" => "I6",
        "I7" => "I7",
        _ => {
            // Fallback classifications.
            if reason.contains("non-monotone") {
                "non_monotone_version"
            } else if reason.contains("max_versions") {
                "quota_exceeded"
            } else {
                "other"
            }
        }
    }
}

fn breach_context_label(ctx: &BreachContext) -> &'static str {
    match ctx {
        BreachContext::Dissolution => "dissolution",
        BreachContext::FusionRound { .. } => "fusion_round",
        BreachContext::Manual { .. } => "manual",
        _ => "unknown",
    }
}

fn breach_magnitude(report: &BreachReport) -> f32 {
    (report.observed_delta - report.cone_radius).max(0.0)
}

fn breach_to_json(report: &BreachReport) -> Value {
    json!({
        "event": "alignment.breach",
        "tenant": report.cone_key.tenant,
        "task": report.cone_key.task,
        "version": report.cone_key.version,
        "swarm_id": report.swarm_id.to_string(),
        "observed_delta": report.observed_delta,
        "cone_radius": report.cone_radius,
        "timestamp_nanos": report.timestamp_nanos,
        "context": breach_context_label(&report.context),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logs::audit::MemoryAuditSink;
    use efe_core::alignment::{BreachFilter as _BreachFilter, ConeKey};
    use efe_core::SwarmId;

    fn sink() -> (RegistrySink, Arc<MemoryAuditSink>) {
        let audit = Arc::new(MemoryAuditSink::new());
        let sink = RegistrySink::new(Some(audit.clone() as Arc<dyn AuditSink>));
        (sink, audit)
    }

    fn sample_breach() -> BreachReport {
        sample_breach_for("acme", "qa")
    }

    /// Build a [`BreachReport`] with explicit tenant/task so tests that
    /// assert on counter deltas can use unique labels (same isolation
    /// discipline as `registered_increments_counter_and_audit`).
    fn sample_breach_for(tenant: &str, task: &str) -> BreachReport {
        BreachReport::new(
            ConeKey::new(tenant, task, 1).unwrap(),
            SwarmId::new(),
            0.9,
            0.5,
            1_000_000_000,
            BreachContext::Dissolution,
        )
    }

    #[test]
    fn registered_increments_counter_and_audit() {
        // ⚠️ Tenant label isolation.
        //
        // `cone_registrations_total` is a process-wide Prometheus
        // `IntCounterVec` exposed through a singleton registry. Every
        // unit test in this binary shares the same counter family, so
        // using a common label like `"acme"` created a race with other
        // tests (notably `run_drains_channel_until_sender_drops` in
        // this very module) when `cargo test` schedules them in
        // parallel: the `before` read and the `assert_eq!` could be
        // interleaved with another `handle(Registered{tenant:"acme"})`
        // and the delta would come back as +2.
        //
        // Using a unique, test-scoped tenant string guarantees the
        // (event, tenant) label tuple is observed by this test alone
        // and restores a stable `before + 1` invariant. Any new test
        // that reads a counter delta MUST follow the same discipline.
        let (s, audit) = sink();
        let tenant = "acme-registered-counter-test";
        let before = instruments::cone_registrations_total()
            .with_label_values(&["registered", tenant])
            .get();
        s.handle(&RegistryEvent::Registered {
            key: ConeKey::new(tenant, "qa", 1).unwrap(),
            at_nanos: 123,
        });
        assert_eq!(
            instruments::cone_registrations_total()
                .with_label_values(&["registered", tenant])
                .get(),
            before + 1
        );
        let entries = audit.entries();
        assert_eq!(entries.last().unwrap()["event"], "cone.registered");
    }

    #[test]
    fn evolved_event_is_audited() {
        let (s, audit) = sink();
        s.handle(&RegistryEvent::Evolved {
            from: ConeKey::new("acme", "qa", 1).unwrap(),
            to: ConeKey::new("acme", "qa", 2).unwrap(),
            op_kind: "append_anchor".to_string(),
        });
        let e = audit.entries().last().cloned().unwrap();
        assert_eq!(e["event"], "cone.evolved");
        assert_eq!(e["op_kind"], "append_anchor");
    }

    #[test]
    fn deleted_event_is_audited() {
        let (s, audit) = sink();
        s.handle(&RegistryEvent::Deleted {
            key: ConeKey::new("t", "k", 1).unwrap(),
        });
        let e = audit.entries().last().cloned().unwrap();
        assert_eq!(e["event"], "cone.deleted");
    }

    #[test]
    fn breach_increments_counter_and_observes_magnitude() {
        // Same isolation discipline as
        // `registered_increments_counter_and_audit`: use a
        // test-unique (tenant, task) tuple so the `before → +1`
        // invariant never collides with another test running in
        // parallel inside this binary.
        let (s, _audit) = sink();
        let tenant = "acme-breach-counter-test";
        let task = "qa-breach-counter-test";
        let before = instruments::breaches_total()
            .with_label_values(&[tenant, task, "dissolution"])
            .get();
        s.handle(&RegistryEvent::BreachReported {
            report: sample_breach_for(tenant, task),
        });
        let after = instruments::breaches_total()
            .with_label_values(&[tenant, task, "dissolution"])
            .get();
        assert_eq!(after, before + 1);
    }

    #[test]
    fn policy_rejected_extracts_invariant_code() {
        // The `"I5"` reason is a canonical invariant code and must be
        // preserved for the test to remain meaningful. Because another
        // parallel test (or a future one) could also emit a
        // `PolicyRejected{I5}`, we assert strict-inequality on the
        // counter rather than an exact `+1` delta. The
        // `invariant_code_exact_match` test covers the pure function
        // mapping with no shared mutable state.
        let (s, _audit) = sink();
        let before = instruments::policy_rejections_total()
            .with_label_values(&["I5"])
            .get();
        s.handle(&RegistryEvent::PolicyRejected {
            attempted_key: None,
            reason: "policy violation: I5: ...".to_string(),
        });
        let after = instruments::policy_rejections_total()
            .with_label_values(&["I5"])
            .get();
        assert!(
            after > before,
            "policy_rejections_total{{reason=\"I5\"}} did not advance: before={before} after={after}"
        );
    }

    #[test]
    fn policy_rejected_unknown_reason_maps_to_other() {
        assert_eq!(extract_invariant_code("unrelated message"), "other");
        assert_eq!(
            extract_invariant_code("register: non-monotone version"),
            "non_monotone_version"
        );
        assert_eq!(
            extract_invariant_code("register: task reached max_versions_per_task"),
            "quota_exceeded"
        );
    }

    // Regression for audit finding M11-#1: the older `contains()`-based
    // parser would return "I1" for a hypothetical "I10" reason because
    // "I10" contains "I1". The strict token-based parser must report
    // "other" for unknown codes.
    #[test]
    fn invariant_code_i10_is_other_not_i1() {
        assert_eq!(
            extract_invariant_code("policy violation: I10: fake future invariant"),
            "other"
        );
        assert_eq!(
            extract_invariant_code("policy violation: I100: bogus"),
            "other"
        );
    }

    #[test]
    fn invariant_code_exact_match() {
        for code in ["I1", "I2", "I3", "I4", "I5", "I6", "I7"] {
            let msg = format!("policy violation: {code}: details");
            assert_eq!(extract_invariant_code(&msg), code);
        }
    }

    #[test]
    fn invariant_code_handles_leading_whitespace() {
        assert_eq!(extract_invariant_code("   policy violation: I3: ok"), "I3");
    }

    #[test]
    fn breach_magnitude_is_non_negative() {
        let mut r = sample_breach();
        r.observed_delta = 0.3;
        r.cone_radius = 0.5;
        assert!(breach_magnitude(&r).abs() < 1e-6);
    }

    #[tokio::test]
    async fn run_drains_channel_until_sender_drops() {
        let (s, _audit) = sink();
        let (tx, rx) = mpsc::unbounded_channel();
        tx.send(RegistryEvent::Registered {
            key: ConeKey::new("acme", "qa", 1).unwrap(),
            at_nanos: 1,
        })
        .unwrap();
        tx.send(RegistryEvent::Deleted {
            key: ConeKey::new("acme", "qa", 1).unwrap(),
        })
        .unwrap();
        drop(tx);
        let count = s.run(rx).await;
        assert_eq!(count, 2);
    }

    // Silence unused import warning in rust-analyzer.
    #[allow(dead_code)]
    fn _assert_breach_filter(_f: _BreachFilter) {}
}
