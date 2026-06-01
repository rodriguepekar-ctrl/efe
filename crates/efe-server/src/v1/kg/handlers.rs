//! M33 — `/v1/kg/*` HTTP handlers.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use efe_kg::{Entity, Predicate, Relation, TraversalNode};
use serde::Deserialize;

use super::{
    errors::KgApiError,
    state::{ExtractRequestBody, ExtractResponse, KgApiState, KgGraphSummary, TraverseRequestBody},
};

/// `GET /v1/kg/graph/status`
pub async fn graph_status<S: KgApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<KgGraphSummary>, KgApiError> {
    let (parts, _body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(KgApiError::Unauthenticated)?;
    Ok(Json(state.graph_summary(&tenant).await?))
}

/// `POST /v1/kg/extract`
pub async fn extract<S: KgApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<ExtractResponse>, KgApiError> {
    let (parts, body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(KgApiError::Unauthenticated)?;
    if !state.caller_has_kg_scope(&parts) {
        return Err(KgApiError::Forbidden);
    }
    let body: ExtractRequestBody = decode_json_body(body).await?;
    if body.text.is_empty() {
        return Err(KgApiError::InvalidBody("text is empty".into()));
    }
    Ok(Json(state.extract(&tenant, &body).await?))
}

/// `GET /v1/kg/entities?limit=`
pub async fn list_entities<S: KgApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<Vec<Entity>>, KgApiError> {
    let (parts, _body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(KgApiError::Unauthenticated)?;
    let limit = parse_limit(&parts, 100, 1000);
    Ok(Json(state.list_entities(&tenant, limit).await?))
}

/// `POST /v1/kg/entities`
pub async fn upsert_entity<S: KgApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<(StatusCode, Json<Entity>), KgApiError> {
    let (parts, body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(KgApiError::Unauthenticated)?;
    if !state.caller_has_kg_scope(&parts) {
        return Err(KgApiError::Forbidden);
    }
    let entity: Entity = decode_json_body(body).await?;
    let saved = state.upsert_entity(&tenant, entity).await?;
    Ok((StatusCode::CREATED, Json(saved)))
}

/// `GET /v1/kg/entities/:id`
pub async fn get_entity<S: KgApiState>(
    State(state): State<S>,
    Path(id): Path<String>,
    req: axum::extract::Request,
) -> Result<Json<Entity>, KgApiError> {
    let (parts, _body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(KgApiError::Unauthenticated)?;
    Ok(Json(state.get_entity(&tenant, &id).await?))
}

/// `DELETE /v1/kg/entities/:id`
pub async fn delete_entity<S: KgApiState>(
    State(state): State<S>,
    Path(id): Path<String>,
    req: axum::extract::Request,
) -> Result<StatusCode, KgApiError> {
    let (parts, _body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(KgApiError::Unauthenticated)?;
    if !state.caller_has_kg_scope(&parts) {
        return Err(KgApiError::Forbidden);
    }
    state.delete_entity(&tenant, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /v1/kg/relations?limit=`
pub async fn list_relations<S: KgApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<Vec<Relation>>, KgApiError> {
    let (parts, _body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(KgApiError::Unauthenticated)?;
    let limit = parse_limit(&parts, 100, 1000);
    Ok(Json(state.list_relations(&tenant, limit).await?))
}

/// `POST /v1/kg/relations`
pub async fn upsert_relation<S: KgApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<(StatusCode, Json<Relation>), KgApiError> {
    let (parts, body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(KgApiError::Unauthenticated)?;
    if !state.caller_has_kg_scope(&parts) {
        return Err(KgApiError::Forbidden);
    }
    let r: Relation = decode_json_body(body).await?;
    let saved = state.upsert_relation(&tenant, r).await?;
    Ok((StatusCode::CREATED, Json(saved)))
}

/// `POST /v1/kg/traverse`
pub async fn traverse<S: KgApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<Vec<TraversalNode>>, KgApiError> {
    let (parts, body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(KgApiError::Unauthenticated)?;
    let body: TraverseRequestBody = decode_json_body(body).await?;
    if !matches!(body.algo.as_str(), "bfs" | "dfs") {
        return Err(KgApiError::InvalidBody(format!(
            "unknown algo: {}",
            body.algo
        )));
    }
    Ok(Json(state.traverse(&tenant, &body).await?))
}

/// `GET /v1/kg/predicates`
pub async fn list_predicates<S: KgApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<Vec<Predicate>>, KgApiError> {
    let (parts, _body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(KgApiError::Unauthenticated)?;
    Ok(Json(state.list_predicates(&tenant).await?))
}

/// `POST /v1/kg/predicates`
pub async fn add_predicate<S: KgApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<(StatusCode, Json<Predicate>), KgApiError> {
    let (parts, body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(KgApiError::Unauthenticated)?;
    if !state.caller_has_kg_scope(&parts) {
        return Err(KgApiError::Forbidden);
    }
    let p: Predicate = decode_json_body(body).await?;
    let saved = state.add_predicate(&tenant, p).await?;
    Ok((StatusCode::CREATED, Json(saved)))
}

/// Build the `/v1/kg/*` router.
pub fn router<S: KgApiState>(state: S) -> Router {
    Router::new()
        .route("/graph/status", get(graph_status::<S>))
        .route("/extract", post(extract::<S>))
        .route(
            "/entities",
            get(list_entities::<S>).post(upsert_entity::<S>),
        )
        .route(
            "/entities/:id",
            get(get_entity::<S>).delete(delete_entity::<S>),
        )
        .route(
            "/relations",
            get(list_relations::<S>).post(upsert_relation::<S>),
        )
        .route("/traverse", post(traverse::<S>))
        .route(
            "/predicates",
            get(list_predicates::<S>).post(add_predicate::<S>),
        )
        .with_state(state)
}

// ─────────────────────────── HELPERS ───────────────────────────

async fn decode_json_body<T: for<'de> Deserialize<'de>>(
    body: axum::body::Body,
) -> Result<T, KgApiError> {
    let bytes = axum::body::to_bytes(body, 256 * 1024)
        .await
        .map_err(|e| KgApiError::InvalidBody(format!("read body: {e}")))?;
    serde_json::from_slice(&bytes).map_err(|e| KgApiError::InvalidBody(format!("decode json: {e}")))
}

fn parse_limit(parts: &axum::http::request::Parts, default: usize, max: usize) -> usize {
    let q = parts.uri.query().unwrap_or("");
    for pair in q.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == "limit" {
                if let Ok(n) = v.parse::<usize>() {
                    return n.clamp(1, max);
                }
            }
        }
    }
    default
}

#[cfg(test)]
mod tests {
    //! M33 router smoke tests.

    use super::*;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use efe_kg::{EntityKind, TraversalFilter};
    use tower::ServiceExt;

    #[derive(Clone)]
    struct MockState {
        tenant: Option<String>,
        scope: bool,
    }

    #[async_trait]
    impl KgApiState for MockState {
        fn auth_tenant_from_request(&self, _p: &axum::http::request::Parts) -> Option<String> {
            self.tenant.clone()
        }
        fn caller_has_kg_scope(&self, _p: &axum::http::request::Parts) -> bool {
            self.scope
        }
        async fn graph_summary(&self, t: &str) -> Result<KgGraphSummary, KgApiError> {
            Ok(KgGraphSummary {
                tenant_id: t.into(),
                entity_count: 5,
                relation_count: 7,
            })
        }
        async fn extract(
            &self,
            _t: &str,
            _r: &ExtractRequestBody,
        ) -> Result<ExtractResponse, KgApiError> {
            Ok(ExtractResponse {
                entities_extracted: 3,
                entities_merged: 1,
                claims_emitted: 2,
                relations_promoted: 1,
            })
        }
        async fn list_entities(&self, _t: &str, _l: usize) -> Result<Vec<Entity>, KgApiError> {
            Ok(vec![Entity::new("e1", EntityKind::Person, "alice")])
        }
        async fn upsert_entity(&self, _t: &str, e: Entity) -> Result<Entity, KgApiError> {
            Ok(e)
        }
        async fn get_entity(&self, _t: &str, id: &str) -> Result<Entity, KgApiError> {
            if id == "missing" {
                return Err(KgApiError::Kg(efe_kg::KgError::EntityNotFound(id.into())));
            }
            Ok(Entity::new(id, EntityKind::Misc, "test"))
        }
        async fn delete_entity(&self, _t: &str, _id: &str) -> Result<(), KgApiError> {
            Ok(())
        }
        async fn list_relations(&self, _t: &str, _l: usize) -> Result<Vec<Relation>, KgApiError> {
            Ok(vec![])
        }
        async fn upsert_relation(&self, _t: &str, r: Relation) -> Result<Relation, KgApiError> {
            Ok(r)
        }
        async fn traverse(
            &self,
            _t: &str,
            _r: &TraverseRequestBody,
        ) -> Result<Vec<TraversalNode>, KgApiError> {
            Ok(vec![TraversalNode {
                entity_id: "e1".into(),
                hop_distance: 0,
                path: vec![],
            }])
        }
        async fn list_predicates(&self, _t: &str) -> Result<Vec<Predicate>, KgApiError> {
            Ok(efe_kg::BASE_PREDICATES
                .iter()
                .map(|p| Predicate::new(*p).unwrap())
                .collect())
        }
        async fn add_predicate(&self, _t: &str, p: Predicate) -> Result<Predicate, KgApiError> {
            Ok(p)
        }
    }

    fn state(tenant: Option<&str>, scope: bool) -> MockState {
        MockState {
            tenant: tenant.map(|s| s.into()),
            scope,
        }
    }

    #[tokio::test]
    async fn status_unauthed_401() {
        let app = router(state(None, true));
        let req = Request::builder()
            .method("GET")
            .uri("/graph/status")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn status_authed_200() {
        let app = router(state(Some("acme"), true));
        let req = Request::builder()
            .method("GET")
            .uri("/graph/status")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn extract_requires_scope() {
        let app = router(state(Some("acme"), false));
        let body = serde_json::json!({"doc_id":"doc1","text":"Alice works at Acme"});
        let req = Request::builder()
            .method("POST")
            .uri("/extract")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn extract_rejects_empty_text() {
        let app = router(state(Some("acme"), true));
        let body = serde_json::json!({"doc_id":"doc1","text":""});
        let req = Request::builder()
            .method("POST")
            .uri("/extract")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn extract_authed_200() {
        let app = router(state(Some("acme"), true));
        let body = serde_json::json!({"doc_id":"doc1","text":"Alice works at Acme"});
        let req = Request::builder()
            .method("POST")
            .uri("/extract")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn get_entity_not_found_404() {
        let app = router(state(Some("acme"), true));
        let req = Request::builder()
            .method("GET")
            .uri("/entities/missing")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn upsert_entity_returns_201() {
        let app = router(state(Some("acme"), true));
        let body = serde_json::json!({
            "id":"e1",
            "kind":"person",
            "canonical_name":"alice",
            "mentions":[],
            "confidence":0.9
        });
        let req = Request::builder()
            .method("POST")
            .uri("/entities")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn traverse_returns_200() {
        let app = router(state(Some("acme"), true));
        let body = serde_json::json!({
            "seed_entity_id": "e1",
            "max_hops": 2,
            "max_results": 10,
            "filter": {},
            "algo": "bfs"
        });
        let req = Request::builder()
            .method("POST")
            .uri("/traverse")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn traverse_rejects_unknown_algo() {
        let app = router(state(Some("acme"), true));
        let body = serde_json::json!({
            "seed_entity_id": "e1",
            "max_hops": 2,
            "max_results": 10,
            "filter": {},
            "algo": "dijkstra"
        });
        let req = Request::builder()
            .method("POST")
            .uri("/traverse")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn list_predicates_returns_base_catalog() {
        let app = router(state(Some("acme"), true));
        let req = Request::builder()
            .method("GET")
            .uri("/predicates")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // Silence unused imports.
    fn _silence() {
        let _ = TraversalFilter::default();
    }
}
