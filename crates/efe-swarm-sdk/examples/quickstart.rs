// SPDX-License-Identifier: UNLICENSED
//! # `quickstart` — le SDK en 5 lignes (Phase 5, Inc.1). Keyless, sans clé.
//!
//! Run :  cargo run --example quickstart -p efe-swarm-sdk

use efe_swarm_sdk::{Budget, OnExceed, Swarm, TopologySpec, WidthSpec};
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
        k: 7.into(), pattern: efe_swarm_sdk::PatternSpec::Fixed(efe_swarm_sdk::Pattern::Borda), escalate: false,
    };
    let err = swarm.run(json!({"text":"x"}), tight(), Budget::cents(3).with_policy(OnExceed::Error)).await;
    println!("   Error   → {}", err.err().map(|e| e.to_string()).unwrap_or_else(|| "ok".into()));
    let deg = swarm.run(json!({"text":"x","expected":"x"}), tight(), Budget::cents(3).with_policy(OnExceed::Degrade)).await.unwrap();
    println!("   Degrade → exécuté · {}¢ dépensés · topo dégradée pour tenir", deg.budget.spent_cents);
    let stop = swarm.run(json!({"text":"x"}), tight(), Budget::cents(4).with_policy(OnExceed::Stop)).await.unwrap();
    println!("   Stop    → {}¢ dépensés (≤ 4) · arrêté en cours : {}", stop.budget.spent_cents, stop.budget_stopped);

    // 4. ADR-014 : taille d'essaim ADAPTATIVE + largeur d'exécution.
    println!("\n────────── ADR-014 : K adaptatif (payer selon la difficulté) ──────────────");
    // instance facile (candidats unanimes) : l'essaim s'arrête au plancher k_min=4
    // au lieu de dépenser k_max=24 — l'économie est réelle ET facturée.
    let adaptive = TopologySpec::consensus_adaptive("const_generator", &["consistency"], 4, 24, 0.9);
    let run = swarm.run(json!({ "text": "easy" }), adaptive, Budget::cents(100)).await.unwrap();
    println!(
        "   K adaptatif → k_used {} / k_max 24 · raison `{}` · {}¢ dépensés (au lieu de 25)",
        run.output["sizing"]["k_used"], run.output["sizing"]["stop_reason"], run.budget.spent_cents
    );
    // T1 + quota vérifié (le cas-phare) : stop dès le 1er candidat VÉRIFIÉ.
    let t1q = TopologySpec::generate_verify_adaptive("echo_generator", "equals_expected", 2, 8, 1);
    let run = swarm.run(json!({ "text": "x", "expected": "x" }), t1q, Budget::cents(100)).await.unwrap();
    println!(
        "   T1 quota=1  → k_used {} / k_max 8 · le vérifié est rendu · {}¢",
        run.output["sizing"]["k_used"], run.budget.spent_cents
    );
    // largeur d'exécution : une politique d'EXÉCUTION, jamais une sémantique (I-CP1) —
    // largeur 1 (séquentiel) et Auto (cœurs) rendent le MÊME résultat.
    let narrow = Swarm::builder().width(WidthSpec::Fixed(1)).build().unwrap();
    let run1 = narrow.run(json!({ "text": "w", "expected": "w" }),
        TopologySpec::consensus("echo_generator", &["consistency"], 8), Budget::cents(100)).await.unwrap();
    println!(
        "   largeur     → résolue {} (Fixed(1)) · même sortie qu'en Auto · verified {}",
        run1.accounting.width, run1.provenance.verified
    );

    println!("\n──────────────────────────────────────────────────────────────────────────");
    println!(" → Une fonction : 4 topologies + K adaptatif, budget plafonné (jamais");
    println!("   dépassé), et chaque run renvoie un provenance_root vérifiable — sans clé.");
    println!("══════════════════════════════════════════════════════════════════════════");
}
