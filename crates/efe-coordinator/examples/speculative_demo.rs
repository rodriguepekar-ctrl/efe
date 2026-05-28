// SPDX-License-Identifier: UNLICENSED
//! Démo Speculative : drafter local rapide + verifier gros LLM.
//!
//! Usage :
//! ```bash
//! PYO3_PYTHON=$(which python3.12) \
//!   cargo run -p efe-coordinator --example speculative_demo
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{
    run_speculative, Coordinator, MockLlmProvider, ProviderRegistry, SpeculativeRequest,
};
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Drafter rapide (1 ms simulé), verifier lent (40 ms simulé).
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("drafter")
            .with_response("The answer is 42 according to local guess.")
            .with_latency(Duration::from_millis(1)),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("claude")
            .with_response("The answer is 42 according to local guess.")
            .with_latency(Duration::from_millis(40)),
    ));

    let coord = Coordinator::builder().registry(reg).build()?;
    let (_token, mut rx) = coord.subscribe(Some("demo-spec"));

    let reader = tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            println!("{}", ev.to_sse_frame());
        }
    });

    let req = SpeculativeRequest {
        user_text: "What is the meaning of life?".into(),
        task_class: TaskClass::FactualQa,
        verifier_llm: "claude".into(),
        verifier_metadata: LlmMetadata {
            cost_input_per_million: 1500,
            cost_output_per_million: 7500,
            // R3.S1 : 0 → _resolved() fallback à cents × 1000.
            cost_input_per_million_microcents: 0,
            cost_output_per_million_microcents: 0,
            p50_latency_ms: 40,
            p99_latency_ms: 200,
            success_rate: 0.99,
            capability_scores: HashMap::new(),
            circuit_open: false,
        },
        budget: Budget {
            max_cost_cents: 50,
            max_latency_ms: 2_000,
            min_quality_score: 0.5,
        },
        max_draft_tokens: 64,
        max_total_latency: Duration::from_secs(3),
        drafter_llm: "drafter".to_string(),
        accept_threshold: 0.7,
        max_words_for_quick_accept: 0,
    };
    let resp = run_speculative(&coord, req).await?;

    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(coord);
    let _ = reader.await;

    eprintln!("\n=== RÉSULTAT SPECULATIVE ===");
    eprintln!("Texte final         : {}", resp.final_text);
    eprintln!("Texte drafté        : {}", resp.draft_text);
    eprintln!("Draft accepté ?     : {}", resp.draft_accepted);
    eprintln!("Tokens acceptés     : {}", resp.tokens_accepted);
    eprintln!("Tokens régénérés    : {}", resp.tokens_regenerated);
    eprintln!("Latence gagnée (ms) : {}", resp.latency_saved_ms);
    eprintln!("Latence totale (ms) : {}", resp.latency_ms);
    Ok(())
}
