// SPDX-License-Identifier: UNLICENSED
//! Démo Mission A : pré-mâcher pour 1 LLM mock, streaming SSE printé sur stdout.
//!
//! Usage :
//! ```bash
//! PYO3_PYTHON=$(which python3.12) \
//!   cargo run -p efe-coordinator --example mission_a_demo
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{
    run_mission_a, Coordinator, HeartbeatHandle, MissionARequest, MockLlmProvider,
    ProviderRegistry,
};
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Registry : 1 provider mock qui répond avec un texte multi-mots.
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("claude_mock").with_response(
        "The Federal Reserve raised interest rates by 25 basis points today.",
    )));

    // 2. Coordinator avec InMemoryBus (par défaut).
    let coord = Coordinator::builder().registry(reg).build()?;

    // 3. Souscrit le bus avant de lancer la mission, afin d'en capter les évènements.
    let (_token, mut rx) = coord.subscribe(Some("demo"));

    // 4. Démarre un heartbeat (utile en prod, ici juste pour montrer l'API).
    //    Période courte pour la démo afin que `hb.stop()` retourne vite.
    let hb = HeartbeatHandle::spawn(Arc::clone(&coord.bus), Duration::from_millis(200));

    // 5. Spawn un reader qui print chaque évènement SSE.
    let reader = tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            print!("{}", ev.to_sse_frame());
        }
    });

    // 6. Lance la Mission A.
    let mut metas = HashMap::new();
    metas.insert(
        "claude_mock".to_string(),
        LlmMetadata {
            cost_input_per_million: 1500,
            cost_output_per_million: 7500,
            // R3.S1 : 0 → _resolved() fallback à cents × 1000.
            cost_input_per_million_microcents: 0,
            cost_output_per_million_microcents: 0,
            p50_latency_ms: 500,
            p99_latency_ms: 2000,
            success_rate: 0.99,
            capability_scores: HashMap::new(),
            circuit_open: false,
        },
    );
    let req = MissionARequest {
        user_text: "What did the Fed announce today?".into(),
        task_class: TaskClass::FactualQa,
        available_llms: vec!["claude_mock".into()],
        llm_metadata: metas,
        budget: Budget {
            max_cost_cents: 100,
            max_latency_ms: 5_000,
            min_quality_score: 0.5,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
        task_class_scoring_table: None,
    };
    let resp = run_mission_a(&coord, req).await?;

    // 7. Stop le heartbeat AVANT de drop le coordinator (sinon la task
    //    garde un Arc<bus> vivant et le reader ne reçoit jamais None).
    hb.stop().await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(coord); // ferme tous les channels
    let _ = reader.await;

    eprintln!("\n=== RÉSULTAT ===");
    eprintln!("LLM utilisé    : {}", resp.llm_used);
    eprintln!("Texte final    : {}", resp.final_text);
    eprintln!("Coût (cents)   : {}", resp.cost_cents);
    eprintln!("Latence (ms)   : {}", resp.latency_ms);
    eprintln!("Audit hash     : {}", &resp.audit_chain_hash[..16]);
    Ok(())
}
