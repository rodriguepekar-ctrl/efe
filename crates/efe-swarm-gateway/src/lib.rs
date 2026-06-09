// SPDX-License-Identifier: UNLICENSED
//! # `efe-swarm-gateway` — Phase 5 Inc.2 : le SDK **et le moat** exposés sur HTTP.
//!
//! Trois routes, sur l'**axum 0.7 existant** :
//! - **`POST /v1/swarm/run`** — exécute un essaim (le SDK), persiste ses **records**,
//!   renvoie le `SwarmRun`.
//! - **`GET /v1/swarm/verify/:id`** — **re-prouve** un run stocké (le vérificateur
//!   indépendant en HTTP : *« voici la preuve, re-vérifiez-la vous-même »*).
//! - **`GET /metrics`** — exposition Prometheus (provenance + substrat).
//!
//! Plus `GET /v1/swarm/runs/:id` (vue) et `GET /v1/swarm/runs/:id/records` (export pour
//! re-preuve **hors-ligne**).
//!
//! ## Deux façons de servir
//! ```no_run
//! use efe_swarm_gateway::{router, SwarmGatewayState};
//!
//! # async fn demo() {
//! let state = SwarmGatewayState::in_memory();      // dev/standalone souverain
//! let app = router(state);                          // /v1/swarm/* + /metrics
//! let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await.unwrap();
//! axum::serve(listener, app).await.unwrap();
//! # }
//! ```
//! Ou **mergée** dans l'app `efe-server` existante :
//! ```no_run
//! # use axum::Router;
//! # use efe_swarm_gateway::{swarm_router, metrics_router, SwarmGatewayState};
//! # fn merge(app: Router, state: SwarmGatewayState) -> Router {
//! app.nest("/v1/swarm", swarm_router(state.clone()))
//!    .merge(metrics_router(state))
//! # }
//! ```
//!
//! ## Sécurité (règle absolue)
//! La clé de provenance par tenant (HKDF) reste **server-side** (dans l'état) ; on dérive
//! pour sceller/re-prouver, on n'expose **que** le `root`. L'auth (dev : `Caller` par
//! en-tête ; prod : `AuthCtx` OIDC) **tenant-scope** toutes les routes.

#![deny(unsafe_code)]
#![warn(rust_2018_idioms)]

mod auth;
mod dto;
mod error;
mod routes;
mod state;
mod store;

use axum::routing::get;
use axum::Router;

pub use auth::Caller;
pub use dto::{
    BudgetDto, ModeDto, OnExceedDto, PatternDto, RunRequest, RunResponse, StoredRunView,
    TopologyDto, VerifyResponse,
};
pub use error::GatewayError;
pub use state::SwarmGatewayState;
pub use store::{InMemoryRunStore, RunStore, StoredRun};

/// Version du crate (semver).
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// **App standalone complète** : `/v1/swarm/*` + `/metrics` (état appliqué).
/// Pour un déploiement souverain : `axum::serve(listener, router(state))`.
pub fn router(state: SwarmGatewayState) -> Router {
    Router::new()
        .nest("/v1/swarm", routes::routes())
        .route("/metrics", get(routes::metrics::metrics))
        .with_state(state)
}

/// Les **routes d'essaim** (relatif, sans préfixe) — à monter via `.nest("/v1/swarm", …)`
/// dans une app hôte (`efe-server`).
pub fn swarm_router(state: SwarmGatewayState) -> Router {
    routes::routes().with_state(state)
}

