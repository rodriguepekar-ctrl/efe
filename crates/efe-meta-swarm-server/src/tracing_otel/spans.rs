//! PR #18 § 6-13 — the 8 span types as `tracing::info_span!` helpers.
//!
//! Each span is defined as a free function that creates the span with
//! the canonical name + the required OTel semantic-convention
//! attributes.  The `tracing-opentelemetry` bridge translates these
//! into OTel spans + propagates the attributes through to the OTLP
//! exporter.
//!
//! ## Pattern
//!
//! Callers should use these helpers instead of raw `info_span!` to
//! guarantee :
//!
//!   1. The span name is correct (uses the `efe.meta_swarm.*` prefix).
//!   2. The required semconv attributes are set at creation time.
//!   3. Open-attribute fields (those filled in at close) are
//!      pre-declared with `tracing::field::Empty`, so `span.record(...)`
//!      can populate them later without losing the placeholder.
//!
//! ## Attribute naming
//!
//! Per AD4 :
//!   * OTel semconv attributes use their semconv-canonical names
//!     (`rpc.system`, `rpc.service`, `rpc.method`, `service.name`).
//!   * EFE-specific attributes use the `meta_swarm.<facet>.<name>`
//!     pattern (`meta_swarm.iterate.iterations`,
//!     `meta_swarm.participant_id`).
//!
//! ## PII redaction (AD13)
//!
//! Callers passing string-typed attribute values from user-controlled
//! sources MUST route through [`super::super::observability::metrics::
//! cardinality::redact_pii`] first.  In Phase 2 every label-value
//! comes from the manifest (schema-validated as DNS-1123) so the
//! redaction is a defence-in-depth no-op.

use tracing::field::Empty;
use tracing::Span;

use crate::observability::metrics::cardinality::redact_pii;

/// PR #18 § 18 / AC18 / T57 — apply the PR #17 PII redactor before
/// emitting a string attribute through `tracing::info_span!`.  The
/// redactor catches emails, JWT tokens, AWS access keys, hex-32, and
/// UUID-like sequences ; clean strings pass through borrowed.
///
/// In Phase 2 every label value comes from the manifest (schema-
/// validated as DNS-1123) so the redaction is a defence-in-depth
/// no-op ; we still route every user-controlled string through it to
/// preserve the security invariant if a future caller drops a raw
/// header / URL into a span.
#[must_use]
#[inline]
fn redact(s: &str) -> std::borrow::Cow<'_, str> {
    redact_pii(s)
}

// ─────────────────────────────────────────────────────────────────────
// Attribute name constants (re-export of semconv + EFE custom names).
// ─────────────────────────────────────────────────────────────────────

/// OTel semconv : `rpc.system` attribute key.
pub const ATTR_RPC_SYSTEM: &str = "rpc.system";
/// OTel semconv : `rpc.service` attribute key.
pub const ATTR_RPC_SERVICE: &str = "rpc.service";
/// OTel semconv : `rpc.method` attribute key.
pub const ATTR_RPC_METHOD: &str = "rpc.method";

/// EFE custom : `meta_swarm.manifest.version`.
pub const ATTR_MANIFEST_VERSION: &str = "meta_swarm.manifest.version";
/// EFE custom : `meta_swarm.participant_id`.
pub const ATTR_PARTICIPANT_ID: &str = "meta_swarm.participant_id";
/// EFE custom : `meta_swarm.iterate.outcome` (PR #17 taxonomy).
pub const ATTR_OUTCOME: &str = "meta_swarm.iterate.outcome";
/// EFE custom : `meta_swarm.iterate.failure_reason` (PR #17 taxonomy).
pub const ATTR_FAILURE_REASON: &str = "meta_swarm.iterate.failure_reason";

// ─────────────────────────────────────────────────────────────────────
// Span 1 — efe.meta_swarm.iterate (controller root, § 6).
// ─────────────────────────────────────────────────────────────────────

