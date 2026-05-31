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

/// **Mode de la couche LLM**, capturé comme DONNÉE (et non un simple side-effect
/// `eprintln!`) pour que la bannière P5.4 puisse raisonner dessus et que le mode
/// soit unit-testable. Les deux variantes de repli distinguent le mock *subi* du
/// mock *voulu* — honnêteté : un repli signale que le réel était demandé.
// Enum **multiplexé par feature** : dans un build donné, seul un sous-ensemble
// des variantes est *construit* — `FallbackFeatureOff` sous `not(real-llm)`,
// `Real`/`FallbackNoKey` sous `real-llm`. L'analyse `dead_code` (qui compte la
// CONSTRUCTION, pas le `match`) signalerait donc le complément dans chaque
// config. `render_banner` matche pourtant les 4 exhaustivement → les variantes
// font partie du contrat. On tait `dead_code` pour cette enum close.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LlmMode {
    /// `EFE_PREVIEW_MOCK=1` — providers déterministes, voulu.
    Mock,
    /// Feature `real-llm` + ≥1 clé détectée — vrais providers.
    Real,
    /// Réel voulu, feature active, mais AUCUNE clé → repli mock (⚠).
    FallbackNoKey,
    /// Réel voulu, mais binaire compilé sans `real-llm` → repli mock (⚠).
    FallbackFeatureOff,
}

/// **Couche LLM** : tout ce qui définit le gros modèle servi par le bridge —
/// mode, registry de providers, LLMs par défaut, et télémétrie. Regroupé pour un
/// assemblage unique, orthogonal à la couche SLM.
struct LlmLayer {
    mode: LlmMode,
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
    // le futur reste `Send` sur un runtime multi-thread. On lit `SlmConfig` ICI
    // (plus de wrapper `build_slm_layer`) pour disposer de `enabled` — distinct
    // de « robots vivants » — nécessaire à la bannière P5.4.
    let slm_cfg = SlmConfig::from_env();
    let clients = build_slm_clients(&slm_cfg).await;
    // Snapshot per-robot AVANT le move de `clients` dans `assemble_coordinator`
    // (même contrainte que `has_robots` en P5.3 : on lit, puis on move).
    let slm_status = SlmStatus::from_clients(slm_cfg.enabled, &clients);

    let llm = build_llm_layer(mock);

    // SEAM P5.4 — bannière consolidée : SOURCE UNIQUE de vérité du mode de
    // démarrage (LLM × SLM × robots vivants × pépite). Remplace les `eprintln!`
    // de mode LLM jadis éparpillés dans `build_llm_layer` / `real_llm_layer`.
    eprintln!("{}", render_banner(llm.mode, &llm.default_llms, slm_status));

    let coord = assemble_coordinator(llm.registry, clients, llm.metrics);
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
        return mock_llm_layer(LlmMode::Mock);
    }
    #[cfg(feature = "real-llm")]
    {
        if let Some(layer) = real_llm_layer() {
            return layer;
        }
        // Réel voulu, feature active, mais aucune clé → repli mock signalé.
        mock_llm_layer(LlmMode::FallbackNoKey)
    }
    #[cfg(not(feature = "real-llm"))]
    {
        // Réel impossible (binaire compilé sans `real-llm`) → repli mock signalé.
        mock_llm_layer(LlmMode::FallbackFeatureOff)
    }
}

// ─────────────────────────────────────────────────────────────────────────
// P5.4 — Bannière de démarrage consolidée (mode LLM × SLM × robots vivants)
// ─────────────────────────────────────────────────────────────────────────

/// Snapshot per-robot de la couche SLM, pris AVANT que `clients` ne soit moved
/// dans [`assemble_coordinator`]. `enabled` (SLM demandé via `EFE_LIVE_SLM=1`)
/// est volontairement distinct de « robots vivants » ([`Self::any_robot`]) : SLM
/// inactif et SLM actif-mais-tous-down racontent deux histoires différentes.
struct SlmStatus {
    enabled: bool,
    analyzer: bool,
    compressor: bool,
    verifier: bool,
    router: bool,
}

