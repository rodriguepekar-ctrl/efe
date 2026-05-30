// SPDX-License-Identifier: UNLICENSED
//! Binaire `efe-preview-server` — sert le bridge SSE du Preview Lab.
//!
//! Modes :
//! - `EFE_PREVIEW_MOCK=1` → coordinator mock (réponses déterministes, gratuit).
//! - sinon + feature `real-llm` → providers réels depuis l'env (Claude/Gemini/GPT).
//! - sinon → fallback mock (avec avertissement).

use std::sync::Arc;

use efe_preview_server::{build_app, mock_coordinator, AppState, PreviewConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    let mock = std::env::var("EFE_PREVIEW_MOCK").as_deref() == Ok("1");
    let (coord, default_llms) = build_coordinator(mock);
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

/// Construit le coordinator + la liste des LLMs par défaut selon le mode.
fn build_coordinator(mock: bool) -> (Arc<efe_coordinator::Coordinator>, Vec<String>) {
    if mock {
        eprintln!("[preview] mode MOCK (EFE_PREVIEW_MOCK=1) — réponses déterministes, gratuites");
        return (mock_coordinator(), vec!["mock".to_string()]);
    }
    #[cfg(feature = "real-llm")]
    {
        if let Some(pair) = real_coordinator() {
            return pair;
        }
        eprintln!("[preview] aucune clé LLM détectée — fallback MOCK");
        (mock_coordinator(), vec!["mock".to_string()])
    }
    #[cfg(not(feature = "real-llm"))]
    {
        eprintln!(
            "[preview] feature `real-llm` désactivée — mode MOCK \
             (recompiler avec --features real-llm pour les vrais LLM)"
        );
        (mock_coordinator(), vec!["mock".to_string()])
    }
}

/// Construit un coordinator avec les providers réels présents dans l'env.
/// `None` si aucune clé n'est détectée.
#[cfg(feature = "real-llm")]
fn real_coordinator() -> Option<(Arc<efe_coordinator::Coordinator>, Vec<String>)> {
    use efe_coordinator::llm::{LlmProvider, ProviderRegistry};
    use efe_coordinator::{ClaudeProvider, Coordinator, GeminiProvider, GptProvider};

    let reg = ProviderRegistry::new();
    let mut ids: Vec<String> = Vec::new();

    if let Ok(p) = ClaudeProvider::from_env() {
        let p: Arc<dyn LlmProvider> = Arc::new(p);
        ids.push(p.id());
        reg.register(p);
    }
    if let Ok(p) = GeminiProvider::from_env() {
        let p: Arc<dyn LlmProvider> = Arc::new(p);
        ids.push(p.id());
        reg.register(p);
    }
    if let Ok(p) = GptProvider::from_env() {
        let p: Arc<dyn LlmProvider> = Arc::new(p);
        ids.push(p.id());
        reg.register(p);
    }

    if ids.is_empty() {
        return None;
    }
    eprintln!("[preview] mode RÉEL — providers : {ids:?}");
    let coord = Coordinator::builder().registry(reg).build().ok()?;
    Some((Arc::new(coord), ids))
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal reçu");
}
