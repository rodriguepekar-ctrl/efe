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
    Coordinator, CoordinatorClients, MetricsRegistry, MockLlmProvider, ProviderRegistry,
    ALL_EVENT_TAGS, EVENT_SCHEMA_VERSION,
};
use efe_nano_app_coordinator::clients::{
    AnalyzerClient, CompressorClient, NanoRobotClient, RobotHealth, RouterClient, VerifierClient,
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

/// Registry des **3 providers mock** déterministes (premium / cheap / mid).
///
/// **P5.1 — source unique** de la couche LLM mock : partagée par
/// [`mock_coordinator`] (tests + `mock_state`) ET par le binaire
/// (`build_llm_layer` → `mock_llm_layer`). Aligné sur [`MOCK_LLMS`].
#[must_use]
pub fn mock_registry() -> ProviderRegistry {
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
    reg
}

/// Coordinator mock : 3 providers déterministes ([`mock_registry`]) +
/// télémétrie (R3.S7) câblée pour alimenter `/metrics`. Permet Mission A
/// (routing), Mission B (vote ≥ 2 LLMs) et Speculative (drafter+verifier).
#[must_use]
pub fn mock_coordinator() -> Arc<Coordinator> {
    Arc::new(
        Coordinator::builder()
            .registry(mock_registry())
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

// ─────────────────────────────────────────────────────────────────────────
// P5.2 — Couche SLM : health pre-flight + from_assets per-robot
// ─────────────────────────────────────────────────────────────────────────

/// Endpoint analyzer par défaut (aligné sur `scripts/slm/`).
pub const DEFAULT_SLM_ANALYZER_URL: &str = "http://127.0.0.1:8090";
/// Endpoint compressor par défaut.
pub const DEFAULT_SLM_COMPRESSOR_URL: &str = "http://127.0.0.1:8092";
/// Endpoint verifier par défaut.
pub const DEFAULT_SLM_VERIFIER_URL: &str = "http://127.0.0.1:8093";

/// Configuration de la couche SLM, lue de l'environnement.
#[derive(Debug, Clone)]
pub struct SlmConfig {
    /// `EFE_LIVE_SLM == "1"` → câble les robots. Sinon couche vide (invariance P5.1).
    pub enabled: bool,
    /// Endpoint analyzer (`EFE_SLM_ANALYZER_URL`, défaut `:8090`).
    pub analyzer_url: String,
    /// Endpoint compressor (`EFE_SLM_COMPRESSOR_URL`, défaut `:8092`).
    pub compressor_url: String,
    /// Endpoint verifier (`EFE_SLM_VERIFIER_URL`, défaut `:8093`).
    pub verifier_url: String,
}

impl SlmConfig {
    /// Lit la config depuis l'environnement (avec défauts).
    #[must_use]
    pub fn from_env() -> Self {
        let url = |var: &str, default: &str| {
            std::env::var(var).unwrap_or_else(|_| default.to_string())
        };
        Self {
            enabled: std::env::var("EFE_LIVE_SLM").as_deref() == Ok("1"),
            analyzer_url: url("EFE_SLM_ANALYZER_URL", DEFAULT_SLM_ANALYZER_URL),
            compressor_url: url("EFE_SLM_COMPRESSOR_URL", DEFAULT_SLM_COMPRESSOR_URL),
            verifier_url: url("EFE_SLM_VERIFIER_URL", DEFAULT_SLM_VERIFIER_URL),
        }
    }
}

/// **P5.2** — Construit la couche SLM : pré-flight `health()` per-robot +
/// `from_assets`.
///
/// Tolérant : un robot injoignable (`Down`/`Degraded`) ou aux assets manquants
/// est laissé `None` (mission_a/b le sautent). Désactivé (`EFE_LIVE_SLM` ≠ 1) →
/// `CoordinatorClients::default()` (invariance P5.1 stricte). Les 3 health-checks
/// sont faits **en parallèle** (timeout 2s intégré → ~2s au pire, pas 6s).
pub async fn build_slm_clients(cfg: &SlmConfig) -> CoordinatorClients {
    let mut clients = CoordinatorClients::default();
    if !cfg.enabled {
        return clients;
    }

    // (a) Construction — `from_assets` est SYNC (lit grammaire + prompt depuis
    //     `assets/`) ; `None` si un asset manque.
    let analyzer = AnalyzerClient::from_assets(cfg.analyzer_url.as_str()).ok();
    let compressor = CompressorClient::from_assets(cfg.compressor_url.as_str()).ok();
    let verifier = VerifierClient::from_assets(cfg.verifier_url.as_str()).ok();

    // (b) Health en PARALLÈLE — `Up` (200) = on câble.
    let (up_a, up_c, up_v) = tokio::join!(
        is_up(analyzer.as_ref()),
        is_up(compressor.as_ref()),
        is_up(verifier.as_ref()),
    );

    log_robot("analyzer", &cfg.analyzer_url, up_a, analyzer.is_some());
    log_robot("compressor", &cfg.compressor_url, up_c, compressor.is_some());
    log_robot("verifier", &cfg.verifier_url, up_v, verifier.is_some());

    if up_a {
        clients.analyzer = analyzer.map(Arc::new);
    }
    if up_c {
        clients.compressor = compressor.map(Arc::new);
    }
    if up_v {
        clients.verifier = verifier.map(Arc::new);
    }

    // (c) Router : heuristique Pareto in-process (pas de serveur, pas de
    //     /health) → toujours câblé quand le SLM est actif.
    clients.router = Some(Arc::new(RouterClient::new()));
    eprintln!("[preview][slm] router     ✓ (heuristique in-process)");

    clients
}

/// `true` ssi le client existe ET répond `Up` (200) à `/health`. `health()` ne
/// renvoie **jamais** `Err` (timeout/transport → `Ok(Down)`), d'où le `matches!`.
async fn is_up<C: NanoRobotClient>(client: Option<&C>) -> bool {
    match client {
        Some(c) => matches!(c.health().await, Ok(RobotHealth::Up)),
        None => false,
    }
}

/// `true` si **au moins un** robot SLM (analyzer/compressor/verifier) est câblé.
/// **P5.3** s'en sert pour décider d'ajouter `.with_default_slm_circuits()`.
#[must_use]
pub fn has_robots(clients: &CoordinatorClients) -> bool {
    clients.analyzer.is_some() || clients.compressor.is_some() || clients.verifier.is_some()
}

/// Log per-robot du résultat du pré-flight (P5.4 fera la bannière consolidée).
fn log_robot(name: &str, url: &str, up: bool, built: bool) {
    if up {
        eprintln!("[preview][slm] {name:<10} ✓ {url}");
    } else if built {
        eprintln!("[preview][slm] {name:<10} ✗ {url} (down/degraded → sauté)");
    } else {
        eprintln!("[preview][slm] {name:<10} ✗ {url} (assets manquants → sauté)");
    }
}

#[cfg(test)]
mod slm_tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// MockServer répondant `status` sur `GET /health`.
    async fn health_server(status: u16) -> MockServer {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(status))
            .mount(&s)
            .await;
        s
    }

    fn cfg(enabled: bool, a: String, c: String, v: String) -> SlmConfig {
        SlmConfig {
            enabled,
            analyzer_url: a,
            compressor_url: c,
            verifier_url: v,
        }
    }

    #[tokio::test]
    async fn disabled_returns_default_no_robots_no_router() {
        let clients = build_slm_clients(&cfg(
            false,
            "http://x".into(),
            "http://x".into(),
            "http://x".into(),
        ))
        .await;
        assert!(!has_robots(&clients), "désactivé → aucun robot");
        assert!(clients.router.is_none(), "désactivé → router non câblé");
    }

    #[tokio::test]
    async fn all_up_wires_all_robots_and_router() {
        let (a, c, v) = (
            health_server(200).await,
            health_server(200).await,
            health_server(200).await,
        );
        let clients = build_slm_clients(&cfg(true, a.uri(), c.uri(), v.uri())).await;
        assert!(clients.analyzer.is_some(), "analyzer câblé");
        assert!(clients.compressor.is_some(), "compressor câblé");
        assert!(clients.verifier.is_some(), "verifier câblé");
        assert!(clients.router.is_some(), "router câblé");
        assert!(has_robots(&clients));
    }

    #[tokio::test]
    async fn down_robot_left_none_others_wired() {
        let (a, v) = (health_server(200).await, health_server(200).await);
        // compressor : port mort → connection refused → Down (rapide).
        let clients = build_slm_clients(&cfg(
            true,
            a.uri(),
            "http://127.0.0.1:9".into(),
            v.uri(),
        ))
        .await;
        assert!(clients.analyzer.is_some(), "analyzer up → câblé");
        assert!(clients.compressor.is_none(), "compressor down → None");
        assert!(clients.verifier.is_some(), "verifier up → câblé");
        // Router toujours câblé tant que le SLM est actif.
        assert!(clients.router.is_some());
    }

    #[tokio::test]
    async fn degraded_non_200_is_skipped() {
        let a = health_server(503).await; // non-200 → Degraded → sauté
        let clients = build_slm_clients(&cfg(
            true,
            a.uri(),
            "http://127.0.0.1:9".into(),
            "http://127.0.0.1:9".into(),
        ))
        .await;
        assert!(clients.analyzer.is_none(), "503 (Degraded) → non câblé");
    }

    #[test]
    fn config_from_env_defaults_when_absent() {
        std::env::remove_var("EFE_LIVE_SLM");
        std::env::remove_var("EFE_SLM_ANALYZER_URL");
        let c = SlmConfig::from_env();
        assert!(!c.enabled, "EFE_LIVE_SLM absent → désactivé");
        assert!(c.analyzer_url.contains("8090"));
        assert!(c.compressor_url.contains("8092"));
        assert!(c.verifier_url.contains("8093"));
    }

    #[tokio::test]
    async fn all_robots_down_keeps_router_but_has_no_robots() {
        // Scénario du smoke + cas CRITIQUE pour P5.3 : tous les robots
        // injoignables → `None`, le router reste câblé, mais `has_robots()` est
        // FALSE (le router n'est PAS un robot). P5.3 ne doit donc PAS ajouter de
        // circuits SLM à un coordinator sans robot.
        let dead = "http://127.0.0.1:9".to_string();
        let clients = build_slm_clients(&cfg(true, dead.clone(), dead.clone(), dead)).await;
        assert!(clients.analyzer.is_none());
        assert!(clients.compressor.is_none());
        assert!(clients.verifier.is_none());
        assert!(clients.router.is_some(), "router toujours câblé si SLM actif");
        assert!(
            !has_robots(&clients),
            "router seul ≠ robots → has_robots() doit être false (contrat P5.3)"
        );
    }

    #[test]
    fn has_robots_counts_robots_not_router() {
        // Vide → false.
        assert!(!has_robots(&CoordinatorClients::default()), "vide → false");

        // Seulement le router → false (le router n'est pas compté comme robot).
        let only_router = CoordinatorClients {
            router: Some(Arc::new(RouterClient::new())),
            ..Default::default()
        };
        assert!(
            !has_robots(&only_router),
            "router seul → false (router ≠ robot SLM)"
        );

        // Un robot réel (analyzer) → true. `from_assets` réussit hors-ligne (il
        // lit les assets présents ; l'URL n'est qu'un endpoint stocké).
        let with_analyzer = CoordinatorClients {
            analyzer: AnalyzerClient::from_assets("http://unused").ok().map(Arc::new),
            ..Default::default()
        };
        assert!(
            with_analyzer.analyzer.is_some(),
            "from_assets doit réussir (assets analyzer présents)"
        );
        assert!(has_robots(&with_analyzer), "analyzer câblé → true");
    }
}
