// SPDX-License-Identifier: UNLICENSED
//! # `swarm_async_slm` — agent Mode C ASYNCHRONE dans le streaming swarm (Phase 2).
//!
//! Démontre le **handler async** (`StreamingSwarm::async_agent`) branché sur un
//! VRAI `LlmClient` (Mode C) : chaque texte injecté déclenche un appel SLM
//! asynchrone (concurrence bornée), dont le verdict est écrit au Blackboard. La
//! terminaison reste détectée par QUIESCENCE (crédit), inchangée par l'asynchronie.
//!
//! Backend keyless par défaut (`BuiltinLlmClient`). Le seam réseau (llama-server)
//! est identique à `swarm_slm.rs`.
//!
//! Run :
//! ```text
//! cargo run --example swarm_async_slm -p efe-blackboard            # 24 textes, conc 8
//! cargo run --example swarm_async_slm -p efe-blackboard -- 64 16
//! ```

use std::sync::Arc;

use efe_blackboard::{Blackboard, StreamingSwarm, TopicRouter};
use efe_nano::runtime::llm_client::{BuiltinLlmClient, LlmCallRequest, LlmClient};
use efe_nano_sdk::{AgentId, SwarmId, TenantId};
use serde_json::json;

const CORPUS: &[&str] = &[
    "the new ai software ships faster code to the internet",
    "the stock market and trading drive investment returns",
    "doctor and patient discuss the medical treatment plan",
    "the court judge reviewed the contract under the law",
    "an election and government policy split the vote",
    "the team player scored in the championship match",
];

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(24);
    let conc: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(8);

    let bb = Arc::new(Blackboard::new(SwarmId::new()));
    let router = Arc::new(TopicRouter::new((n * 2).max(256)));
    let mut swarm = StreamingSwarm::new(bb.clone(), router);

    // Le SLM (trait LlmClient réel, backend keyless).
    let client: Arc<dyn LlmClient> = Arc::new(BuiltinLlmClient::new());
    let tenant = TenantId::system();

    println!("══════════════════════════════════════════════════════════════");
    println!(" STREAMING + AGENT MODE C ASYNC — {n} textes · concurrence {conc}");
    println!(" agent = async_agent → LlmClient::call(m50.classify_topic.v1)");
    println!("══════════════════════════════════════════════════════════════");

    // Agent ASYNC : appelle le SLM puis écrit le verdict au Blackboard.
    swarm.async_agent(
        AgentId::new("classifier").unwrap(),
        "text",
        conc,
        Arc::new(move |ctx, ev| {
            let client = client.clone();
            let tenant = tenant;
            Box::pin(async move {
                let text = ev.payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let id = ev.payload.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
                let req = LlmCallRequest::builder(tenant, "m50.classify_topic.v1")
                    .template_inputs(json!({ "text": text }))
                    .max_tokens(48)
                    .build();
                let topic = match client.call(req).await {
                    Ok(r) => r
                        .content
                        .get("topic")
                        .and_then(|v| v.as_str())
                        .unwrap_or("general")
                        .to_string(),
                    Err(_) => "general".to_string(),
                };
                let _ = ctx.blackboard().write(
                    &format!("/verdict/{id}"),
                    json!({ "topic": topic }),
                    ctx.agent_id().clone(),
                );
            })
        }),
    );

    for i in 0..n {
        let text = CORPUS[i % CORPUS.len()];
        swarm.inject("text", json!({ "id": i, "text": text }));
    }
    let report = swarm.run_until_quiescent(std::time::Duration::from_secs(30)).await;

    // Compte les topics émergés.
    let snap = bb.snapshot();
    let mut counts: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    for (k, v) in snap.value_map() {
        if k.starts_with("/verdict/") {
            if let Some(t) = v.get("topic").and_then(|x| x.as_str()) {
                *counts.entry(t.to_string()).or_default() += 1;
            }
        }
    }

    println!("\n verdicts SLM écrits : {}", counts.values().sum::<u64>());
    println!(" quiescence atteinte : {} en {:.1} ms", report.quiesced, report.elapsed.as_secs_f64() * 1000.0);
    println!(" inférences async    : {} (concurrence ≤ {conc})", report.consumed);
    println!(" distribution topics :");
    for (t, c) in &counts {
        println!("   {t:<12} {c}");
    }
    println!("══════════════════════════════════════════════════════════════");
    println!("→ Agents SLM Mode C exécutés en parallèle dans le streaming swarm,");
    println!("  concurrence bornée, terminaison par quiescence. (Phase 2, livrable 1.)");
}