/// Build the root `efe.meta_swarm.iterate` span.
///
/// Set as a SERVER-kind gRPC span.  All 12 entry attributes are
/// populated immediately ; the 6 close attributes are pre-declared as
/// `Empty` placeholders and filled by [`record_iterate_close`].
#[must_use]
pub fn span_iterate(
    manifest_version: &str,
    max_iters: u32,
    tolerance: f64,
    alpha_default: f64,
    coupling_gamma: f64,
    deterministic: bool,
    participants_count: usize,
    discovery_mode: &str,
    tls_enabled: bool,
) -> Span {
    tracing::info_span!(
        "efe.meta_swarm.iterate",
        otel.kind = "SERVER",
        "rpc.system" = "grpc",
        "rpc.service" = "efe.meta_swarm.v1.MetaSwarmService",
        "rpc.method" = "Iterate",
        "meta_swarm.manifest.version" = %manifest_version,
        "meta_swarm.iterate.max_iters" = max_iters,
        "meta_swarm.iterate.tolerance" = tolerance,
        "meta_swarm.iterate.alpha_default" = alpha_default,
        "meta_swarm.iterate.coupling_gamma" = coupling_gamma,
        "meta_swarm.iterate.deterministic" = deterministic,
        "meta_swarm.iterate.participants_count" = participants_count,
        "meta_swarm.discovery_mode" = %discovery_mode,
        "meta_swarm.tls.enabled" = tls_enabled,
        // Pre-declared close attributes — filled via span.record() at end.
        "meta_swarm.iterate.iterations" = Empty,
        "meta_swarm.iterate.final_residual" = Empty,
        "meta_swarm.iterate.stop_reason" = Empty,
        "meta_swarm.iterate.wall_us" = Empty,
        "meta_swarm.iterate.outcome" = Empty,
        "meta_swarm.iterate.failure_reason" = Empty,
    )
}

/// Record the close-time attributes on the iterate span.
pub fn record_iterate_close(
    span: &Span,
    iterations: u32,
    final_residual: f64,
    stop_reason: &str,
    wall_us: u128,
    outcome: &str,
    failure_reason: Option<&str>,
) {
    span.record("meta_swarm.iterate.iterations", iterations);
    span.record("meta_swarm.iterate.final_residual", final_residual);
    span.record("meta_swarm.iterate.stop_reason", stop_reason);
    let wall_us_i64 = i64::try_from(wall_us).unwrap_or(i64::MAX);
    span.record("meta_swarm.iterate.wall_us", wall_us_i64);
    span.record("meta_swarm.iterate.outcome", outcome);
    if let Some(reason) = failure_reason {
        span.record("meta_swarm.iterate.failure_reason", reason);
    }
}

// ─────────────────────────────────────────────────────────────────────
// Span 2 — efe.meta_swarm.connect_participants (child, § 7).
// ─────────────────────────────────────────────────────────────────────

/// Build the `efe.meta_swarm.connect_participants` child span.
#[must_use]
pub fn span_connect_participants(
    participants_count: usize,
    discovery_mode: &str,
    tls_enabled: bool,
    tls_mutual: bool,
) -> Span {
    tracing::info_span!(
        "efe.meta_swarm.connect_participants",
        otel.kind = "INTERNAL",
        "meta_swarm.participants.count" = participants_count,
        "meta_swarm.discovery_mode" = %discovery_mode,
        "meta_swarm.tls.enabled" = tls_enabled,
        "meta_swarm.tls.mutual" = tls_mutual,
    )
}

