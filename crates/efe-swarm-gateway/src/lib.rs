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
//! en-tête ; prod : `AuthCtx` OIDC) **tenant-scope** toutes les routes **de données**
//! (`/v1/swarm/*`). Exceptions ASSUMÉES (revue ADR-014) : `GET /metrics` et
//! `/dashboard` sont des surfaces **agrégées sans données tenant** servies SANS auth —
//! le plan scrape/probe (Prometheus, liveness k8s) l'exige ; à protéger au niveau
//! réseau (NetworkPolicy), ne jamais les router publiquement via l'ingress.

#![deny(unsafe_code)]
#![warn(rust_2018_idioms)]

mod auth;
mod dto;
mod error;
#[cfg(feature = "oidc")]
mod oidc;
#[cfg(feature = "pii")]
mod pii;
mod router_map;
mod routes;
mod state;
mod store;
#[cfg(test)]
mod test_support; // ADR-016 : synthetic_mode_c_run partagé (lib tests + store_pg tests)
#[cfg(feature = "postgres")]
mod store_pg;

use axum::routing::get;
use axum::Router;

pub use auth::Caller;
pub use dto::{
    BudgetDto, JobAccepted, ModeDto, OnExceedDto, PatternDto, RunParams, RunRequest, RunResponse,
    RunStatusView, TopologyDto, VerifyResponse,
};
pub use error::GatewayError;
pub use router_map::RouterMap;
pub use state::{AdmissionPermit, AdmitReject, SwarmGatewayState};
pub use store::{InMemoryRunStore, RunStore, StoredRun};
#[cfg(feature = "oidc")]
pub use oidc::OidcVerifier;
#[cfg(feature = "pii")]
pub use pii::{GatewayRedactor, PiiConfig, PolicyKind};
#[cfg(feature = "postgres")]
pub use store_pg::PgRunStore;

