// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : `efe-gateway` binary entrypoint.

use std::process::ExitCode;
use std::sync::Arc;

use efe_agentic_bridge::{AgenticBridge, AgenticResponse, AgenticTask, BridgeError};
use efe_gateway::{
    auth::Hs256JwtValidator, build_router, config::GatewayConfig, shutdown_signal,
    state::AppState, N22GatewayMetrics,
};
use efe_gold_test_gate::{
    DefaultAssertionEngine, DefaultGoldApi, GoldGate, InMemoryGoldRunLedger,
    InMemoryGoldRunner, InMemoryGoldSetRegistry, N19Metrics, StubEmbeddingProvider,
};
use efe_nano_sdk::ContextBundle;
use efe_publisher_identity::{
    DefaultPublisherApi, Ed25519Verifier, InMemoryKeyStore, InMemoryPublishLedger,
    InMemoryPublisherRegistry, N18Metrics, StubKycProvider, StubSanctionsScreener,
};
use efe_swarm_orchestrator::SwarmOrchestrator;

#[tokio::main]
async fn main() -> ExitCode {
    let cfg = match GatewayConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("config error : {e}");
            return ExitCode::from(2);
        }
    };
    if let Err(e) = cfg.validate() {
        eprintln!("config invalid : {e}");
        return ExitCode::from(2);
    }
    init_tracing(&cfg);

    tracing::info!(
        version = efe_gateway::CRATE_VERSION,
        bind = %cfg.bind,
        body_limit_bytes = cfg.body_limit_bytes,
        rate_limit_rps = cfg.rate_limit_rps,
        "efe-gateway starting"
    );

    let state = build_app_state(&cfg).await;
    let router = build_router(state.clone(), cfg.body_limit_bytes);
    let listener = match tokio::net::TcpListener::bind(&cfg.bind).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = %e, "failed to bind");
            return ExitCode::from(3);
        }
    };
    tracing::info!(addr = ?listener.local_addr(), "listening");

    let serve = axum::serve(listener, router).with_graceful_shutdown(shutdown_signal());
    match serve.await {
        Ok(()) => {
            tracing::info!("graceful shutdown complete");
            state.metrics.tick_shutdown("ok");
            ExitCode::SUCCESS
        }
        Err(e) => {
            tracing::error!(error = %e, "server error");
            state.metrics.tick_shutdown("error");
            ExitCode::from(1)
        }
    }
}

fn init_tracing(cfg: &GatewayConfig) {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(cfg.log_level.clone()));
    match cfg.log_format {
        efe_gateway::config::LogFormat::Json => {
            fmt()
                .json()
                .with_env_filter(filter)
                .with_target(true)
                .init();
        }
        efe_gateway::config::LogFormat::Pretty => {
            fmt().pretty().with_env_filter(filter).init();
        }
    }
}

