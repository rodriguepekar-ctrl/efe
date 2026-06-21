// SPDX-License-Identifier: UNLICENSED
//! # `run_bench` — B1.7 : l'entrée bout-en-bout, prête à pointer sur le GPU.
//!
//! Charge un dataset labellisé, balaie K pour l'essaim (la **courbe** justesse × coût) + une
//! référence gros-modèle (K=1), à dataset/scorer/budget identiques (I-BENCH-FAIR), imprime la table
//! comparative, puis **scelle + re-prouve** le bundle (I-BENCH-REPROVE) et imprime sa racine de
//! manifeste. **L'unique aiguillage** est `BenchBackend::SelfHost { endpoint, model }** : sur la box
//! GPU, vLLM écoute en loopback et le harnais — déjà prouvé juste sur Mac (cf. `tests/bench_mac_seam`)
//! — n'a plus qu'à mesurer du vrai. Compile sur Mac ; ne MESURE qu'avec un endpoint réel (B2).
//!
//! Run (sur la box GPU) :
//!   cargo run --release --example run_bench -p efe-bench-inference -- \
//!     --endpoint http://127.0.0.1:8000/v1 --model qwen2.5:3b --ks 4,8,16,24 \
//!     --dataset crates/efe-bench-inference/fixtures/topics.jsonl

use efe_bench_inference::{
    consensus_sweep, run_config, BenchBackend, BenchBundle, BenchConfig, Comparison, Dataset, PriceModel,
};
use efe_swarm_sdk::{KSpec, TopologySpec};

const CLASSIFY: &str = "Classify the topic in one word.";
const JUDGES: &[&str] = &["consistency"];

/// Récupère `--clé valeur` depuis les arguments (parsing minimal, sans dépendance clap).
fn arg(flag: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1).cloned())
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = arg("--endpoint").unwrap_or_else(|| "http://127.0.0.1:8000/v1".to_string());
    let model = arg("--model").unwrap_or_else(|| "qwen2.5:3b".to_string());
    let ks: Vec<usize> = arg("--ks")
        .unwrap_or_else(|| "4,8,16,24".to_string())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let dataset_path = arg("--dataset");
    let concurrency: usize = arg("--concurrency").and_then(|s| s.parse().ok()).unwrap_or(8);
    let budget_cents: u32 = arg("--budget-cents").and_then(|s| s.parse().ok()).unwrap_or(1_000_000);

    // Le dataset : depuis un fichier, ou le corpus de départ embarqué (réponses fermées).
    let dataset = match &dataset_path {
        Some(p) => Dataset::from_jsonl(&std::fs::read_to_string(p)?)?,
        None => Dataset::from_jsonl(include_str!("../fixtures/topics.jsonl"))?,
    };
    println!("dataset : {} cas (digest {})", dataset.len(), &dataset.digest()[..16]);

    // L'UNIQUE aiguillage vers le réel. Sur Mac, le test seam injecte un ScriptedBackend à la place.
    let backend = BenchBackend::SelfHost { endpoint: endpoint.clone(), model: model.clone() };
    println!("backend : self-host {endpoint} · modèle {model}\n");

    // La thèse : balayage de K (courbe de l'essaim) + une référence gros-modèle (K=1), même dataset.
    let mut configs = consensus_sweep(CLASSIFY, JUDGES, &ks);
    configs.push(BenchConfig::new(
        "baseline-k1",
        TopologySpec::GenerateVerify { generator: CLASSIFY.into(), verifier: "non_empty_text".into(), k: KSpec::Fixed(1) },
    ));

    let mut reports = Vec::new();
    for cfg in &configs {
        let report = run_config(&dataset, cfg, &backend, budget_cents, concurrency).await?;
        println!(
            "  {:<14} exactitude {:>5.1}%  jetons/moy {:>7.1}  R0 {:>3.0}%",
            cfg.name,
            report.accuracy * 100.0,
            report.mean_tokens,
            report.verified_pct * 100.0
        );
        reports.push(report);
    }

    // Le coût comparable : un modèle de prix (¢/1k jetons) — petit modèle ≪ gros modèle. À ajuster.
    let mut price = PriceModel::new().with("baseline-k1", 4.0);
    for &k in &ks {
        price = price.with(format!("swarm-k{k}"), 0.2);
    }

    println!("\n{}", Comparison::new(reports.clone()).with_price(price).table());

    // I-BENCH-REPROVE : le benchmark est lui-même dans le moat.
    let bundle = BenchBundle::seal(&dataset, reports);
    let verdict = bundle.verify();
    println!("bundle : racine de manifeste {} · re-prouvable = {}", &bundle.manifest_root[..16], verdict.ok);
    if !verdict.ok {
        eprintln!("AVERTISSEMENT : bundle non re-prouvable : {verdict:?}");
    }

    Ok(())
}