impl SlmStatus {
    /// Dérive le snapshot des `Option` câblés (lecture seule, ne move rien).
    fn from_clients(enabled: bool, c: &CoordinatorClients) -> Self {
        Self {
            enabled,
            analyzer: c.analyzer.is_some(),
            compressor: c.compressor.is_some(),
            verifier: c.verifier.is_some(),
            router: c.router.is_some(),
        }
    }

    /// `true` ssi ≥1 robot SLM (analyzer/compressor/verifier) est vivant.
    /// **Invariant** : `self.any_robot() == has_robots(clients)` pour les mêmes
    /// `clients` (mêmes 3 `Option`). Le router n'est PAS un robot.
    fn any_robot(&self) -> bool {
        self.analyzer || self.compressor || self.verifier
    }
}

/// **Bannière de démarrage — fonction PURE** (zéro env, zéro I/O) ⇒ testable par
/// sous-chaînes. Source unique de vérité du mode : LLM (mock/réel/repli) × SLM
/// (inactif/actif + robots ✓/✗) × pépite (LLM mock + robots réels = pré-mâchage
/// réel gratuit) × avertissements (repli LLM, SLM actif sans robot vivant).
fn render_banner(mode: LlmMode, default_llms: &[String], slm: SlmStatus) -> String {
    const RULE: &str = "──────────────────────────────────────────────────────────";
    let mut out = String::new();

    out.push_str(RULE);
    out.push_str("\n EFE Preview Lab — pré-mâchage live\n");
    out.push_str(RULE);
    out.push('\n');

    // ── Couche LLM ──────────────────────────────────────────────────────────
    let llm_line = match mode {
        LlmMode::Mock => "MOCK (déterministe, gratuit)",
        LlmMode::Real => "RÉEL",
        LlmMode::FallbackNoKey => "MOCK  ⚠ repli — aucune clé API détectée",
        LlmMode::FallbackFeatureOff => "MOCK  ⚠ repli — feature real-llm désactivée",
    };
    out.push_str(&format!(" LLM  · {llm_line}\n"));
    let providers = if default_llms.is_empty() {
        "(aucun)".to_string()
    } else {
        default_llms.join(", ")
    };
    out.push_str(&format!("        providers : {providers}\n"));

    // ── Couche SLM ──────────────────────────────────────────────────────────
    if slm.enabled {
        out.push_str(" SLM  · ACTIF (EFE_LIVE_SLM=1)\n");
        let mark = |b: bool| if b { '✓' } else { '✗' };
        out.push_str(&format!(
            "        analyzer {}   compressor {}   verifier {}   router {}\n",
            mark(slm.analyzer),
            mark(slm.compressor),
            mark(slm.verifier),
            mark(slm.router),
        ));
    } else {
        out.push_str(" SLM  · INACTIF (EFE_LIVE_SLM≠1) — pré-mâchage simulé par le moteur\n");
    }

    // ── Pépite / avertissements ─────────────────────────────────────────────
    if mode == LlmMode::Mock && slm.any_robot() {
        out.push_str(" ✨ PÉPITE · LLM mock + robots réels → pré-mâchage RÉEL, coût LLM nul\n");
    }
    if slm.enabled && !slm.any_robot() {
        out.push_str(
            " ⚠ SLM actif mais aucun robot vivant — voir ./scripts/slm/start.sh \
             (le moteur fera le pré-mâchage en interne)\n",
        );
    }

    out.push_str(RULE);
    out
}

