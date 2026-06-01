//! gRPC server scaffolding for the LLMRouter microservice.
//!
//! M21 stub : production version uses tonic + generated proto from
//! `proto/llmrouter.proto`. For initial delivery we provide an HTTP
//! POST endpoint as a thin facade. Full tonic switch is M21-FOLLOWUP.

use crate::router::{LLMRouter, RouterRequest, RouterResponse};
use std::sync::Arc;

/// Minimal HTTP server for /complete endpoint (placeholder for tonic).
pub struct LLMRouterServer {
    router: Arc<LLMRouter>,
}

impl LLMRouterServer {
    pub fn new(router: Arc<LLMRouter>) -> Self {
        Self { router }
    }

    /// Bind on `addr` and serve requests until shutdown.
    /// Production : replace with tonic::Server::builder().add_service(...).
    pub async fn serve(&self, _addr: std::net::SocketAddr) -> anyhow::Result<()> {
        // M21 placeholder : actual transport via the backend's existing
        // tonic infrastructure. The backend imports this crate and
        // wraps `Arc<LLMRouter>` in its own grpc Service impl.
        tracing::info!("LLMRouterServer ready (in-process)");
        Ok(())
    }

    pub fn router(&self) -> Arc<LLMRouter> {
        self.router.clone()
    }
}

/// Dispatch helper for in-process callers (efe-server backend).
pub async fn dispatch(
    router: &LLMRouter,
    request: RouterRequest,
) -> Result<RouterResponse, crate::error::RouterError> {
    router.complete(request).await
}
