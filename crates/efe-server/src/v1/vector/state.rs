//! M32 — Handler state trait for `/v1/vector/*`.

use async_trait::async_trait;
use efe_retrieval::{FusedHit, HybridConfig};
use efe_vector::{CollectionConfig, VectorDoc};
use serde::{Deserialize, Serialize};

use super::errors::VectorApiError;

/// Brief collection summary (for the list endpoint).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionSummary {
    /// Collection slug.
    pub name: String,
    /// Embedding model name.
    pub embedding_model_name: String,
    /// Dim.
    pub dim: usize,
    /// Distance metric name.
    pub metric: String,
    /// Number of documents currently stored.
    pub cardinality: u64,
}

/// Detailed collection view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionDetail {
    /// Summary fields.
    pub summary: CollectionSummary,
    /// HNSW + cap config.
    pub config: CollectionConfig,
}

/// Search request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRequestBody {
    /// Dense query vector (optional in sparse-only mode).
    #[serde(default)]
    pub query_vector: Option<Vec<f32>>,
    /// Sparse text query (optional in dense-only mode).
    #[serde(default)]
    pub query_text: Option<String>,
    /// Top-k cap.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    /// Hybrid configuration. Defaults to RRF.
    #[serde(default)]
    pub hybrid: HybridConfig,
}

fn default_top_k() -> usize {
    10
}

/// Snapshot status returned by `POST /snapshot` + `GET /status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionStatus {
    /// Summary.
    pub summary: CollectionSummary,
    /// Last snapshot timestamp (ISO-8601 UTC) or None.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_snapshot_at: Option<String>,
    /// Last snapshot URL on S3 (or None).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_snapshot_url: Option<String>,
}

/// Handler state trait.
#[async_trait]
pub trait VectorApiState: Clone + Send + Sync + 'static {
    /// Authenticate a request.
    fn auth_tenant_from_request(&self, parts: &axum::http::request::Parts) -> Option<String>;

    /// `true` iff the caller has the `vector:write` scope.
    fn caller_has_vector_scope(&self, parts: &axum::http::request::Parts) -> bool;

    /// List collections owned by the tenant.
    async fn list_collections(
        &self,
        tenant: &str,
    ) -> Result<Vec<CollectionSummary>, VectorApiError>;

    /// Create a new collection.
    async fn create_collection(
        &self,
        tenant: &str,
        cfg: CollectionConfig,
    ) -> Result<CollectionDetail, VectorApiError>;

    /// Get one collection by name.
    async fn get_collection(
        &self,
        tenant: &str,
        name: &str,
    ) -> Result<CollectionDetail, VectorApiError>;

    /// Drop a collection (idempotent).
    async fn drop_collection(&self, tenant: &str, name: &str) -> Result<(), VectorApiError>;

    /// Insert / upsert one document.
    async fn upsert_document(
        &self,
        tenant: &str,
        name: &str,
        doc: VectorDoc,
    ) -> Result<(), VectorApiError>;

    /// Delete one document by id.
    async fn delete_document(
        &self,
        tenant: &str,
        name: &str,
        id: &str,
    ) -> Result<(), VectorApiError>;

    /// Run search.
    async fn search(
        &self,
        tenant: &str,
        name: &str,
        body: &SearchRequestBody,
    ) -> Result<Vec<FusedHit>, VectorApiError>;

    /// Force a snapshot.
    async fn snapshot(&self, tenant: &str, name: &str) -> Result<CollectionStatus, VectorApiError>;

    /// Get collection status.
    async fn status(&self, tenant: &str, name: &str) -> Result<CollectionStatus, VectorApiError>;
}
