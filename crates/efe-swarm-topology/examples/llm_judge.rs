// SPDX-License-Identifier: UNLICENSED
//! # `llm_judge` — juge **Mode C** (LLM) qui implémente [`Judge`] (Phase 3 inc.4).
//!
//! Un [`LlmJudge`] interroge un `LlmClient` pour SCORER chaque candidat → ballot.
//! Il se branche directement dans [`ConsensusTopology`] (Borda/Condorcet) comme un
//! juge Mode A. Démo keyless (`BuiltinLlmClient`) ; le seam réseau (llama-server)
//! est identique à `t1_slm`. ⚠ La QUALITÉ réelle du juge (atteint-il v ≥ 0,7 ? —
//! le gate d'ADR-SWARM-004) se mesure avec un VRAI modèle (llama), pas le builtin.
//!
//! Run :  cargo run --example llm_judge -p efe-swarm-topology

use std::sync::Arc;

use async_trait::async_trait;
use efe_nano::runtime::llm_client::{BuiltinLlmClient, LlmCallRequest, LlmClient};
use efe_nano_sdk::TenantId;
use efe_swarm_topology::{ConsensusTopology, Generator, Judge, Pattern, SwarmContext, Topology};
use serde_json::{json, Value};

/// Juge Mode C : score chaque candidat via un appel `LlmClient`.
struct LlmJudge {
    client: Arc<dyn LlmClient>,
    tenant: TenantId,
    name: String,
}
impl LlmJudge {
    fn new(name: impl Into<String>, client: Arc<dyn LlmClient>) -> Self {
        Self { client, tenant: TenantId::system(), name: name.into() }
    }
}
#[async_trait]
impl Judge for LlmJudge {
    async fn assess(&self, _task: &Value, candidates: &[Value]) -> Vec<f64> {
        let mut scores = Vec::with_capacity(candidates.len());
        for c in candidates {
            let text = c.get("value").and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| c.to_string());
            // En réel : un PROMPT DE JUGE (« note la qualité de cette réponse 0-1 »).
            // Keyless : on réutilise classify_topic et on prend la `confidence` comme proxy.
            let req = LlmCallRequest::builder(self.tenant.clone(), "m50.classify_topic.v1")
                .template_inputs(json!({ "text": text }))
                .max_tokens(32)
                .build();
            let score = match self.client.call(req).await {
                Ok(r) => r
                    .content
                    .get("confidence")
                    .and_then(|v| v.as_f64())
                    .or_else(|| r.content.get("intensity").and_then(|v| v.as_f64()))
                    .unwrap_or(0.5),
                Err(_) => 0.5,
            };
            scores.push(score);
        }
        scores
    }
    fn name(&self) -> &str {
        &self.name
    }
}

/// Générateur de démo : variants 0..n_correct corrects ; le reste leurre.
struct DemoGen;
#[async_trait]
impl Generator for DemoGen {
    async fn generate(&self, task: &Value, variant: usize) -> Value {
        let n = task.get("n_correct").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        if variant < n {
            json!({ "value": task.get("expected").cloned().unwrap_or(Value::Null) })
        } else {
            json!({ "value": "banana banana" })
        }
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let client: Arc<dyn LlmClient> = Arc::new(BuiltinLlmClient::new());

    println!("══════════════════════════════════════════════════════════════════════════");
    println!(" JUGE MODE C (LlmClient) — implémente le trait Judge, branché dans ConsensusTopology");
    println!("══════════════════════════════════════════════════════════════════════════");

    // 1. démonstration : LlmJudge.assess produit un ballot via de vrais appels LLM.
    let judge = LlmJudge::new("llm_judge", client.clone());
    let candidates = vec![
        json!({ "value": "the new ai software ships code" }),
        json!({ "value": "the stock market drives trading" }),
        json!({ "value": "banana banana" }),
    ];
    let ballot = judge.assess(&json!({}), &candidates).await;
    println!("\n LlmJudge.assess (scores via appels LLM réels) :");
    for (c, s) in candidates.iter().zip(&ballot) {
        println!("   {:<40} → {:.3}", c["value"].as_str().unwrap_or("?"), s);
    }

    // 2. branché dans ConsensusTopology : 3 juges Mode C + Borda.
    let judges: Vec<Arc<dyn Judge>> = (0..3)
        .map(|i| Arc::new(LlmJudge::new(format!("llm_{i}"), client.clone())) as Arc<dyn Judge>)
        .collect();
    let topo = ConsensusTopology::new(Arc::new(DemoGen), 7, judges).with_pattern(Pattern::Borda);
    let task = json!({ "expected": "ai software code", "n_correct": 3 });
    let out = topo.run(&task, &SwarmContext::standard()).await;
    println!("\n ConsensusTopology(Borda) avec 3 juges Mode C :");
    println!("   candidats={} · ballots={} · sélectionné={}", out.quality.candidates, out.quality.subtasks, out.output["value"]);

    println!("\n──────────────────────────────────────────────────────────────────────────");
    println!(" → Le juge Mode C produit un ballot via LlmClient et se branche dans la");
    println!("   topologie consensus comme un juge Mode A. QUALITÉ réelle (v≥0,7) = run llama.");
    println!("══════════════════════════════════════════════════════════════════════════");
}
