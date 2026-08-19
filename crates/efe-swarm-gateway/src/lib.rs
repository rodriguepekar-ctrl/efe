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
#[cfg(feature = "frontier-llm")]
mod frontier; // Couche 2 : construit le Coordinator frontière (Claude/Gemini/GPT) depuis l'env
#[cfg(feature = "frontier-llm")]
mod mission; // Couche 2 (M1) : capture de provenance par appel LLM (décorateur + scope task-local)
#[cfg(feature = "frontier-llm")]
mod mission_graph; // Couche 2 (M2) : le reçu honnête — graphe depuis topologie déclarée + scellement + store
#[cfg(feature = "frontier-llm")]
pub mod frontier_metrics; // Couche 2 (M4 §2.4) : compteurs de coût/egress frontière pour /metrics
// ADR-016 : `synthetic_mode_c_run` partagé (lib tests + store_pg tests). La feature `demo`
// l'expose hors `#[cfg(test)]` pour l'exemple `local_receipt_demo` : un fixture dupliqué
// dériverait du seul littéral `LlmProvenanceRecord` de référence, ce que ce module existe
// précisément pour empêcher. OFF par défaut.
#[cfg(any(test, feature = "demo"))]
pub mod test_support;
#[cfg(feature = "postgres")]
mod store_pg;
#[cfg(feature = "postgres")]
mod store_pg_memory; // ADR-019 (MEM-3) : PgMemoryStore (mémoire durable Postgres)

use axum::routing::get;
use axum::Router;

pub use auth::Caller;
pub use dto::{
    BudgetDto, JobAccepted, ModeDto, OnExceedDto, PatternDto, RunParams, RunRequest, RunResponse,
    RunStatusView, TopologyDto, VerifyResponse,
};
pub use error::GatewayError;
pub use router_map::RouterMap;
// M4 §2.1 : le plafond serveur de budget, re-exporté pour que le binaire l'annonce au boot.
#[cfg(feature = "frontier-llm")]
pub use routes::llm::server_budget_ceiling_cents;
pub use state::{bundle_export_from_env, AdmissionPermit, AdmitReject, SwarmGatewayState};
// Fil P (P.3) : le signataire d'attestation, re-exporté pour que le binaire le construise
// depuis l'env sans dépendre directement du pont.
pub use provenance_bridge::{AnchorSink, BundleSigner, FileAnchorSink};
pub use store::{InMemoryRunStore, RunStore, StoredRun};
#[cfg(feature = "frontier-llm")]
pub use frontier::coordinator_from_env;
#[cfg(feature = "frontier-llm")]
pub use mission::{scoped, MissionProvenanceSink, MissionScope}; // Couche 2 (M1) : surface de capture consommée par les handlers M3
#[cfg(feature = "frontier-llm")]
pub use mission_graph::{
    build_and_seal_mission, seal_and_store_mission, specialist_declared_arity, topology_cascade,
    topology_debate, topology_relay, topology_specialist, topology_vote, BuildError, DeclaredCall,
    MissionSeal, MissionTopology,
}; // Couche 2 (M2) : le reçu honnête — surface consommée par les handlers M3
#[cfg(feature = "oidc")]
pub use oidc::OidcVerifier;
#[cfg(feature = "pii")]
pub use pii::{GatewayRedactor, PiiConfig, PolicyKind};
#[cfg(feature = "postgres")]
pub use store_pg::PgRunStore;
#[cfg(feature = "postgres")]
pub use store_pg_memory::PgMemoryStore;

/// Version du crate (semver).
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// **App standalone complète** : `/v1/swarm/*` + `/metrics` + `/dashboard` (UI live).
/// Pour un déploiement souverain : `axum::serve(listener, router(state))`.
pub fn router(state: SwarmGatewayState) -> Router {
    let app = Router::new()
        .nest("/v1/swarm", routes::routes())
        .route("/metrics", get(routes::metrics::metrics))
        // Sondes k8s (M4 §G4) : liveness triviale + readiness reflétant le câblage réel
        // (frontière annoncée mais coordinator absent ⟹ 503, retiré du load-balancer).
        .route("/healthz", get(routes::health::healthz))
        .route("/readyz", get(routes::health::readyz))
        .route("/dashboard", get(routes::dashboard::dashboard));
    // Couche 2 (M3) : la surface frontière servie — montée SOUS le feature `frontier-llm`
    // uniquement (build par défaut byte-identique). `#[cfg]` sur un `let` shadow : le type reste
    // `Router<SwarmGatewayState>` jusqu'au `with_state` final.
    #[cfg(feature = "frontier-llm")]
    let app = app.nest("/v1/llm", routes::llm::routes_llm());
    // Fil P (P.2) : l'export du bundle de provenance — monté **seulement** sur opt-in explicite.
    // Sans lui, l'URI n'existe pas (404), donc le comportement servi est byte-identique à
    // l'avant-P.2 : pas de 501 qui trahirait l'existence d'une surface désactivée.
    let app = if state.bundle_export_enabled() {
        app.route("/v1/swarm/bundle/:id", get(routes::bundle::bundle))
    } else {
        app
    };
    app.with_state(state)
}

