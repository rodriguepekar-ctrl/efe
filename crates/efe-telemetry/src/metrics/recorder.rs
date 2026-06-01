//! Handler-level metric emission helpers.
//!
//! **Why not only the tower layer?** (Audit finding M11-#6.) The
//! `MetricsLayer` sits outside the auth interceptor stack in tonic's
//! server pipeline, so by the time it runs, the authenticated tenant
//! has not yet been resolved from the JWT. Every request would label
//! `tenant="anonymous"`, defeating the multi-tenant split the spec
//! requires.
//!
//! Instead, handlers call [`record_request_completion`] **after**
//! auth has run but **before** returning, supplying the authenticated
//! tenant id. The tower layer remains in place for the unauthenticated
//! `in_flight` gauge (which doesn't need the tenant label).

use std::time::Duration;

use super::instruments;

/// Record one gRPC request completion with a precise tenant label.
///
/// - Increments `efe_grpc_requests_total{method, code, tenant}`.
/// - Observes `efe_grpc_request_duration_seconds{method, tenant}`.
///
/// `code` is expected to be one of the stable gRPC status names
/// (`"OK"`, `"InvalidArgument"`, ...) — same set as
/// `metrics::tower_layer::map_grpc_status`.
pub fn record_request_completion(method: &str, code: &str, tenant: &str, duration: Duration) {
    instruments::grpc_requests_total()
        .with_label_values(&[method, code, tenant])
        .inc();
    instruments::grpc_request_duration_seconds()
        .with_label_values(&[method, tenant])
        .observe(duration.as_secs_f64());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_counter_and_histogram_samples() {
        let counter = instruments::grpc_requests_total();
        let hist = instruments::grpc_request_duration_seconds();
        let count_before = counter.with_label_values(&["/svc/M", "OK", "acme"]).get();
        let hist_before = hist
            .with_label_values(&["/svc/M", "acme"])
            .get_sample_count();

        record_request_completion("/svc/M", "OK", "acme", Duration::from_millis(5));

        assert_eq!(
            counter.with_label_values(&["/svc/M", "OK", "acme"]).get(),
            count_before + 1
        );
        assert_eq!(
            hist.with_label_values(&["/svc/M", "acme"])
                .get_sample_count(),
            hist_before + 1
        );
    }
}
