//! Inject an `x-correlation-id` header on every request that lacks one.
//! Generates a UUIDv4 fallback. Cheap layer — runs first in the chain.
//!
//! Why : Envoy injects this already at the edge, but :
//! - internal-only callers (CI, batch jobs, health probes) don't go
//!   through Envoy and would otherwise show empty IDs in logs ;
//! - if Envoy is temporarily bypassed (DR / fallback), the inner
//!   service should still log a coherent ID.

use http::{HeaderName, HeaderValue, Request, Response};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::{Layer, Service};

static HEADER: HeaderName = HeaderName::from_static("x-correlation-id");

/// Tower layer wrapping any service that takes `http::Request<B>`.
#[derive(Clone, Debug, Default)]
pub struct CorrelationIdLayer;

impl CorrelationIdLayer {
    pub fn new() -> Self {
        Self
    }
}

impl<S> Layer<S> for CorrelationIdLayer {
    type Service = CorrelationIdService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        CorrelationIdService { inner }
    }
}

#[derive(Clone, Debug)]
pub struct CorrelationIdService<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for CorrelationIdService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = Response<ResBody>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        // Inject if absent.
        if !req.headers().contains_key(&HEADER) {
            let id = uuid::Uuid::new_v4().to_string();
            if let Ok(v) = HeaderValue::from_str(&id) {
                req.headers_mut().insert(HEADER.clone(), v);
            }
        }
        // Clone the corr id (cheap : it's a HeaderValue refcount) so we
        // can also echo it back on the response (useful for clients).
        let corr = req.headers().get(&HEADER).cloned();
        let mut inner = self.inner.clone();
        std::mem::swap(&mut self.inner, &mut inner);
        Box::pin(async move {
            let mut resp = inner.call(req).await?;
            if let Some(v) = corr {
                resp.headers_mut().insert(HEADER.clone(), v);
            }
            Ok(resp)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_name_is_lowercased() {
        assert_eq!(HEADER.as_str(), "x-correlation-id");
    }

    #[tokio::test]
    async fn injects_when_missing() {
        // Build a tiny service that echoes the corr id back.
        let svc = tower::service_fn(|req: Request<()>| async move {
            let id = req
                .headers()
                .get("x-correlation-id")
                .map(|v| v.to_str().unwrap().to_string())
                .unwrap_or_default();
            Ok::<_, std::convert::Infallible>(Response::new(id))
        });
        let mut layered = CorrelationIdLayer::new().layer(svc);
        let req = Request::builder().body(()).unwrap();
        let resp = <CorrelationIdService<_> as Service<Request<()>>>::call(&mut layered, req)
            .await
            .unwrap();
        assert!(!resp.into_body().is_empty(), "corr id should be injected");
    }

    #[tokio::test]
    async fn preserves_when_present() {
        let svc = tower::service_fn(|req: Request<()>| async move {
            let id = req
                .headers()
                .get("x-correlation-id")
                .map(|v| v.to_str().unwrap().to_string())
                .unwrap_or_default();
            Ok::<_, std::convert::Infallible>(Response::new(id))
        });
        let mut layered = CorrelationIdLayer::new().layer(svc);
        let req = Request::builder()
            .header("x-correlation-id", "fixed-id-123")
            .body(())
            .unwrap();
        let resp = <CorrelationIdService<_> as Service<Request<()>>>::call(&mut layered, req)
            .await
            .unwrap();
        assert_eq!(resp.body(), "fixed-id-123");
    }
}
