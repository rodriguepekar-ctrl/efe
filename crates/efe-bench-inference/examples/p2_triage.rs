// SPDX-License-Identifier: UNLICENSED
//! **P2 (b) — la sous-tache ADAPTEE : triage de demande d'achat (procurement).**
//!
//! La lecon de P2 : la substitution tient PAR SOUS-TACHE ETROITE, pas globalement, et elle depend
//! de la SPECIALISATION du petit. Ce driver teste la vraie mecanique : le MEME petit modele, prompt
//! **generique** (aucune politique -> il devine) vs **specialise** (la politique + few-shot -> il
//! applique), chacun compare au gros a instruction IDENTIQUE (I-HA-FAIR). On regarde si la
//! specialisation FERME l'ecart vers le point doux (petit non-inferieur, sous plafond, moins cher).
//!
//! Corpus : `fixtures/p2_triage.jsonl` (150 cas, labels objectifs par regle deterministe = I-BENCH-TRUTH).
//! Scorer : `lenient` (par cas) — casse/ponctuation ignorees, le label compte comme mot ; prompt strict.
//!
//!   cargo run --release --example p2_triage -p efe-bench-inference -- \
//!     --endpoint http://127.0.0.1:11434 --big gemma3:12b --small phi4-mini-local --n 150

use std::collections::BTreeMap;

use efe_bench_inference::{
    analyze_cell, reprove, run_config_with_evidence, substitution::GapMap, BenchBackend, BenchConfig, ConfigReport,
    Dataset, RunEvidence,
};
use efe_nano_sdk::TenantId;
use efe_swarm_sdk::{HmacKeySource, KSpec, TopologySpec};

fn arg(flag: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == flag).and_then(|i| a.get(i + 1).cloned())
}

struct Cond {
    name: &'static str,
    instruction: &'static str,
}

const GENERIC: &str = "You are a procurement triage assistant. For the purchase request below, decide APPROVE, REVIEW, or BLOCK. Reply with ONLY one word: APPROVE, REVIEW, or BLOCK.";

