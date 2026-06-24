// SPDX-License-Identifier: UNLICENSED
//! # `gen_corpus` — HA.2 : génère les corpus pilotes de substitution (T-MATH + T-EXTRACT).
//!
//! Écrit `fixtures/ha_math.jsonl` et `fixtures/ha_extract.jsonl` (anti-contamination par
//! construction : items synthétiques à paramètres aléatoires DÉTERMINISTES → jamais dans un corpus
//! d'entraînement). Re-dérivable depuis la graine ; imprime l'empreinte (digest) de chaque corpus
//! et un échantillon, et VÉRIFIE l'auto-cohérence (chaque label se score correct contre lui-même).
//!
//!   cargo run --example gen_corpus -p efe-bench-inference -- --n 300 --seed 20260624

use efe_bench_inference::{corpus, Dataset};

fn arg(flag: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == flag).and_then(|i| a.get(i + 1).cloned())
}

fn write_corpus(name: &str, class: &str, jsonl: String) -> std::io::Result<()> {
    let dir = arg("--out-dir").unwrap_or_else(|| "crates/efe-bench-inference/fixtures".to_string());
    let path = format!("{dir}/{name}");
    std::fs::write(&path, &jsonl)?;
    let ds = Dataset::from_jsonl(&jsonl).expect("le corpus généré se recharge");
    // Auto-cohérence : chaque label, présenté comme réponse, est scoré CORRECT (corpus bien posé).
    let ok = ds.cases.iter().all(|c| c.scorer.is_correct(&serde_json::json!({ "value": &c.label }), &c.label));
    println!(
        "  {class:<8} → {path}  ({} cas · digest {} · auto-cohérent={ok})",
        ds.len(),
        &ds.digest()[..16],
    );
    assert!(ok, "corpus {class} INCOHÉRENT — un label ne se score pas correct contre lui-même");
    Ok(())
}

fn main() -> std::io::Result<()> {
    let n: usize = arg("--n").and_then(|s| s.parse().ok()).unwrap_or(300);
    let seed: u64 = arg("--seed").and_then(|s| s.parse().ok()).unwrap_or(20_260_624);
    println!("Génération des corpus pilotes HA.2 (n={n}/classe, seed={seed}) :");

    write_corpus("ha_math.jsonl", "T-MATH", corpus::to_jsonl(&corpus::math(n, seed), &corpus::provenance_header("math", n, seed)))?;
    write_corpus("ha_extract.jsonl", "T-EXTRACT", corpus::to_jsonl(&corpus::extract(n, seed), &corpus::provenance_header("extract", n, seed)))?;

    // Échantillon lisible (les 2 premiers de chaque classe).
    println!("\nÉchantillon T-MATH :");
    for c in corpus::math(n, seed).iter().take(2) {
        println!("  [{}] {}  → {}", c.id, c.task["text"].as_str().unwrap(), c.label);
    }
    println!("Échantillon T-EXTRACT :");
    for c in corpus::extract(n, seed).iter().take(2) {
        println!("  [{}] {}  → {}", c.id, c.task["text"].as_str().unwrap(), c.label);
    }
    Ok(())
}