async fn build_app_state(cfg: &GatewayConfig) -> AppState {
    // Auth.
    let efe_gateway::config::AuthMode::Hs256 { secret } = &cfg.auth;
    let jwt = Hs256JwtValidator::new(secret.as_bytes().to_vec())
        .with_expected_issuer("efe-gateway");

    // Subsystem wiring (Sprint 14 : all in-memory ; Sprint 15
    // swaps to Postgres-backed).
    let swarm = Arc::new(SwarmOrchestrator::new());

    // **Sprint 18 N26 + G13** : wire the N17 provenance store +
    // collector.  Opt-out via `EFE_GATEWAY_DISABLE_PROVENANCE=1`.
    // When wired, every Claude dispatch produces one
    // `LlmProvenanceRecord` ; `GET /v1/llm-provenance` exposes
    // the read side.
    let llm_provenance_store: Option<Arc<dyn efe_llm_provenance::LlmProvenanceStore>> =
        match std::env::var("EFE_GATEWAY_DISABLE_PROVENANCE") {
            Ok(v) if v == "1" => {
                tracing::warn!(
                    "EFE_GATEWAY_DISABLE_PROVENANCE=1 — N26 disabled (no LlmProvenanceRecord emission)"
                );
                None
            }
            _ => Some(Arc::new(efe_llm_provenance::InMemoryLlmProvenanceStore::new())),
        };
    let llm_provenance_collector: Option<Arc<dyn efe_llm_provenance::LlmProvenanceCollector>> =
        if let Some(store) = llm_provenance_store.as_ref() {
            let registry = Arc::new(efe_llm_provenance::InMemoryModelRegistry::new());
            // Pre-register the three Anthropic models with
            // `dev_skip_hash_check = true` (hosted models, no
            // content hash to verify).  `build_app_state` is
            // async (called from `#[tokio::main]`) so we await
            // the registrations directly.
            use efe_llm_provenance::ModelRegistry as _;
            for model in [
                efe_agentic_bridge::DEFAULT_CLAUDE_MODEL,
                "claude-haiku-3-5",
                "claude-opus-4",
            ] {
                let mut reg = efe_llm_provenance::RegisteredModel::hosted(
                    efe_nano_sdk::ModelIdentity::hosted(
                        "anthropic",
                        model,
                        "2023-06-01",
                    ),
                    0.0,
                    0.0,
                    1,
                );
                reg.dev_skip_hash_check = true;
                // Best-effort registration : duplicates from a
                // hot reload are silently ignored.
                let _ = registry.register(reg).await;
            }
            let collector = efe_llm_provenance::DefaultLlmProvenanceCollector::new(
                store.clone(),
                registry as Arc<dyn efe_llm_provenance::ModelRegistry>,
            );
            Some(collector as Arc<dyn efe_llm_provenance::LlmProvenanceCollector>)
        } else {
            None
        };

    // **Sprint 14 fix** : Wire `ClaudeBridge` if `ANTHROPIC_API_KEY`
    // is set, fall back to `StubAgenticBridge` otherwise.  Allows
    // the gateway to boot in dev (no key needed) AND to use the
    // real Claude API in prod just by setting the env var.
    //
    // **Sprint 18 N26** : when a provenance collector is wired,
    // inject it into ClaudeBridge so every dispatch produces an
    // `LlmProvenanceRecord`.
    let agentic: Arc<dyn AgenticBridge> = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(k) if !k.is_empty() => {
            tracing::info!(
                provenance_wired = llm_provenance_collector.is_some(),
                "ClaudeBridge wired (model = claude-sonnet-4-6 default)"
            );
            let mut bridge = efe_agentic_bridge::ClaudeBridge::new(k);
            if let Ok(model) = std::env::var("ANTHROPIC_MODEL") {
                bridge = bridge.with_model(model);
            }
            if let Some(c) = llm_provenance_collector.as_ref() {
                bridge = bridge.with_provenance(c.clone());
            }
            // **Sprint 19 — N27** : wire the retry policy +
            // streaming config from env (see architecture § 17.1).
            // Both fall back to canonical defaults if the env
            // vars are unset.  An operator can disable retries
            // entirely with `EFE_BRIDGE_RETRY_ENABLED=0`.
            let retry_enabled = std::env::var("EFE_BRIDGE_RETRY_ENABLED")
                .map(|v| v != "0")
                .unwrap_or(true);
            let retry_policy = if retry_enabled {
                efe_agentic_bridge::ExponentialBackoff::from_env()
            } else {
                tracing::warn!(
                    "EFE_BRIDGE_RETRY_ENABLED=0 — N27 retries DISABLED \
                     (transient 5xx will surface as 503)"
                );
                efe_agentic_bridge::ExponentialBackoff::disabled()
            };
            bridge = bridge.with_retry_policy(retry_policy);
            bridge =
                bridge.with_stream_config(efe_agentic_bridge::claude::StreamConfig::from_env());
            Arc::new(bridge)
        }
        _ => {
            tracing::warn!(
                "ANTHROPIC_API_KEY not set ; using StubAgenticBridge \
                 (calls to /v1/agentic/call will return canned responses)"
            );
            Arc::new(StubAgenticBridge)
        }
    };

    // Publisher API.
    let publisher_api = Arc::new(DefaultPublisherApi {
        registry: Arc::new(InMemoryPublisherRegistry::new()),
        ledger: Arc::new(InMemoryPublishLedger::new([0u8; 32])),
        kyc: Arc::new(StubKycProvider::permissive()),
        sanctions: Arc::new(StubSanctionsScreener::clear()),
        verifier: Arc::new(Ed25519Verifier::new()),
        metrics: N18Metrics::new(),
    });
    // Keystore reference retained but not yet consumed by the
    // gateway (Sprint 15 wires rotate-key endpoint).
    let _keystore = Arc::new(InMemoryKeyStore::default());

    // Gold API.
    let gold_registry = Arc::new(InMemoryGoldSetRegistry::new());
    let gold_ledger = Arc::new(InMemoryGoldRunLedger::new([0u8; 32]));
    let gold_gate = Arc::new(GoldGate::new(
        gold_registry.clone(),
        Arc::new(InMemoryGoldRunner::echo()),
        Arc::new(DefaultAssertionEngine::new(Arc::new(
            StubEmbeddingProvider::new(),
        ))),
        gold_ledger.clone(),
        N19Metrics::new(),
    ));
    let gold_api: Arc<dyn efe_gold_test_gate::GoldApi> =
        Arc::new(DefaultGoldApi::new(gold_registry, gold_gate, gold_ledger));

    // **N24 + G2 + G4 + G13** : wire the compliance runner with
    // the real publisher registry + Ed25519 verifier + gold API.
    // Opt-out via env var `EFE_GATEWAY_DISABLE_COMPLIANCE=1` for
    // bench builds where the audit overhead is intentional.
    let publisher_registry: Arc<dyn efe_publisher_identity::PublisherRegistry> =
        Arc::new(InMemoryPublisherRegistry::new());
    let verifier: Arc<dyn efe_publisher_identity::SignatureVerifier> =
        Arc::new(Ed25519Verifier::new());
    let compliance: Option<Arc<efe_compliance_wiring::WiredComplianceRunner>> =
        match std::env::var("EFE_GATEWAY_DISABLE_COMPLIANCE") {
            Ok(v) if v == "1" => {
                tracing::warn!(
                    "EFE_GATEWAY_DISABLE_COMPLIANCE=1 — running in N23 \
                     experimental_fast mode (no signature / gold / status gates)"
                );
                None
            }
            _ => {
                let runner = efe_compliance_wiring::WiredComplianceRunner::with_defaults(
                    publisher_registry,
                    verifier,
                    gold_api.clone(),
                );
                tracing::info!(
                    builtin_publisher_id = %runner.builtin_publisher_identity().publisher_id,
                    "N24 compliance runner wired (tier1_strict for Tier1+, experimental_relaxed for TierExperimental)"
                );
                Some(Arc::new(runner))
            }
        };

    // **N27 + § 17.1** : read the streaming kill-switch ONCE at
    // boot, stash it in AppState so the handler can read a
    // per-instance value (not racy with cargo's parallel tests).
    let stream_enabled =
        efe_gateway::handlers::agentic_stream::stream_endpoint_enabled_from_env();
    if !stream_enabled {
        tracing::warn!(
            "EFE_BRIDGE_STREAM_ENABLED=0 — POST /v1/agentic/call-stream will return 404"
        );
    }

    let mut builder = AppState::builder()
        .with_swarm(swarm)
        .with_agentic(agentic)
        .with_publisher_api(publisher_api)
        .with_gold_api(gold_api)
        .with_jwt_validator(Arc::new(jwt))
        .with_metrics(N22GatewayMetrics::new())
        .with_stream_endpoint_enabled(stream_enabled);
    if let Some(c) = compliance {
        builder = builder.with_compliance(c);
    }
    if let Some(store) = llm_provenance_store {
        builder = builder.with_llm_provenance_store(store);
    }
    builder.build().expect("AppState built")
}

// ─────────────────────────────────────────────────────────────────────
//  StubAgenticBridge — fallback when no real LLM is wired
// ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct StubAgenticBridge;

#[axum::async_trait]
impl AgenticBridge for StubAgenticBridge {
    async fn call_with_context(
        &self,
        bundle: &ContextBundle,
        _task: AgenticTask,
    ) -> Result<AgenticResponse, BridgeError> {
        Ok(AgenticResponse {
            text: format!(
                "[stub bridge] would have called Claude with bundle containing {} \
                 entities, {} redactions",
                bundle.entities.len(),
                bundle.redactions.len()
            ),
            stop_reason: efe_agentic_bridge::StopReason::EndTurn,
            usage: efe_agentic_bridge::TokenUsage {
                input_tokens: 0,
                output_tokens: 0,
            },
            model_identity: efe_nano_sdk::ModelIdentity::hosted(
                "stub",
                "stub-agentic",
                "v0",
            ),
            cost_microcents: 0,
        })
    }
}
