//! M32 — `/v1/vector/*` HTTP handlers.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use efe_retrieval::FusedHit;
use efe_vector::{CollectionConfig, VectorDoc};
use serde::{Deserialize, Serialize};

use super::{
    errors::VectorApiError,
    state::{
        CollectionDetail, CollectionStatus, CollectionSummary, SearchRequestBody, VectorApiState,
    },
};

/// Document body for upsert.
#[derive(Debug, Deserialize)]
pub struct DocumentBody {
    /// Document id (caller-assigned).
    pub id: String,
    /// Vector embedding.
    pub vector: Vec<f32>,
    /// Optional original text for hybrid retrieval.
    #[serde(default)]
    pub text: Option<String>,
    /// Optional JSON payload.
    #[serde(default)]
    pub payload: serde_json::Value,
}

impl From<DocumentBody> for VectorDoc {
    fn from(b: DocumentBody) -> Self {
        VectorDoc {
            id: b.id,
            vector: b.vector,
            text: b.text,
            payload: b.payload,
        }
    }
}

/// Response for the search endpoint.
#[derive(Debug, Serialize)]
pub struct SearchResponse {
    /// Hits sorted by score (highest first).
    pub hits: Vec<FusedHit>,
}

// ─────────────────────────── HANDLERS ───────────────────────────

