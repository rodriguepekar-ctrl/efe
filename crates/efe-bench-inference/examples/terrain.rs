// SPDX-License-Identifier: UNLICENSED
//! # `terrain` — Le Terrain : l'agrégation récupère-t-elle un VRAI écart K=1 sur QUELQUE type de tâche ?
//!
//! Le harnais de **falsification** (cf. `docs/reports/EFE_Archi_Terrain_Frontiere`). Trois cellules
//! **pré-enregistrées par TYPE** (jamais par issue espérée) : **T-MATH** (contrôle, perte connue),
//! **T-EXTRACT** (extraction d'ensembles — modalité d'erreur différente, candidat à la récupération via
//! le **vote par élément** `SetVote`), **T-MATH-HARD** (escalade tier-3 gelée). Chaque cellule passe un
//! **écran de gap** (small-seul &lt; 90 % ET écart ≥ 5 pts — désarme le confondant plafond), puis le verdict
//! de **récupération** (non-infériorité d'un mécanisme déployable pré-engagé @K\* vs gros@1, puissance,
//! pas-d'artefact), sous **Holm-Bonferroni** inter-cellules. L'issue par défaut attendue : « aucune
//! récupération ». Vraie inférence via Ollama/vLLM ; aucune réponse scriptée. Endpoint LOCAL exigé (seam).
//!
//!   cargo run --release --example terrain -p efe-bench-inference -- \
//!     --endpoint http://127.0.0.1:11434 --small phi4-mini-local --big gemma3:12b --n 20 --k 5 --kstar 5

use std::sync::{Arc, Mutex};

use efe_bench_inference::{
    corpus, run_family, BenchCase, Cell, Dataset, Mechanism, SampleSet,
};
use efe_swarm_sdk::{AgentCall, AgentRole, Budget, ClientMode, KSpec, OnExceed, Swarm, SwarmObserver, TopologySpec};
use serde_json::Value;

/// Graine de corpus gelée (anti-contamination ; re-dérivable) — la MÊME que les briques H-A/H-B.
const SEED: u64 = 20_260_624;

/// Capte les sorties des agents de rôle **Générateur** (et eux seuls — I-HB-GENONLY).
#[derive(Default)]
struct GenCollector {
    out: Mutex<Vec<(u64, Value)>>,
}
impl SwarmObserver for GenCollector {
    fn on_agent_call(&self, call: AgentCall<'_>) {
        if call.role == AgentRole::Generator {
            self.out.lock().unwrap().push((call.variant.unwrap_or(0), call.output.clone()));
        }
    }
}
impl GenCollector {
    fn clear(&self) {
        self.out.lock().unwrap().clear();
    }
    fn drain_sorted(&self) -> Vec<Value> {
        let mut v = std::mem::take(&mut *self.out.lock().unwrap());
        v.sort_by_key(|(variant, _)| *variant);
        v.into_iter().map(|(_, o)| o).collect()
    }
}