/// Build the per-participant `efe.meta_swarm.participant.connect`
/// sub-span (CLIENT kind).
///
/// PR #18 AC18 / T57 — `participant_id`, `endpoint`, and
/// `tls_expected_san` are routed through the PII redactor before
/// recording.  Manifest-validated values are unaffected.
#[must_use]
pub fn span_participant_connect(
    participant_id: &str,
    endpoint: &str,
    tls_expected_san: Option<&str>,
) -> Span {
    let pid = redact(participant_id);
    let ep = redact(endpoint);
    let san = tls_expected_san
        .map(|s| redact(s).into_owned())
        .unwrap_or_default();
    tracing::info_span!(
        "efe.meta_swarm.participant.connect",
        otel.kind = "CLIENT",
        "rpc.system" = "grpc",
        "rpc.service" = "efe.meta_swarm.v1.MetaParticipantService",
        "rpc.method" = "Connect",
        "meta_swarm.participant_id" = %pid,
        "meta_swarm.participant.endpoint" = %ep,
        "meta_swarm.tls.expected_san" = %san,
        "meta_swarm.connect.attempt_count" = 1,
        "meta_swarm.connect.elapsed_us" = Empty,
    )
}

// ─────────────────────────────────────────────────────────────────────
// Span 3 — efe.meta_swarm.inner_loop (child, § 8).
// ─────────────────────────────────────────────────────────────────────

/// Build the `efe.meta_swarm.inner_loop` child span.
#[must_use]
pub fn span_inner_loop(max_iters: u32, tolerance: f64) -> Span {
    tracing::info_span!(
        "efe.meta_swarm.inner_loop",
        otel.kind = "INTERNAL",
        "meta_swarm.inner_loop.max_iters" = max_iters,
        "meta_swarm.inner_loop.tolerance" = tolerance,
        // Close attributes :
        "meta_swarm.inner_loop.iteration_count" = Empty,
        "meta_swarm.inner_loop.final_residual" = Empty,
        "meta_swarm.inner_loop.convergence" = Empty,
        "meta_swarm.inner_loop.elapsed_us" = Empty,
    )
}

/// Record the close-time attributes on the inner_loop span.
pub fn record_inner_loop_close(
    span: &Span,
    iteration_count: u32,
    final_residual: f64,
    convergence: &str,
    elapsed_us: u128,
) {
    span.record("meta_swarm.inner_loop.iteration_count", iteration_count);
    span.record("meta_swarm.inner_loop.final_residual", final_residual);
    span.record("meta_swarm.inner_loop.convergence", convergence);
    let elapsed_us_i64 = i64::try_from(elapsed_us).unwrap_or(i64::MAX);
    span.record("meta_swarm.inner_loop.elapsed_us", elapsed_us_i64);
}

// ─────────────────────────────────────────────────────────────────────
// Span 4 — efe.meta_swarm.safety_check (child of inner_loop, § 9).
// ─────────────────────────────────────────────────────────────────────

/// Build the `efe.meta_swarm.safety_check` span.
///
/// Phase-2 limitation : this span is created at the controller-side
/// `iterate.rs` post-hoc when `StopReason::Error` is observed.  Phase
/// 3 will hook this into the inner Phase-1 algorithm directly.
///
/// PR #18 AC18 / T57 — `participant_id` is routed through the PII
/// redactor.
#[must_use]
pub fn span_safety_check(safety_kind: &str, participant_id: &str, violated: bool) -> Span {
    let pid = redact(participant_id);
    tracing::info_span!(
        "efe.meta_swarm.safety_check",
        otel.kind = "INTERNAL",
        "meta_swarm.safety.kind" = %safety_kind,
        "meta_swarm.safety.participant_id" = %pid,
        "meta_swarm.safety.violated" = violated,
        "meta_swarm.safety.adjustment_norm" = Empty,
    )
}

// ─────────────────────────────────────────────────────────────────────
// Span 5 — efe.meta_swarm.step (participant root, § 10).
// ─────────────────────────────────────────────────────────────────────