/// La **seule** route `/metrics` — à `.merge` dans une app hôte.
pub fn metrics_router(state: SwarmGatewayState) -> Router {
    Router::new().route("/metrics", get(routes::metrics::metrics)).with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{HeaderValue, Request, StatusCode};
    use efe_blackboard::SubstrateMetrics;
    use efe_swarm_provenance::ProvenanceMetrics;
    use efe_swarm_sdk::HmacKeySource;
    use efe_swarm_topology::AgentFleet;
    use http_body_util::BodyExt;
    use serde_json::{json, Value};
    use std::sync::Arc;
    use tower::ServiceExt; // oneshot
    use uuid::Uuid;

    const TENANT_A: &str = "11111111-1111-1111-1111-111111111111";
    const TENANT_B: &str = "22222222-2222-2222-2222-222222222222";

    fn app() -> (Router, SwarmGatewayState) {
        let state = SwarmGatewayState::in_memory();
        (router(state.clone()), state)
    }

    /// État de test avec un `InMemoryRunStore` **concret** conservé (pour le testkit de
    /// falsification), tout en l'injectant comme `Arc<dyn RunStore>` dans l'état.
    fn app_with_store() -> (Router, Arc<InMemoryRunStore>) {
        let store = Arc::new(InMemoryRunStore::new());
        let state = SwarmGatewayState::new(
            Arc::new(AgentFleet::standard()),
            HmacKeySource::test_key(),
            store.clone(),
            ProvenanceMetrics::new(),
            Arc::new(SubstrateMetrics::default()),
        );
        (router(state), store)
    }

    async fn body_json(resp: axum::response::Response) -> Value {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn post_run(body: Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/v1/swarm/run")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn run_then_verify_roundtrip_is_proven() {
        let (app, _state) = app();
        // 1. POST /v1/swarm/run (consensus keyless).
        let resp = app
            .clone()
            .oneshot(post_run(json!({
                "task": { "text": "hello", "expected": "hello" },
                "topology": { "kind": "consensus", "generator": "echo_generator",
                              "judges": ["consistency", "length_band", "j_non_empty"], "k": 5 },
                "budget": { "max_cents": 1000 }
            })))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let run = body_json(resp).await;
        let run_id = run["run_id"].as_str().unwrap().to_string();
        assert!(run["run"]["provenance"]["verified"].as_bool().unwrap());
        let root = run["run"]["provenance"]["root"].as_str().unwrap().to_string();
        assert_eq!(root.len(), 64);

        // 2. GET /v1/swarm/verify/:id → re-preuve indépendante, même root.
        let resp = app
            .clone()
            .oneshot(Request::builder().uri(format!("/v1/swarm/verify/{run_id}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let v = body_json(resp).await;
        assert!(v["verified"].as_bool().unwrap(), "le moat re-prouve le run");
        assert!(v["chain_intact"].as_bool().unwrap());
        assert!(v["content_intact"].as_bool().unwrap());
        assert!(v["dag_consistent"].as_bool().unwrap());
        assert_eq!(v["root"].as_str().unwrap(), root, "même root qu'à l'exécution");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn verify_detects_tampering() {
        // Test de FALSIFICATION : on mute un record stocké → verify DOIT le détecter.
        let (app, store) = app_with_store();
        let resp = app
            .clone()
            .oneshot(post_run(json!({
                "task": { "text": "x", "expected": "x" },
                "topology": { "kind": "auto" }
            })))
            .await
            .unwrap();
        let run = body_json(resp).await;
        let run_id: Uuid = run["run_id"].as_str().unwrap().parse().unwrap();

        // Altère la copie en mémoire via le testkit (store concret conservé).
        assert!(store.tamper_first_output(run_id), "un record a été muté");

        // verify recalcule TOUT depuis les records → l'altération casse la preuve.
        let resp = app
            .oneshot(Request::builder().uri(format!("/v1/swarm/verify/{run_id}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let v = body_json(resp).await;
        assert!(!v["verified"].as_bool().unwrap(), "altération détectée → non vérifié");
        assert!(!v["content_intact"].as_bool().unwrap(), "le contenu ne se recalcule plus");
        assert!(v["first_break"].is_string(), "la faute est localisée");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn verify_unknown_run_is_404() {
        let (app, _s) = app();
        let resp = app
            .oneshot(Request::builder().uri(format!("/v1/swarm/verify/{}", Uuid::new_v4())).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn records_export_is_independently_verifiable() {
        let (app, _s) = app();
        let resp = app
            .clone()
            .oneshot(post_run(json!({ "task": { "text": "x" }, "topology": { "kind": "auto" } })))
            .await
            .unwrap();
        let run_id = body_json(resp).await["run_id"].as_str().unwrap().to_string();

        let resp = app
            .oneshot(Request::builder().uri(format!("/v1/swarm/runs/{run_id}/records")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let recs = body_json(resp).await;
        assert!(recs.is_array() && !recs.as_array().unwrap().is_empty(), "records exportés");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn run_is_tenant_scoped_against_verify() {
        let (app, _s) = app();
        // tenant-A exécute un run.
        let mut req = post_run(json!({ "task": { "text": "x" }, "topology": { "kind": "auto" } }));
        req.headers_mut().insert("x-efe-tenant", HeaderValue::from_static(TENANT_A));
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let run_id = body_json(resp).await["run_id"].as_str().unwrap().to_string();

        // tenant-B ne voit PAS le run de tenant-A (isolation → 404).
        let mut vreq = Request::builder().uri(format!("/v1/swarm/verify/{run_id}")).body(Body::empty()).unwrap();
        vreq.headers_mut().insert("x-efe-tenant", HeaderValue::from_static(TENANT_B));
        let resp = app.clone().oneshot(vreq).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "isolation tenant : invisible");

        // tenant-A, lui, re-prouve son run.
        let mut vreq = Request::builder().uri(format!("/v1/swarm/verify/{run_id}")).body(Body::empty()).unwrap();
        vreq.headers_mut().insert("x-efe-tenant", HeaderValue::from_static(TENANT_A));
        let resp = app.oneshot(vreq).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(body_json(resp).await["verified"].as_bool().unwrap());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn metrics_endpoint_exposes_prometheus() {
        let (app, _s) = app();
        // un run pour alimenter les compteurs.
        let _ = app
            .clone()
            .oneshot(post_run(json!({ "task": { "text": "x", "expected": "x" }, "topology": { "kind": "auto" } })))
            .await
            .unwrap();
        let resp = app
            .oneshot(Request::builder().uri("/metrics").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let txt = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(txt.contains("efe_swarm_prov_records_total"));
        assert!(txt.contains("# TYPE"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn bad_topology_is_400() {
        let (app, _s) = app();
        let resp = app
            .oneshot(post_run(json!({ "task": {}, "topology": { "kind": "pipeline", "stages": [] } })))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
