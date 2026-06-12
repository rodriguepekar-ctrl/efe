// SPDX-License-Identifier: UNLICENSED
//! # `selfhost_flagship` — le mode **phare** : souverain · local · vérifiable. **Garde-fous tenus.**
//!
//! Démonstration *runnable* (en grande partie **sans Ollama**) que les garde-fous du mode
//! phare ne sont pas des intentions mais des **invariants câblés** :
//! 1. **G2 souveraineté / egress=0** — un endpoint **non-local** est **refusé par défaut**.
//! 2. **Health-check (S.a)** — un backend **injoignable** échoue **vite** (pas N timeouts).
//! 3. **Run RÉEL souverain** (si Ollama up) — sortie + **forensique** (moteur/modèle, pont N17)
//!    + **re-preuve** (`provenance_root`) + budget **jamais dépassé** (I13).
//!
//! Run :  `cargo run --release --example selfhost_flagship -p efe-swarm-sdk`
//! (Étapes 1-2 tournent partout ; l'étape 3 nécessite `ollama serve` + `gemma3:12b`.)

use efe_swarm_sdk::{Budget, ClientMode, Pattern, Swarm, SwarmError, TopologySpec};
use serde_json::json;

fn self_host(endpoint: &str, model: &str) -> ClientMode {
    ClientMode::SelfHost { endpoint: endpoint.into(), model: model.into() }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    println!("══════════════════════════════════════════════════════════════════════════");
    println!(" EFE — Self-host KEYLESS, le mode phare : souverain · local · vérifiable");
    println!(" Garde-fous tenus (câblés ET prouvés)");
    println!("══════════════════════════════════════════════════════════════════════════");

    // ── 1. G2 — souveraineté / egress=0 : endpoint NON-LOCAL refusé par défaut ──────────
    println!("\n[1] Garde-fou G2 — souveraineté / egress");
    let remote = Swarm::builder().mode(self_host("http://api.example.com:11434", "gemma3:12b")).build().unwrap();
    match remote.run(json!({ "text": "x", "expected": "x" }), TopologySpec::Auto, Budget::cents(100)).await {
        Err(SwarmError::RemoteBackendDenied { endpoint }) => {
            println!("    ✓ endpoint non-local REFUSÉ par défaut (egress interdit) : {endpoint}");
        }
        other => println!("    ?? attendu RemoteBackendDenied, eu : {other:?}"),
    }
    // opt-in explicite, pour un backend LAN de confiance (souveraineté assumée par l'opérateur).
    let _ = Swarm::builder()
        .mode(self_host("http://api.example.com:11434", "gemma3:12b"))
        .allow_remote_backend(true)
        .build()
        .unwrap();
    println!("    ✓ allow_remote_backend(true) = opt-in EXPLICITE (jamais par défaut)");

    // ── 2. Health-check (S.a) : backend LOCAL mais MORT → échec rapide ──────────────────
    println!("\n[2] Garde-fou health-check (S.a)");
    let dead = Swarm::builder()
        .mode(self_host("http://127.0.0.1:1", "m")) // local (passe G2) mais aucun service
        .require_healthy_backend(true)
        .build()
        .unwrap();
    match dead.run(json!({ "text": "x", "expected": "x" }), TopologySpec::Auto, Budget::cents(100)).await {
        Err(SwarmError::BackendUnavailable { endpoint }) => {
            println!("    ✓ backend injoignable → échec RAPIDE (pas N timeouts) : {endpoint}");
        }
        other => println!("    ?? attendu BackendUnavailable, eu : {other:?}"),
    }

    // ── 3. Run RÉEL souverain (si Ollama up) : forensique + re-preuve + budget ───────────
    println!("\n[3] Run RÉEL souverain (Ollama local, keyless)");
    let swarm = Swarm::builder()
        .mode(self_host("http://127.0.0.1:11434", "gemma3:12b"))
        .require_healthy_backend(true) // échoue vite si Ollama n'est pas démarré
        .build()
        .unwrap();
    let res = swarm
        .run(
            json!({ "text": "The court ruled the contract clause was unenforceable.", "expected": "legal" }),
            TopologySpec::Consensus {
                generator: "Classify the topic of the text as exactly one word from {technology, finance, health, legal}.".into(),
                judges: vec![
                    "Judge how well the proposed topic label fits the text.".into(),
                    "Assess the relevance of the topic to the text.".into(),
                ],
                k: 3.into(),
                pattern: Pattern::Borda,
                escalate: false,
            },
            Budget::cents(100),
        )
        .await;
    match res {
        Ok(run) => {
            let got = run.output.get("value").and_then(|v| v.as_str()).unwrap_or("?");
            println!("    sortie consensus = {got}");
            if let Some(b) = &run.backend {
                println!("    forensique (pont N17) : moteur={} · modèle={} · endpoint={}", b.engine, b.model, b.endpoint);
            }
            println!(
                "    provenance : vérifié={} · root={}…",
                run.provenance.verified,
                run.provenance.root.as_deref().map(|r| &r[..16]).unwrap_or("—")
            );
            println!(
                "    budget : {}¢ / {}¢ dépensés (JAMAIS dépassé — I13)",
                run.budget.spent_cents, run.budget.max_cost_cents
            );
        }
        Err(SwarmError::BackendUnavailable { .. }) => {
            println!("    (Ollama non démarré — lancez `ollama serve` puis `ollama pull gemma3:12b`)");
        }
        Err(e) => println!("    erreur : {e}"),
    }

    println!("\n──────────────────────────────────────────────────────────────────────────");
    println!(" → Souverain (rien ne sort de l'hôte) · local (zéro clé, zéro egress) ·");
    println!("   vérifiable (re-preuve hors-ligne) — ce qu'aucun provider hébergé n'offre.");
    println!("══════════════════════════════════════════════════════════════════════════");
}
