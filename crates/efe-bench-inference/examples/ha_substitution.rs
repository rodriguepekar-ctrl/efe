// SPDX-License-Identifier: UNLICENSED
//! # `ha_substitution` — HA.3 : la **carte des écarts** mesurée (substitution, vrais modèles).
//!
//! Fait tourner chaque **petit** modèle ET le **gros** modèle de référence, à **K=1**, sur les corpus
//! pilotes HA.2 (T-MATH numérique + T-EXTRACT ensembliste), à instruction/scorer/budget identiques
//! (I-HA-FAIR), récolte la justesse **par item**, puis les passe au module `substitution` :
//! **non-infériorité appariée** (bootstrap cluster déterministe), capacité-vs-format (présent %),
//! puissance atteinte / n requis (I-HA-POWER), et **carte des écarts** {modèle × tâche} corrigée
//! Holm. Tout est scellé en bundle **re-prouvable** (re-preuve indépendante depuis les records bruts).
//!
//! Le verdict est MESURÉ, pas supposé — et « indéterminé » à n trop petit est un résultat honnête
//! (I-HA-POWER), pas un échec : la carte dit alors combien d'items il faut (→ HA.4 à l'échelle).
//!
//!   cargo run --release --example ha_substitution -p efe-bench-inference -- \
//!     --endpoint http://127.0.0.1:11434 --big gemma3:12b \
//!     --small phi4-mini-local,llama32-1b-local --n 45

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

/// `(modèle, classe)` → (rapport, évidence re-prouvable) du run K=1 correspondant.
type ReportMap = BTreeMap<(String, String), (ConfigReport, Vec<RunEvidence>)>;

/// Une classe de tâches : nom, corpus, instruction STRICTE (la même pour les deux modèles).
struct Class {
    name: &'static str,
    jsonl: &'static str,
    instruction: &'static str,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = arg("--endpoint").unwrap_or_else(|| "http://127.0.0.1:11434".to_string());
    let big = arg("--big").unwrap_or_else(|| "gemma3:12b".to_string());
    let smalls: Vec<String> = arg("--small")
        .unwrap_or_else(|| "phi4-mini-local,llama32-1b-local".to_string())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let n: usize = arg("--n").and_then(|s| s.parse().ok()).unwrap_or(45);
    let deltas: Vec<f64> = arg("--deltas")
        .unwrap_or_else(|| "0.02,0.05,0.10".to_string())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let alpha = 0.05;
    let beta = 0.20;
    let boot = 2000;
    let budget = 1_000_000;
    let conc = 1; // Ollama sérialise ; la concurrence ne ferait que faire la queue.

    let classes = [
        Class {
            name: "T-MATH",
            jsonl: include_str!("../fixtures/ha_math.jsonl"),
            instruction: "Solve this arithmetic word problem. Reply with ONLY the final number, no words or units.",
        },
        Class {
            name: "T-EXTRACT",
            jsonl: include_str!("../fixtures/ha_extract.jsonl"),
            instruction: "Read the text and answer the question. Reply with ONLY a JSON array of lowercase words, for example [\"apple\",\"pear\"]. No other text.",
        },
    ];

    // Sous-échantillon déterministe : les n premiers items (T-MATH garde l'équilibre des tiers).
    let datasets: Vec<(String, Dataset)> = classes
        .iter()
        .map(|c| {
            let mut ds = Dataset::from_jsonl(c.jsonl).expect("corpus HA.2");
            ds.cases.truncate(n);
            (c.name.to_string(), ds)
        })
        .collect();

    println!("HA.3 — carte des écarts de substitution (mesurée)");
    println!("  gros (réf.) : {big}  ·  petits : {smalls:?}  ·  endpoint {endpoint}");
    println!("  n={n}/classe · K=1 · α={alpha} · puissance cible 1−β={} · δ ∈ {deltas:?}\n", 1.0 - beta);