/// Couche LLM mock — 3 providers déterministes ([`mock_registry`]) + métriques.
/// `mode` distingue le mock VOULU (`EFE_PREVIEW_MOCK=1`) du mock SUBI (repli
/// faute de clé / de feature) — porté tel quel par la bannière P5.4.
fn mock_llm_layer(mode: LlmMode) -> LlmLayer {
    LlmLayer {
        mode,
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
    Some(LlmLayer {
        mode: LlmMode::Real,
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
        assert_eq!(layer.mode, LlmMode::Mock, "EFE_PREVIEW_MOCK=1 → mode Mock");
    }

    /// Couche SLM **vide quand désactivée** (`EFE_LIVE_SLM` ≠ 1) → invariance
    /// stricte de P5.1, désormais via le chemin P5.2 (`build_slm_clients`).
    #[tokio::test]
    async fn slm_layer_empty_when_disabled() {
        std::env::remove_var("EFE_LIVE_SLM"); // garantit le mode désactivé
        // `build_slm_layer` (wrapper) a été supprimé en P5.4 → on appelle
        // directement le cœur testable `build_slm_clients`.
        let c = build_slm_clients(&SlmConfig::from_env()).await;
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

    // ── P5.4 — bannière consolidée (fonction pure `render_banner`) ──────────

    /// Petit constructeur de `SlmStatus` pour les tests de bannière.
    fn slm(enabled: bool, a: bool, c: bool, v: bool, r: bool) -> SlmStatus {
        SlmStatus {
            enabled,
            analyzer: a,
            compressor: c,
            verifier: v,
            router: r,
        }
    }

    /// Mock + SLM inactif → annonce MOCK + INACTIF + providers ; PAS de pépite,
    /// AUCUN robot coché (la ligne robots n'est même pas émise).
    #[test]
    fn banner_mock_no_slm() {
        let llms = vec!["mock-claude".to_string(), "mock-gpt".to_string()];
        let b = render_banner(LlmMode::Mock, &llms, slm(false, false, false, false, false));
        assert!(b.contains("MOCK (déterministe"), "mode mock annoncé");
        assert!(b.contains("INACTIF"), "SLM inactif annoncé");
        // Lock du JOIN CSV (`, `) — pas seulement la présence des deux ids :
        // un join vide (`mock-claudemock-gpt`) contiendrait aussi les sous-chaînes.
        assert!(
            b.contains("mock-claude, mock-gpt"),
            "providers listés ET joints par ', '"
        );
        assert!(!b.contains("PÉPITE"), "pas de pépite sans robot");
        assert!(!b.contains('✓'), "aucun robot coché quand SLM inactif");
    }

    /// Mock + ≥1 robot vivant → bloc PÉPITE + SLM ACTIF + robots cochés.
    #[test]
    fn banner_pepite_mock_plus_robots() {
        let llms = vec!["mock-claude".to_string()];
        let b = render_banner(LlmMode::Mock, &llms, slm(true, true, true, false, true));
        assert!(b.contains("PÉPITE"), "mock + robots réels = pépite");
        assert!(b.contains("ACTIF"), "SLM actif");
        assert!(b.contains('✓'), "robots cochés");
    }

    /// Réel → RÉEL + ids providers ; JAMAIS de pépite (le LLM n'est pas gratuit).
    #[test]
    fn banner_real_llm_no_pepite() {
        let llms = vec!["claude-3-5".to_string()];
        let b = render_banner(LlmMode::Real, &llms, slm(true, true, false, false, true));
        assert!(b.contains("RÉEL"), "mode réel annoncé");
        assert!(b.contains("claude-3-5"), "provider réel listé");
        assert!(!b.contains("PÉPITE"), "pas de pépite en mode réel");
    }

    /// Repli faute de clé → avertissement ⚠ + mention « clé » + providers vides.
    #[test]
    fn banner_fallback_no_key_warns() {
        let b = render_banner(
            LlmMode::FallbackNoKey,
            &[],
            slm(false, false, false, false, false),
        );
        assert!(b.contains('⚠'), "repli signalé par ⚠");
        assert!(b.contains("clé"), "raison: aucune clé");
        assert!(b.contains("(aucun)"), "providers vides → (aucun)");
        assert!(!b.contains("PÉPITE"), "repli n'est pas une pépite");
    }

    /// SLM actif mais tous les robots down (router seul) → avert. dégradé,
    /// router toujours affiché, PAS de pépite.
    #[test]
    fn banner_slm_enabled_all_down_warns() {
        let llms = vec!["mock-claude".to_string()];
        let b = render_banner(LlmMode::Mock, &llms, slm(true, false, false, false, true));
        assert!(b.contains("aucun robot vivant"), "dégradé signalé");
        assert!(!b.contains("PÉPITE"), "pas de pépite sans robot vivant");
        assert!(b.contains("router ✓"), "router seul reste affiché");
        assert!(b.contains("analyzer ✗"), "analyzer down → ✗");
    }

    /// La ligne robots reflète CHAQUE drapeau indépendamment.
    #[test]
    fn banner_robot_line_reflects_each_flag() {
        let llms = vec!["mock-claude".to_string()];
        let b = render_banner(LlmMode::Mock, &llms, slm(true, true, true, false, true));
        assert!(b.contains("analyzer ✓"), "analyzer up → ✓");
        assert!(b.contains("compressor ✓"), "compressor up → ✓");
        assert!(b.contains("verifier ✗"), "verifier down → ✗");
        assert!(b.contains("router ✓"), "router up → ✓");
    }

    /// **Invariant** : `SlmStatus::any_robot()` ⇔ `has_robots(clients)` pour les
    /// mêmes `clients` (router NON compté comme robot).
    #[test]
    fn slm_status_any_robot_matches_has_robots() {
        use efe_nano_app_coordinator::clients::{AnalyzerClient, RouterClient};

        // (a) router seul → aucun robot.
        let only_router = CoordinatorClients {
            router: Some(Arc::new(RouterClient::new())),
            ..Default::default()
        };
        let st = SlmStatus::from_clients(true, &only_router);
        assert_eq!(st.any_robot(), has_robots(&only_router));
        assert!(!st.any_robot(), "router seul → pas de robot");

        // (b) analyzer câblé (from_assets réussit hors-ligne) → robot présent.
        let with_analyzer = CoordinatorClients {
            analyzer: AnalyzerClient::from_assets("http://unused").ok().map(Arc::new),
            ..Default::default()
        };
        let st2 = SlmStatus::from_clients(true, &with_analyzer);
        assert_eq!(st2.any_robot(), has_robots(&with_analyzer));
        assert!(st2.any_robot(), "analyzer câblé → robot présent");
    }

    // ── P5.4 — trous comblés à la vérification ─────────────────────────────

    /// **Gap A** — 4e mode LLM : repli faute de feature `real-llm`. C'est le
    /// repli du binaire par DÉFAUT en mode non-mock ; sa ligne de bannière
    /// n'était couverte par AUCUN test (les 3 autres modes l'étaient).
    #[test]
    fn banner_fallback_feature_off_warns() {
        let b = render_banner(
            LlmMode::FallbackFeatureOff,
            &["mock-claude".to_string()],
            slm(false, false, false, false, false),
        );
        assert!(b.contains('⚠'), "repli signalé par ⚠");
        assert!(
            b.contains("feature real-llm désactivée"),
            "raison: feature real-llm absente"
        );
        assert!(!b.contains("PÉPITE"), "repli n'est pas une pépite");
    }

    /// **Gap B** — décision d'architecture §5 VERROUILLÉE : la PÉPITE est
    /// réservée au mock EXPLICITE (`EFE_PREVIEW_MOCK=1`). Un repli mock (réel
    /// voulu, non obtenu) AVEC robots vivants — état runtime réel
    /// (`EFE_LIVE_SLM=1` + pas de clé) — ne doit PAS afficher PÉPITE : l'avert.
    /// ⚠ repli prime, pour ne pas masquer la mauvaise config.
    #[test]
    fn banner_no_pepite_for_fallback_mock_with_robots() {
        for mode in [LlmMode::FallbackNoKey, LlmMode::FallbackFeatureOff] {
            let b = render_banner(mode, &["mock-claude".to_string()], slm(true, true, true, true, true));
            assert!(!b.contains("PÉPITE"), "repli mock + robots ≠ pépite ({mode:?})");
            assert!(b.contains('⚠'), "le repli reste signalé ({mode:?})");
            assert!(b.contains("analyzer ✓"), "robots restent affichés ({mode:?})");
        }
    }

    /// **Gap C** — `from_clients` mappe CHAQUE `Option` vers le BON drapeau
    /// (anti-swap compressor↔verifier↔…) et passe `enabled` tel quel. On câble
    /// un seul robot et on vérifie l'isolation du mapping.
    #[test]
    fn slm_status_from_clients_maps_each_field() {
        use efe_nano_app_coordinator::clients::CompressorClient;

        let only_compressor = CoordinatorClients {
            compressor: CompressorClient::from_assets("http://unused").ok().map(Arc::new),
            ..Default::default()
        };
        assert!(only_compressor.compressor.is_some(), "from_assets compressor OK hors-ligne");

        let st = SlmStatus::from_clients(true, &only_compressor);
        assert!(st.enabled, "enabled passé tel quel");
        assert!(st.compressor, "compressor câblé → champ compressor true");
        assert!(
            !st.analyzer && !st.verifier && !st.router,
            "aucun autre champ ne doit fuiter (anti-swap)"
        );

        // `enabled=false` passe aussi tel quel (indépendant des robots).
        let st_off = SlmStatus::from_clients(false, &only_compressor);
        assert!(!st_off.enabled, "enabled=false passé tel quel");
        assert!(st_off.compressor, "le mapping robot reste indépendant de enabled");
    }

    // ── P5.4 — vérification 2e passe : côtés NÉGATIFs des gardes + squelette ──

    /// **Gap D** — les avertissements n'apparaissent QUE quand ils sont mérités.
    /// Les tests positifs (dégradé, repli) verrouillent l'apparition ; ici on
    /// verrouille l'ABSENCE : en mock explicite + état sain (SLM inactif OU
    /// robots vivants), AUCUN `⚠` ne doit apparaître. Couvre le côté négatif des
    /// deux gardes (`repli` et `enabled && !any_robot`) — un `⚠` toujours imprimé
    /// passerait tous les tests positifs.
    #[test]
    fn banner_no_warning_when_mock_and_healthy() {
        let llms = vec!["mock-claude".to_string()];

        // (a) mock + SLM inactif → zéro avertissement.
        let b_off = render_banner(LlmMode::Mock, &llms, slm(false, false, false, false, false));
        assert!(!b_off.contains('⚠'), "mock + SLM inactif → aucun ⚠");
        assert!(
            !b_off.contains("aucun robot vivant"),
            "SLM inactif → pas de ligne dégradée"
        );

        // (b) mock + robots vivants → zéro avertissement (pépite, pas dégradé).
        let b_up = render_banner(LlmMode::Mock, &llms, slm(true, true, true, true, true));
        assert!(!b_up.contains('⚠'), "mock + robots vivants → aucun ⚠");
        assert!(
            !b_up.contains("aucun robot vivant"),
            "robots vivants → pas de ligne dégradée"
        );
        assert!(b_up.contains("PÉPITE"), "mock + robots vivants = pépite");
    }

    /// **Gap E** — squelette de la bannière : titre + exactement 3 filets pleins
    /// (haut, sous-titre, bas), bornée par des filets (pas de `\n` final —
    /// `eprintln!` l'ajoute). Verrouille la structure : un refactor ne peut pas
    /// larguer silencieusement le cadre ou le titre.
    #[test]
    fn banner_has_frame_and_title() {
        let b = render_banner(
            LlmMode::Mock,
            &["mock-claude".to_string()],
            slm(false, false, false, false, false),
        );
        assert!(
            b.contains("EFE Preview Lab — pré-mâchage live"),
            "titre présent"
        );
        let rule_lines = b
            .lines()
            .filter(|l| !l.is_empty() && l.chars().all(|c| c == '─'))
            .count();
        assert_eq!(rule_lines, 3, "3 filets pleins (haut, sous-titre, bas)");
        assert!(b.starts_with('─'), "commence par un filet");
        assert!(b.ends_with('─'), "se termine par un filet (pas de \\n final)");
    }
}
