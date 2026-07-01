// SPDX-License-Identifier: UNLICENSED
//! **P2 porte ② — le DIAGNOSTIC : la dispersion predit-elle l'erreur du petit specialise ?**
//!
//! La premisse du routeur d'escalade : faire generer le petit K fois, mesurer la DISPERSION de ses
//! reponses (self-agreement label-free), et escalader vers le gros quand elle est haute. Pour que le
//! routeur soit reel, la dispersion doit CORRELER a l'erreur : les MAUVAISES reponses du petit doivent
//! etre plus dispersees que les bonnes. Ce driver le mesure, sur la sous-tache de triage, et compare
//! **escalade-par-dispersion** vs **aleatoire** (baseline) vs **oracle** (borne haute label-using).
//!
//! THERMOMETRE, PAS BULLETIN : la dispersion est lue comme un SIGNAL d'escalade, jamais agregee en
//! reponse. On garde la majorite du petit ou on escalade la reponse du gros — on ne fabrique rien.
//!
//!   cargo run --release --example p2_diagnostic -p efe-bench-inference -- \
//!     --endpoint http://127.0.0.1:11434 --big gemma3:12b --small phi4-mini-local --n 150 --k 5

use std::sync::{Arc, Mutex};

use efe_bench_inference::Dataset;
use efe_swarm_sdk::{AgentCall, AgentRole, Budget, ClientMode, KSpec, OnExceed, Swarm, SwarmObserver, TopologySpec};
use serde_json::Value;

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

const CATS: [Option<&str>; 4] = [Some("APPROVE"), Some("REVIEW"), Some("BLOCK"), None];

fn arg(flag: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == flag).and_then(|i| a.get(i + 1).cloned())
}

/// La classe portee par une reponse : exactement UNE des 3 etiquettes presente => elle ; sinon None
/// (ambigu / absente = incertain, ce qui est un signal legitime de faible confiance).
fn class_of(text: &str) -> Option<&'static str> {
    let t = text.to_lowercase();
    let present: Vec<&'static str> = [("approve", "APPROVE"), ("review", "REVIEW"), ("block", "BLOCK")]
        .iter()
        .filter(|(w, _)| t.contains(w))
        .map(|(_, c)| *c)
        .collect();
    if present.len() == 1 { Some(present[0]) } else { None }
}

