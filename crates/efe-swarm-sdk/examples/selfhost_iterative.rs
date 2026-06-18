// SPDX-License-Identifier: UNLICENSED
//! # Self-host ITÉRATIF — Gossip & Debate sur un SLM local (Mode C, keyless).
//!
//! Le runbook Mode C des topologies d'influence mutuelle : les « noms » d'agents
//! deviennent des INSTRUCTIONS au modèle local — le refiner révise *s'il est
//! convaincu par les pairs* (anti-chambre-d'écho), les arguers plaident PRO/CONTRA,
//! le jury note sur dossier. Zéro clé ; garde-fou G2 (endpoint local) ; chaque run
//! est scellé, re-prouvé, et son graphe d'influence (`reads`) est dans les records.
//!
//! Pré-requis : un Ollama local servant un modèle (ex. `ollama pull gemma3:12b`).
//! Sans Ollama vivant, l'exemple le DIT et sort proprement (health-check S.a).
//!
//! Run : `cargo run --release --example selfhost_iterative -p efe-swarm-sdk`
//!       (env optionnel : EFE_OLLAMA_ENDPOINT, EFE_OLLAMA_MODEL)

use efe_swarm_sdk::{Budget, ClientMode, ConvergenceMetric, Pattern, Swarm, TopologySpec};
use serde_json::json;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let endpoint =
        std::env::var("EFE_OLLAMA_ENDPOINT").unwrap_or_else(|_| "http://127.0.0.1:11434".into());
    let model = std::env::var("EFE_OLLAMA_MODEL").unwrap_or_else(|_| "gemma3:12b".into());
    println!("══════════════════════════════════════════════════════════════════════");
    println!(" Self-host itératif — {model} @ {endpoint} (Mode C, zéro clé)");
    println!("══════════════════════════════════════════════════════════════════════");

    let swarm = Swarm::builder()
        .mode(ClientMode::SelfHost { endpoint: endpoint.clone(), model })
        .require_healthy_backend(true) // S.a : échec net si Ollama est mort.
        .build()
        .expect("swarm self-host (keyless par construction)");

    // ── T4 Gossip : 4 estimateurs LLM convergent sur une estimation chiffrée. ──
    let gossip = TopologySpec::Gossip {
        refiner: "Estimate how many minutes a careful human needs to read a 12-page \
                  technical report. Respond with ONLY a number."
            .into(),
        k: 4,
        max_rounds: 3,
        epsilon: 4.0, // variance en minutes² — l'accord « à ±2 min près »
        metric: ConvergenceMetric::Variance,
    };
    println!("\n— Gossip (K=4, r≤3) : estimation par influence mutuelle —");
    match swarm.run_traced(json!({ "text": "reading-time estimation" }), gossip, Budget::cents(500)).await {
        Ok(out) => {
            println!("  estimation : {}", out.run.output["value"]);
            println!(
                "  tours : {} · résiduel : {} · convergé : {}",
                out.run.output["gossip"]["rounds"],
                out.run.output["gossip"]["residual"],
                out.run.output["gossip"]["converged"]
            );
            let reads: usize =
                out.sealed.as_ref().map(|s| s.records.iter().map(|r| r.reads.len()).sum()).unwrap_or(0);
            println!(
                "  preuve : verified={} · {} records · {} lectures scellées (graphe d'influence)",
                out.run.provenance.verified,
                out.run.provenance.records,
                reads
            );
            if let Some(n17) = &out.run.n17 {
                println!("  N17 : {} appels LLM, cohérent={}", n17.calls, n17.all_consistent);
            }
        }
        Err(e) => {
            println!("  backend indisponible ({e}) — lancez Ollama puis relancez. Sortie propre.");
            return;
        }
    }

    // ── Debate escaladé : payé seulement si le jury hésite. ────────────────────
    let debate = TopologySpec::Debate {
        generator: "Name the single biggest operational risk when autonomous agents \
                    run procurement. Respond with a short noun phrase."
            .into(),
        judges: vec![
            "Judge how plausible and specific the proposed risk is.".into(),
            "Assess whether the risk is truly the most material one.".into(),
        ],
        pro: "Argue FOR the candidate risk: cite its strongest concrete consequence.".into(),
        contra: "Argue AGAINST the candidate risk: name a stronger competing risk.".into(),
        k: 3,
        pattern: Pattern::Borda, // Debate : pattern fixe (l'adaptatif est consensus-only, ADR-019)
        rounds: 1,
        escalate: true, // Vote → jury → débat : l'argumentation n'est payée qu'à l'hésitation.
    };
    println!("\n— Debate escaladé (K=3, r≤1) : arbitrage subjectif sur dossier —");
    match swarm.run_traced(json!({ "text": "agentic procurement risk" }), debate, Budget::cents(500)).await {
        Ok(out) => {
            println!("  verdict : {}", out.run.output);
            println!(
                "  appels : {} (l'escalade dit si le débat a été payé) · verified={}",
                out.run.accounting.agents_used, out.run.provenance.verified
            );
        }
        Err(e) => println!("  backend indisponible ({e}) — sortie propre."),
    }
    println!("══════════════════════════════════════════════════════════════════════");
}
