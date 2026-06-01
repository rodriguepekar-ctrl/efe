//! M30 — Trait abstraction over the multi-region plane.

use async_trait::async_trait;
use efe_region::{DrillConfig, DrillReport, FailoverRequest, FailoverResult, PeerHealth};

use super::errors::RegionApiError;

/// Handler state trait : provides the dependency surface for
/// `/v1/region/*` handlers.
#[async_trait]
pub trait RegionApiState: Clone + Send + Sync + 'static {
    /// Authenticate a peer probe. Returns `true` on success.
    fn auth_peer_from_request(&self, parts: &axum::http::request::Parts) -> bool;

    /// Authenticate an operator. Returns `Some(operator_id)` on
    /// success.
    fn auth_operator_from_request(&self, parts: &axum::http::request::Parts) -> Option<String>;

    /// Return this region's health snapshot (called by peers).
    async fn current_health(&self) -> Result<PeerHealth, RegionApiError>;

    /// Return this region's identity + topology.
    async fn current_identity(&self) -> Result<serde_json::Value, RegionApiError>;

    /// Run a failover (caller : operator).
    async fn run_failover(&self, req: &FailoverRequest) -> Result<FailoverResult, RegionApiError>;

    /// Run the local-side promotion (caller : peer orchestrator).
    async fn run_promote(
        &self,
        operator: &str,
        reason: &str,
    ) -> Result<serde_json::Value, RegionApiError>;

    /// Run a DR drill (caller : operator).
    async fn run_drill(&self, cfg: &DrillConfig) -> Result<DrillReport, RegionApiError>;
}