fn cand_text(c: &Value) -> String {
    match c.get("value").unwrap_or(c) {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

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

/// Un item apres inference : l'id, la verite-terrain, et les K classes du petit (ou la 1 du gros).
struct Item {
    id: String,
    label: String,
    classes: Vec<Option<&'static str>>,
}

async fn run_model(
    endpoint: &str,
    model: &str,
    dataset: &Dataset,
    k: usize,
    gen_tokens: u32,
) -> Result<(Vec<Item>, usize), Box<dyn std::error::Error>> {
    let collector = Arc::new(GenCollector::default());
    let swarm = Swarm::builder()
        .mode(ClientMode::SelfHost { endpoint: endpoint.to_string(), model: model.to_string() })
        .max_gen_tokens(gen_tokens)
        .with_observer(collector.clone())
        .build()?;
    let mut items = Vec::with_capacity(dataset.len());
    let mut verified = 0usize;
    for case in &dataset.cases {
        collector.clear();
        let spec = TopologySpec::GenerateVerify {
            generator: SPECIALISE.into(),
            verifier: "non_empty_text".into(),
            k: KSpec::Fixed(k),
        };
        let outcome = swarm
            .run_traced(case.blind_task(), spec, Budget::cents(1_000_000).with_policy(OnExceed::Stop))
            .await?;
        if outcome.run.provenance.verified {
            verified += 1;
        }
        let classes: Vec<Option<&'static str>> = collector.drain_sorted().iter().map(|c| class_of(&cand_text(c))).collect();
        items.push(Item { id: case.id.clone(), label: case.label.as_str().unwrap_or("").to_string(), classes });
    }
    Ok((items, verified))
}

/// (classe-majoritaire deterministe, dispersion = 1 - freq_max/K) des K classes d'un item.
fn majority_and_dispersion(classes: &[Option<&'static str>]) -> (Option<&'static str>, f64) {
    let k = classes.len().max(1);
    let counts: Vec<usize> = CATS.iter().map(|cat| classes.iter().filter(|c| *c == cat).count()).collect();
    let mut best = 0usize;
    for i in 1..CATS.len() {
        if counts[i] > counts[best] {
            best = i; // premier-max : depart d'egalite deterministe par l'ordre de CATS
        }
    }
    (CATS[best], 1.0 - counts[best] as f64 / k as f64)
}

#[derive(Clone)]
struct Row {
    id: String,
    disp: f64,
    small_ok: bool,
    big_ok: bool,
}

/// Exactitude melangee : on escalade les `esc` premiers de `order` (reponse du gros), le reste garde
/// la majorite du petit. THERMOMETRE, PAS BULLETIN : aucune reponse fabriquee.
fn blended(order: &[Row], esc: usize) -> f64 {
    let n = order.len().max(1);
    let ok = order.iter().enumerate().filter(|(i, r)| if *i < esc { r.big_ok } else { r.small_ok }).count();
    ok as f64 / n as f64
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = arg("--endpoint").unwrap_or_else(|| "http://127.0.0.1:11434".to_string());
    let big = arg("--big").unwrap_or_else(|| "gemma3:12b".to_string());
    let small = arg("--small").unwrap_or_else(|| "phi4-mini-local".to_string());
    let n: usize = arg("--n").and_then(|s| s.parse().ok()).unwrap_or(150);
    let k: usize = arg("--k").and_then(|s| s.parse().ok()).unwrap_or(5);
    let gen_tokens: u32 = arg("--gen-tokens").and_then(|s| s.parse().ok()).unwrap_or(48);

    let mut ds = Dataset::from_jsonl(include_str!("../fixtures/p2_triage.jsonl"))?;
    ds.cases.truncate(n);
    let n = ds.cases.len();

    println!("P2 porte 2 — DIAGNOSTIC : la dispersion predit-elle l'erreur du petit specialise ?");
    println!("  petit {small} (K={k}, specialise)  ·  gros {big} (K=1, specialise, cible d'escalade)  ·  n={n}\n");

    println!("[1/2] petit ({small}, K={k}) ...");
    let (small_items, small_v) = run_model(&endpoint, &small, &ds, k, gen_tokens).await?;
    println!("[2/2] gros ({big}, K=1) ...");
    let (big_items, big_v) = run_model(&endpoint, &big, &ds, 1, gen_tokens).await?;

    // Aligne par id, calcule dispersion + justesse.
    let mut rows: Vec<Row> = Vec::with_capacity(n);
    for si in &small_items {
        let (maj, disp) = majority_and_dispersion(&si.classes);
        let big_ok = big_items
            .iter()
            .find(|b| b.id == si.id)
            .map(|b| majority_and_dispersion(&b.classes).0 == Some(si.label.as_str()))
            .unwrap_or(false);
        rows.push(Row { id: si.id.clone(), disp, small_ok: maj == Some(si.label.as_str()), big_ok });
    }

    let small_acc = rows.iter().filter(|r| r.small_ok).count() as f64 / n as f64;
    let big_acc = rows.iter().filter(|r| r.big_ok).count() as f64 / n as f64;

    // Separation : la dispersion moyenne des MAUVAISES vs BONNES reponses du petit.
    let mean = |f: &dyn Fn(&Row) -> bool| {
        let v: Vec<f64> = rows.iter().filter(|r| f(r)).map(|r| r.disp).collect();
        if v.is_empty() { f64::NAN } else { v.iter().sum::<f64>() / v.len() as f64 }
    };
    let disp_wrong = mean(&|r: &Row| !r.small_ok);
    let disp_right = mean(&|r: &Row| r.small_ok);

    // Ordres d'escalade : par dispersion (deployable), aleatoire (= interpolation lineaire), oracle (label).
    let mut by_disp = rows.clone();
    by_disp.sort_by(|a, b| b.disp.partial_cmp(&a.disp).unwrap().then(a.id.cmp(&b.id)));
    let mut by_oracle = rows.clone();
    by_oracle.sort_by(|a, b| a.small_ok.cmp(&b.small_ok).then(a.id.cmp(&b.id))); // faux (false) d'abord

    println!("\n-- exactitudes --");
    println!("  petit seul (SC@{k}) : {:.1}%   ·   gros : {:.1}%   ·   ecart {:+.1}pts", small_acc * 100.0, big_acc * 100.0, (big_acc - small_acc) * 100.0);
    println!("\n-- SEPARATION (le diagnostic porte-t-il du signal ?) --");
    println!("  dispersion moyenne  |  MAUVAISES {:.3}   BONNES {:.3}   ecart {:+.3}", disp_wrong, disp_right, disp_wrong - disp_right);
    println!("  (ecart > 0 => les erreurs sont plus dispersees => le routeur peut les cibler)");

    println!("\n-- ESCALADE : exactitude melangee vs taux d'escalade --");
    println!("  {:>8} │ {:>12} {:>10} {:>8}", "escalade", "dispersion", "aleatoire", "oracle");
    for &r in &[0.0, 0.10, 0.20, 0.30, 0.40, 0.50, 1.0] {
        let esc = (r * n as f64).round() as usize;
        let d = blended(&by_disp, esc);
        let rand = small_acc * (1.0 - r) + big_acc * r; // esperance d'un routage aleatoire
        let orac = blended(&by_oracle, esc);
        println!("  {:>7.0}% │ {:>11.1}% {:>9.1}% {:>7.1}%", r * 100.0, d * 100.0, rand * 100.0, orac * 100.0);
    }

    // A quel taux d'escalade la dispersion atteint-elle la qualite du gros ?
    let mut reach = None;
    for esc in 0..=n {
        if blended(&by_disp, esc) >= big_acc - 1e-9 {
            reach = Some(esc);
            break;
        }
    }
    match reach {
        Some(e) => println!("\n  -> escalade-par-dispersion atteint la qualite du gros ({:.1}%) en escaladant {:.0}% des cas.", big_acc * 100.0, e as f64 / n as f64 * 100.0),
        None => println!("\n  -> escalade-par-dispersion n'atteint pas la qualite du gros meme a 100% (diagnostic faible)."),
    }

    println!("\nre-preuve : petit {}/{} · gros {}/{} runs re-prouves · la mesure est scellee et re-prouvable", small_v, small_items.len(), big_v, big_items.len());
    println!("NB : dispersion = self-agreement label-free (thermometre) ; jamais agregee en reponse (bulletin). Cout GPU reel = B2.");
    Ok(())
}
