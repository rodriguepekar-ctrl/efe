//! `MetricsLayer` — tower layer that tracks **concurrency only**.
//!
//! After audit finding M11-#6 we realised the tower layer sits outside
//! tonic's auth interceptor stack, so the authenticated tenant is not
//! yet known at layer entry. Tenant-aware counters and histograms are
//! therefore emitted by handlers via
//! [`crate::metrics::recorder::record_request_completion`].
//!
//! The layer remains useful for one signal that genuinely needs
//! request-lifecycle observation:
//!
//! - `efe_grpc_requests_in_flight{method}` — incremented on entry,
//!   decremented on exit (including cancellation via RAII `Drop`).
//!
//! Health checks, reflection traffic, and other non-authenticated
//! paths therefore contribute to `in_flight` without producing any
//! `requests_total` or latency samples — which is the desired shape
//! (see audit finding M11-#7).

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use http::{Request, Response};
use tower::{Layer, Service};

use super::instruments;

/// The layer itself is trivially cloneable and only carries a debug tag.
#[derive(Clone, Debug, Default)]
pub struct MetricsLayer;

impl MetricsLayer {
    /// Create a new layer.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl<S> Layer<S> for MetricsLayer {
    type Service = MetricsService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        MetricsService { inner }
    }
}

/// Wrapped service produced by [`MetricsLayer`].
#[derive(Clone, Debug)]
pub struct MetricsService<S> {
    inner: S,
}

impl<S, B, BodyIn> Service<Request<BodyIn>> for MetricsService<S>
where
    S: Service<Request<BodyIn>, Response = Response<B>> + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
    B: Send + 'static,
    BodyIn: Send + 'static,
{
    type Response = Response<B>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<BodyIn>) -> Self::Future {
        let method_label = req.uri().path().to_string();
        let gauge = instruments::grpc_requests_in_flight().with_label_values(&[&method_label]);
        gauge.inc();
        let guard = InFlightGuard { gauge };

        let fut = self.inner.call(req);
        Box::pin(async move {
            // Keep the guard alive until the future resolves (including
            // cancellation, which drops the future and thus the guard).
            let _guard = guard;
            fut.await
        })
    }
}

/// Deterministic mapping from a gRPC integer status code to its stable
/// textual label. Shared with the recorder so callers can produce the
/// same `code` value the layer used to emit.
#[must_use]
pub fn map_grpc_status(code: i32) -> &'static str {
    match code {
        0 => "OK",
        1 => "Cancelled",
        2 => "Unknown",
        3 => "InvalidArgument",
        4 => "DeadlineExceeded",
        5 => "NotFound",
        6 => "AlreadyExists",
        7 => "PermissionDenied",
        8 => "ResourceExhausted",
        9 => "FailedPrecondition",
        10 => "Aborted",
        11 => "OutOfRange",
        12 => "Unimplemented",
        13 => "Internal",
        14 => "Unavailable",
        15 => "DataLoss",
        16 => "Unauthenticated",
        _ => "Unknown",
    }
}

struct InFlightGuard {
    gauge: prometheus::IntGauge,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.gauge.dec();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{HeaderValue, Request, Response};
    use tower::{ServiceBuilder, ServiceExt};

    async fn ok_service(_req: Request<()>) -> Result<Response<()>, std::convert::Infallible> {
        let mut r = Response::new(());
        r.headers_mut()
            .insert("grpc-status", HeaderValue::from_static("0"));
        Ok(r)
    }

    #[tokio::test]
    async fn in_flight_gauge_tracks_concurrency() {
        instruments::register_all();
        let gauge = instruments::grpc_requests_in_flight();
        let before = gauge.with_label_values(&["/svc/InFlight"]).get();

        let svc = ServiceBuilder::new()
            .layer(MetricsLayer::new())
            .service_fn(ok_service);
        let req = Request::builder().uri("/svc/InFlight").body(()).unwrap();
        svc.oneshot(req).await.unwrap();

        // After completion the gauge is back to baseline.
        let after = gauge.with_label_values(&["/svc/InFlight"]).get();
        assert_eq!(after, before);
    }

    #[test]
    fn map_grpc_status_covers_standard_codes() {
        assert_eq!(map_grpc_status(0), "OK");
        assert_eq!(map_grpc_status(3), "InvalidArgument");
        assert_eq!(map_grpc_status(8), "ResourceExhausted");
        assert_eq!(map_grpc_status(13), "Internal");
        assert_eq!(map_grpc_status(16), "Unauthenticated");
        assert_eq!(map_grpc_status(99), "Unknown");
    }
}
