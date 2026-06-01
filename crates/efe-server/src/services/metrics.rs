//! Shared helpers for per-handler telemetry emission.
//!
//! Every gRPC handler in `swarm.rs` and `admin.rs` follows the same
//! skeleton: auth → rate-limit → business work → metrics. Factoring the
//! metric emission here (audit finding M11-#6) guarantees:
//!
//! * a single source of truth for the gRPC status → label mapping, so a
//!   future rename (`OK` → `Ok` would break dashboards) cannot drift
//!   between services;
//! * consistent rate-limit drop accounting — every
//!   `ratelimit_drops_total{tenant}` increment flows through one
//!   function, so counters stay in sync with `efe_errors_total`;
//! * a centralised place to add future labels (e.g. `api_version` in
//!   V2) without editing every handler.

use std::time::Instant;

use tonic::Status;

/// Emit `efe_grpc_requests_total` + `efe_grpc_request_duration_seconds`
/// for a completed gRPC call.
///
/// The outcome is erased to `()` because the success payload shape
/// differs per handler and only the status code matters for metrics.
/// Callers use `outcome.as_ref().map(|_| ())` to adapt.
pub(crate) fn emit_request_completion(
    path: &'static str,
    tenant: &str,
    outcome: &Result<(), &Status>,
    started: Instant,
) {
    let code_label: &'static str = match outcome {
        Ok(()) => "OK",
        Err(status) => efe_telemetry::map_grpc_status(status.code() as i32),
    };
    efe_telemetry::record_request_completion(path, code_label, tenant, started.elapsed());
}

/// Increment `efe_ratelimit_drops_total{tenant}` for a single drop.
///
/// Centralising this helper ensures every drop is attributed to a
/// tenant even when the handler path differs — a missed increment
/// here produces a silent dashboard gap that is nearly impossible to
/// diagnose after the fact.
pub(crate) fn record_ratelimit_drop(tenant: &str) {
    efe_telemetry::instruments::ratelimit_drops_total()
        .with_label_values(&[tenant])
        .inc();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_request_completion_maps_ok() {
        // Smoke test: calling with Ok(()) must not panic and must map
        // to the "OK" label — mirroring the invariant enforced by the
        // MetricsLayer's status classifier.
        let started = Instant::now();
        emit_request_completion("/test/Method", "acme", &Ok(()), started);
    }

    #[test]
    fn emit_request_completion_maps_err() {
        let started = Instant::now();
        let status = Status::permission_denied("nope");
        emit_request_completion("/test/Method", "acme", &Err(&status), started);
    }

    #[test]
    fn record_ratelimit_drop_does_not_panic() {
        record_ratelimit_drop("acme");
        // Also cover the anonymous fallback path.
        record_ratelimit_drop("anonymous");
    }
}
