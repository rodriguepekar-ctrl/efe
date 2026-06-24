// SPDX-License-Identifier: UNLICENSED
//! # `run_bench` — B1.7 + **PoC de la thèse** : essaim de petits modèles vs gros modèle (réel).
//!
//! Charge un dataset labellisé, balaie K pour l'**essaim** sur le **petit** modèle (la courbe
//! justesse × coût), exécute une **référence** K=1 sur le **gros** modèle, à dataset/scorer/budget
//! identiques (I-BENCH-FAIR), imprime la table comparative + le **verdict de domination** (la thèse
//! tranchée, §7), puis **scelle ET re-prouve** le bundle (re-preuve indépendante : on re-dérive la
//! preuve de chaque run depuis les records bruts + la clé tenant). Vraie inférence via Ollama/vLLM ;
//! aucune réponse n'est scriptée.
//!
//! Le seul aiguillage vs la preuve-sur-Mac (seam) est `BenchBackend::SelfHost`. Endpoint LOCAL exigé
//! par le garde-fou de souveraineté du SDK (boucle locale).
//!
//! PoC thèse (Ollama, petit=gemma3:1b vs gros=gemma3:12b) :
//!   cargo run --example run_bench -p efe-bench-inference -- \
//!     --endpoint http://127.0.0.1:11434 --model gemma3:1b --baseline-model gemma3:12b \
//!     --ks 1,3,5 --lenient

use efe_bench_inference::{
    consensus_sweep, reprove, run_config_with_evidence, BenchBackend, BenchBundle, BenchConfig, Comparison, Dataset,
    PriceModel, RunEvidence, ScorerKind,
};
use efe_nano_sdk::TenantId;
use efe_swarm_sdk::{HmacKeySource, KSpec, TopologySpec};

/// Récupère `--clé valeur` (parsing minimal, sans dépendance clap).
fn arg(flag: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1).cloned())
}

/// Présence d'un drapeau booléen `--flag`.
fn has_flag(flag: &str) -> bool {
    std::env::args().any(|a| a == flag)
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = arg("--endpoint").unwrap_or_else(|| "http://127.0.0.1:11434".to_string());
    let small = arg("--model").unwrap_or_else(|| "gemma3:1b".to_string());
    let big = arg("--baseline-model").unwrap_or_else(|| small.clone());
    let ks: Vec<usize> = arg("--ks")
        .unwrap_or_else(|| "1,3,5".to_string())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    // Prompt STRICT : la tâche donne l'ensemble des catégories (cadre légitime), jamais le label du cas.
    let instruction = arg("--instruction").unwrap_or_else(|| {
        "Classify the topic of the text. Reply with ONE lowercase word chosen from: finance, sport, \
         health, science. Output only that single word."
            .to_string()
    });
    let lenient = has_flag("--lenient");
    let concurrency: usize = arg("--concurrency").and_then(|s| s.parse().ok()).unwrap_or(2);
    let budget_cents: u32 = arg("--budget-cents").and_then(|s| s.parse().ok()).unwrap_or(1_000_000);

    let mut dataset = match arg("--dataset") {
        Some(p) => Dataset::from_jsonl(&std::fs::read_to_string(p)?)?,
        None => Dataset::from_jsonl(include_str!("../fixtures/topics.jsonl"))?,
    };
    if lenient {
        for c in &mut dataset.cases {
            c.scorer = ScorerKind::Lenient; // éval de vrais modèles : forme ignorée, fond mesuré.
        }
    }
    println!("dataset : {} cas (digest {})", dataset.len(), &dataset.digest()[..16]);
    println!(
        "essaim : {small}  ·  référence : {big}  ·  endpoint {endpoint}  ·  scorer {}\n",
        if lenient { "lenient" } else { "exact" }
    );

    let small_backend = BenchBackend::SelfHost { endpoint: endpoint.clone(), model: small.clone() };
    let big_backend = BenchBackend::SelfHost { endpoint: endpoint.clone(), model: big.clone() };

    let swarm_configs = consensus_sweep(&instruction, &["consistency"], &ks);
    let baseline = BenchConfig::new(
        "baseline-big-k1",
        TopologySpec::GenerateVerify { generator: instruction.clone(), verifier: "non_empty_text".into(), k: KSpec::Fixed(1) },
    );

    let mut reports = Vec::new();
    let mut evidence: Vec<RunEvidence> = Vec::new();

    // Essaim : balayage de K sur le PETIT modèle.
    for cfg in &swarm_configs {
        let (r, ev) = run_config_with_evidence(&dataset, cfg, &small_backend, budget_cents, concurrency).await?;
        println!(
            "  {:<16} exactitude {:>5.1}%  présent {:>4.0}%  jetons/moy {:>7.1}  agents {:>4.1}  R0 {:>3.0}%",
            r.config, r.accuracy * 100.0, r.present_pct * 100.0, r.mean_tokens, r.mean_agents, r.verified_pct * 100.0
        );
        evidence.extend(ev);
        reports.push(r);
    }
    // Référence : K=1 sur le GROS modèle.
    let (rb, evb) = run_config_with_evidence(&dataset, &baseline, &big_backend, budget_cents, concurrency).await?;
    println!(
        "  {:<16} exactitude {:>5.1}%  présent {:>4.0}%  jetons/moy {:>7.1}  agents {:>4.1}  R0 {:>3.0}%",
        rb.config, rb.accuracy * 100.0, rb.present_pct * 100.0, rb.mean_tokens, rb.mean_agents, rb.verified_pct * 100.0
    );
    evidence.extend(evb);
    reports.push(rb);

    // Modèle de prix ~ proportionnel aux paramètres : petit ≈ 0.1, gros ≈ 1.2 (≈ 12× / jeton).
    let mut price = PriceModel::new().with("baseline-big-k1", 1.2);
    for &k in &ks {
        price = price.with(format!("swarm-k{k}"), 0.1);
    }
    let cmp = Comparison::new(reports.clone()).with_price(price);
    println!("\n{}", cmp.table());

    // La thèse tranchée : une config d'essaim domine-t-elle la référence (≥ justesse ET < coût) ?
    println!("— verdict de domination (la thèse tient si au moins un `oui`) —");
    for (name, dominates) in cmp.dominates_baseline("baseline-big-k1") {
        println!("  {name:<16} domine la référence : {}", if dominates { "OUI" } else { "non" });
    }

    // Le bundle re-prouvable + re-preuve INDÉPENDANTE (clé tenant du builder par défaut : test_key ∘ system).
    let bundle = BenchBundle::seal(&dataset, reports);
    let key = HmacKeySource::test_key().derive(TenantId::system())?;
    let rv = reprove(&evidence, &key);
    println!(
        "\nbundle : manifeste {} · structure OK = {} · re-preuve indépendante {}/{} runs (barreau min rang {:?})",
        &bundle.manifest_root[..16],
        bundle.verify().ok,
        rv.fully_proven,
        rv.rows,
        rv.weakest_rank
    );

    Ok(())
}
