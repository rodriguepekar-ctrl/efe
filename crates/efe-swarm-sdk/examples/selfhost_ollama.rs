// SPDX-License-Identifier: UNLICENSED
//! # `selfhost_ollama` — le SDK en mode **Self-host RÉEL** (Ollama local, keyless).
//!
//! `ClientMode::SelfHost` fait tourner de **VRAIS agents LLM** : dans `TopologySpec`, le
//! *nom* d'un agent devient une **instruction** en langage naturel. Une seule API —
//! `swarm.run` — plafonnée en coût et auto-tracée (provenance_root re-vérifiable).
//!
//! Prérequis : Ollama sur `http://127.0.0.1:11434` avec `gemma3:12b`.
//! Run :  cargo run --release --example selfhost_ollama -p efe-swarm-sdk

use efe_swarm_sdk::{Budget, ClientMode, Pattern, Swarm, TopologySpec};
use serde_json::json;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    // SDK en mode self-host : agents LLM Ollama, KEYLESS (aucune clé).
    let swarm = Swarm::builder()
        .mode(ClientMode::SelfHost { endpoint: "http://127.0.0.1:11434".into(), model: "gemma3:12b".into() })
        .build()
        .unwrap();

    println!("══════════════════════════════════════════════════════════════════════════");
    println!(" EFE SWARM SDK — Self-host RÉEL (Ollama gemma3:12b, keyless) · swarm.run");
    println!("══════════════════════════════════════════════════════════════════════════");

    // Consensus avec de VRAIS agents : les « noms » sont des INSTRUCTIONS.
    let cases = [
        ("The court ruled the contract clause was unenforceable.", "legal"),
        ("The central bank raised interest rates to curb inflation.", "finance"),
        ("Clinical trials showed the vaccine reduces hospitalizations.", "health"),
    ];

    for (text, expected) in cases {
        let run = swarm
            .run(
                json!({ "text": text, "expected": expected }),
                TopologySpec::Consensus {
                    generator: "Classify the topic of the text as exactly one word from {technology, finance, health, legal}.".into(),
                    judges: vec![
                        "Judge how well the proposed topic label fits the text.".into(),
                        "Assess the relevance of the topic to the text.".into(),
                    ],
                    k: 3,
                    pattern: Pattern::Borda,
                    escalate: false,
                },
                Budget::cents(100),
            )
            .await
            .expect("run");

        let got = run.output.get("value").and_then(|v| v.as_str()).unwrap_or("?");
        println!(
            "\n « {:.42}… »\n   consensus = {:<10} (attendu {}) {}\n   {} appels · provenance: vérifié={} · root={}…",
            text, got, expected, if got.eq_ignore_ascii_case(expected) { "✓" } else { "≈" },
            run.accounting.agents_used, run.provenance.verified,
            run.provenance.root.as_deref().map(|r| &r[..16]).unwrap_or("—"),
        );
    }

    println!("\n──────────────────────────────────────────────────────────────────────────");
    println!(" → Le MÊME swarm.run, mode self-host : vrais agents LLM keyless, budget");
    println!("   plafonné, et chaque run re-prouvé par verify_swarm — sans aucune clé.");
    println!("══════════════════════════════════════════════════════════════════════════");
}