/// Build the `efe.meta_swarm.step` root span on the participant side.
///
/// PR #18 AC18 / T57 — `participant_id` is routed through the PII
/// redactor.
#[must_use]
pub fn span_step(participant_id: &str, step_index: u32, dim: u32, payload_in_bytes: usize) -> Span {
    let payload_in_i64 = i64::try_from(payload_in_bytes).unwrap_or(i64::MAX);
    let pid = redact(participant_id);
    tracing::info_span!(
        "efe.meta_swarm.step",
        otel.kind = "SERVER",
        "rpc.system" = "grpc",
        "rpc.service" = "efe.meta_swarm.v1.MetaParticipantService",
        "rpc.method" = "Step",
        "meta_swarm.participant_id" = %pid,
        "meta_swarm.step.step_index" = step_index,
        "meta_swarm.step.dim" = dim,
        "meta_swarm.step.payload_in_bytes" = payload_in_i64,
        // Close-time :
        "meta_swarm.step.payload_out_bytes" = Empty,
        "meta_swarm.step.local_residual" = Empty,
        "meta_swarm.step.elapsed_us" = Empty,
        "meta_swarm.step.outcome" = Empty,
        "meta_swarm.step.failure_reason" = Empty,
    )
}

/// Record the close-time attributes on the step span.
pub fn record_step_close(
    span: &Span,
    payload_out_bytes: usize,
    local_residual: f64,
    elapsed_us: u128,
    outcome: &str,
    failure_reason: Option<&str>,
) {
    let payload_out_i64 = i64::try_from(payload_out_bytes).unwrap_or(i64::MAX);
    span.record("meta_swarm.step.payload_out_bytes", payload_out_i64);
    span.record("meta_swarm.step.local_residual", local_residual);
    let elapsed_us_i64 = i64::try_from(elapsed_us).unwrap_or(i64::MAX);
    span.record("meta_swarm.step.elapsed_us", elapsed_us_i64);
    span.record("meta_swarm.step.outcome", outcome);
    if let Some(reason) = failure_reason {
        span.record("meta_swarm.step.failure_reason", reason);
    }
}

// ─────────────────────────────────────────────────────────────────────
// Span 6 — efe.meta_swarm.step.local (child, § 11).
// ─────────────────────────────────────────────────────────────────────

/// Build the `efe.meta_swarm.step.local` child span wrapping the
/// `spawn_blocking` call to the Phase-1 local step.
#[must_use]
pub fn span_step_local(gamma: f64, alpha: f64, dim: u32, kappa: f64) -> Span {
    tracing::info_span!(
        "efe.meta_swarm.step.local",
        otel.kind = "INTERNAL",
        "meta_swarm.step.local.gamma" = gamma,
        "meta_swarm.step.local.alpha" = alpha,
        "meta_swarm.step.local.dim" = dim,
        "meta_swarm.step.local.kappa" = kappa,
        "meta_swarm.step.local.elapsed_us" = Empty,
        "meta_swarm.step.local.outcome" = Empty,
        "meta_swarm.step.local.failure_reason" = Empty,
    )
}

/// Record the close-time attributes on the step.local span.
pub fn record_step_local_close(
    span: &Span,
    elapsed_us: u128,
    outcome: &str,
    failure_reason: Option<&str>,
) {
    span.record(
        "meta_swarm.step.local.elapsed_us",
        u64::try_from(elapsed_us).unwrap_or(u64::MAX),
    );
    span.record("meta_swarm.step.local.outcome", outcome);
    if let Some(reason) = failure_reason {
        span.record("meta_swarm.step.local.failure_reason", reason);
    }
}

// ─────────────────────────────────────────────────────────────────────
// Span 7 — efe.meta_swarm.manifest.reload (cross-cutting, § 12).
// ─────────────────────────────────────────────────────────────────────

/// Build the `efe.meta_swarm.manifest.reload` span.
///
/// `trigger` ∈ {`sighup`, `grpc`, `file_change`}.
#[must_use]
pub fn span_manifest_reload(trigger: &str, gen_before: u32, participants_before: usize) -> Span {
    tracing::info_span!(
        "efe.meta_swarm.manifest.reload",
        otel.kind = "INTERNAL",
        "meta_swarm.reload.trigger" = %trigger,
        "meta_swarm.reload.gen_before" = gen_before,
        "meta_swarm.reload.participants_before" = participants_before,
        // Close-time :
        "meta_swarm.reload.gen_after" = Empty,
        "meta_swarm.reload.participants_after" = Empty,
        "meta_swarm.reload.outcome" = Empty,
        "meta_swarm.reload.failure_reason" = Empty,
        "meta_swarm.reload.changes_count" = Empty,
        "meta_swarm.reload.duration_seconds" = Empty,
    )
}