const SPECIALISE: &str = "You are a procurement triage assistant. Apply this policy EXACTLY.\n\
- BLOCK if the vendor is sanctioned / flagged / on a sanctions, OFAC or EU watchlist, OR the item is prohibited (weapons, firearm parts, military optics, munitions).\n\
- Otherwise REVIEW if the total amount is over 10,000 (any currency), OR the vendor is new / unverified / not yet onboarded, OR the item is export-controlled / restricted / dual-use.\n\
- Otherwise APPROVE.\n\
Parse the amount carefully first: '12k'=12000, '$0.5k'=500, 'ten thousand five hundred'=10500. Then compare to the 10,000 threshold.\n\
Examples:\n\
- '50 reams of paper from Acme, our approved vendor, $420.' => APPROVE\n\
- 'sensors from a new supplier we have not onboarded, $3,000.' => REVIEW\n\
- 'night-vision scopes from Acme, $2,000.' => BLOCK\n\
Reply with ONLY one word: APPROVE, REVIEW, or BLOCK.";

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = arg("--endpoint").unwrap_or_else(|| "http://127.0.0.1:11434".to_string());
    let big = arg("--big").unwrap_or_else(|| "gemma3:12b".to_string());
    let small = arg("--small").unwrap_or_else(|| "phi4-mini-local".to_string());
    let n_arg: usize = arg("--n").and_then(|s| s.parse().ok()).unwrap_or(150);
    let deltas: Vec<f64> = arg("--deltas")
        .unwrap_or_else(|| "0.05,0.10,0.15".to_string())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let alpha = 0.05;
    let beta = 0.20;
    let boot = 2000;
    let budget = 1_000_000;
    let conc = 1; // Ollama serialise.

    let conditions = [
        Cond { name: "generique", instruction: GENERIC },
        Cond { name: "specialise", instruction: SPECIALISE },
    ];

    let mut ds = Dataset::from_jsonl(include_str!("../fixtures/p2_triage.jsonl"))?;
    ds.cases.truncate(n_arg);
    let n = ds.cases.len();

    println!("P2 (b) — TRIAGE demande d'achat : sous-tache adaptee (procurement, vertical Kovant)");
    println!("  gros(ref) : {big}  ·  petit : {small}  ·  n={n}  ·  generique vs specialise  ·  a={alpha}  ·  d in {deltas:?}\n");

    let models = [big.clone(), small.clone()];
    let mut reports: BTreeMap<(String, String), (ConfigReport, Vec<RunEvidence>)> = BTreeMap::new();
    for model in &models {
        let backend = BenchBackend::SelfHost { endpoint: endpoint.clone(), model: model.clone() };
        for cond in &conditions {
            let cfg = BenchConfig::new(
                format!("{model}/{}", cond.name),
                TopologySpec::GenerateVerify {
                    generator: cond.instruction.into(),
                    verifier: "non_empty_text".into(),
                    k: KSpec::Fixed(1),
                },
            );
            let t0 = std::time::Instant::now();
            let (r, ev) = run_config_with_evidence(&ds, &cfg, &backend, budget, conc).await?;
            println!(
                "  {model:<16} {:<11} exact {:>5.1}%  present {:>4.0}%  ({:.0}s)",
                cond.name,
                r.accuracy * 100.0,
                r.present_pct * 100.0,
                t0.elapsed().as_secs_f64()
            );
            reports.insert((model.clone(), cond.name.to_string()), (r, ev));
        }
    }

    // Carte des ecarts par delta : non-inferiorite appariee (Holm sur la famille des 2 conditions).
    let mut max_n_required = 0usize;
    for (di, &delta) in deltas.iter().enumerate() {
        let mut cells = Vec::new();
        for (ci, cond) in conditions.iter().enumerate() {
            let small_rep = &reports[&(small.clone(), cond.name.to_string())].0;
            let big_rep = &reports[&(big.clone(), cond.name.to_string())].0;
            let seed = 0x5052_3200 ^ ((di as u64) << 16) ^ ((ci as u64) << 8) ^ (cond.name.len() as u64);
            let cell = analyze_cell(
                small.clone(),
                cond.name.to_string(),
                std::slice::from_ref(small_rep),
                std::slice::from_ref(big_rep),
                delta,
                alpha,
                beta,
                boot,
                seed,
            )?;
            max_n_required = max_n_required.max(if cell.n_required == usize::MAX { 0 } else { cell.n_required });
            cells.push(cell);
        }
        let map = GapMap::build(cells, alpha);
        println!("\n== carte des ecarts a d = {:.0} points ==", delta * 100.0);
        print!("{}", map.table());
        for c in &map.cells {
            println!(
                "   {:<16} {:<11} D={:+.1}pts  IC[{:+.1},{:+.1}]  puiss {:.0}%  n_req {}",
                c.small_model,
                c.task,
                c.nin.diff_point * 100.0,
                c.nin.ci_lower * 100.0,
                c.nin.ci_upper * 100.0,
                c.power * 100.0,
                if c.n_required == usize::MAX { "inf".into() } else { c.n_required.to_string() },
            );
        }
        println!("  -> substitution tient sur >=1 condition apres Holm : {}", map.any_substitution());
    }

    // Bundle re-prouvable : la mesure est elle-meme dans le moat.
    let mut all_ev = Vec::new();
    for (_k, (_r, ev)) in reports {
        all_ev.extend(ev);
    }
    let key = HmacKeySource::test_key().derive(TenantId::system())?;
    let rv = reprove(&all_ev, &key);
    println!(
        "\nI-HA-POWER : n={n}, n requis (max sur cellules determinables) ~ {max_n_required}  ->  {}",
        if max_n_required > n { "SOUS-PUISSANCE : elargir le corpus".to_string() } else { "n suffisant pour les cellules concluantes".into() }
    );
    println!(
        "re-preuve independante : {}/{} runs re-prouves  ·  la mesure est re-prouvable",
        rv.fully_proven, rv.rows
    );
    Ok(())
}