/// Les **routes d'essaim** (relatif, sans préfixe) — à monter via `.nest("/v1/swarm", …)`
/// dans une app hôte (`efe-server`).
pub fn swarm_router(state: SwarmGatewayState) -> Router {
    routes::routes().with_state(state)
}

/// Couche 2 (M3) : les **routes frontière** (relatif : `vote`, `relay`, …) — à monter via
/// `.nest("/v1/llm", llm_router(state))` dans une app hôte. Séparé de [`swarm_router`] pour
/// que l'hôte les préfixe indépendamment (`/v1/llm/*`, PAS `/v1/swarm/v1/llm/*`).
#[cfg(feature = "frontier-llm")]
pub fn llm_router(state: SwarmGatewayState) -> Router {
    routes::llm::routes_llm().with_state(state)
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

    /// État de test avec un `InMemoryMemoryStore` **concret** conservé (pour le testkit de
    /// falsification du journal mémoire), injecté comme `Arc<dyn MemoryStore>` dans l'état. Clé
    /// `test_key` côté store ET côté état → le témoin re-vérifie (cohérence de clé).
    fn app_with_memory_store() -> (Router, Arc<efe_swarm_memory::InMemoryMemoryStore>) {
        let mem = Arc::new(efe_swarm_memory::InMemoryMemoryStore::new());
        let state = SwarmGatewayState::new(
            Arc::new(AgentFleet::standard()),
            HmacKeySource::test_key(),
            Arc::new(InMemoryRunStore::new()),
            ProvenanceMetrics::new(),
            Arc::new(SubstrateMetrics::default()),
        )
        .with_memory_store(mem.clone());
        (router(state), mem)
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

    // ── P.0 — la COMPLÉTUDE du reçu ────────────────────────────────────────────────────────
    // Ces quatre tests couvrent le défaut fermé par P.0 : `verify` recalculait la racine sans
    // jamais la confronter à la racine scellée, si bien qu'une troncature — invisible à
    // l'intégrité, puisqu'un préfixe de chaîne HMAC valide l'est aussi — rendait `verified:true`
    // à un attaquant ne détenant **aucune clé**.

    /// Helper P.0 : exécute un run d'essaim réel (plusieurs records) et rend `(app, store, id)`.
    async fn seeded_run() -> (Router, std::sync::Arc<InMemoryRunStore>, Uuid) {
        let (app, store) = app_with_store();
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
        let id: Uuid = run["run_id"].as_str().unwrap().parse().unwrap();
        (app, store, id)
    }

    async fn get_verify(app: &Router, id: Uuid) -> Value {
        let resp = app
            .clone()
            .oneshot(Request::builder().uri(format!("/v1/swarm/verify/{id}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        body_json(resp).await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn verify_reports_root_match_on_healthy_run() {
        // Nominal : la racine reconstruite reproduit la racine scellée.
        let (app, _store, id) = seeded_run().await;
        let v = get_verify(&app, id).await;
        assert_eq!(v["root_matches"], json!(true), "run intact ⟹ la racine scellée est reproduite");
        assert_eq!(v["verified"], json!(true), "P.0 ne dégrade pas un run sain");
        assert_eq!(v["chain_intact"], json!(true));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn verify_detects_tail_truncation() {
        // **LE test de P.0.** On retire le dernier enregistrement en base, sans aucune clé.
        let (app, store, id) = seeded_run().await;
        let before = get_verify(&app, id).await;
        assert_eq!(before["verified"], json!(true), "prérequis : le run est vérifié avant l'attaque");
        let n_before = before["records"].as_u64().unwrap();
        assert!(n_before >= 2, "le run doit porter assez de records pour être tronqué (obtenu {n_before})");

        assert!(store.truncate_tail(id, 1), "un enregistrement a été retiré");

        let after = get_verify(&app, id).await;
        // (1) Le point crucial : l'intégrité SEULE ne voit RIEN — le préfixe est valide.
        assert_eq!(after["chain_intact"], json!(true), "un préfixe de chaîne HMAC valide reste valide");
        assert_eq!(after["content_intact"], json!(true), "aucun contenu n'a été muté");
        assert_eq!(after["dag_consistent"], json!(true), "le DAG restant est cohérent");
        assert!(after["first_break"].is_null(), "aucune faute localisée : rien n'est *cassé*");
        assert_eq!(after["records"].as_u64().unwrap(), n_before - 1, "il manque bien un record");
        // (2) …et pourtant le run n'est PAS celui qui a été scellé.
        assert_eq!(after["root_matches"], json!(false), "la racine scellée n'est plus reproduite");
        assert_eq!(after["verified"], json!(false), "P.0 : un run tronqué n'est PAS vérifié");
        assert_ne!(after["root"], before["root"], "la racine a changé");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn verify_detects_total_record_deletion() {
        // Cas-limite : tout supprimer. `verify_swarm` déclare un run vide *trivialement* intègre
        // et ne reconstruit AUCUNE racine — un contrôle naïf (comparer seulement deux `Some`)
        // laisserait donc passer la suppression totale.
        let (app, store, id) = seeded_run().await;
        assert!(store.drop_all_records(id), "tous les records ont été supprimés");

        let v = get_verify(&app, id).await;
        assert_eq!(v["records"], json!(0));
        assert_eq!(v["chain_intact"], json!(true), "un run vide est trivialement intègre");
        assert!(v["root"].is_null(), "aucune racine reconstruite");
        assert_eq!(v["root_matches"], json!(false), "une racine scellée non reproduite ⟹ faux");
        assert_eq!(v["verified"], json!(false), "P.0 : la suppression totale n'est PAS vérifiée");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn verify_without_sealed_root_does_not_accuse() {
        // Non-accusation rétroactive : un run scellé AVANT la persistance de la racine ne peut
        // ni être confirmé ni être accusé. Le verdict doit rester celui d'avant P.0.
        let (app, store, id) = seeded_run().await;
        assert!(store.clear_root(id), "la racine scellée a été effacée (run « legacy »)");

        let v = get_verify(&app, id).await;
        assert!(v["root_matches"].is_null(), "sans racine scellée ⟹ None, jamais false");
        assert_eq!(v["verified"], json!(true), "P.0 n'accuse PAS un run honnête antérieur");
        assert!(v["root"].is_string(), "la racine reste reconstruite et exposée");
    }

    // ── Fil P (P.2) — l'export du bundle de provenance ─────────────────────────────────────

    /// État de test avec l'export ACTIVÉ + le store concret conservé. On passe par
    /// `with_bundle_export` et jamais par l'environnement : manipuler une variable d'env dans un
    /// test serait une course avec les autres tests du même processus.
    fn app_with_bundle() -> (Router, Arc<InMemoryRunStore>) {
        let store = Arc::new(InMemoryRunStore::new());
        let state = SwarmGatewayState::new(
            Arc::new(AgentFleet::standard()),
            HmacKeySource::test_key(),
            store.clone(),
            ProvenanceMetrics::new(),
            Arc::new(SubstrateMetrics::default()),
        )
        .with_bundle_export(true);
        (router(state), store)
    }

    async fn seed_run(app: &Router) -> Uuid {
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
        body_json(resp).await["run_id"].as_str().unwrap().parse().unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn bundle_route_is_not_mounted_without_the_opt_in() {
        // L'invariant byte-identique : sans opt-in, l'URI n'EXISTE pas. Pas un 501 — un 404, comme
        // n'importe quelle route inconnue avant P.2. Une surface désactivée ne doit même pas
        // trahir son existence.
        let (app, _store) = app_with_store();
        let id = seed_run(&app).await;
        let resp = app
            .oneshot(Request::builder().uri(format!("/v1/swarm/bundle/{id}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "route non montée ⟹ l'URI n'existe pas");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn bundle_export_serves_a_reprovable_bundle() {
        let (app, _store) = app_with_bundle();
        let id = seed_run(&app).await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/swarm/bundle/{id}"))
                    .header("x-efe-roles", HeaderValue::from_static("auditor"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let b = body_json(resp).await;

        let jsonl = b["bundle_jsonl"].as_str().unwrap();
        let root = b["manifest_root"].as_str().unwrap();
        let n = b["node_count"].as_u64().unwrap();
        assert!(n >= 1, "le bundle porte des nœuds");
        assert_eq!(jsonl.lines().count() as u64, n, "une ligne JSON par nœud");
        assert_eq!(b["anchored"], json!(false), "P.4 non fait : la racine n'est PAS ancrée, et on le dit");
        assert!(b["mapping_version"].as_str().unwrap().starts_with("fil-p/"));

        // Aucune clé ne sort par cette route — ni la clé A, ni la clé B.
        assert!(!jsonl.contains("key"), "le bundle ne transporte aucune clé");

        // …et le bundle se re-prouve réellement, avec la clé de la PILE B dérivée à part.
        let bkey = provenance_bridge::stack_b_key(&HmacKeySource::test_key().derive(
            efe_nano_sdk::TenantId::system(),
        ).unwrap(), "00000000-0000-0000-0000-000000000001");
        let records = provenance_dag::SealedDag::records_from_jsonl(jsonl).unwrap();
        let v = provenance_dag::verify_against_root(&records, &bkey, root);
        assert!(v.ok, "le bundle servi se re-prouve contre sa racine : {:?}", v.reason);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn bundle_export_refuses_a_viewer() {
        // Un export de provenance entière n'est pas une lecture ordinaire.
        let (app, _store) = app_with_bundle();
        let id = seed_run(&app).await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/swarm/bundle/{id}"))
                    .header("x-efe-roles", HeaderValue::from_static("viewer"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "viewer n'exporte pas la provenance");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn bundle_export_refuses_to_whitewash_a_truncated_run() {
        // LE refus du jalon : donner un bundle re-prouvable à un run tronqué offrirait un témoin
        // crédible à un reçu menteur. Aucun bundle, même partiel.
        let (app, store) = app_with_bundle();
        let id = seed_run(&app).await;
        assert!(store.truncate_tail(id, 1), "un enregistrement retiré en base");

        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/swarm/bundle/{id}"))
                    .header("x-efe-roles", HeaderValue::from_static("auditor"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR, "aucun bundle sur un run tronqué");
    }

    // ── Fil P (P.3) — l'attestation de racine servie ───────────────────────────────────────

    /// La graine de test. Sa clé publique correspondante est celle qu'un auditeur épinglerait
    /// hors bande.
    const TEST_SEED: [u8; 32] = [42u8; 32];

    fn app_with_attested_bundle() -> (Router, Arc<InMemoryRunStore>) {
        let store = Arc::new(InMemoryRunStore::new());
        let state = SwarmGatewayState::new(
            Arc::new(AgentFleet::standard()),
            HmacKeySource::test_key(),
            store.clone(),
            ProvenanceMetrics::new(),
            Arc::new(SubstrateMetrics::default()),
        )
        .with_bundle_export(true)
        .with_bundle_signer(BundleSigner::from_seed(TEST_SEED));
        (router(state), store)
    }

    async fn get_bundle(app: &Router, id: Uuid) -> Value {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/swarm/bundle/{id}"))
                    .header("x-efe-roles", HeaderValue::from_static("auditor"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        body_json(resp).await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn without_a_signing_key_the_attestation_field_is_absent_not_empty() {
        // Un champ vide se confondrait avec une signature. Absent veut dire absent.
        let (app, _s) = app_with_bundle();
        let id = seed_run(&app).await;
        let b = get_bundle(&app, id).await;
        assert!(b.get("attestation").is_none(), "champ ABSENT, jamais vide ni nul");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_served_attestation_verifies_with_the_public_key_alone() {
        // LE point de P.3 : l'auditeur ne détient AUCUN secret — seulement la clé publique qu'il a
        // épinglée hors bande.
        let (app, _s) = app_with_attested_bundle();
        let id = seed_run(&app).await;
        let b = get_bundle(&app, id).await;

        let att = b.get("attestation").expect("clé déployée ⟹ attestation servie");
        assert_eq!(att["run_id"].as_str().unwrap(), id.to_string());
        assert_eq!(att["manifest_root"], b["manifest_root"], "atteste LA racine servie");
        assert_eq!(att["node_count"], b["node_count"]);
        assert_eq!(att["sig"].as_str().unwrap().len(), 128, "ed25519 = 64 octets");

        // La route ne sert JAMAIS la clé publique : la servir rendrait la vérification circulaire.
        let signer = BundleSigner::from_seed(TEST_SEED);
        let body = serde_json::to_string(&b).unwrap();
        assert!(!body.contains(&signer.public_key_hex()), "la clé publique ne sort PAS par ici");
        assert_eq!(att["key_id"].as_str().unwrap(), signer.key_id(), "seule l'empreinte la nomme");

        // …et la vérification tient, avec la seule publique épinglée.
        let attestation = provenance_bridge::RootAttestation {
            schema: att["schema"].as_u64().unwrap() as u16,
            run_id: id,
            tenant_id: att["tenant_id"].as_str().unwrap().to_string(),
            manifest_root: att["manifest_root"].as_str().unwrap().to_string(),
            node_count: att["node_count"].as_u64().unwrap() as u32,
            key_id: att["key_id"].as_str().unwrap().to_string(),
            signed_at: att["signed_at"].as_u64().unwrap(),
            sig: att["sig"].as_str().unwrap().to_string(),
        };
        let vk = ed25519_dalek::SigningKey::from_bytes(&TEST_SEED).verifying_key();
        assert!(
            provenance_bridge::verify_attestation(&attestation, &vk).is_ok(),
            "vérifiée sans aucun secret"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_served_attestation_binds_the_served_bundle() {
        // Une signature authentique portant sur AUTRE CHOSE ne vaut rien : on vérifie qu'elle lie
        // bien le bundle rendu dans la même réponse.
        let (app, _s) = app_with_attested_bundle();
        let id = seed_run(&app).await;
        let b = get_bundle(&app, id).await;
        let att = &b["attestation"];

        let bundle = provenance_bridge::Bundle {
            jsonl: b["bundle_jsonl"].as_str().unwrap().to_string(),
            manifest_root: b["manifest_root"].as_str().unwrap().to_string(),
            node_count: b["node_count"].as_u64().unwrap() as u32,
        };
        let attestation = provenance_bridge::RootAttestation {
            schema: att["schema"].as_u64().unwrap() as u16,
            run_id: id,
            tenant_id: att["tenant_id"].as_str().unwrap().to_string(),
            manifest_root: att["manifest_root"].as_str().unwrap().to_string(),
            node_count: att["node_count"].as_u64().unwrap() as u32,
            key_id: att["key_id"].as_str().unwrap().to_string(),
            signed_at: att["signed_at"].as_u64().unwrap(),
            sig: att["sig"].as_str().unwrap().to_string(),
        };
        let vk = ed25519_dalek::SigningKey::from_bytes(&TEST_SEED).verifying_key();
        let tenant = attestation.tenant_id.clone();
        assert!(
            provenance_bridge::verify_attestation_binds(&attestation, &vk, id, &tenant, &bundle)
                .is_ok(),
            "l'attestation lie CE bundle, CE run, CE tenant"
        );
    }

    // ── Fil P (P.4) — l'ancre publiée hors du bundle ───────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn without_an_anchor_sink_the_bundle_says_it_is_not_anchored() {
        // Sans support d'ancrage, le bundle ne prouve sa cohérence que CONTRE LUI-MÊME. Le dire.
        let (app, _s) = app_with_bundle();
        let id = seed_run(&app).await;
        let b = get_bundle(&app, id).await;
        assert_eq!(b["anchored"], json!(false), "aucune ancre ⟹ le bundle l'annonce");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn serving_a_bundle_publishes_its_anchor_outside_the_bundle() {
        // LE geste de P.4 : la racine attendue est écrite AILLEURS, et c'est cet « ailleurs » qui
        // rend une troncature détectable par quelqu'un ne détenant AUCUNE clé.
        let mut path = std::env::temp_dir();
        path.push(format!("efe-p4-gw-{}.log", Uuid::new_v4()));
        let sink = Arc::new(provenance_bridge::FileAnchorSink::new(&path));

        let store = Arc::new(InMemoryRunStore::new());
        let state = SwarmGatewayState::new(
            Arc::new(AgentFleet::standard()),
            HmacKeySource::test_key(),
            store.clone(),
            ProvenanceMetrics::new(),
            Arc::new(SubstrateMetrics::default()),
        )
        .with_bundle_export(true)
        .with_anchor_sink(sink);
        let app = router(state);

        let id = seed_run(&app).await;
        let b = get_bundle(&app, id).await;
        assert_eq!(b["anchored"], json!(true), "l'ancre a RÉELLEMENT été publiée");

        // Le journal existe, hors du bundle, et dit la même chose que lui.
        let journal = std::fs::read_to_string(&path).expect("le journal d'ancres existe");
        let idx = provenance_dag::AnchorIndex::parse(&journal).expect("journal relisible");
        let a = idx.get(&id.to_string()).expect("ce run est ancré");
        assert_eq!(a.manifest_root, b["manifest_root"].as_str().unwrap());
        assert_eq!(u64::from(a.node_count), b["node_count"].as_u64().unwrap());

        // …et un bundle tronqué échoue CONTRE cette ancre, alors qu'il reste intègre seul.
        let bkey = provenance_bridge::stack_b_key(
            &HmacKeySource::test_key().derive(efe_nano_sdk::TenantId::system()).unwrap(),
            "00000000-0000-0000-0000-000000000001",
        );
        let full = provenance_dag::SealedDag::records_from_jsonl(
            b["bundle_jsonl"].as_str().unwrap(),
        )
        .unwrap();
        let short = &full[..full.len() - 1];
        assert!(provenance_dag::reprove_dag(short, &bkey).ok, "tronqué mais intègre — le piège");
        assert!(
            !provenance_dag::verify_against_anchor(short, &bkey, a).ok,
            "l'ancre publiée l'attrape"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn bundle_export_unknown_run_is_404() {
        let (app, _store) = app_with_bundle();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/swarm/bundle/{}", Uuid::new_v4()))
                    .header("x-efe-roles", HeaderValue::from_static("auditor"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
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

    // ════════════════════════════════════════════════════════════════════════════════
    //  ADR-019 (MEM-3) — mémoire inter-runs en HTTP (gardiens passerelle)
    // ════════════════════════════════════════════════════════════════════════════════

    fn post_run_as(tenant: &str, body: Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/v1/swarm/run")
            .header("content-type", "application/json")
            .header("x-efe-tenant", tenant)
            .body(Body::from(body.to_string()))
            .unwrap()
    }
    fn get_as(uri: &str, tenant: &str) -> Request<Body> {
        Request::builder().uri(uri).header("x-efe-tenant", tenant).body(Body::empty()).unwrap()
    }
    fn mem_run_body() -> Value {
        json!({
            "task": { "text": "hello", "expected": "hello" },
            "topology": { "kind": "consensus", "generator": "echo_generator",
                          "judges": ["consistency", "j_non_empty"], "k": 3 },
            "memory": { "recall": true, "learn": true }
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn memory_off_by_default_over_http() {
        // I-MEM8 : sans bloc `memory`, la réponse ne porte aucune interaction mémoire.
        let (app, _s) = app();
        let resp = app
            .oneshot(post_run(json!({
                "task": { "text": "x", "expected": "x" },
                "topology": { "kind": "auto" }
            })))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let v = body_json(resp).await;
        assert!(v["run"]["memory"].is_null(), "mémoire OFF par défaut → run.memory = null");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn memory_run_learns_then_snapshot_and_verify_endpoints() {
        // Bout-en-bout HTTP : un run avec mémoire apprend (I-MEM3) + trace le recall (I-MEM2) ;
        // GET /memory/:family reflète l'apprentissage ; GET …/verify re-prouve le memory_root.
        let (app, _s) = app();
        let resp = app.clone().oneshot(post_run(mem_run_body())).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let v = body_json(resp).await;
        let m = &v["run"]["memory"];
        assert_eq!(m["recalled"], json!(true), "recall effectué");
        assert_eq!(m["learned"], json!(true), "run propre → replié (I-MEM3)");
        let family = m["family"].as_str().expect("famille exposée").to_string();
        assert_eq!(family.len(), 64, "famille = hash hex");

        // GET /memory/:family → l'instantané reflète le repli (version 1, content_hash, K=3).
        let snap = app
            .clone()
            .oneshot(Request::builder().uri(format!("/v1/swarm/memory/{family}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(snap.status(), StatusCode::OK);
        let sv = body_json(snap).await;
        assert_eq!(sv["version"], json!(1), "un repli → version 1");
        assert_eq!(sv["content_hash"].as_str().unwrap().len(), 64, "épingle content_hash");
        assert_eq!(sv["size_histogram"]["3"], json!(1), "K=3 observé une fois");

        // GET /memory/:family/verify → re-preuve du memory_root (I-MEM5).
        let ver = app
            .oneshot(Request::builder().uri(format!("/v1/swarm/memory/{family}/verify")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(ver.status(), StatusCode::OK, "chaîne mémoire intègre → 200");
        let vv = body_json(ver).await;
        assert_eq!(vv["intact"], json!(true));
        assert_eq!(vv["records"], json!(1));
        assert_eq!(vv["memory_root"].as_str().unwrap().len(), 64, "memory_root re-prouvé");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn memory_verify_detects_out_of_band_tampering_over_http() {
        // MEM-3.b / I-MEM5 : après apprentissage, éditer une ligne du journal HORS-BANDE (sans
        // pouvoir recalculer le témoin HMAC, qui exige la clé tenant) fait passer /verify à
        // `intact:false` + LOCALISE le repli. Le jumeau mémoire de `verify_detects_tampering`
        // (chaîne N17). Prouve que l'endpoint compare au TÉMOIN persisté, pas au journal re-scellé.
        use efe_swarm_memory::MemoryStore; // pour appeler `journal()` sur le store concret
        let (app, mem) = app_with_memory_store();

        // ① un run apprend (replie 1 record + écrit le témoin racine).
        let resp = app.clone().oneshot(post_run(mem_run_body())).await.unwrap();
        let v = body_json(resp).await;
        assert_eq!(v["run"]["memory"]["learned"], json!(true), "run propre → replié");
        let family_hex = v["run"]["memory"]["family"].as_str().unwrap().to_string();

        // /verify INTÈGRE avant falsification.
        let ok = app
            .clone()
            .oneshot(Request::builder().uri(format!("/v1/swarm/memory/{family_hex}/verify")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(body_json(ok).await["intact"], json!(true), "intègre avant falsification");

        // ② FALSIFICATION hors-bande : on édite le contenu du repli ; le témoin, lui, reste.
        let tenant = efe_nano_sdk::TenantId::system().to_string();
        let fam = efe_swarm_memory::FamilyKey::from_hex(family_hex.clone());
        let mut journal = mem.journal(&tenant, &fam).await.unwrap();
        assert_eq!(journal.len(), 1, "un repli en journal");
        journal[0].summary.k_used = 999; // édition de ligne (impossible légitimement)
        mem.overwrite_journal_for_test(&tenant, &fam, journal);

        // ③ /verify DÉTECTE + localise (≠ tautologie « toujours intact »).
        let bad = app
            .oneshot(Request::builder().uri(format!("/v1/swarm/memory/{family_hex}/verify")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::OK, "verdict dans le corps, toujours 200");
        let vv = body_json(bad).await;
        assert_eq!(vv["intact"], json!(false), "falsification hors-bande DÉTECTÉE (témoin persisté)");
        assert!(vv["localized_break"].is_string(), "le repli fautif est LOCALISÉ (run_id)");
        assert_eq!(vv["unanchored"], json!(false), "témoin présent → ce n'est PAS un cas non-ancré");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn memory_verify_reports_unanchored_when_witness_missing_not_tampering() {
        // Revue adversariale (LOW) : un journal NON vide SANS témoin (table des témoins tronquée /
        // témoin perdu) → /verify dit `unanchored:true` (perte de donnée), PAS une falsification
        // localisée. On distingue honnêtement la PERTE d'une ATTAQUE.
        use efe_swarm_memory::{family_key, FamilyMode, FoldRecord, MemoryStore, RunSummary as MR};
        let (app, mem) = app_with_memory_store();
        let tenant = efe_nano_sdk::TenantId::system().to_string();
        let task = json!({ "text": "x" });
        let fam = family_key(&task, "consensus", &["g".into()], FamilyMode::Structural);

        // pose un journal DIRECTEMENT (aucun fold → aucun témoin écrit) : l'état « non ancré ».
        let rec = FoldRecord { run_id: uuid::Uuid::from_u128(1), family: fam.clone(), summary: MR::clean(6) };
        mem.overwrite_journal_for_test(&tenant, &fam, vec![rec]);
        assert!(mem.witness(&tenant, &fam).await.unwrap().is_none(), "aucun témoin pour cette famille");

        let resp = app
            .oneshot(Request::builder().uri(format!("/v1/swarm/memory/{}/verify", fam.as_str())).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let v = body_json(resp).await;
        assert_eq!(v["unanchored"], json!(true), "journal non vide sans témoin → NON ANCRÉ");
        assert_eq!(v["intact"], json!(false), "non prouvable → intact:false");
        assert_eq!(v["localized_break"], json!(null), "mais AUCUN repli fautif localisé (≠ attaque)");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn memory_is_tenant_scoped_over_http() {
        // I-MEM6 : le tenant A apprend ; le tenant B ne voit RIEN sur la même famille.
        let (app, _s) = app();
        let resp = app.clone().oneshot(post_run_as(TENANT_A, mem_run_body())).await.unwrap();
        let v = body_json(resp).await;
        let family = v["run"]["memory"]["family"].as_str().unwrap().to_string();

        // A voit son apprentissage…
        let a = app.clone().oneshot(get_as(&format!("/v1/swarm/memory/{family}"), TENANT_A)).await.unwrap();
        assert_eq!(body_json(a).await["version"], json!(1), "A voit sa mémoire");
        // …B ne voit rien (même famille, autre tenant → instantané genèse v0).
        let b = app.oneshot(get_as(&format!("/v1/swarm/memory/{family}"), TENANT_B)).await.unwrap();
        assert_eq!(body_json(b).await["version"], json!(0), "B ne voit PAS la mémoire de A (I-MEM6)");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn memory_self_view_is_consistent_for_system_alias() {
        // RÉGRESSION (revue MEM-3, I-MEM6 self-view) : un appelant via l'alias "system" replie
        // sous la clé CANONIQUE (UUID système) ; l'endpoint canonicalise pareil → le tenant voit
        // SA mémoire. Avant le fix, le GET sous "system" brut lisait une autre clé de map → v0.
        let (app, _s) = app();
        let resp = app.clone().oneshot(post_run_as("system", mem_run_body())).await.unwrap();
        let v = body_json(resp).await;
        let family = v["run"]["memory"]["family"].as_str().unwrap().to_string();
        let snap = app.oneshot(get_as(&format!("/v1/swarm/memory/{family}"), "system")).await.unwrap();
        assert_eq!(snap.status(), StatusCode::OK);
        assert_eq!(body_json(snap).await["version"], json!(1), "l'alias 'system' voit SA mémoire (canonicalisé)");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn memory_bogus_tenant_is_a_clean_400() {
        // Bonus du fix : un tenant non-UUID (et ≠ "system") sur l'endpoint mémoire → 400 honnête
        // (au lieu d'une vue vide silencieuse).
        let (app, _s) = app();
        let resp = app
            .oneshot(get_as("/v1/swarm/memory/deadbeef", "pas-un-uuid"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "tenant bogué → 400");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn metrics_expose_memory_counters() {
        let (app, _s) = app();
        let _ = app.clone().oneshot(post_run(mem_run_body())).await.unwrap();
        let resp = app
            .oneshot(Request::builder().uri("/metrics").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let txt = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(txt.contains("efe_swarm_gateway_memory_recalls_total"), "compteur recalls exposé");
        assert!(txt.contains("efe_swarm_gateway_memory_learns_total"), "compteur learns exposé");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn replay_r1_repins_memory_and_reproduces_warm_started_run() {
        // ADR-019 (MEM-3, extension R1 d'ADR-017) : un run warm-starté par la mémoire se REPRODUIT
        // via R1 quand le caller ré-épingle l'instantané (famille + version). Sans le ré-épinglage,
        // R1 repartirait du k_min par défaut (4) → un multiset de générateurs différent (6 vs 4) →
        // task_mismatch ; AVEC, il lit le MÊME instantané → MÊME warm-start (6) → sorties reproduites.
        use efe_swarm_memory::{family_key, FamilyMode, MemoryUpdate, RunSummary as MR};
        let (app, state) = app();
        let tenant = efe_nano_sdk::TenantId::system().to_string();
        let task = json!({ "text": "x", "expected": "x" });
        let family = family_key(
            &task,
            "consensus",
            &["const_generator".into(), "consistency".into(), "j_non_empty".into()],
            FamilyMode::Structural,
        );
        // seed : 4 runs passés à K=6 → le prior suggère 6.
        for r in 0u128..4 {
            state
                .memory
                .fold(&tenant, MemoryUpdate { run_id: uuid::Uuid::from_u128(0x7000 + r), family: family.clone(), summary: MR::clean(6) })
                .await
                .unwrap();
        }
        let topo = json!({
            "kind": "consensus", "generator": "const_generator",
            "judges": ["consistency", "j_non_empty"],
            "k": { "min": 4, "max": 24, "agreement": 0.9 }
        });
        // run avec recall (warm-start k_min=6), learn:false (ne bouge pas le prior).
        let run_resp = app
            .clone()
            .oneshot(post_run(json!({ "task": task, "topology": topo, "memory": { "recall": true, "learn": false } })))
            .await
            .unwrap();
        let rv = body_json(run_resp).await;
        let run_id = rv["run_id"].as_str().unwrap().to_string();
        assert_eq!(rv["run"]["memory"]["warm_start_k_min"], json!(6), "warm-start appliqué (K=6)");
        let version = rv["run"]["memory"]["version"].as_u64().unwrap();
        assert_eq!(version, 4, "instantané = 4 replis seedés");

        // R1 AVEC ré-épinglage (famille + version) → MÊME instantané → reproduit.
        let r1 = app
            .oneshot(replay_post(
                &run_id,
                json!({
                    "task": task, "topology": topo,
                    "memory": { "recall": true, "learn": false, "family": family.as_str(), "pin_version": version }
                }),
            ))
            .await
            .unwrap();
        assert_eq!(r1.status(), StatusCode::OK);
        let v = body_json(r1).await;
        assert_eq!(v["execution"]["ran"], json!(true), "R1 a re-exécuté");
        assert_eq!(v["execution"]["task_mismatch"], json!(false), "même multiset de générateurs (re-pin)");
        assert_eq!(
            v["execution"]["outputs_reproduced"],
            json!(true),
            "R1 ré-épingle l'instantané → reproduit le run warm-starté (I-MEM7)"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn replay_r1_without_repinning_memory_does_not_falsely_reproduce() {
        // I-MEM7 (moitié HONNÊTETÉ) : un run warm-starté par la mémoire (k=6) NE se reproduit PAS
        // si R1 ne RÉ-ÉPINGLE pas l'instantané — sans bloc mémoire, R1 repart du k_min par défaut
        // (4) → multiset de générateurs DIFFÉRENT → task_mismatch, donc PAS de reproduction. La
        // reproductibilité EXIGE le ré-épinglage (≠ la re-provabilité R0, toujours vraie). Le
        // pendant NÉGATIF de replay_r1_repins_memory_and_reproduces_warm_started_run.
        use efe_swarm_memory::{family_key, FamilyMode, MemoryUpdate, RunSummary as MR};
        let (app, state) = app();
        let tenant = efe_nano_sdk::TenantId::system().to_string();
        let task = json!({ "text": "x", "expected": "x" });
        let family = family_key(
            &task,
            "consensus",
            &["const_generator".into(), "consistency".into(), "j_non_empty".into()],
            FamilyMode::Structural,
        );
        for r in 0u128..4 {
            state
                .memory
                .fold(&tenant, MemoryUpdate { run_id: uuid::Uuid::from_u128(0x8000 + r), family: family.clone(), summary: MR::clean(6) })
                .await
                .unwrap();
        }
        let topo = json!({
            "kind": "consensus", "generator": "const_generator",
            "judges": ["consistency", "j_non_empty"],
            "k": { "min": 4, "max": 24, "agreement": 0.9 }
        });
        let run_resp = app
            .clone()
            .oneshot(post_run(json!({ "task": task, "topology": topo, "memory": { "recall": true, "learn": false } })))
            .await
            .unwrap();
        let rv = body_json(run_resp).await;
        let run_id = rv["run_id"].as_str().unwrap().to_string();
        assert_eq!(rv["run"]["memory"]["warm_start_k_min"], json!(6), "le run original est warm-starté (K=6)");

        // R1 SANS bloc mémoire → aucun recall → k_min par défaut (4) → multiset ≠ → task_mismatch.
        let r1 = app.oneshot(replay_post(&run_id, json!({ "task": task, "topology": topo }))).await.unwrap();
        assert_eq!(r1.status(), StatusCode::OK);
        let v = body_json(r1).await;
        assert_eq!(v["execution"]["ran"], json!(true), "R1 a re-exécuté");
        assert_eq!(
            v["execution"]["task_mismatch"],
            json!(true),
            "sans ré-épinglage → multiset de générateurs DIFFÉRENT (k=4 vs 6) — I-MEM7"
        );
        assert_ne!(
            v["execution"]["outputs_reproduced"],
            json!(true),
            "donc PAS de reproduction sans le pin (l'honnêteté de R1)"
        );
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

    /// **Couche 2 (M3, DoD #8)** — défaut **byte-identique** : sans la feature `frontier-llm`,
    /// AUCUNE route `/v1/llm/*` n'est montée (le `.nest` est sous `#[cfg]`). Un POST y répond
    /// 404 — la propriété structurelle (gating cfg) devient un gardien exécutable. Ce test ne
    /// compile QUE dans le build par défaut (retiré dès que la frontière est servie).
    #[cfg(not(feature = "frontier-llm"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn no_llm_routes_when_frontier_off() {
        let (app, _s) = app();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/llm/vote")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"user_text":"x","task_class":"FACTUAL_QA","llms":["a","b","c"]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "défaut : /v1/llm/* non monté ⟹ 404");
    }

    /// **M4 (§5, #6)** — non-régression essaim quand `frontier-llm` est COMPILÉE : monter
    /// `/v1/llm/*` ne perturbe pas le routeur essaim. Complète `no_llm_routes_when_frontier_off`
    /// (côté « flag off »). 100 % côté harnais — aucune touche à `verify.rs`.
    #[cfg(feature = "frontier-llm")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn swarm_run_unchanged_when_frontier_compiled() {
        let (app, _s) = app();
        let resp = app
            .clone()
            .oneshot(post_run(json!({
                "task": { "text": "x", "expected": "x" },
                "topology": { "kind": "consensus", "generator": "echo_generator", "judges": ["consistency"], "k": 3 },
                "budget": { "max_cents": 100 }
            })))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "l'essaim reste servi sous frontier-llm");
        let run = body_json(resp).await;
        assert_eq!(run["run"]["provenance"]["verified"], json!(true));
        let run_id = run["run_id"].as_str().unwrap().to_string();
        let resp = app
            .oneshot(Request::builder().uri(format!("/v1/swarm/verify/{run_id}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_json(resp).await["verified"], json!(true), "re-preuve essaim intacte");
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