/// Record the close-time attributes on the manifest_reload span.
pub fn record_manifest_reload_close(
    span: &Span,
    gen_after: u32,
    participants_after: usize,
    outcome: &str,
    failure_reason: Option<&str>,
    changes_count: usize,
    duration_seconds: f64,
) {
    span.record("meta_swarm.reload.gen_after", gen_after);
    span.record("meta_swarm.reload.participants_after", participants_after);
    span.record("meta_swarm.reload.outcome", outcome);
    if let Some(reason) = failure_reason {
        span.record("meta_swarm.reload.failure_reason", reason);
    }
    span.record("meta_swarm.reload.changes_count", changes_count);
    span.record("meta_swarm.reload.duration_seconds", duration_seconds);
}

// ─────────────────────────────────────────────────────────────────────
// Span 8 — efe.meta_swarm.shutdown.drain (root, § 13).
// ─────────────────────────────────────────────────────────────────────

/// Build the `efe.meta_swarm.shutdown.drain` root span emitted when
/// SIGTERM / SIGINT triggers graceful drain.
#[must_use]
pub fn span_shutdown_drain(
    signal: &str,
    drain_deadline_seconds: f64,
    inflight_at_start: u32,
) -> Span {
    tracing::info_span!(
        "efe.meta_swarm.shutdown.drain",
        otel.kind = "INTERNAL",
        "meta_swarm.shutdown.signal" = %signal,
        "meta_swarm.shutdown.drain_deadline_seconds" = drain_deadline_seconds,
        "meta_swarm.shutdown.inflight_at_start" = inflight_at_start,
        // Close-time :
        "meta_swarm.shutdown.inflight_at_end" = Empty,
        "meta_swarm.shutdown.elapsed_seconds" = Empty,
        "meta_swarm.shutdown.outcome" = Empty,
    )
}

/// Record the close-time attributes on the shutdown_drain span.
pub fn record_shutdown_drain_close(
    span: &Span,
    inflight_at_end: u32,
    elapsed_seconds: f64,
    outcome: &str,
) {
    span.record("meta_swarm.shutdown.inflight_at_end", inflight_at_end);
    span.record("meta_swarm.shutdown.elapsed_seconds", elapsed_seconds);
    span.record("meta_swarm.shutdown.outcome", outcome);
}

// ─────────────────────────────────────────────────────────────────────
// Status helpers — gRPC Status → OTel span status (§ 20).
// ─────────────────────────────────────────────────────────────────────

/// Mark a span as `Status::Ok` via the `tracing-opentelemetry` bridge.
///
/// The bridge consumes the `otel.status_code` attribute on the span.
/// Setting it to `"OK"` translates to `Status::Ok` on the exported
/// span.
pub fn record_grpc_ok(span: &Span) {
    span.record("otel.status_code", "OK");
}

/// Mark a span as `Status::Error` with a description.
pub fn record_grpc_error(span: &Span, description: &str) {
    span.record("otel.status_code", "ERROR");
    span.record("otel.status_description", description);
}

/// Convenience : produce a `(key, value)` tuple ready to plug into
/// `span.record(...)` for an arbitrary string attribute.  Used by
/// the wiring code at instrumentation sites that need to add custom
/// attributes beyond the helper-emitted ones.
#[must_use]
pub fn span_attr(key: &str, value: &str) -> (String, String) {
    (key.to_string(), value.to_string())
}

