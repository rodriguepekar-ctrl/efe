//! M33 — Handler state trait for `/v1/graphrag/*`.

use async_trait::async_trait;
use efe_graphrag::GraphRagHit;
use serde::{Deserialize, Serialize};

use super::errors::GraphRagApiError;

/// Search request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphRagSearchBody {
    /// Vector collection name (M32) to seed from.
    pub collection: String,
    /// Query vector.
    pub query_vector: Vec<f32>,
    /// Optional explicit graph seeds.
    #[serde(default)]
    pub explicit_seed_entity_ids: Vec<String>,
    /// Top-k.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    /// Max hops.
    #[serde(default = "default_max_hops")]
    pub max_hops: usize,
    /// Fusion alpha.
    #[serde(default = "default_alpha")]
    pub alpha: f32,
    /// Decay rate.
    #[serde(default = "default_decay")]
    pub decay_rate: f32,
}

fn default_top_k() -> usize {
    10
}
fn default_max_hops() -> usize {
    2
}
fn default_alpha() -> f32 {
    efe_graphrag::DEFAULT_ALPHA
}
fn default_decay() -> f32 {
    efe_graphrag::DEFAULT_DECAY
}

/// Response envelope.
#[derive(Debug, Clone, Serialize)]
pub struct GraphRagSearchResponse {
    /// Hits sorted by score (highest first).
    pub hits: Vec<GraphRagHit>,
}

/// Handler state trait.
#[async_trait]
pub trait GraphRagApiState: Clone + Send + Sync + 'static {
    /// Authenticate.
    fn auth_tenant_from_request(&self, parts: &axum::http::request::Parts) -> Option<String>;

    /// Run search.
    async fn search(
        &self,
        tenant: &str,
        body: &GraphRagSearchBody,
    ) -> Result<Vec<GraphRagHit>, GraphRagApiError>;
}
