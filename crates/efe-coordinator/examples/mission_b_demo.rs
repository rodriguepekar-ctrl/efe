// SPDX-License-Identifier: UNLICENSED
//! Démo Mission B : fan-out vers 3 LLMs mock + vote consensus.
//!
//! Usage :
//! ```bash
//! PYO3_PYTHON=$(which python3.12) \
//!   cargo run -p efe-coordinator --example mission_b_demo
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{
    run_mission_b, Coordinator, MissionBRequest, MockLlmProvider, ProviderRegistry,
};
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::LlmMetadata;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 3 LLMs mock, 2 d'accord (claude+gpt4), 1 dissident (gemini).
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("claude").with_response("Paris is the capital of France."),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("gpt4").with_response("Paris is the capital of France."),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("gemini").with_response("Lyon is a major French city."),
    ));

    let coord = Coordinator::builder().registry(reg).build()?;
    let (_token, mut rx) = coord.subscribe(Some("demo-b"));

    let reader = tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            // Filtre les tokens (trop verbeux) ; garde les évènements de haut niveau.
            if !matches!(ev.tag(), "LLM_TOKEN_STREAMED" | "HEARTBEAT") {
                println!("{}", ev.to_sse_frame());
            }
        }
    });

    let mut metas = HashMap::new();
    for id in ["claude", "gpt4", "gemini"] {
        metas.insert(
            id.to_string(),
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
    }
    let req = MissionBRequest {
        user_text: "What is the capital of France?".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["claude".into(), "gpt4".into(), "gemini".into()],
        llm_metadata: metas,
        vote_weights: HashMap::new(), // uniforme
        quorum: Some(2),
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
    };
    let resp = run_mission_b(&coord, req).await?;

    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(coord);
    let _ = reader.await;

    eprintln!("\n=== RÉSULTAT MISSION B ===");
    eprintln!("Gagnant         : {}", resp.winner_llm);
    eprintln!("Texte consensus : {}", resp.final_text);
    eprintln!("Score gagnant   : {:.2}", resp.winner_score);
    eprintln!("Scores per-LLM  :");
    for (k, v) in &resp.per_llm_scores {
        eprintln!("  {k:15} = {v:.2}");
    }
    eprintln!("Coût total      : {} cents", resp.total_cost_cents);
    eprintln!("Latence         : {} ms", resp.latency_ms);
    Ok(())
}