/// Version du crate (semver).
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// **App standalone complète** : `/v1/swarm/*` + `/metrics` + `/dashboard` (UI live).
/// Pour un déploiement souverain : `axum::serve(listener, router(state))`.
pub fn router(state: SwarmGatewayState) -> Router {
    Router::new()
        .nest("/v1/swarm", routes::routes())
        .route("/metrics", get(routes::metrics::metrics))
        .route("/dashboard", get(routes::dashboard::dashboard))
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

    // ── ADR-016 : le helper d'un run Mode C SYNTHÉTIQUE (sans Ollama) vit désormais dans
    //    `crate::test_support` — partagé avec les tests Postgres, une seule définition.
    use crate::test_support::synthetic_mode_c_run;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn mode_c_n17_is_persisted_and_reproven_over_http() {
        // I-PN1 + I-PN3 (HTTP) : un run Mode C persisté re-prouve sa forensique — le pont
        // résout, la chaîne N17 se recompose, le verdict GLOBAL est verified.
        let (app, store) = app_with_store();
        let (_recs, stored) = synthetic_mode_c_run("gemma3:12b");
        let id = Uuid::new_v4();
        store.insert_stored(id, stored);

        let resp = app
            .oneshot(Request::builder().uri(format!("/v1/swarm/verify/{id}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let v = body_json(resp).await;
        assert_eq!(v["verified"], json!(true), "essaim + N17 + pont re-prouvés");
        assert_eq!(v["n17"]["records"], json!(1), "la forensique a SURVÉCU à la persistance");
        assert_eq!(v["n17"]["chain_intact"], json!(true), "la chaîne N17 se recompose");
        assert_eq!(v["n17"]["bridge_consistent"], json!(true));
        assert_eq!(v["n17"]["mode_c_consistent"], json!(1), "le lien Mode C résout (plus de pointeur pendant)");
        assert!(v["n17"]["root"].is_string(), "n17_root présent");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn tampered_n17_forensics_break_the_reproof() {
        // I-PN2 (HTTP) : muter un champ forensique d'un record N17 persisté casse la
        // re-preuve — le verdict GLOBAL tombe à verified=false.
        let (app, store) = app_with_store();
        let (_recs, mut stored) = synthetic_mode_c_run("gemma3:12b");
        // un attaquant réécrit le MODÈLE déclaré du record N17 persisté (sans la clé).
        stored.llm_records[0].model_identity =
            efe_nano_sdk::ModelIdentity::hosted("ollama", "MENTEUR:70b", "local");
        let id = Uuid::new_v4();
        store.insert_stored(id, stored);

        let resp = app
            .oneshot(Request::builder().uri(format!("/v1/swarm/verify/{id}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let v = body_json(resp).await;
        assert_eq!(v["verified"], json!(false), "forensique altérée → re-preuve cassée");
        assert_eq!(v["chain_intact"], json!(true), "la chaîne d'ESSAIM, elle, reste intègre");
        assert_eq!(v["n17"]["chain_intact"], json!(false), "mais la chaîne N17 détecte la mutation");
        // ADR-016 : la faute forensique est LOCALISÉE sur le fil HTTP (quel appel LLM a été muté).
        assert_eq!(
            v["n17"]["first_break"],
            json!(Uuid::from_u128(0xC0FFEE).to_string()),
            "le record N17 muté (provenance 0xC0FFEE) est localisé dans n17.first_break"
        );
    }

    // ── ADR-016 §9.1 : les gardiens I-PN au niveau PERSISTANCE/RELECTURE ──────────────
    //    (l'invariant tient « sur le store relu, pas seulement sur le collecteur mémoire »).

    /// Décode 32 octets depuis 64 hex (miroir test de `store::unhex32`).
    #[cfg(test)]
    fn unhex32_t(s: &str) -> Option<[u8; 32]> {
        if s.len() != 64 {
            return None;
        }
        let b = s.as_bytes();
        let nib = |c: u8| -> Option<u8> {
            match c {
                b'0'..=b'9' => Some(c - b'0'),
                b'a'..=b'f' => Some(c - b'a' + 10),
                b'A'..=b'F' => Some(c - b'A' + 10),
                _ => None,
            }
        };
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = (nib(b[i * 2])? << 4) | nib(b[i * 2 + 1])?;
        }
        Some(out)
    }

    /// La clé tenant par défaut (system) — celle qu'un appelant sans en-tête fait dériver.
    #[cfg(test)]
    fn key_system() -> Vec<u8> {
        HmacKeySource::test_key().derive(efe_nano_sdk::TenantId::system()).unwrap()
    }

    #[test]
    fn n17_round_trips_through_the_store() {
        // I-PN1 : durabilité — zéro perte entre la fin du run et la relecture ; le compte relu
        // == le compte produit, **n17_root relu == scellé** (critère GO §8.1), et la re-preuve
        // hors ligne tient depuis les SEULES valeurs relues (pas l'objet vivant).
        use efe_swarm_provenance::verify_run_offline;
        let (_recs, stored) = synthetic_mode_c_run("gemma3:12b");
        let produced = stored.llm_records.len();
        assert!(produced >= 1, "le run synthétique EST bien Mode C");
        // aller-retour serde des TROIS colonnes N17 (la frontière JSONB exacte du store).
        let llm: Vec<efe_nano_sdk::LlmProvenanceRecord> =
            serde_json::from_str(&serde_json::to_string(&stored.llm_records).unwrap()).unwrap();
        let leaves_hex: Vec<String> =
            serde_json::from_str(&serde_json::to_string(&stored.n17_leaves).unwrap()).unwrap();
        let root_hex: Option<String> =
            serde_json::from_str(&serde_json::to_string(&stored.n17_root).unwrap()).unwrap();
        assert_eq!(llm.len(), produced, "I-PN1 : compte relu == compte produit");
        assert_eq!(llm, stored.llm_records, "forensique identique après relecture");
        assert_eq!(root_hex, stored.n17_root, "I-PN1 (§8.1) : n17_root relu == scellé");
        assert_eq!(leaves_hex, stored.n17_leaves, "feuilles relues == scellées");
        // re-preuve depuis les valeurs RELUES décodées (hex → bytes), jamais l'objet vivant.
        let leaves: Vec<[u8; 32]> = leaves_hex.iter().filter_map(|h| unhex32_t(h)).collect();
        let root = root_hex.as_deref().and_then(unhex32_t);
        let v = verify_run_offline(&stored.records, &llm, &leaves, root, &key_system());
        assert!(v.fully_proven, "la forensique relue se re-prouve");
        assert_eq!(v.n17_chain.records, produced, "tous les records ont survécu");
    }

    #[test]
    fn persisted_link_resolves_or_is_flagged_missing() {
        // I-PN4 : plus de pointeur pendant — un lien Mode C persisté résout (a) ; une
        // forensique manquante à la relecture est détectée Missing, jamais ignorée (b).
        use efe_swarm_provenance::{verify_run_offline, LinkStatus};
        let (_recs, stored) = synthetic_mode_c_run("gemma3:12b");
        let key = key_system();
        // (a) résout.
        let ok = verify_run_offline(&stored.records, &stored.llm_records, &stored.n17_leaf_bytes(), stored.n17_root_bytes(), &key);
        assert!(ok.bridge.consistent >= 1 && ok.bridge.all_mode_c_consistent(), "I-PN4 (a) : le lien résout");
        // (b) forensique perdue → Missing.
        let lost = verify_run_offline(&stored.records, &[], &[], None, &key);
        assert!(!lost.fully_proven, "lien pendant → re-preuve cassée");
        assert!(lost.bridge.broken >= 1, "le pont compte la rupture");
        assert!(
            lost.bridge.broken_links().any(|l| l.status == LinkStatus::Missing),
            "I-PN4 (b) : statut = Missing (jamais ignoré)"
        );
    }

    #[test]
    fn cross_tenant_n17_is_rejected_after_reload() {
        // I-PN5 : la triple cohérence (existe · même tenant · même swarm) tient sur le store
        // RELU. Un N17 injecté d'un autre tenant/swarm, RE-SCELLÉ (chaîne N17 intacte), doit être
        // CLASSÉ TenantMismatch/SwarmMismatch par le pont — pas un simple Missing (sinon le
        // gardien d'un invariant de SÉCURITÉ resterait vert même si l'isolation dégénérait).
        use efe_swarm_provenance::{seal_n17, verify_run_offline, LinkStatus};
        let key = key_system();

        // (a) injection tenant → TenantMismatch, sur le store relu.
        let (_r, mut t) = synthetic_mode_c_run("gemma3:12b");
        t.llm_records[0].tenant_id = efe_nano_sdk::TenantId::new();
        let rs = seal_n17(&t.llm_records, &key);
        let reloaded: Vec<efe_nano_sdk::LlmProvenanceRecord> =
            serde_json::from_str(&serde_json::to_string(&t.llm_records).unwrap()).unwrap();
        let v = verify_run_offline(&t.records, &reloaded, &rs.leaf_hashes, rs.n17_root, &key);
        assert!(v.swarm.ok() && v.n17_chain.intact, "essaim + chaîne N17 intacts (re-scellés)");
        assert!(!v.fully_proven, "I-PN5 (a) : run NON prouvé");
        assert!(
            v.bridge.broken_links().any(|l| l.status == LinkStatus::TenantMismatch),
            "I-PN5 (a) : N17 cross-tenant CLASSÉ TenantMismatch (pas un simple Missing) sur le store relu"
        );

        // (b) injection swarm → SwarmMismatch, AUSSI sur le store relu (parité avec (a)).
        let (_r2, mut s) = synthetic_mode_c_run("gemma3:12b");
        s.llm_records[0].swarm_request_id = uuid::Uuid::from_u128(0xDEAD_BEEF);
        let rs2 = seal_n17(&s.llm_records, &key);
        let reloaded2: Vec<efe_nano_sdk::LlmProvenanceRecord> =
            serde_json::from_str(&serde_json::to_string(&s.llm_records).unwrap()).unwrap();
        let v2 = verify_run_offline(&s.records, &reloaded2, &rs2.leaf_hashes, rs2.n17_root, &key);
        assert!(!v2.fully_proven, "I-PN5 (b) : run NON prouvé");
        assert!(
            v2.bridge.broken_links().any(|l| l.status == LinkStatus::SwarmMismatch),
            "I-PN5 (b) : N17 cross-swarm CLASSÉ SwarmMismatch sur le store relu"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn verify_endpoint_reports_bridge_verdict() {
        // HTTP (§9.1) : l'endpoint verify/:id EXPOSE le verdict du pont — cohérent sur un run
        // sain, rompu (mode_c_broken) sur un run dont la forensique a été perdue.
        let (app, store) = app_with_store();
        let (_r, healthy) = synthetic_mode_c_run("gemma3:12b");
        let id_ok = Uuid::new_v4();
        store.insert_stored(id_ok, healthy);
        let v = body_json(
            app.clone()
                .oneshot(Request::builder().uri(format!("/v1/swarm/verify/{id_ok}")).body(Body::empty()).unwrap())
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(v["n17"]["bridge_consistent"], json!(true));
        assert_eq!(v["n17"]["mode_c_consistent"], json!(1));
        assert_eq!(v["n17"]["mode_c_broken"], json!(0));

        // forensique perdue (le lien Mode C devient pendant) → le pont REPORTE la rupture.
        let (_r2, mut broken) = synthetic_mode_c_run("gemma3:12b");
        broken.llm_records.clear();
        broken.n17_leaves.clear();
        broken.n17_root = None;
        let id_ko = Uuid::new_v4();
        store.insert_stored(id_ko, broken);
        let v2 = body_json(
            app.oneshot(Request::builder().uri(format!("/v1/swarm/verify/{id_ko}")).body(Body::empty()).unwrap())
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(v2["verified"], json!(false), "pont rompu → non vérifié");
        assert!(v2["n17"]["mode_c_broken"].as_u64().unwrap() >= 1, "l'endpoint reporte le verdict du pont (Missing)");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn n17_export_round_trips_for_offline_verifier() {
        // HTTP (§9.1) : les exports /records + /n17 d'un run Mode C, récupérés par un tiers,
        // se rejouent en verify_run_offline avec sa propre clé — re-preuve complète, sans serveur.
        use efe_swarm_provenance::{verify_run_offline, AgentCallRecord};
        let (app, store) = app_with_store();
        let (_r, stored) = synthetic_mode_c_run("gemma3:12b");
        let id = Uuid::new_v4();
        store.insert_stored(id, stored);

        let recs_v = body_json(
            app.clone()
                .oneshot(Request::builder().uri(format!("/v1/swarm/runs/{id}/records")).body(Body::empty()).unwrap())
                .await
                .unwrap(),
        )
        .await;
        let exp = body_json(
            app.oneshot(Request::builder().uri(format!("/v1/swarm/runs/{id}/n17")).body(Body::empty()).unwrap())
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(exp["llm_records"].as_array().unwrap().len(), 1, "export forensique NON vide (Mode C)");
        assert!(exp["n17_root"].is_string(), "n17_root exporté");

        let swarm: Vec<AgentCallRecord> = serde_json::from_value(recs_v).unwrap();
        let llm: Vec<efe_nano_sdk::LlmProvenanceRecord> = serde_json::from_value(exp["llm_records"].clone()).unwrap();
        let leaves: Vec<[u8; 32]> =
            exp["n17_leaves"].as_array().unwrap().iter().filter_map(|x| unhex32_t(x.as_str().unwrap_or(""))).collect();
        let root = exp["n17_root"].as_str().and_then(unhex32_t);
        let v = verify_run_offline(&swarm, &llm, &leaves, root, &key_system());
        assert!(v.fully_proven, "/records + /n17 + clé → re-preuve tierce complète (HTTP)");
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
    async fn metrics_endpoint_exposes_real_provenance_and_substrate() {
        let (app, _s) = app();
        // un consensus keyless : exerce le SUBSTRAT (écrit /cand/* + /ballot/* sur le
        // Blackboard interne) ET la provenance → les deux familles de métriques bougent.
        let _ = app
            .clone()
            .oneshot(post_run(json!({
                "task": { "text": "hello", "expected": "hello" },
                "topology": { "kind": "consensus", "generator": "echo_generator",
                              "judges": ["consistency", "length_band", "j_non_empty"], "k": 5 }
            })))
            .await
            .unwrap();
        let resp = app
            .oneshot(Request::builder().uri("/metrics").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let txt = String::from_utf8(bytes.to_vec()).unwrap();
        // provenance (réelle).
        assert!(txt.contains("efe_swarm_prov_records_total"));
        assert!(txt.contains("# TYPE efe_swarm_prov_traced_ratio gauge"));
        // substrat (réel, PAS un placeholder mort) : les écritures Blackboard ont été comptées.
        let writes = txt
            .lines()
            .find_map(|l| l.strip_prefix("swarm_blackboard_writes_total "))
            .and_then(|v| v.trim().parse::<u64>().ok())
            .expect("métrique swarm_blackboard_writes_total présente");
        assert!(writes > 0, "le substrat est réellement alimenté (writes={writes})");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn run_accepts_deadline_ms_and_reports_termination_in_response() {
        // ADR-018 (HTTP) : `deadline_ms` est un champ ACCEPTÉ (pas un 400 deny_unknown_fields)
        // et le statut de terminaison est exposé dans la réponse. Deadline GÉNÉREUX → le run
        // rapide finit dans la fenêtre → `completed` (la troncature par le temps est, elle,
        // prouvée bout-en-bout côté SDK : run_traced est le code que ce handler appelle).
        let (app, _s) = app();
        let resp = app
            .oneshot(post_run(json!({
                "task": { "text": "hello", "expected": "hello" },
                "topology": { "kind": "consensus", "generator": "echo_generator",
                              "judges": ["consistency", "j_non_empty"], "k": 3 },
                "deadline_ms": 60000
            })))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "deadline_ms accepté");
        let v = body_json(resp).await;
        assert_eq!(v["run"]["termination"], json!("completed"), "fini dans la fenêtre");
        assert_eq!(v["run"]["provenance"]["verified"], json!(true));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn metrics_expose_per_tenant_admission_surfaces() {
        // ADR-018 : /metrics expose les surfaces d'équité (plafond par-tenant + tenants
        // actifs), en plus des surfaces process ADR-014.
        let (app, _s) = app();
        let resp = app
            .oneshot(Request::builder().uri("/metrics").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let txt = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(txt.contains("efe_swarm_gateway_inflight_per_tenant_max"), "plafond par-tenant exposé");
        assert!(txt.contains("efe_swarm_gateway_admission_tenants_active"), "tenants actifs exposés");
    }

    /// **E2E self-host RÉEL** (Ollama local, keyless). Ignoré par défaut (CI sans modèle) ;
    /// `ollama serve` + `gemma3:12b` requis. La passerelle exécute un consensus LLM par
    /// HTTP, persiste les records, puis re-prouve via `verify/:id`.
    /// Lancer : `cargo test -p efe-swarm-gateway --lib -- --ignored selfhost_e2e_real`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "nécessite un Ollama local (ollama serve + gemma3:12b)"]
    async fn selfhost_e2e_real() {
        let endpoint =
            std::env::var("EFE_OLLAMA_ENDPOINT").unwrap_or_else(|_| "http://127.0.0.1:11434".into());
        let model = std::env::var("EFE_OLLAMA_MODEL").unwrap_or_else(|_| "gemma3:12b".into());
        let (app, _s) = app();
        let resp = app
            .clone()
            .oneshot(post_run(json!({
                "task": { "text": "The court ruled the clause unenforceable.", "expected": "legal" },
                "topology": { "kind": "consensus",
                              "generator": "Classify the topic in one word from {technology,finance,health,legal}.",
                              "judges": ["Judge how well the label fits the task.",
                                         "Assess the relevance and correctness of the answer."],
                              "k": 3, "pattern": "borda" },
                "budget": { "max_cents": 1000 },
                "mode": { "self_host": { "endpoint": endpoint, "model": model } }
            })))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let run = body_json(resp).await;
        let run_id = run["run_id"].as_str().unwrap().to_string();
        assert!(run["run"]["provenance"]["verified"].as_bool().unwrap());

        // re-preuve indépendante d'un run LLM réel.
        let resp = app
            .oneshot(Request::builder().uri(format!("/v1/swarm/verify/{run_id}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(body_json(resp).await["verified"].as_bool().unwrap(), "run self-host re-prouvé");
    }

    /// **Cost-governance Network RÉEL** (Inc.4.b) : on pointe le routeur Network sur l'endpoint
    /// **OpenAI-compat d'Ollama** (`/v1`) — un vrai backend frontière, localement. Prouve que le
    /// tarif Network est **bien facturé** (`spent_cents > 0`, multiple du tarif) après le correctif :
    /// sans lui, la traduction `Network → SelfHost` facturerait 0¢. Re-preuve incluse.
    /// Lancer : `cargo test -p efe-swarm-gateway --lib -- --ignored network_budget_charges_real`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "nécessite un Ollama local (ollama serve + un modèle OpenAI-compat)"]
    async fn network_budget_charges_real_via_ollama_openai_endpoint() {
        let endpoint = std::env::var("EFE_OLLAMA_ENDPOINT").unwrap_or_else(|_| "http://127.0.0.1:11434".into());
        let model = std::env::var("EFE_OLLAMA_MODEL").unwrap_or_else(|_| "gemma3:12b".into());
        let router_v1 = format!("{}/v1", endpoint.trim_end_matches('/'));
        // Le « routeur Network » = l'endpoint OpenAI-compat d'Ollama ; provider "ollama" → modèle réel.
        let state = SwarmGatewayState::in_memory()
            .with_router_endpoint(router_v1)
            .with_router_models(RouterMap::empty().with("ollama", model));
        let app = router(state);

        let resp = app
            .clone()
            .oneshot(post_run(json!({
                "task": { "text": "The court ruled the clause unenforceable.", "expected": "legal" },
                "topology": { "kind": "consensus",
                              "generator": "Classify the topic in one word from {technology,finance,health,legal}.",
                              "judges": ["Judge how well the label fits the text."],
                              "k": 3, "pattern": "borda" },
                "budget": { "max_cents": 100000 },
                "mode": { "network": { "provider": "ollama" } }
            })))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let run = body_json(resp).await;

        // Le correctif : le tarif Network est facturé (≠ 0¢), multiple du tarif par appel.
        let spent = run["run"]["budget"]["spent_cents"].as_u64().unwrap();
        let net_price =
            efe_swarm_sdk::ClientMode::Network { provider: "x".into() }.price_per_call_cents() as u64;
        assert!(spent > 0, "Network DOIT facturer (spent_cents > 0), eu {spent}");
        assert_eq!(spent % net_price, 0, "spent_cents multiple du tarif Network ({net_price}¢)");
        // le run reste re-prouvable, et le backend reflète le routage OpenAI-compat.
        assert!(run["run"]["provenance"]["verified"].as_bool().unwrap());
        assert_eq!(run["run"]["backend"]["engine"].as_str().unwrap(), "openai_local");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn network_mode_without_router_is_501() {
        // Inc.4 : Network demandé mais aucun routeur câblé (déploiement souverain) → 501.
        let (app, _s) = app();
        let resp = app
            .oneshot(post_run(json!({
                "task": { "text": "x" },
                "topology": { "kind": "auto" },
                "mode": { "network": { "provider": "anthropic" } }
            })))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);
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

    // ── Inc.3 — dashboard live ────────────────────────────────────────────────

    async fn collect_text(resp: axum::response::Response) -> String {
        let bytes = tokio::time::timeout(std::time::Duration::from_secs(10), resp.into_body().collect())
            .await
            .expect("SSE doit se terminer au terminal")
            .unwrap()
            .to_bytes();
        String::from_utf8_lossy(&bytes).to_string()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn job_streams_live_events_then_persists_and_verifies() {
        let (app, _s) = app();
        // 1. POST ?stream=true → 202 job.
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/swarm/run?stream=true")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "task": { "text": "hello", "expected": "hello" },
                            "topology": { "kind": "consensus", "generator": "echo_generator",
                                          "judges": ["consistency", "length_band", "j_non_empty"], "k": 5 }
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let job = body_json(resp).await;
        assert_eq!(job["status"], "running");
        let run_id = job["run_id"].as_str().unwrap().to_string();

        // 2. GET …/events : rejeu + live → se termine au `run_finished`.
        let resp = app
            .clone()
            .oneshot(Request::builder().uri(format!("/v1/swarm/runs/{run_id}/events")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let sse = collect_text(resp).await;
        // chaque famille de panneau est présente dans le flux live (frames `event: <kind>`).
        for needle in ["event: run_started", "event: agent_write", "event: candidate", "event: ballot",
                       "event: decided", "event: budget_tick", "event: sealed", "event: run_finished"] {
            assert!(sse.contains(needle), "le flux SSE doit contenir {needle}\n---\n{sse}");
        }

        // 3. le job a persisté les records → /runs/:id Finished, puis verify re-prouve.
        let mut finished = false;
        for _ in 0..100 {
            let resp = app
                .clone()
                .oneshot(Request::builder().uri(format!("/v1/swarm/runs/{run_id}")).body(Body::empty()).unwrap())
                .await
                .unwrap();
            if resp.status() == StatusCode::OK
                && body_json(resp).await["status"] == "finished" {
                    finished = true;
                    break;
                }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(finished, "le job persiste le run");

        let resp = app
            .clone()
            .oneshot(Request::builder().uri(format!("/v1/swarm/verify/{run_id}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(body_json(resp).await["verified"].as_bool().unwrap(), "run live re-prouvé (le moat)");

        // 4. /metrics expose les surfaces du dashboard, réellement alimentées par le job.
        let resp = app
            .oneshot(Request::builder().uri("/metrics").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let txt = collect_text(resp).await;
        let emitted = txt
            .lines()
            .find_map(|l| l.strip_prefix("efe_swarm_dash_events_emitted_total "))
            .and_then(|v| v.trim().parse::<u64>().ok())
            .expect("métrique efe_swarm_dash_events_emitted_total présente");
        assert!(emitted > 0, "le dashboard a diffusé des events (emitted={emitted})");
        assert!(txt.contains("efe_swarm_dash_active_runs"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn dashboard_html_is_served() {
        let (app, _s) = app();
        let resp = app
            .oneshot(Request::builder().uri("/dashboard").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let html = collect_text(resp).await;
        assert!(html.contains("essaim live"));
        assert!(html.contains("EventSource"), "UI vanilla : SSE natif");
        assert!(html.contains("4 · budget"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn events_of_unknown_run_is_404() {
        let (app, _s) = app();
        let resp = app
            .oneshot(Request::builder().uri(format!("/v1/swarm/runs/{}/events", Uuid::new_v4())).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ── ADR-017 : rejeu déterministe (/replay) — gardiens I-RP ─────────────────────────

    /// Un run keyless DÉTERMINISTE (echo_generator + juges purs) : re-exécuté à l'identique,
    /// il reproduit la racine bit-à-bit.
    fn keyless_run_body(task: Value) -> Value {
        json!({
            "task": task,
            "topology": { "kind": "consensus", "generator": "echo_generator",
                          "judges": ["consistency", "length_band", "j_non_empty"], "k": 5 },
            "budget": { "max_cents": 1000 }
        })
    }
    fn replay_post(id: &str, body: Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(format!("/v1/swarm/replay/{id}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }
    fn replay_get(id: &str) -> Request<Body> {
        Request::builder().uri(format!("/v1/swarm/replay/{id}")).body(Body::empty()).unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn replay_r0_reseals_to_same_root_offline() {
        // I-RP3 : GET /replay/:id re-scelle les records scellés (ordre canonique ADR-007) →
        // la racine se reproduit, hors-ligne, sans re-exécuter aucun agent.
        let (app, _s) = app();
        let run = body_json(app.clone().oneshot(post_run(keyless_run_body(json!({ "text": "x", "expected": "x" })))).await.unwrap()).await;
        let id = run["run_id"].as_str().unwrap().to_string();
        let v = body_json(app.oneshot(replay_get(&id)).await.unwrap()).await;
        assert_eq!(v["tier"], json!("R0"));
        assert_eq!(v["root_reproduced"], json!(true), "I-RP3 : racine reproduite depuis les records scellés");
        assert_eq!(v["content_intact"], json!(true));
        assert_eq!(v["reproduced_root"], v["stored_root"], "racine re-dérivée == racine scellée");
        assert!(v["determinism"]["full_deterministic"].as_u64().unwrap() >= 1, "Mode A → FullDeterministic");
        assert_eq!(v["n17"]["records"], json!(0), "run keyless → 0 forensique N17");
        assert!(v["execution"].is_null(), "R0 ne re-exécute pas");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn replay_reexecutes_mode_a_to_identical_outputs() {
        // I-RP1 : R1 re-exécute le squelette Mode A sur la MÊME task → re-dérive les MÊMES
        // SORTIES (output_hash + flux de données). La racine, elle, lie l'identité par-run
        // (swarm_id/record_id) → re-prouvée par R0 (root_reproduced), NON reproduite par
        // re-exécution. C'est l'asymétrie honnête : la computation se reproduit, l'empreinte non.
        let (app, _s) = app();
        let body = keyless_run_body(json!({ "text": "hello", "expected": "hello" }));
        let run = body_json(app.clone().oneshot(post_run(body.clone())).await.unwrap()).await;
        let id = run["run_id"].as_str().unwrap().to_string();
        let v = body_json(app.oneshot(replay_post(&id, body)).await.unwrap()).await;
        assert_eq!(v["tier"], json!("R1"));
        assert_eq!(v["root_reproduced"], json!(true), "R0 base : les records scellés re-prouvent la racine");
        assert_eq!(v["execution"]["ran"], json!(true), "Mode A re-exécuté (keyless, sans provider)");
        assert_eq!(v["execution"]["outputs_reproduced"], json!(true), "I-RP1 : sorties re-dérivées à l'identique");
        assert!(v["first_divergence"].is_null(), "aucune divergence de sortie");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn replay_is_width_invariant() {
        // I-RP8 (I-CP1 héritée) : rejouer à une largeur DIFFÉRENTE re-dérive les mêmes sorties.
        let (app, _s) = app();
        let mut narrow = keyless_run_body(json!({ "text": "w", "expected": "w" }));
        narrow["width"] = json!(1);
        let run = body_json(app.clone().oneshot(post_run(narrow)).await.unwrap()).await;
        let id = run["run_id"].as_str().unwrap().to_string();
        let mut wide = keyless_run_body(json!({ "text": "w", "expected": "w" }));
        wide["width"] = json!(8);
        let v = body_json(app.oneshot(replay_post(&id, wide)).await.unwrap()).await;
        assert_eq!(v["execution"]["outputs_reproduced"], json!(true), "I-RP8 : largeur neutre aux sorties");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn replay_detects_and_localizes_divergence() {
        // I-RP2 : muter un record stocké → R0 ne reproduit plus la racine, et LOCALISE la faute.
        let (app, store) = app_with_store();
        let run = body_json(app.clone().oneshot(post_run(keyless_run_body(json!({ "text": "x", "expected": "x" })))).await.unwrap()).await;
        let id: Uuid = run["run_id"].as_str().unwrap().parse().unwrap();
        assert!(store.tamper_first_output(id), "un record stocké est muté");
        let v = body_json(app.oneshot(replay_get(&id.to_string())).await.unwrap()).await;
        assert_eq!(v["root_reproduced"], json!(false), "I-RP2 : rejeu cassé par la mutation");
        assert_eq!(v["content_intact"], json!(false));
        assert!(v["first_divergence"].is_object(), "1ʳᵉ divergence localisée");
        assert_eq!(v["first_divergence"]["reason"], json!("content_break"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn replay_rejects_task_mismatch() {
        // Garde anti-confusion (§5.1) : re-fournir une AUTRE task ne doit PAS produire un faux
        // `outputs_reproduced: false` (qui ferait croire le run non reproductible) ; c'est une
        // ERREUR D'OPÉRATEUR signalée `task_mismatch`, distincte d'une vraie non-reproduction.
        let (app, _s) = app();
        let run = body_json(app.clone().oneshot(post_run(keyless_run_body(json!({ "text": "original", "expected": "original" })))).await.unwrap()).await;
        let id = run["run_id"].as_str().unwrap().to_string();
        let v = body_json(app.oneshot(replay_post(&id, keyless_run_body(json!({ "text": "DIFFERENT", "expected": "DIFFERENT" })))).await.unwrap()).await;
        assert_eq!(v["execution"]["task_mismatch"], json!(true), "task différente → erreur d'opérateur signalée");
        assert!(v["execution"]["outputs_reproduced"].is_null(), "PAS de faux verdict de (non-)reproductibilité");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn replay_r1_localizes_output_divergence() {
        // I-RP2 (R1, vecteur §8.1 ②(b)) : MÊME task + MÊMES générateurs, mais un jeu de JUGES
        // différent → les générateurs reproduisent (task_mismatch=false), un record de juge
        // DIVERGE → R1 émet outputs_reproduced=false et localise un `output_mismatch` (≠ le
        // `content_break` de R0). C'est la non-reproduction RÉELLE de la computation, pas une
        // erreur d'opérateur de task.
        let (app, _s) = app();
        let three = json!({
            "task": { "text": "x", "expected": "x" },
            "topology": { "kind": "consensus", "generator": "echo_generator",
                          "judges": ["consistency", "length_band", "j_non_empty"], "k": 5 },
            "budget": { "max_cents": 1000 }
        });
        let run = body_json(app.clone().oneshot(post_run(three)).await.unwrap()).await;
        let id = run["run_id"].as_str().unwrap().to_string();
        // rejoue avec un juge en MOINS (générateurs identiques) → un ballot scellé sans pendant.
        let two = json!({
            "task": { "text": "x", "expected": "x" },
            "topology": { "kind": "consensus", "generator": "echo_generator",
                          "judges": ["consistency", "length_band"], "k": 5 },
            "budget": { "max_cents": 1000 }
        });
        let v = body_json(app.oneshot(replay_post(&id, two)).await.unwrap()).await;
        assert_eq!(v["execution"]["task_mismatch"], json!(false), "générateurs identiques → PAS un task_mismatch");
        assert_eq!(v["execution"]["outputs_reproduced"], json!(false), "jeu de juges différent → sorties non re-dérivées");
        assert!(v["first_divergence"].is_object(), "divergence localisée");
        assert_eq!(v["first_divergence"]["reason"], json!("output_mismatch"), "R1 : output_mismatch (≠ content_break de R0)");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn replay_classifies_mode_c_honestly_and_reverifies_n17() {
        // I-RP6 + I-RP7 : un run Mode C synthétique — R0 classe (Seeded) + re-vérifie la chaîne
        // N17 (sans la recomputer) ; R1 ne re-exécute PAS le provider (hors-périmètre) et classe
        // honnêtement (Seeded → SkippedNonDeterministic, jamais faux Match).
        let (app, store) = app_with_store();
        let (_r, stored) = synthetic_mode_c_run("gemma3:12b");
        let id = Uuid::new_v4();
        store.insert_stored(id, stored);
        let v = body_json(app.clone().oneshot(replay_get(&id.to_string())).await.unwrap()).await;
        assert_eq!(v["records"], json!(1));
        assert_eq!(v["determinism"]["seeded"], json!(1), "le record Mode C (graine posée) → Seeded");
        assert_eq!(v["root_reproduced"], json!(true), "la chaîne d'ESSAIM se reproduit (même Mode C)");
        assert_eq!(v["n17"]["chain_intact"], json!(true), "I-RP7 : chaîne N17 re-vérifiée");
        assert_eq!(v["n17"]["records"], json!(1));
        // R1 : Mode C → re-exécution provider hors-périmètre, classé honnêtement.
        let v1 = body_json(app.oneshot(replay_post(&id.to_string(), keyless_run_body(json!({ "value": "ok" })))).await.unwrap()).await;
        assert_eq!(v1["execution"]["ran"], json!(false), "I-RP6 : pas de re-exécution provider en aveugle");
        assert_eq!(v1["per_op"][0]["outcome"], json!("skipped_non_deterministic"), "Seeded → Skipped, jamais faux Match");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn replay_reverifies_n17_without_recomputing_root() {
        // I-RP7 : la forensique N17 est RE-PROUVÉE, pas RE-PRODUITE. Muter un record N17 persisté
        // → le rejeu R0 signale `n17.chain_intact=false` (verify_run_offline relit les feuilles
        // scellées) ET localise le record fautif, ALORS QUE la racine d'ESSAIM reste reproduite
        // (chaîne d'essaim intacte) — preuve que la N17 est re-vérifiée SÉPARÉMENT, jamais
        // repliée dans la racine (qui, elle, est seulement re-prouvée par re-scellement).
        let (app, store) = app_with_store();
        let (_r, mut stored) = synthetic_mode_c_run("gemma3:12b");
        stored.llm_records[0].model_identity =
            efe_nano_sdk::ModelIdentity::hosted("ollama", "MENTEUR:70b", "local");
        let id = Uuid::new_v4();
        store.insert_stored(id, stored);
        let v = body_json(app.oneshot(replay_get(&id.to_string())).await.unwrap()).await;
        assert_eq!(v["n17"]["chain_intact"], json!(false), "I-RP7 : forensique N17 falsifiée DÉTECTÉE au rejeu");
        assert!(v["n17"]["first_break"].is_string(), "le record N17 muté est localisé");
        assert_eq!(v["root_reproduced"], json!(true), "la racine d'ESSAIM reste reproduite — N17 re-vérifiée À PART, jamais recomputée dans la racine");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn replay_is_tenant_scoped() {
        // I-RP5 : un run d'un autre tenant est invisible au rejeu (404).
        let (app, _s) = app();
        let mut req = post_run(keyless_run_body(json!({ "text": "x", "expected": "x" })));
        req.headers_mut().insert("x-efe-tenant", HeaderValue::from_static(TENANT_A));
        let run = body_json(app.clone().oneshot(req).await.unwrap()).await;
        let id = run["run_id"].as_str().unwrap().to_string();
        let mut vreq = replay_get(&id);
        vreq.headers_mut().insert("x-efe-tenant", HeaderValue::from_static(TENANT_B));
        let resp = app.oneshot(vreq).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "I-RP5 : isolation tenant à la relecture");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn replay_never_mutates_the_store() {
        // I-RP4 : le rejeu est lecture-seule — après rejeu, le run reste intègre et idempotent.
        let (app, store) = app_with_store();
        let (_r, stored) = synthetic_mode_c_run("gemma3:12b");
        let id = Uuid::new_v4();
        store.insert_stored(id, stored);
        let _ = app.clone().oneshot(replay_get(&id.to_string())).await.unwrap();
        // verify passe encore (le rejeu n'a rien altéré).
        let v = body_json(app.clone().oneshot(Request::builder().uri(format!("/v1/swarm/verify/{id}")).body(Body::empty()).unwrap()).await.unwrap()).await;
        assert_eq!(v["verified"], json!(true), "I-RP4 : rejeu sans effet de bord (verify intact)");
        // R0 idempotent.
        let v2 = body_json(app.oneshot(replay_get(&id.to_string())).await.unwrap()).await;
        assert_eq!(v2["root_reproduced"], json!(true), "rejeu idempotent");
    }
}