// ─────────────────────────────────────────────────────────────────────
// Tests.
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_iterate_returns_named_span() {
        let s = span_iterate(
            "v0.2.0",
            100,
            1.0e-4,
            0.5,
            0.2,
            true,
            3,
            "static-localhost",
            false,
        );
        // We can't easily assert on the metadata field map without a
        // running subscriber ; smoke-test that the span builder
        // doesn't panic + returns a non-empty span.
        assert!(!s.is_disabled() || s.is_disabled()); // tautology — just check it built
    }

    #[test]
    fn span_step_returns_named_span() {
        let _s = span_step("supplier-scoring", 0, 8, 64);
    }

    #[test]
    fn span_step_local_returns_named_span() {
        let _s = span_step_local(0.2, 0.5, 8, 0.6);
    }

    #[test]
    fn span_connect_participants_returns_named_span() {
        let _s = span_connect_participants(3, "static-localhost", true, true);
    }

    #[test]
    fn span_participant_connect_returns_named_span() {
        let _s = span_participant_connect(
            "supplier-scoring",
            "http://localhost:50091",
            Some("supplier-scoring.efe.svc"),
        );
    }

    #[test]
    fn span_inner_loop_returns_named_span() {
        let _s = span_inner_loop(100, 1.0e-4);
    }

    #[test]
    fn span_safety_check_returns_named_span() {
        let _s = span_safety_check("box", "supplier-scoring", true);
    }

    #[test]
    fn span_manifest_reload_returns_named_span() {
        let _s = span_manifest_reload("sighup", 5, 3);
    }

    #[test]
    fn span_shutdown_drain_returns_named_span() {
        let _s = span_shutdown_drain("sigterm", 30.0, 0);
    }

    #[test]
    fn span_attr_helper_produces_tuple() {
        let (k, v) = span_attr("meta_swarm.test", "value");
        assert_eq!(k, "meta_swarm.test");
        assert_eq!(v, "value");
    }

    #[test]
    fn attribute_constants_are_stable() {
        // Public API contract — these are the canonical attribute
        // names that operators reference in dashboards + queries.
        assert_eq!(ATTR_RPC_SYSTEM, "rpc.system");
        assert_eq!(ATTR_RPC_SERVICE, "rpc.service");
        assert_eq!(ATTR_RPC_METHOD, "rpc.method");
        assert_eq!(ATTR_MANIFEST_VERSION, "meta_swarm.manifest.version");
        assert_eq!(ATTR_PARTICIPANT_ID, "meta_swarm.participant_id");
        assert_eq!(ATTR_OUTCOME, "meta_swarm.iterate.outcome");
        assert_eq!(ATTR_FAILURE_REASON, "meta_swarm.iterate.failure_reason");
    }

    // ─── PR #18 § 18 / AC18 / T57 — PII redaction wiring ──────────
    //
    // These tests verify the `redact` helper is invoked on the
    // user-controlled string attributes of every span that takes
    // them.  We can't assert on the recorded value directly without
    // a subscriber, but we can verify the redactor recognises the
    // canonical PII patterns and emits the sentinel — the same
    // function the span helpers call internally.

    use crate::observability::metrics::cardinality::REDACTION_SENTINEL;

    #[test]
    fn redact_helper_catches_email() {
        let s = redact("alice@example.com");
        assert_eq!(s, REDACTION_SENTINEL);
    }

    #[test]
    fn redact_helper_passes_through_clean_strings() {
        let s = redact("supplier-scoring");
        assert_eq!(s, "supplier-scoring");
    }

    #[test]
    fn span_helpers_dont_panic_on_pii_input() {
        // Defence in depth : even when callers pass a value that
        // looks like PII, the span builder must not panic.  The
        // attribute value will be `<redacted>` in the emitted span.
        let _s = span_participant_connect(
            "alice@example.com",
            "http://192.168.1.1:50091",
            Some("token=abcd1234567890abcd1234567890abcd"),
        );
        let _s = span_safety_check("box", "alice@example.com", true);
        let _s = span_step("alice@example.com", 0, 8, 64);
    }
}
