// SPDX-License-Identifier: UNLICENSED
//! **Phase 4 · P4.S2 — `efe-preview-server`** : bridge HTTP/SSE devant
//! `efe-coordinator`.
//!
//! Le moteur de pré-mâchage publie tout son cycle de vie sur un bus
//! d'événements ([`efe_coordinator::StreamEvent`], protocole SSE v1.0.0). Ce
//! crate expose ce flux à un frontend (Preview Lab) :
//!
//! - `POST /v1/missions/a` → lance une Mission A et **streame** ses
//!   `StreamEvent` en `text/event-stream` (1 frame `data: <json>\n\n` / event).
//! - `GET /v1/schema` → `{ event_schema_version, variants[] }` (garde-fou de
//!   non-dérive côté UI).
//! - `GET /healthz` → liveness.
//!
//! ## Abonnement sans course
//!
//! Le handler appelle [`efe_coordinator::Coordinator::run_mission_a_streamed`]
//! (P4.S1) : abonnement au bus AVANT le spawn de la mission → `SwarmStarted`
//! toujours capté. Le [`efe_coordinator::MissionStream`] est **possédé** par le
//! stream SSE : sa garde RAII `unsubscribe` à la fermeture (déconnexion client
//! ou fin de mission), sans fuite de subscriber.
//!
//! ## Sécurité (outil dev)
//!
//! Bind localhost par défaut. Les clés LLM restent **côté serveur** (le front ne
//! voit que des `StreamEvent`). Bearer optionnel (`EFE_PREVIEW_TOKEN`).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderValue, Method};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::routing::{get, post};
use axum::Router;
use serde::Serialize;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

use efe_coordinator::{
    Coordinator, MetricsRegistry, MockLlmProvider, ProviderRegistry, ALL_EVENT_TAGS,
    EVENT_SCHEMA_VERSION,
};

pub mod ab;
pub mod dto;
pub mod handlers;
pub mod resilience;

/// Adresse d'écoute par défaut (localhost — pas d'exposition réseau directe).
pub const DEFAULT_ADDR: &str = "127.0.0.1:8700";
/// Origine CORS par défaut (le dev-server Next.js).
pub const DEFAULT_CORS_ORIGIN: &str = "http://localhost:3000";
/// Chemin par défaut du livrable savings v2 (relatif au CWD = racine workspace).
pub const DEFAULT_SAVINGS_REPORT_PATH: &str =
    "crates/efe-coordinator/CHEWING_SAVINGS_REPORT_v2.json";
/// LLMs mock enregistrés par [`mock_coordinator`] — **source de vérité** des
/// `default_llms` en mode mock (le binaire et `for_mock` doivent s'aligner
/// dessus, sinon le router sélectionne un provider non enregistré → mission KO).
pub const MOCK_LLMS: &[&str] = &["mock-claude", "mock-gemini", "mock-gpt"];

// ─────────────────────────────────────────────────────────────────────────
// Configuration
// ─────────────────────────────────────────────────────────────────────────

/// Configuration runtime du Preview Lab.
#[derive(Debug, Clone)]
pub struct PreviewConfig {
    /// Adresse de bind (défaut `127.0.0.1:8700`).
    pub addr: SocketAddr,
    /// Origine CORS autorisée (défaut `http://localhost:3000`).
    pub cors_origin: String,
    /// Bearer token requis si `Some` (défaut `None` = pas d'auth, localhost).
    pub token: Option<String>,
    /// LLMs par défaut si la requête n'en précise pas.
    pub default_llms: Vec<String>,
    /// Chemin du livrable `CHEWING_SAVINGS_REPORT_v2.json` servi par
    /// `GET /v1/savings-report`.
    pub savings_report_path: PathBuf,
    /// **P4.S6** — Si `Some` et que le dossier existe, le frontend statique
    /// (export Next.js) est servi en fallback (`/`, `/savings/`, `/_next/*`).
    /// `None` ⇒ bridge API-only.
    pub static_dir: Option<PathBuf>,
}

impl PreviewConfig {
    /// Configuration mock (déterministe, offline) pour tests + démo gratuite.
    #[must_use]
    pub fn for_mock() -> Self {
        Self {
            addr: DEFAULT_ADDR.parse().expect("addr par défaut valide"),
            cors_origin: DEFAULT_CORS_ORIGIN.to_string(),
            token: None,
            default_llms: MOCK_LLMS.iter().map(|s| (*s).to_string()).collect(),
            savings_report_path: PathBuf::from(DEFAULT_SAVINGS_REPORT_PATH),
            static_dir: None,
        }
    }

