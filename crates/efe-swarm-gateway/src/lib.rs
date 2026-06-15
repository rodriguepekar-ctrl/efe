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
pub use state::SwarmGatewayState;
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

    // ── ADR-016 : un run Mode C SYNTHÉTIQUE (sans Ollama) pour éprouver la persistance
    //    forensique + la re-preuve N17 de bout en bout en HTTP. ─────────────────────
    #[cfg(test)]
    fn synthetic_mode_c_run(model: &str) -> (Vec<efe_swarm_provenance::AgentCallRecord>, StoredRun) {
        use efe_nano_sdk::{
            Blake3Hash, Completeness, CostAccounting, FinishReason, LlmProvenanceRecord,
            ModelIdentity, NanoAgentId, NanoAgentVersion, PiiStrategy, ProvenanceId, RetryOutcome,
            SamplingParams, SwarmId, TenantId, TokenUsage,
        };
        use efe_swarm_provenance::{seal_n17, AgentCallDraft, AgentKind, AgentRole, SwarmChain};

        let tenant = TenantId::system();
        let swarm = SwarmId::from_uuid(Uuid::from_u128(0x5EED));
        let key = HmacKeySource::test_key().derive(tenant).unwrap();
        let prov = ProvenanceId::from_uuid(Uuid::from_u128(0xC0FFEE));

        // la forensique N17 d'un appel LLM.
        let llm = LlmProvenanceRecord {
            provenance_id: prov,
            trace_id: Uuid::from_u128(0xBEEF),
            swarm_request_id: swarm.as_uuid(),
            op_id: "swarm.generator".into(),
            tenant_id: tenant,
            agent_id: NanoAgentId::new("generator").unwrap(),
            agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            model_identity: ModelIdentity::hosted("ollama", model, "local"),
            model_hash_verified: false,
            prompt_template_id: "swarm.selfhost".into(),
            prompt_template_version: "1.0.0".into(),
            prompt_inputs_hash: Blake3Hash::ZERO,
            system_prompt_hash: Blake3Hash::ZERO,
            user_prompt_hash: Blake3Hash::from_bytes([3u8; 32]),
            final_prompt_hash: Blake3Hash::from_bytes([3u8; 32]),
            sampling: SamplingParams { temperature: 0.2, top_p: None, max_tokens: 16, seed: Some(7), stop_sequences: vec![] },
            token_usage: TokenUsage { client_estimated_input: 5, client_estimated_output: 2, provider_reported_input: None, provider_reported_output: None, is_estimated: true },
            cost_accounting: CostAccounting { cost_cents: 0.0, prices_table_version: 0, provider_quoted_cost_cents: None, overflow_capped: false },
            provider_request_id: None,
            provider_response_time_ms: 5,
            provider_finish_reason: FinishReason::Stop,
            provider_region: None,
            publisher_id: None,
            publisher_kyc_tier: None,
            gold_case_id: None,
            provider_metadata: std::collections::HashMap::new(),
            output_hash: Some(Blake3Hash::from_bytes([5u8; 32])),
            completeness: Completeness::Complete,
            safety_flags: vec![],
            tool_calls: vec![],
            pii_strategy_applied: PiiStrategy::None,
            redaction_token_set: vec![],
            replay_of: None,
            cache_hit: None,
            recorded_at: std::time::SystemTime::UNIX_EPOCH,
            streaming_mode: false,
            retry_count: 0,
            retry_outcome: RetryOutcome::NoRetry,
            time_to_first_token_ms: None,
        };

        // l'AgentCallRecord Mode C qui RÉFÉRENCE la forensique (le lien), scellé en chaîne.
        let draft = AgentCallDraft::new(
            swarm, tenant, efe_nano_sdk::AgentId::new("generator").unwrap(), AgentRole::Generator,
            AgentKind::ModeC, Blake3Hash::from_bytes([1u8; 32]), Blake3Hash::from_bytes([2u8; 32]),
        )
        .with_llm_provenance(prov);
        let mut chain = SwarmChain::new(tenant, swarm, key.clone());
        let records = chain.append_all(std::iter::once(draft));
        let root_hex = chain.root_hex();

        let sealed = seal_n17(std::slice::from_ref(&llm), &key);
        let stored = StoredRun {
            // Le tenant par défaut d'un appelant sans en-tête = `TenantId::system()` rendu en
            // UUID (00…001) ; on s'y aligne pour que `store.get` retrouve le run injecté, et
            // pour que `derive_key` recompose la MÊME clé (from_str(uuid) == system).
            tenant: tenant.to_string(),
            records: records.clone(),
            root: root_hex,
            llm_records: vec![llm],
            n17_leaves: sealed.leaves_hex(),
            n17_root: sealed.root_hex(),
            output: json!({ "value": "ok" }),
            topology: "T-consensus".into(),
            created_at: std::time::SystemTime::now(),
        };
        (records, stored)
    }

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
}