/// `GET /v1/vector/collections`
pub async fn list_collections<S: VectorApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<Vec<CollectionSummary>>, VectorApiError> {
    let (parts, _body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(VectorApiError::Unauthenticated)?;
    Ok(Json(state.list_collections(&tenant).await?))
}

/// `POST /v1/vector/collections`
pub async fn create_collection<S: VectorApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<(StatusCode, Json<CollectionDetail>), VectorApiError> {
    let (parts, body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(VectorApiError::Unauthenticated)?;
    if !state.caller_has_vector_scope(&parts) {
        return Err(VectorApiError::Forbidden);
    }
    let mut cfg: CollectionConfig = decode_json_body(body).await?;
    // Force the tenant_id from the auth context (defence in depth :
    // never trust the body's tenant_id).
    cfg.tenant_id = tenant.clone();
    let detail = state.create_collection(&tenant, cfg).await?;
    Ok((StatusCode::CREATED, Json(detail)))
}

/// `GET /v1/vector/collections/:name`
pub async fn get_collection<S: VectorApiState>(
    State(state): State<S>,
    Path(name): Path<String>,
    req: axum::extract::Request,
) -> Result<Json<CollectionDetail>, VectorApiError> {
    let (parts, _body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(VectorApiError::Unauthenticated)?;
    efe_vector::validate_collection_name(&name)?;
    Ok(Json(state.get_collection(&tenant, &name).await?))
}

/// `DELETE /v1/vector/collections/:name`
pub async fn drop_collection<S: VectorApiState>(
    State(state): State<S>,
    Path(name): Path<String>,
    req: axum::extract::Request,
) -> Result<StatusCode, VectorApiError> {
    let (parts, _body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(VectorApiError::Unauthenticated)?;
    if !state.caller_has_vector_scope(&parts) {
        return Err(VectorApiError::Forbidden);
    }
    efe_vector::validate_collection_name(&name)?;
    state.drop_collection(&tenant, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/vector/collections/:name/documents`
pub async fn upsert_document<S: VectorApiState>(
    State(state): State<S>,
    Path(name): Path<String>,
    req: axum::extract::Request,
) -> Result<StatusCode, VectorApiError> {
    let (parts, body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(VectorApiError::Unauthenticated)?;
    if !state.caller_has_vector_scope(&parts) {
        return Err(VectorApiError::Forbidden);
    }
    efe_vector::validate_collection_name(&name)?;
    let body: DocumentBody = decode_json_body(body).await?;
    state.upsert_document(&tenant, &name, body.into()).await?;
    Ok(StatusCode::CREATED)
}

/// `DELETE /v1/vector/collections/:name/documents/:id`
pub async fn delete_document<S: VectorApiState>(
    State(state): State<S>,
    Path((name, id)): Path<(String, String)>,
    req: axum::extract::Request,
) -> Result<StatusCode, VectorApiError> {
    let (parts, _body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(VectorApiError::Unauthenticated)?;
    if !state.caller_has_vector_scope(&parts) {
        return Err(VectorApiError::Forbidden);
    }
    efe_vector::validate_collection_name(&name)?;
    state.delete_document(&tenant, &name, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/vector/collections/:name/search`
pub async fn search<S: VectorApiState>(
    State(state): State<S>,
    Path(name): Path<String>,
    req: axum::extract::Request,
) -> Result<Json<SearchResponse>, VectorApiError> {
    let (parts, body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(VectorApiError::Unauthenticated)?;
    efe_vector::validate_collection_name(&name)?;
    let body: SearchRequestBody = decode_json_body(body).await?;
    if body.top_k == 0 || body.top_k > efe_vector::MAX_K {
        return Err(VectorApiError::InvalidBody(format!(
            "top_k must be 1..={}",
            efe_vector::MAX_K
        )));
    }
    let hits = state.search(&tenant, &name, &body).await?;
    Ok(Json(SearchResponse { hits }))
}

/// `POST /v1/vector/collections/:name/snapshot`
pub async fn snapshot<S: VectorApiState>(
    State(state): State<S>,
    Path(name): Path<String>,
    req: axum::extract::Request,
) -> Result<Json<CollectionStatus>, VectorApiError> {
    let (parts, _body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(VectorApiError::Unauthenticated)?;
    if !state.caller_has_vector_scope(&parts) {
        return Err(VectorApiError::Forbidden);
    }
    efe_vector::validate_collection_name(&name)?;
    Ok(Json(state.snapshot(&tenant, &name).await?))
}

/// `GET /v1/vector/collections/:name/status`
pub async fn status<S: VectorApiState>(
    State(state): State<S>,
    Path(name): Path<String>,
    req: axum::extract::Request,
) -> Result<Json<CollectionStatus>, VectorApiError> {
    let (parts, _body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(VectorApiError::Unauthenticated)?;
    efe_vector::validate_collection_name(&name)?;
    Ok(Json(state.status(&tenant, &name).await?))
}

/// Build the `/v1/vector/*` router.
pub fn router<S: VectorApiState>(state: S) -> Router {
    Router::new()
        .route(
            "/collections",
            get(list_collections::<S>).post(create_collection::<S>),
        )
        .route(
            "/collections/:name",
            get(get_collection::<S>).delete(drop_collection::<S>),
        )
        .route("/collections/:name/documents", post(upsert_document::<S>))
        .route(
            "/collections/:name/documents/:id",
            delete(delete_document::<S>),
        )
        .route("/collections/:name/search", post(search::<S>))
        .route("/collections/:name/snapshot", post(snapshot::<S>))
        .route("/collections/:name/status", get(status::<S>))
        .with_state(state)
}

// ─────────────────────────── HELPERS ───────────────────────────

async fn decode_json_body<T: for<'de> Deserialize<'de>>(
    body: axum::body::Body,
) -> Result<T, VectorApiError> {
    let bytes = axum::body::to_bytes(body, 4 * 1024 * 1024)
        .await
        .map_err(|e| VectorApiError::InvalidBody(format!("read body: {e}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| VectorApiError::InvalidBody(format!("decode json: {e}")))
}

#[cfg(test)]
mod tests {
    //! M32 router smoke tests.

    use super::*;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::Request;
    use efe_retrieval::HybridMode;
    use efe_vector::{Distance, HnswConfig};
    use tower::ServiceExt;

    #[derive(Clone)]
    struct MockState {
        tenant: Option<String>,
        scope: bool,
    }

    #[async_trait]
    impl VectorApiState for MockState {
        fn auth_tenant_from_request(&self, _p: &axum::http::request::Parts) -> Option<String> {
            self.tenant.clone()
        }
        fn caller_has_vector_scope(&self, _p: &axum::http::request::Parts) -> bool {
            self.scope
        }
        async fn list_collections(
            &self,
            _t: &str,
        ) -> Result<Vec<CollectionSummary>, VectorApiError> {
            Ok(vec![CollectionSummary {
                name: "tickets".into(),
                embedding_model_name: "efe-classifier-768".into(),
                dim: 768,
                metric: "cosine".into(),
                cardinality: 0,
            }])
        }
        async fn create_collection(
            &self,
            _t: &str,
            cfg: CollectionConfig,
        ) -> Result<CollectionDetail, VectorApiError> {
            Ok(CollectionDetail {
                summary: CollectionSummary {
                    name: cfg.name.clone(),
                    embedding_model_name: cfg.embedding_model_name.clone(),
                    dim: cfg.dim,
                    metric: cfg.metric.name().to_string(),
                    cardinality: 0,
                },
                config: cfg,
            })
        }
        async fn get_collection(
            &self,
            _t: &str,
            name: &str,
        ) -> Result<CollectionDetail, VectorApiError> {
            if name != "tickets" {
                return Err(VectorApiError::Vector(
                    efe_vector::VectorError::CollectionNotFound(name.into()),
                ));
            }
            let cfg = CollectionConfig {
                tenant_id: "acme".into(),
                name: name.into(),
                embedding_model_name: "efe-classifier-768".into(),
                dim: 3,
                metric: Distance::Cosine,
                hnsw_cfg: HnswConfig::default(),
                max_docs: 100,
                force_hnsw: false,
            };
            Ok(CollectionDetail {
                summary: CollectionSummary {
                    name: name.into(),
                    embedding_model_name: cfg.embedding_model_name.clone(),
                    dim: cfg.dim,
                    metric: cfg.metric.name().into(),
                    cardinality: 0,
                },
                config: cfg,
            })
        }
        async fn drop_collection(&self, _t: &str, _n: &str) -> Result<(), VectorApiError> {
            Ok(())
        }
        async fn upsert_document(
            &self,
            _t: &str,
            _n: &str,
            _d: VectorDoc,
        ) -> Result<(), VectorApiError> {
            Ok(())
        }
        async fn delete_document(
            &self,
            _t: &str,
            _n: &str,
            _id: &str,
        ) -> Result<(), VectorApiError> {
            Ok(())
        }
        async fn search(
            &self,
            _t: &str,
            _n: &str,
            _b: &SearchRequestBody,
        ) -> Result<Vec<FusedHit>, VectorApiError> {
            Ok(vec![FusedHit {
                id: "a".into(),
                score: 1.0,
            }])
        }
        async fn snapshot(&self, _t: &str, n: &str) -> Result<CollectionStatus, VectorApiError> {
            Ok(CollectionStatus {
                summary: CollectionSummary {
                    name: n.into(),
                    embedding_model_name: "efe-classifier-768".into(),
                    dim: 768,
                    metric: "cosine".into(),
                    cardinality: 1,
                },
                last_snapshot_at: Some("2026-05-11T00:00:00Z".into()),
                last_snapshot_url: Some("s3://efe-snapshots/...".into()),
            })
        }
        async fn status(&self, _t: &str, n: &str) -> Result<CollectionStatus, VectorApiError> {
            Ok(CollectionStatus {
                summary: CollectionSummary {
                    name: n.into(),
                    embedding_model_name: "efe-classifier-768".into(),
                    dim: 768,
                    metric: "cosine".into(),
                    cardinality: 1,
                },
                last_snapshot_at: None,
                last_snapshot_url: None,
            })
        }
    }

    fn state(tenant: Option<&str>, scope: bool) -> MockState {
        MockState {
            tenant: tenant.map(|s| s.into()),
            scope,
        }
    }

    #[tokio::test]
    async fn list_unauthenticated_is_401() {
        let app = router(state(None, true));
        let req = Request::builder()
            .method("GET")
            .uri("/collections")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn list_authenticated_is_200() {
        let app = router(state(Some("acme"), true));
        let req = Request::builder()
            .method("GET")
            .uri("/collections")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn create_without_scope_is_403() {
        let app = router(state(Some("acme"), false));
        let body = serde_json::json!({
            "tenant_id": "ignored",
            "name": "tickets",
            "embedding_model_name": "efe-classifier-768",
            "dim": 768,
            "metric": "cosine"
        });
        let req = Request::builder()
            .method("POST")
            .uri("/collections")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn create_with_scope_is_201() {
        let app = router(state(Some("acme"), true));
        let body = serde_json::json!({
            "tenant_id": "ignored",
            "name": "tickets",
            "embedding_model_name": "efe-classifier-768",
            "dim": 768,
            "metric": "cosine"
        });
        let req = Request::builder()
            .method("POST")
            .uri("/collections")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn invalid_collection_name_400() {
        let app = router(state(Some("acme"), true));
        let req = Request::builder()
            .method("GET")
            .uri("/collections/UPPERCASE_NAME")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn collection_not_found_404() {
        let app = router(state(Some("acme"), true));
        let req = Request::builder()
            .method("GET")
            .uri("/collections/missing")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn drop_idempotent_204() {
        let app = router(state(Some("acme"), true));
        let req = Request::builder()
            .method("DELETE")
            .uri("/collections/tickets")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn upsert_returns_201() {
        let app = router(state(Some("acme"), true));
        let body = serde_json::json!({
            "id": "d1",
            "vector": [1.0, 0.0, 0.0],
            "text": "ticket",
            "payload": {}
        });
        let req = Request::builder()
            .method("POST")
            .uri("/collections/tickets/documents")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn delete_document_204() {
        let app = router(state(Some("acme"), true));
        let req = Request::builder()
            .method("DELETE")
            .uri("/collections/tickets/documents/d1")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn search_returns_200() {
        let app = router(state(Some("acme"), true));
        let body = serde_json::json!({
            "query_vector": [1.0, 0.0, 0.0],
            "query_text": "test",
            "top_k": 5,
            "hybrid": {"mode": "hybrid_rrf"}
        });
        let req = Request::builder()
            .method("POST")
            .uri("/collections/tickets/search")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn search_rejects_zero_top_k() {
        let app = router(state(Some("acme"), true));
        let body = serde_json::json!({
            "query_vector": [1.0, 0.0, 0.0],
            "top_k": 0,
            "hybrid": {"mode": "dense"}
        });
        let req = Request::builder()
            .method("POST")
            .uri("/collections/tickets/search")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn snapshot_authed_returns_200() {
        let app = router(state(Some("acme"), true));
        let req = Request::builder()
            .method("POST")
            .uri("/collections/tickets/snapshot")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn status_returns_200_with_status_body() {
        let app = router(state(Some("acme"), true));
        let req = Request::builder()
            .method("GET")
            .uri("/collections/tickets/status")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // Silence unused-import lint.
    fn _silence_hybrid_mode() {
        let _ = HybridMode::HybridRrf;
    }
}