    /// Configuration depuis l'environnement.
    ///
    /// - `EFE_PREVIEW_ADDR` (défaut `127.0.0.1:8700`)
    /// - `EFE_PREVIEW_CORS_ORIGIN` (défaut `http://localhost:3000`)
    /// - `EFE_PREVIEW_TOKEN` (optionnel ; si présent → bearer exigé)
    /// - `EFE_PREVIEW_LLMS` (CSV ; défaut selon mode)
    #[must_use]
    pub fn from_env(default_llms: Vec<String>) -> Self {
        let addr = std::env::var("EFE_PREVIEW_ADDR")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| DEFAULT_ADDR.parse().expect("addr par défaut valide"));
        let cors_origin =
            std::env::var("EFE_PREVIEW_CORS_ORIGIN").unwrap_or_else(|_| DEFAULT_CORS_ORIGIN.to_string());
        let token = std::env::var("EFE_PREVIEW_TOKEN").ok().filter(|s| !s.is_empty());
        let llms = std::env::var("EFE_PREVIEW_LLMS")
            .ok()
            .map(|csv| {
                csv.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty())
            .unwrap_or(default_llms);
        let savings_report_path = std::env::var("EFE_PREVIEW_SAVINGS_REPORT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_SAVINGS_REPORT_PATH));
        let static_dir = std::env::var("EFE_PREVIEW_STATIC_DIR")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        Self {
            addr,
            cors_origin,
            token,
            default_llms: llms,
            savings_report_path,
            static_dir,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// État applicatif
// ─────────────────────────────────────────────────────────────────────────

/// État partagé injecté dans chaque handler (cheap-clone : `Arc`).
#[derive(Clone)]
pub struct AppState {
    /// Le coordinator (moteur de pré-mâchage).
    pub coord: Arc<Coordinator>,
    /// La configuration.
    pub config: Arc<PreviewConfig>,
}

// ─────────────────────────────────────────────────────────────────────────
// Schéma (garde-fou non-dérive)
// ─────────────────────────────────────────────────────────────────────────

/// Réponse de `GET /v1/schema`.
#[derive(Debug, Clone, Serialize)]
pub struct SchemaResponse {
    /// Version SemVer du protocole `StreamEvent`.
    pub event_schema_version: &'static str,
    /// Tags canoniques des variants (source : `efe_coordinator::ALL_EVENT_TAGS`).
    pub variants: Vec<&'static str>,
}

impl SchemaResponse {
    /// Construit la réponse depuis les constantes du coordinator (source de vérité).
    #[must_use]
    pub fn current() -> Self {
        Self {
            event_schema_version: EVENT_SCHEMA_VERSION,
            variants: ALL_EVENT_TAGS.to_vec(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Construction du Router (testable via `tower::ServiceExt::oneshot`)
// ─────────────────────────────────────────────────────────────────────────

/// Construit l'application axum complète.
///
/// Routes protégées (`/v1/*`) derrière le middleware bearer ; `/healthz`
/// ouvert (liveness probe). CORS depuis `state.config.cors_origin`.
pub fn build_app(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(
            state
                .config
                .cors_origin
                .parse::<HeaderValue>()
                .unwrap_or_else(|_| HeaderValue::from_static(DEFAULT_CORS_ORIGIN)),
        )
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([CONTENT_TYPE, AUTHORIZATION]);

    // Routes protégées par le bearer (si configuré).
    let protected = Router::new()
        .route("/v1/missions/a", post(handlers::post_mission_a))
        .route("/v1/missions/b", post(handlers::post_mission_b))
        .route("/v1/missions/speculative", post(handlers::post_mission_speculative))
        .route("/v1/ab/run", post(ab::ab_run))
        .route("/v1/resilience/run", post(resilience::resilience_run))
        .route("/v1/savings-report", get(handlers::savings_report))
        .route("/v1/schema", get(handlers::schema))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            handlers::auth_mw,
        ));

    // Routes ouvertes (liveness + métriques — conventions Prometheus/probes).
    let mut app = Router::new()
        .route("/healthz", get(handlers::healthz))
        .route("/metrics", get(handlers::metrics))
        .merge(protected);

    // P4.S6 — sert le frontend statique (export Next.js) en fallback si présent.
    // Les routes API ci-dessus priment ; le reste (`/`, `/savings/`, `/_next/*`)
    // tombe sur ServeDir.
    if let Some(dir) = &state.config.static_dir {
        if dir.is_dir() {
            app = app.fallback_service(
                ServeDir::new(dir).append_index_html_on_directories(true),
            );
        }
    }

    app.layer(cors).with_state(state)
}

// ─────────────────────────────────────────────────────────────────────────
// Helpers mock (tests + démo gratuite)
// ─────────────────────────────────────────────────────────────────────────

/// Coordinator mock : 3 providers déterministes (premium / cheap / mid) +
/// télémétrie (R3.S7) câblée pour alimenter `/metrics`. Permet Mission A
/// (routing), Mission B (vote ≥ 2 LLMs) et Speculative (drafter+verifier).
#[must_use]
pub fn mock_coordinator() -> Arc<Coordinator> {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("mock-claude")
            .with_response("Paris est la capitale de la France.")
            .with_cost_microcents(3_000),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("mock-gemini")
            .with_response("Paris.")
            .with_cost_microcents(37),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("mock-gpt")
            .with_response("La capitale est Paris.")
            .with_cost_microcents(1_500),
    ));
    Arc::new(
        Coordinator::builder()
            .registry(reg)
            .metrics(Arc::new(MetricsRegistry::new()))
            .build()
            .expect("mock coordinator buildable"),
    )
}

/// État mock prêt à l'emploi (pour `build_app` en test).
#[must_use]
pub fn mock_state() -> AppState {
    AppState {
        coord: mock_coordinator(),
        config: Arc::new(PreviewConfig::for_mock()),
    }
}
