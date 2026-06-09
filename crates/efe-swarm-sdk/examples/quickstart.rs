// SPDX-License-Identifier: UNLICENSED
//! # `quickstart` — le SDK en 5 lignes (Phase 5, Inc.1). Keyless, sans clé.
//!
//! Run :  cargo run --example quickstart -p efe-swarm-sdk

use efe_swarm_sdk::{Budget, OnExceed, Swarm, TopologySpec};
use serde_json::json;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    // 1. un essaim keyless (flotte Mode A, provenance activée).
    let swarm = Swarm::keyless();

    println!("══════════════════════════════════════════════════════════════════════════");
    println!(" EFE SWARM SDK — swarm.run(task, topology, budget) · keyless, sans clé");
    println!("══════════════════════════════════════════════════════════════════════════");

    // 2. quatre topologies, une seule API.
    let cases = vec![
        ("Auto (vérifiable)", json!({ "text": "hello", "expected": "hello" }), TopologySpec::Auto, Budget::cents(100)),
        ("T-Consensus", json!({ "text": "abc" }),
            TopologySpec::consensus("echo_generator", &["consistency", "length_band", "j_non_empty"], 5), Budget::cents(100)),
        ("T2 Pipeline", json!({ "text": "  Hello   World  " }),
            TopologySpec::Pipeline { stages: vec!["normalize_whitespace".into(), "uppercase".into()] }, Budget::cents(100)),
        ("T3 Hiérarchique", json!({ "text": "alpha beta gamma" }),
            TopologySpec::Hierarchical { decomposer: "word_split".into(), solver: "uppercase".into(), merger: "join_lines".into() }, Budget::cents(100)),
    ];

    for (label, task, spec, budget) in cases {
        let run = swarm.run(task, spec, budget).await.expect("run");
        println!(
            "\n {:<20} [{}]\n   sortie   : {}\n   coût     : {}¢ / {}¢ (jamais dépassé)\n   provenance: vérifié={} · root={}…",
            label, run.topology,
            serde_json::to_string(&run.output).unwrap().chars().take(54).collect::<String>(),
            run.budget.spent_cents, run.budget.max_cost_cents,
            run.provenance.verified,
            run.provenance.root.as_deref().map(|r| &r[..16]).unwrap_or("—"),
        );
    }

    // 3. gouvernance de budget : 3 politiques.
    println!("\n────────── gouvernance de budget (plafond serré : 3¢ @ 1¢/appel) ──────────");
    let tight = || TopologySpec::Consensus {
        generator: "echo_generator".into(),
        judges: vec!["consistency".into(), "length_band".into(), "j_non_empty".into()],
        k: 7, pattern: efe_swarm_sdk::Pattern::Borda, escalate: false,
    };
    let err = swarm.run(json!({"text":"x"}), tight(), Budget::cents(3).with_policy(OnExceed::Error)).await;
    println!("   Error   → {}", err.err().map(|e| e.to_string()).unwrap_or_else(|| "ok".into()));
    let deg = swarm.run(json!({"text":"x","expected":"x"}), tight(), Budget::cents(3).with_policy(OnExceed::Degrade)).await.unwrap();
    println!("   Degrade → exécuté · {}¢ dépensés · topo dégradée pour tenir", deg.budget.spent_cents);
    let stop = swarm.run(json!({"text":"x"}), tight(), Budget::cents(4).with_policy(OnExceed::Stop)).await.unwrap();
    println!("   Stop    → {}¢ dépensés (≤ 4) · arrêté en cours : {}", stop.budget.spent_cents, stop.budget_stopped);

    println!("\n──────────────────────────────────────────────────────────────────────────");
    println!(" → Une fonction : 4 topologies, budget plafonné (jamais dépassé), et chaque");
    println!("   run renvoie un provenance_root que ton régulateur vérifie — sans clé.");
    println!("══════════════════════════════════════════════════════════════════════════");
}