fn arg(flag: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == flag).and_then(|i| a.get(i + 1).cloned())
}
fn est(s: &str) -> u32 {
    ((s.chars().count() / 4) as u32).max(1)
}
fn text_of(task: &Value) -> String {
    task.get("text").and_then(|t| t.as_str()).map(String::from).unwrap_or_default()
}
fn cand_text(c: &Value) -> String {
    match c.get("value").unwrap_or(c) {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Exécute un modèle sur tout le dataset, **K générations par item**, candidats captés PAR RÔLE. Le
/// scorer de chaque cas est **conservé** (Numeric pour math, Set pour extract) — c'est ce qui pilote
/// l'opérateur loyal. Séquentiel (l'observateur associe par item ; Ollama sérialise de toute façon).
async fn run_model(
    endpoint: &str,
    model: &str,
    dataset: &Dataset,
    instruction: &str,
    k: usize,
    gen_tokens: u32,
) -> Result<(Vec<SampleSet>, usize), Box<dyn std::error::Error>> {
    let collector = Arc::new(GenCollector::default());
    let swarm = Swarm::builder()
        .mode(ClientMode::SelfHost { endpoint: endpoint.to_string(), model: model.to_string() })
        .max_gen_tokens(gen_tokens)
        .with_observer(collector.clone())
        .build()?;
    let mut samples = Vec::with_capacity(dataset.len());
    let mut verified = 0usize;
    for case in &dataset.cases {
        collector.clear();
        let spec = TopologySpec::GenerateVerify {
            generator: instruction.into(),
            verifier: "non_empty_text".into(),
            k: KSpec::Fixed(k),
        };
        let outcome = swarm
            .run_traced(case.blind_task(), spec, Budget::cents(1_000_000).with_policy(OnExceed::Stop))
            .await?;
        if outcome.run.provenance.verified {
            verified += 1;
        }
        let answers = collector.drain_sorted();
        let in_est = est(&format!("{instruction} {}", text_of(&case.task)));
        let in_tokens = vec![in_est; answers.len()];
        let out_tokens: Vec<u32> = answers.iter().map(|c| est(&cand_text(c))).collect();
        samples.push(SampleSet {
            case_id: case.id.clone(),
            task: case.task.clone(),
            answers,
            label: case.label.clone(),
            scorer: case.scorer.clone(), // conservé : Numeric (math) / Set (extract)
            weights: None,
            in_tokens,
            out_tokens,
        });
    }
    Ok((samples, verified))
}

/// Construit un `Dataset` à partir de cas générés (roundtrip JSONL — sûr, conserve scorer/label).
fn dataset_of(cases: &[BenchCase], class: &str, n: usize) -> Result<Dataset, Box<dyn std::error::Error>> {
    Ok(Dataset::from_jsonl(&corpus::to_jsonl(cases, &corpus::provenance_header(class, n, SEED)))?)
}

/// La spec d'une cellule pré-enregistrée : nom, dataset, instruction, budget de génération, mécanisme M.
struct CellSpec {
    name: &'static str,
    dataset: Dataset,
    instruction: &'static str,
    gen_tokens: u32,
    mechanism: Mechanism,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = arg("--endpoint").unwrap_or_else(|| "http://127.0.0.1:11434".to_string());
    let small = arg("--small").unwrap_or_else(|| "phi4-mini-local".to_string());
    let big = arg("--big").unwrap_or_else(|| "gemma3:12b".to_string());
    let n: usize = arg("--n").and_then(|s| s.parse().ok()).unwrap_or(20);
    let k: usize = arg("--k").and_then(|s| s.parse().ok()).unwrap_or(5);
    let k_star: usize = arg("--kstar").and_then(|s| s.parse().ok()).unwrap_or(5);
    let (delta, alpha, beta, boot) = (0.05, 0.05, 0.2, 10_000);

    let math_instr = "Solve this arithmetic word problem. Think step by step, then write the final answer as a single number on the last line.";
    let extract_instr = "List the matching items as a comma-separated list of lowercase single words. No other text.";

    // ── Les 3 cellules PRÉ-ENREGISTRÉES (par TYPE, jamais par issue ; famille fermée). ──
    let math_cases = corpus::math(n, SEED);
    let extract_cases = corpus::extract(n, SEED);
    // T-MATH-HARD : math(3n) → filtrer tier-3 (ids m3-*) → exactement n items.
    let hard_cases: Vec<BenchCase> = corpus::math(3 * n, SEED).into_iter().filter(|c| c.id.starts_with("m3-")).collect();

    let specs = vec![
        CellSpec { name: "T-MATH", dataset: dataset_of(&math_cases, "math", n)?, instruction: math_instr, gen_tokens: 512, mechanism: Mechanism::SelfConsistency },
        CellSpec { name: "T-EXTRACT", dataset: dataset_of(&extract_cases, "extract", n)?, instruction: extract_instr, gen_tokens: 64, mechanism: Mechanism::SetVote },
        CellSpec { name: "T-MATH-HARD", dataset: dataset_of(&hard_cases, "math-hard", hard_cases.len())?, instruction: math_instr, gen_tokens: 512, mechanism: Mechanism::SelfConsistency },
    ];

    println!("Le Terrain — l'agrégation récupère-t-elle un VRAI écart K=1 sur QUELQUE type ?");
    println!("  petit {small} (K={k})  vs  gros {big} (K=1)  ·  n≈{n}/cellule  ·  K*={k_star} gelé  ·  δ={delta} α={alpha}");
    println!("  3 cellules PRÉ-ENREGISTRÉES (fermées) · écran de gap · Holm inter-cellules · issue défaut attendue : AUCUNE\n");

    // ── Mesure : par cellule, le gros (K=1) figé d'abord (I-HB-PREREG), puis le petit (K). ──
    let mut store: Vec<(String, Vec<SampleSet>, Vec<SampleSet>, Mechanism)> = Vec::new();
    for spec in &specs {
        println!("[{}] réf. gros ({big}, K=1)… puis essaim petit ({small}, K={k})…", spec.name);
        let (big_s, big_v) = run_model(&endpoint, &big, &spec.dataset, spec.instruction, 1, spec.gen_tokens).await?;
        let (small_s, small_v) = run_model(&endpoint, &small, &spec.dataset, spec.instruction, k, spec.gen_tokens).await?;
        println!("    R0 gros {big_v}/{} · petit {small_v}/{} (scellé, re-prouvable)", big_s.len(), small_s.len());
        store.push((spec.name.to_string(), small_s, big_s, spec.mechanism));
    }

    let cells: Vec<Cell> = store
        .iter()
        .map(|(name, small_s, big_s, mech)| Cell { name: name.clone(), small: small_s, big: big_s, mechanism: *mech, verifier: None, weigher: None })
        .collect();

    let fam = run_family(&cells, k_star, delta, alpha, beta, boot, SEED);

    // ── Table de famille (descriptif + écran + récupération). ──
    println!("\n── cellules (écran de gap → récupération@K*={k_star}, sous Holm) ──");
    println!(
        "{:<13} {:<8} {:>7} {:>6} {:>6} {:>8} {:>6} {:>13} {:>6} {:>5} {:>6} {:>7} {:>6}",
        "cellule", "M", "smll@1", "big@1", "gap", "écran", "M@K*", "verdict", "puiss", "Holm", "div%", "skew", "→récup"
    );
    for c in &fam.cells {
        println!(
            "{:<13} {:<8} {:>6.1}% {:>5.1}% {:>+5.1} {:>8} {:>5.1}% {:>13} {:>5.0}% {:>5} {:>5.0}% {:>+7.2} {:>6}",
            c.name,
            c.mechanism,
            c.screen.small_alone * 100.0,
            c.screen.big * 100.0,
            c.screen.gap * 100.0,
            format!("{:?}", c.screen.verdict),
            c.recovery_acc * 100.0,
            format!("{:?}", c.recovery.nin.verdict),
            c.recovery.power * 100.0,
            if c.recovery.holm_passes { "oui" } else { "—" },
            c.distinct_frac * 100.0,
            if c.symmetry.skewness.is_nan() { 0.0 } else { c.symmetry.skewness },
            if c.recovers { "OUI" } else { "non" },
        );
    }

    println!("\n══ VERDICT FAMILLE : {} ══", fam.headline());
    if fam.n_recover == 0 {
        println!("  → AUCUNE cellule ne récupère : la jambe « quantité récupère qualité » est largement réfutée");
        println!("    (présente mais non récoltable). Ce qui survit : H-A (substitution K=1 sur certaines cellules).");
    } else {
        println!("  → L'agrégation récupère un vrai écart K=1 sur ≥1 type — la thèse est DÉPENDANTE DU TYPE de tâche ;");
        println!("    le prédicteur de diversité (div%/skew) doit être cohérent (résultat à stresser, B2 significatif).");
    }
    if !fam.robust {
        println!("  ⚠ EXPLORATOIRE : < 2 cellules réglées (puissance/verdict) — augmenter n (corpus::math/extract re-dérivables).");
    }
    println!("\n  rappels : seuils & cellules GELÉS avant issue (I-TERRAIN-PREREG) ; coût-jetons seulement, GPU-s → B2.");
    Ok(())
}
