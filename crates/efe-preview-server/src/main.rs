// SPDX-License-Identifier: UNLICENSED
//! Binaire `efe-preview-server` — sert le bridge SSE du Preview Lab.
//!
//! Modes :
//! - `EFE_PREVIEW_MOCK=1` → coordinator mock (réponses déterministes, gratuit).
//! - sinon + feature `real-llm` → providers réels depuis l'env (Claude/Gemini/GPT).
//! - sinon → fallback mock (avec avertissement).

use std::sync::Arc;

use efe_coordinator::{Coordinator, CoordinatorClients, MetricsRegistry, ProviderRegistry};
use efe_preview_server::{
    build_app, build_slm_clients, has_robots, mock_registry, AppState, PreviewConfig, SlmConfig,
    MOCK_LLMS,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    let mock = std::env::var("EFE_PREVIEW_MOCK").as_deref() == Ok("1");
    let (coord, default_llms) = build_coordinator(mock).await;
    let config = PreviewConfig::from_env(default_llms);
    let addr = config.addr;

    let state = AppState {
        coord,
        config: Arc::new(config),
    };
    let app = build_app(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "EFE Preview Lab — bridge SSE up");
    eprintln!(
        "EFE Preview Lab → http://{addr}  (POST /v1/missions/a · GET /v1/schema · GET /healthz)"
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────
// P5.1 — Couches orthogonales LLM / SLM + assemblage async
// ─────────────────────────────────────────────────────────────────────────
//
// `build_coordinator` ne construit plus le `Coordinator` en bloc : il compose
// deux décisions INDÉPENDANTES — la couche LLM (quel « gros modèle ») et la
// couche SLM (quels nano-robots) — en un assemblage UNIQUE. La couche SLM est
// vide en P5.1 (seam pour P5.2) ; l'async est introduit dès maintenant pour que
// P5.2 (health-check HTTP) slotte sans re-changer la signature.

/// **Couche LLM** : tout ce qui définit le gros modèle servi par le bridge —
/// registry de providers, LLMs par défaut, et télémétrie. Regroupé pour un
/// assemblage unique, orthogonal à la couche SLM.
struct LlmLayer {
    registry: ProviderRegistry,
    default_llms: Vec<String>,
    metrics: Arc<MetricsRegistry>,
}

/// **Orchestrateur P5.1.** Compose la couche LLM (registry/ids/métriques) et la
/// couche SLM (clients), en async, en déléguant l'assemblage à
/// [`assemble_coordinator`] (P5.3).
async fn build_coordinator(mock: bool) -> (Arc<Coordinator>, Vec<String>) {
    // On `await` la couche SLM EN PREMIER : la `registry` (potentiellement
    // non-`Send`) n'est ainsi jamais tenue à travers un point de suspension →
    // le futur reste `Send` sur un runtime multi-thread.
    let clients = build_slm_layer().await;
    let llm = build_llm_layer(mock);
    let coord = assemble_coordinator(llm.registry, clients, llm.metrics);

    // SEAM P5.4 : bannière de démarrage (mode LLM × SLM actif × robots vivants).
    (coord, llm.default_llms)
}

/// **P5.3 — Assemblage du `Coordinator`.** **Sync, sans I/O** → unit-testable.
///
/// Ajoute les **circuit breakers SLM** (R3.S2) **si et seulement si** des robots
/// sont câblés (`has_robots`) : ça éteint le warning `build()` (« SLM clients
/// sans circuits ») ET active la résilience aux crashes SLM mid-session (3
/// échecs → circuit Open → skip <1ms). Sans robot (mock / `EFE_LIVE_SLM` absent)
/// → pas de circuits → invariance P5.1/P5.2.
///
/// **Ordre critique** : `.with_default_slm_circuits()` (qui mute
/// `clients.slm_circuits`) DOIT venir **après** `.clients()` (qui remplace tout
/// `clients`, circuits compris). Inversé, les circuits seraient silencieusement
/// écrasés.
fn assemble_coordinator(
    registry: ProviderRegistry,
    clients: CoordinatorClients,
    metrics: Arc<MetricsRegistry>,
) -> Arc<Coordinator> {
    // `has_robots` AVANT le move de `clients` dans `.clients()`.
    let slm_active = has_robots(&clients);

    let mut builder = Coordinator::builder()
        .registry(registry)
        .clients(clients) // (1) set clients
        .metrics(metrics); // (neutre vis-à-vis des circuits)
    if slm_active {
        builder = builder.with_default_slm_circuits(); // (2) APRÈS clients
    }
    Arc::new(builder.build().expect("preview coordinator buildable"))
}

/// **Couche LLM** selon le mode : mock / réel (feature-gated) / fallback mock.
/// **Sync** — aucune I/O.
fn build_llm_layer(mock: bool) -> LlmLayer {
    if mock {
        eprintln!("[preview] couche LLM : MOCK (EFE_PREVIEW_MOCK=1) — déterministe, gratuit");
        return mock_llm_layer();
    }
    #[cfg(feature = "real-llm")]
    {
        if let Some(layer) = real_llm_layer() {
            return layer;
        }
        eprintln!("[preview] couche LLM : aucune clé détectée — fallback MOCK");
        mock_llm_layer()
    }
    #[cfg(not(feature = "real-llm"))]
    {
        eprintln!(
            "[preview] couche LLM : feature `real-llm` désactivée — MOCK \
             (recompiler avec --features real-llm pour les vrais LLM)"
        );
        mock_llm_layer()
    }
}

/// **Couche SLM (P5.2).** Délègue au cœur testable `build_slm_clients` : health
/// pre-flight + `from_assets` per-robot selon `SlmConfig::from_env()`. Si
/// `EFE_LIVE_SLM` ≠ 1 → couche vide (`Default`, invariance P5.1).
async fn build_slm_layer() -> CoordinatorClients {
    build_slm_clients(&SlmConfig::from_env()).await
}

/// Couche LLM mock — 3 providers déterministes ([`mock_registry`]) + métriques.
fn mock_llm_layer() -> LlmLayer {
    LlmLayer {
        registry: mock_registry(),
        default_llms: mock_default_llms(),
        metrics: Arc::new(MetricsRegistry::new()),
    }
}

/// LLMs par défaut en mode mock — alignés sur les providers de `mock_registry`
/// (cf. [`MOCK_LLMS`]).
fn mock_default_llms() -> Vec<String> {
    MOCK_LLMS.iter().map(|s| (*s).to_string()).collect()
}

/// Couche LLM **réelle** — providers présents dans l'env. `None` si aucune clé.
/// Câble aussi `MetricsRegistry` (corrige l'asymétrie historique : `/metrics`
/// fonctionne désormais aussi en mode réel).
#[cfg(feature = "real-llm")]
fn real_llm_layer() -> Option<LlmLayer> {
    use efe_coordinator::llm::LlmProvider;
    use efe_coordinator::{ClaudeProvider, GeminiProvider, GptProvider};

    let registry = ProviderRegistry::new();
    let mut default_llms: Vec<String> = Vec::new();

    if let Ok(p) = ClaudeProvider::from_env() {
        let p: Arc<dyn LlmProvider> = Arc::new(p);
        default_llms.push(p.id());
        registry.register(p);
    }
    if let Ok(p) = GeminiProvider::from_env() {
        let p: Arc<dyn LlmProvider> = Arc::new(p);
        default_llms.push(p.id());
        registry.register(p);
    }
    if let Ok(p) = GptProvider::from_env() {
        let p: Arc<dyn LlmProvider> = Arc::new(p);
        default_llms.push(p.id());
        registry.register(p);
    }

    if default_llms.is_empty() {
        return None;
    }
    eprintln!("[preview] couche LLM : RÉEL — providers : {default_llms:?}");
    Some(LlmLayer {
        registry,
        default_llms,
        metrics: Arc::new(MetricsRegistry::new()),
    })
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal reçu");
}

#[cfg(test)]
mod p51_tests {
    //! Verrouille les invariants de P5.1 (refactor build_coordinator). Les tests
    //! `bridge.rs` passent par `mock_state()`/`build_app` et n'exercent donc PAS
    //! `build_coordinator` ni les couches LLM/SLM ; ces tests comblent ce trou.
    use super::*;

    /// La couche LLM mock expose `MOCK_LLMS` comme `default_llms` — garde contre
    /// la régression cc648f6 (default_llms ↔ providers réellement enregistrés).
    #[test]
    fn mock_llm_layer_defaults_to_mock_llms() {
        let layer = build_llm_layer(true);
        assert_eq!(layer.default_llms, mock_default_llms());
    }

    /// Couche SLM **vide quand désactivée** (`EFE_LIVE_SLM` ≠ 1) → invariance
    /// stricte de P5.1, désormais via le chemin P5.2 (`build_slm_clients`).
    #[tokio::test]
    async fn slm_layer_empty_when_disabled() {
        std::env::remove_var("EFE_LIVE_SLM"); // garantit le mode désactivé
        let c = build_slm_layer().await;
        assert!(c.analyzer.is_none(), "analyzer None quand SLM désactivé");
        assert!(c.compressor.is_none(), "compressor None");
        assert!(c.verifier.is_none(), "verifier None");
        assert!(c.router.is_none(), "router None quand SLM désactivé");
        assert!(c.reformulator.is_none(), "reformulator None");
        assert!(c.filter.is_none(), "filter None");
    }

    /// Assemblage complet (mock) : le coordinator se build et renvoie `MOCK_LLMS`.
    /// Couvre la composition couche LLM × couche SLM × `.build()`.
    #[tokio::test]
    async fn build_coordinator_mock_composes_and_returns_defaults() {
        let (_coord, default_llms) = build_coordinator(true).await;
        assert_eq!(default_llms, mock_default_llms());
    }

    // ── P5.3 — circuits SLM conditionnels ──────────────────────────────────

    /// Robots câblés → circuits SLM présents. `default_slm_circuits` couvre les
    /// 3 robots (analyzer/compressor/verifier), même si un seul est wired.
    /// `has_policy=false` → lookup direct dans `clients.slm_circuits`.
    #[test]
    fn circuits_wired_when_robots_present() {
        use efe_nano_app_coordinator::clients::AnalyzerClient;
        use efe_nano_app_coordinator::TenantId;

        let clients = CoordinatorClients {
            analyzer: AnalyzerClient::from_assets("http://unused").ok().map(Arc::new),
            ..Default::default()
        };
        assert!(clients.analyzer.is_some(), "from_assets doit réussir (assets présents)");

        let coord = assemble_coordinator(mock_registry(), clients, Arc::new(MetricsRegistry::new()));
        let t = TenantId::default();
        assert!(
            coord.slm_circuit_for(t, "analyzer", false).is_some(),
            "circuit analyzer câblé"
        );
        assert!(
            coord.slm_circuit_for(t, "compressor", false).is_some(),
            "default_slm_circuits couvre les 3 robots"
        );
        assert!(coord.slm_circuit_for(t, "verifier", false).is_some());
    }

    /// Aucun robot → aucun circuit (invariance ; le warning `build()` reste muet).
    #[test]
    fn no_circuits_when_no_robots() {
        use efe_nano_app_coordinator::TenantId;

        let coord = assemble_coordinator(
            mock_registry(),
            CoordinatorClients::default(),
            Arc::new(MetricsRegistry::new()),
        );
        let t = TenantId::default();
        assert!(
            coord.slm_circuit_for(t, "analyzer", false).is_none(),
            "pas de robot → pas de circuit"
        );
    }

    /// **Cas dégradé le plus fréquent** : SLM activé mais robots tous down →
    /// `build_slm_clients` renvoie `router=Some`, robots `None`. L'assemblage ne
    /// doit AJOUTER AUCUN circuit (le router n'est pas un robot → `has_robots`
    /// false). Verrouille l'intégration `has_robots` × conditionnel circuits sur
    /// le scénario exact du smoke P5.2.
    #[test]
    fn no_circuits_when_only_router_wired() {
        use efe_nano_app_coordinator::clients::RouterClient;
        use efe_nano_app_coordinator::TenantId;

        let clients = CoordinatorClients {
            router: Some(Arc::new(RouterClient::new())),
            ..Default::default()
        };
        let coord = assemble_coordinator(mock_registry(), clients, Arc::new(MetricsRegistry::new()));
        let t = TenantId::default();
        assert!(
            coord.slm_circuit_for(t, "analyzer", false).is_none(),
            "router seul (robots down) → aucun circuit"
        );
    }

    /// Chemin mock end-to-end (`build_coordinator`) → aucun circuit. Invariance
    /// P5.1/P5.2 préservée par le conditionnel `has_robots`.
    #[tokio::test]
    async fn mock_path_has_no_circuits() {
        use efe_nano_app_coordinator::TenantId;

        std::env::remove_var("EFE_LIVE_SLM");
        let (coord, _) = build_coordinator(true).await;
        let t = TenantId::default();
        assert!(
            coord.slm_circuit_for(t, "analyzer", false).is_none(),
            "mock → aucun circuit"
        );
    }
}
