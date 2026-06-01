//! M33 — `/v1/graphrag/*` HTTP handlers.

use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use serde::Deserialize;

use super::{
    errors::GraphRagApiError,
    state::{GraphRagApiState, GraphRagSearchBody, GraphRagSearchResponse},
};

/// `POST /v1/graphrag/search`
pub async fn search<S: GraphRagApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<GraphRagSearchResponse>, GraphRagApiError> {
    let (parts, body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(GraphRagApiError::Unauthenticated)?;
    let body: GraphRagSearchBody = decode_json_body(body).await?;
    if body.query_vector.is_empty() && body.explicit_seed_entity_ids.is_empty() {
        return Err(GraphRagApiError::InvalidBody(
            "query_vector or explicit_seed_entity_ids must be provided".into(),
        ));
    }
    if body.top_k == 0 {
        return Err(GraphRagApiError::InvalidBody("top_k must be > 0".into()));
    }
    let hits = state.search(&tenant, &body).await?;
    Ok(Json(GraphRagSearchResponse { hits }))
}

/// Build the `/v1/graphrag/*` router.
pub fn router<S: GraphRagApiState>(state: S) -> Router {
    Router::new()
        .route("/search", post(search::<S>))
        .with_state(state)
}

async fn decode_json_body<T: for<'de> Deserialize<'de>>(
    body: axum::body::Body,
) -> Result<T, GraphRagApiError> {
    let bytes = axum::body::to_bytes(body, 256 * 1024)
        .await
        .map_err(|e| GraphRagApiError::InvalidBody(format!("read body: {e}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| GraphRagApiError::InvalidBody(format!("decode json: {e}")))
}

// Suppress unused warning for StatusCode (used by error response).
#[allow(dead_code)]
const _: StatusCode = StatusCode::OK;

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use efe_graphrag::GraphRagHit;
    use tower::ServiceExt;

    #[derive(Clone)]
    struct MockState {
        tenant: Option<String>,
    }

    #[async_trait]
    impl GraphRagApiState for MockState {
        fn auth_tenant_from_request(&self, _p: &axum::http::request::Parts) -> Option<String> {
            self.tenant.clone()
        }
        async fn search(
            &self,
            _t: &str,
            _b: &GraphRagSearchBody,
        ) -> Result<Vec<GraphRagHit>, GraphRagApiError> {
            Ok(vec![GraphRagHit {
                id: "a".into(),
                score: 0.9,
                hop_distance: 0,
                dense_origin: true,
                graph_origin: false,
            }])
        }
    }

    fn state(tenant: Option<&str>) -> MockState {
        MockState {
            tenant: tenant.map(|s| s.into()),
        }
    }

    #[tokio::test]
    async fn search_unauthed_401() {
        let app = router(state(None));
        let body = serde_json::json!({"collection":"x","query_vector":[1.0]});
        let req = Request::builder()
            .method("POST")
            .uri("/search")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn search_empty_query_400() {
        let app = router(state(Some("acme")));
        let body = serde_json::json!({"collection":"x"});
        let req = Request::builder()
            .method("POST")
            .uri("/search")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn search_top_k_zero_400() {
        let app = router(state(Some("acme")));
        let body = serde_json::json!({
            "collection":"x",
            "query_vector":[1.0,0.0,0.0],
            "top_k": 0
        });
        let req = Request::builder()
            .method("POST")
            .uri("/search")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn search_returns_200_with_hits() {
        let app = router(state(Some("acme")));
        let body = serde_json::json!({
            "collection":"x",
            "query_vector":[1.0,0.0,0.0],
            "top_k": 10
        });
        let req = Request::builder()
            .method("POST")
            .uri("/search")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
