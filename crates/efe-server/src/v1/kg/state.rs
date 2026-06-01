//! M33 — Handler state trait for `/v1/kg/*`.

use async_trait::async_trait;
use efe_kg::{Entity, Predicate, Relation, TraversalFilter, TraversalNode};
use serde::{Deserialize, Serialize};

use super::errors::KgApiError;

/// Brief graph summary (entity + relation counts).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KgGraphSummary {
    /// Tenant identifier.
    pub tenant_id: String,
    /// Number of entities.
    pub entity_count: u64,
    /// Number of relations.
    pub relation_count: u64,
}

/// Extraction request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractRequestBody {
    /// Source document identifier (used for provenance).
    pub doc_id: String,
    /// Text to extract from.
    pub text: String,
}

/// Extraction outcome (counts only).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractResponse {
    /// Number of entities discovered (before dedup).
    pub entities_extracted: usize,
    /// Number of entities merged into existing ones.
    pub entities_merged: usize,
    /// Number of claims emitted.
    pub claims_emitted: usize,
    /// Number of relations promoted from claims.
    pub relations_promoted: usize,
}

/// Traversal request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraverseRequestBody {
    /// Seed entity id.
    pub seed_entity_id: String,
    /// Max hops (1..=MAX_HOPS).
    #[serde(default = "default_max_hops")]
    pub max_hops: usize,
    /// Max results.
    #[serde(default = "default_max_results")]
    pub max_results: usize,
    /// Filter.
    #[serde(default)]
    pub filter: TraversalFilter,
    /// Algorithm to use (`bfs` or `dfs`).
    #[serde(default = "default_algo")]
    pub algo: String,
}

fn default_max_hops() -> usize {
    2
}
fn default_max_results() -> usize {
    100
}
fn default_algo() -> String {
    "bfs".into()
}

/// KG handler state trait.
#[async_trait]
pub trait KgApiState: Clone + Send + Sync + 'static {
    /// Authenticate a request.
    fn auth_tenant_from_request(&self, parts: &axum::http::request::Parts) -> Option<String>;

    /// `true` iff the caller has the `kg:write` scope.
    fn caller_has_kg_scope(&self, parts: &axum::http::request::Parts) -> bool;

    /// Graph summary.
    async fn graph_summary(&self, tenant: &str) -> Result<KgGraphSummary, KgApiError>;

    /// Run extraction (NER + RE) + merge into graph.
    async fn extract(
        &self,
        tenant: &str,
        req: &ExtractRequestBody,
    ) -> Result<ExtractResponse, KgApiError>;

    /// List entities (paginated ; M33 ships a small first-page).
    async fn list_entities(&self, tenant: &str, limit: usize) -> Result<Vec<Entity>, KgApiError>;

    /// Insert / upsert an entity.
    async fn upsert_entity(&self, tenant: &str, entity: Entity) -> Result<Entity, KgApiError>;

    /// Get one entity by id.
    async fn get_entity(&self, tenant: &str, id: &str) -> Result<Entity, KgApiError>;

    /// Drop an entity (idempotent).
    async fn delete_entity(&self, tenant: &str, id: &str) -> Result<(), KgApiError>;

    /// List relations.
    async fn list_relations(&self, tenant: &str, limit: usize)
        -> Result<Vec<Relation>, KgApiError>;

    /// Insert / upsert a relation.
    async fn upsert_relation(
        &self,
        tenant: &str,
        relation: Relation,
    ) -> Result<Relation, KgApiError>;

    /// Run traversal.
    async fn traverse(
        &self,
        tenant: &str,
        req: &TraverseRequestBody,
    ) -> Result<Vec<TraversalNode>, KgApiError>;

    /// List allowed predicates (base + tenant custom).
    async fn list_predicates(&self, tenant: &str) -> Result<Vec<Predicate>, KgApiError>;

    /// Add a custom predicate.
    async fn add_predicate(
        &self,
        tenant: &str,
        predicate: Predicate,
    ) -> Result<Predicate, KgApiError>;
}
