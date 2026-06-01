//! Minimal axum HTTP server exposing the Prometheus `/metrics` endpoint
//! plus `/healthz`.
//!
//! Intended to run as a **sidecar** task alongside the gRPC server on a
//! distinct port (default `127.0.0.1:9090`). Bind to `0.0.0.0` when
//! scraping from a separate pod.

use std::future::Future;
use std::net::SocketAddr;

use axum::{response::IntoResponse, routing::get, Router};
use prometheus::{Encoder, TextEncoder};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::registry::global_registry;

/// Handle to the running HTTP server. Dropping it does **not** stop the
/// server; call [`MetricsServerHandle::shutdown`] explicitly for a
/// graceful stop.
#[derive(Debug)]
pub struct MetricsServerHandle {
    local_addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Result<(), std::io::Error>>>,
}

impl MetricsServerHandle {
    /// Bound address reported by the kernel (useful when binding to
    /// port 0 in tests).
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Signal graceful shutdown and wait for the task to exit.
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for MetricsServerHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

// Prometheus text exposition format — stable content type across
// Prometheus versions (cf. `prometheus::TextEncoder::format_type`).
const PROMETHEUS_TEXT_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

async fn metrics_handler() -> impl IntoResponse {
    let mut buffer = Vec::with_capacity(4096);
    let encoder = TextEncoder::new();
    let metric_families = global_registry().gather();
    match encoder.encode(&metric_families, &mut buffer) {
        Ok(()) => (
            axum::http::StatusCode::OK,
            [(
                axum::http::header::CONTENT_TYPE,
                PROMETHEUS_TEXT_CONTENT_TYPE,
            )],
            buffer,
        ),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; charset=utf-8",
            )],
            e.to_string().into_bytes(),
        ),
    }
}

async fn healthz_handler() -> impl IntoResponse {
    (axum::http::StatusCode::OK, "ok")
}

/// Spawn the `/metrics` HTTP server on `addr` and return a handle.
///
/// # Errors
///
/// Returns the underlying `std::io::Error` if binding fails.
pub async fn spawn_metrics_server(addr: SocketAddr) -> Result<MetricsServerHandle, std::io::Error> {
    spawn_with_shutdown(addr, oneshot::channel()).await
}

async fn spawn_with_shutdown(
    addr: SocketAddr,
    (tx, rx): (oneshot::Sender<()>, oneshot::Receiver<()>),
) -> Result<MetricsServerHandle, std::io::Error> {
    let listener = TcpListener::bind(addr).await?;
    let local_addr = listener.local_addr()?;
    let app = Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/healthz", get(healthz_handler));
    let shutdown_fut = async move {
        let _ = rx.await;
    };
    let task = tokio::spawn(serve_with_shutdown(listener, app, shutdown_fut));
    Ok(MetricsServerHandle {
        local_addr,
        shutdown: Some(tx),
        task: Some(task),
    })
}

async fn serve_with_shutdown(
    listener: TcpListener,
    app: Router,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), std::io::Error> {
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::instruments;

    async fn get_body(url: &str) -> (u16, String) {
        // Avoid pulling reqwest; do a minimal HTTP/1 GET over TcpStream.
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let uri = url.parse::<http::Uri>().unwrap();
        let authority = uri.authority().unwrap().as_str();
        let path = uri.path();
        let mut stream = tokio::net::TcpStream::connect(authority).await.unwrap();
        let req = format!("GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n");
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        let text = String::from_utf8_lossy(&buf).to_string();
        let status = text
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        // Body starts after the blank line.
        let body = text
            .split_once("\r\n\r\n")
            .map(|(_, b)| b.to_string())
            .unwrap_or_default();
        (status, body)
    }

    #[tokio::test]
    async fn healthz_returns_ok() {
        instruments::register_all();
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let h = spawn_metrics_server(addr).await.unwrap();
        let url = format!("http://{}/healthz", h.local_addr());
        let (status, body) = get_body(&url).await;
        assert_eq!(status, 200);
        assert_eq!(body, "ok");
        h.shutdown().await;
    }

    #[tokio::test]
    async fn metrics_exposes_registered_names() {
        instruments::register_all();
        // Increment one counter so it shows up in output.
        instruments::grpc_requests_total()
            .with_label_values(&["/m", "OK", "t"])
            .inc();
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let h = spawn_metrics_server(addr).await.unwrap();
        let url = format!("http://{}/metrics", h.local_addr());
        let (status, body) = get_body(&url).await;
        assert_eq!(status, 200);
        assert!(body.contains("efe_grpc_requests_total"));
        assert!(body.contains("efe_swarm_rounds_total"));
        h.shutdown().await;
    }

    #[tokio::test]
    async fn shutdown_is_graceful() {
        instruments::register_all();
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let h = spawn_metrics_server(addr).await.unwrap();
        let a = h.local_addr();
        h.shutdown().await;
        // After shutdown, we expect connection refused.
        let res = tokio::net::TcpStream::connect(a).await;
        assert!(res.is_err(), "expected connection to fail after shutdown");
    }
}