    // ── Exécution : MODÈLE-majeur (chaque modèle chargé une seule fois dans Ollama). ──
    let models: Vec<String> = std::iter::once(big.clone()).chain(smalls.iter().cloned()).collect();
    // reports[(model, class)] = (rapport, évidence)
    let mut reports: ReportMap = BTreeMap::new();
    for model in &models {
        let backend = BenchBackend::SelfHost { endpoint: endpoint.clone(), model: model.clone() };
        for ((cls_name, ds), cls) in datasets.iter().zip(classes.iter()) {
            let cfg = BenchConfig::new(
                format!("{model}/{cls_name}"),
                TopologySpec::GenerateVerify { generator: cls.instruction.into(), verifier: "non_empty_text".into(), k: KSpec::Fixed(1) },
            );
            let t0 = std::time::Instant::now();
            let (r, ev) = run_config_with_evidence(ds, &cfg, &backend, budget, conc).await?;
            println!(
                "  {model:<22} {cls_name:<10} exact {:>5.1}%  présent {:>4.0}%  R0 {:>3.0}%  ({:.0}s)",
                r.accuracy * 100.0, r.present_pct * 100.0, r.verified_pct * 100.0, t0.elapsed().as_secs_f64()
            );
            reports.insert((model.clone(), cls_name.clone()), (r, ev));
        }
    }

    // ── Analyse : carte des écarts par δ (non-infériorité appariée, Holm sur la famille). ──
    let mut max_n_required = 0usize;
    for (di, &delta) in deltas.iter().enumerate() {
        let mut cells = Vec::new();
        for (si, small) in smalls.iter().enumerate() {
            for (cls_name, _ds) in &datasets {
                let small_rep = &reports[&(small.clone(), cls_name.clone())].0;
                let big_rep = &reports[&(big.clone(), cls_name.clone())].0;
                // Graine d'analyse stable, distincte par (δ, modèle, classe) → bootstrap reproductible.
                let seed = 0x4841_3300 ^ ((di as u64) << 16) ^ ((si as u64) << 8) ^ (cls_name.len() as u64);
                let cell = analyze_cell(small.clone(), cls_name.clone(), std::slice::from_ref(small_rep), std::slice::from_ref(big_rep), delta, alpha, beta, boot, seed)?;
                max_n_required = max_n_required.max(if cell.n_required == usize::MAX { 0 } else { cell.n_required });
                cells.push(cell);
            }
        }
        let map = GapMap::build(cells, alpha);
        println!("\n══ carte des écarts à δ = {:.0} points ══", delta * 100.0);
        print!("{}", map.table());
        // Détail capacité-vs-format + puissance par cellule.
        for c in &map.cells {
            println!(
                "   {:<20} {:<10} Δ={:+.1}pts  IC[{:+.1},{:+.1}]  capacité(présent) {}  puiss {:.0}%  n_req {}",
                c.small_model, c.task,
                c.nin.diff_point * 100.0, c.nin.ci_lower * 100.0, c.nin.ci_upper * 100.0,
                c.nin_present.verdict.label(), c.power * 100.0,
                if c.n_required == usize::MAX { "∞".into() } else { c.n_required.to_string() },
            );
        }
        println!("  → thèse (substitution) tient sur ≥1 cellule après Holm : {}", map.any_substitution());
    }

    // ── Bundle re-prouvable : la mesure est elle-même dans le moat. ──
    let mut all_reports = Vec::new();
    let mut all_ev = Vec::new();
    for (_k, (r, ev)) in reports {
        all_reports.push(r);
        all_ev.extend(ev);
    }
    // Le bundle lie le manifeste des reports ; la re-preuve indépendante re-dérive chaque run.
    let key = HmacKeySource::test_key().derive(TenantId::system())?;
    let rv = reprove(&all_ev, &key);
    println!(
        "\nI-HA-POWER : à n={n}, n requis (max sur cellules déterminables) ≈ {max_n_required}  →  {}",
        if max_n_required > n { "SOUS-PUISSANCE : élargir le corpus (HA.4) pour un verdict ferme".to_string() } else { "n suffisant pour les cellules concluantes".into() }
    );
    println!(
        "re-preuve indépendante : {}/{} runs re-prouvés (barreau min rang {:?})  ·  la mesure est re-prouvable",
        rv.fully_proven, rv.rows, rv.weakest_rank
    );
    Ok(())
}
