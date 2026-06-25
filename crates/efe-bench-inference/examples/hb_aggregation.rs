// SPDX-License-Identifier: UNLICENSED
//! # `hb_aggregation` — HB.2 : la mesure de l'agrégation (« la quantité vérifiée paie-t-elle ? »).
//!
//! Fait raisonner (CoT) un **petit** modèle **K fois** par problème de T-MATH, capture les **K
//! candidats bruts par rôle GÉNÉRATEUR** (I-HB-GENONLY, via un `SwarmObserver` — jamais des juges),
//! et passe le tout au module [`efe_bench_inference::aggregation`] : courbes justesse × K
//! (self-consistency, vote pondéré, vérifié, médiane) vs le **plafond pass@K**, comparées à la
//! **référence du gros** (K=1, **même** affordance CoT — I-HB-FAIR), à coût-jetons (entrée/sortie,
//! bornes cache). Verdict à un **K\* pré-spécifié** (`verdict_at_k`, réutilise la non-infériorité de
//! `substitution`) ET sur le **sup-K** (`verdict_sup_k`, FWER contrôlé par max-t). Chaque run est
//! scellé + R0-re-prouvable. La référence du gros est mesurée et **figée AVANT** de lire les courbes
//! (I-HB-PREREG). La domination de DÉPLOIEMENT (GPU-s) reste suspendue à B2.
//!
//! Caveat connu : la température du SDK suit une rampe `0.2+0.3·variant` (non bornée) ; on RAPPORTE la
//! diversité et la dégénérescence par K (la borne T≤1 est un raffinement SDK ultérieur).
//!
//!   cargo run --release --example hb_aggregation -p efe-bench-inference -- \
//!     --endpoint http://127.0.0.1:11434 --small phi4-mini-local --big gemma3:12b \
//!     --n 30 --ks 1,3,5,7 --kstar 5

use std::sync::Mutex;
use std::sync::Arc;

use efe_bench_inference::{
    aggregation::{curve, knee, verdict_at_k, verdict_sup_k, Mechanism, PositiveInteger, SampleSet},
    achieved_power, symmetry_test, Dataset, IoPrice, ReExecAudit, ScorerKind,
};
use efe_swarm_sdk::{
    AgentCall, AgentRole, Budget, ClientMode, KSpec, OnExceed, Swarm, SwarmObserver, TopologySpec,
};
use serde_json::Value;

/// Observateur qui capture les sorties des agents de rôle **Générateur** (et eux seuls — I-HB-GENONLY).
#[derive(Default)]
struct GenCollector {
    out: Mutex<Vec<(u64, Value)>>, // (variant, candidat brut)
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
    /// Les candidats de l'item courant, triés par variante (préfixe stable pour la courbe).
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

/// Estimation grossière de jetons (≈ 4 car./jeton) — proxy client (is_estimated), assumé.
fn est(s: &str) -> u32 {
    ((s.chars().count() / 4) as u32).max(1)
}
fn text_of(task: &Value) -> String {
    task.get("text").and_then(|t| t.as_str()).map(String::from).unwrap_or_default()
}
/// Texte d'un candidat pour l'ESTIMATION de jetons. Ne pèle que `{value}` — suffisant ici car le driver
/// utilise la topologie `GenerateVerify` (réponses `{value: texte}`, jamais d'emballage diagnostic). NB
/// (revue) : si l'on branchait gossip/débat (qui emballent), il faudrait `scorer::extract` pour rester
/// aligné sur `reexec::audit_candidate` ; sous `GenerateVerify`, les deux voient le même texte.
fn cand_text(c: &Value) -> String {
    match c.get("value").unwrap_or(c) {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Exécute un modèle sur tout le dataset, **K générations CoT par item**, candidats capturés PAR
/// RÔLE. Renvoie (samples, runs R0-vérifiés). Séquentiel (l'observateur associe par item ; Ollama
/// sérialise de toute façon).
async fn run_model(
    endpoint: &str,
    model: &str,
    dataset: &Dataset,
    instruction: &str,
    k: usize,
    cot_tokens: u32,
) -> Result<(Vec<SampleSet>, usize), Box<dyn std::error::Error>> {
    let collector = Arc::new(GenCollector::default());
    let swarm = Swarm::builder()
        .mode(ClientMode::SelfHost { endpoint: endpoint.to_string(), model: model.to_string() })
        .max_gen_tokens(cot_tokens) // CoT RÉEL (I-HB-COT) : budget de raisonnement, symétrique petit/gros
        .with_observer(collector.clone())
        .build()?;
    let mut samples = Vec::with_capacity(dataset.len());
    let mut verified = 0usize;
    for case in &dataset.cases {
        collector.clear();
        let spec = TopologySpec::GenerateVerify {
            generator: instruction.into(),
            verifier: "non_empty_text".into(), // vérificateur PUR (non-LLM) → llm_records = K générations
            k: KSpec::Fixed(k),
        };
        let outcome = swarm
            .run_traced(case.blind_task(), spec, Budget::cents(1_000_000).with_policy(OnExceed::Stop))
            .await?;
        if outcome.run.provenance.verified {
            verified += 1;
        }
        let answers = collector.drain_sorted(); // K candidats bruts {value: CoT}
        let in_est = est(&format!("{instruction} {}", text_of(&case.task)));
        let in_tokens = vec![in_est; answers.len()];
        let out_tokens: Vec<u32> = answers.iter().map(|c| est(&cand_text(c))).collect();
        samples.push(SampleSet {
            case_id: case.id.clone(),
            task: case.task.clone(),
            answers,
            label: case.label.clone(),
            scorer: case.scorer.clone(),
            weights: None,
            in_tokens,
            out_tokens,
        });
    }
    Ok((samples, verified))
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = arg("--endpoint").unwrap_or_else(|| "http://127.0.0.1:11434".to_string());
    let small = arg("--small").unwrap_or_else(|| "phi4-mini-local".to_string());
    let big = arg("--big").unwrap_or_else(|| "gemma3:12b".to_string());
    let n: usize = arg("--n").and_then(|s| s.parse().ok()).unwrap_or(30);
    // HB.4 : grille IMPAIRE **et** PAIRE (la synthèse n'a lieu qu'à K pair). K* pré-spécifié = 4 (plus
    // petit K pair au-dessus du genou de coût HB.3, justifié par la latence — JAMAIS par « où ça pique »).
    let ks: Vec<usize> = arg("--ks").unwrap_or_else(|| "1,2,3,4,5,6".to_string()).split(',').filter_map(|s| s.trim().parse().ok()).collect();
    let k_max = *ks.iter().max().unwrap_or(&6);
    let k_star: usize = arg("--kstar").and_then(|s| s.parse().ok()).unwrap_or(4);
    let cot_tokens: u32 = arg("--cot-tokens").and_then(|s| s.parse().ok()).unwrap_or(512);
    let delta: f64 = arg("--delta").and_then(|s| s.parse().ok()).unwrap_or(0.05);
    let (alpha, boot) = (0.05, 2000);
    // CoT SYMÉTRIQUE (I-HB-COT) + extraction du dernier nombre (le scorer Numeric le fait).
    let instruction = "Solve this arithmetic word problem. Think step by step, then write the final answer as a single number on the last line.";

    // T-MATH (corpus HA.2), forcé en scorer numérique exact, sous-échantillon des n premiers.
    let mut dataset = Dataset::from_jsonl(include_str!("../fixtures/ha_math.jsonl"))?;
    dataset.cases.truncate(n);
    for c in &mut dataset.cases {
        c.scorer = ScorerKind::Numeric { tolerance: 0.0 };
    }
    println!("HB.4 — SYNTHÈSE MESURÉE  ·  petit {small} (K≤{k_max} CoT)  vs  gros {big} (K=1 CoT)");
    println!("  T-MATH n={n} · δ={delta} · K* pré-spécifié={k_star} (pair) · ks={ks:?} (impairs+pairs) · max_gen_tokens={cot_tokens} (CoT)\n");

    // ── I-HB-PREREG : la référence du gros est mesurée et FIGÉE d'abord. ──
    println!("[1/2] référence du gros ({big}, K=1, CoT)…");
    let (big_s, big_v) = run_model(&endpoint, &big, &dataset, instruction, 1, cot_tokens).await?;
    // ── Puis le petit, K_max générations CoT par item. ──
    println!("[2/2] essaim petit ({small}, K={k_max}, CoT)…");
    let (small_s, small_v) = run_model(&endpoint, &small, &dataset, instruction, k_max, cot_tokens).await?;

    // ── Courbe justesse × K. ──
    let ver = PositiveInteger::default(); // sanité (filtre) — alimente `Verified`
    let weigher = ReExecAudit::default(); // HB.3 : re-exécution (pondérateur) — alimente `ReExecWeighted`
    let cur = curve(&small_s, &ks, Some(&ver), Some(&weigher));
    let pct = |x: f64| if x.is_nan() { 0.0 } else { x * 100.0 };
    println!("\n── SÉLECTION (plafond pass@K)  vs  SYNTHÈSE (plafond env=enveloppe) · fabrication% · dispersion ──");
    println!(
        "{:>3} {:>6} {:>6} {:>6} │ {:>6} {:>6} {:>6} {:>6} │ {:>6} {:>5} │ {:>6} {:>5} {:>5}",
        "K", "SC", "RXW", "pass@K", "méd", "tronq", "AWM", "env", "fab%", "out", "var", "div%", "ent"
    );
    for p in &cur {
        println!(
            "{:>3} {:>5.1}% {:>5.1}% {:>5.1}% │ {:>5.1}% {:>5.1}% {:>5.1}% {:>5.1}% │ {:>5.0}% {:>5.0} │ {:>6.1} {:>4.0}% {:>5.2}",
            p.k, pct(p.self_consistency), pct(p.re_exec_weighted), pct(p.pass_at_k),
            pct(p.median), pct(p.trimmed_mean), pct(p.audit_weighted_median), pct(p.within_range),
            pct(p.fabrication_frac), p.mean_out,
            if p.dispersion_var.is_nan() { 0.0 } else { p.dispersion_var },
            pct(p.dispersion_distinct), if p.dispersion_entropy.is_nan() { 0.0 } else { p.dispersion_entropy }
        );
    }
    println!("  (fab% = la médiane fabrique-t-elle ? ≈0 ⇒ SÉLECTION lue au plafond pass@K ; >0 ⇒ SYNTHÈSE au plafond env)");
    let big_acc = {
        // SC@1 du gros = sa justesse brute (un échantillon).
        let c = curve(&big_s, &[1], None, None);
        c[0].self_consistency
    };
    println!("référence gros (K=1) : exactitude {:.1}%  ·  R0 {}/{}", big_acc * 100.0, big_v, big_s.len());
    println!("petit : R0 {}/{}  (la mesure est scellée + re-prouvable)", small_v, small_s.len());
    println!("genou (τ=2pts) ≈ K={}", knee(&cur, 0.02));

    // ── Verdict : à K* PRÉ-SPÉCIFIÉ (=4), avec PUISSANCE rapportée (I-HB4-MULTIPLICITÉ, I-HB4-PUISSANCE). ──
    let price_small = IoPrice { in_per_1k: 0.1, out_per_1k: 0.1 };
    let price_big = IoPrice { in_per_1k: 1.2, out_per_1k: 1.2 };
    // (mécanisme, étiquette, synthèse ?) — la synthèse a pour plafond l'enveloppe, pas pass@K.
    let mechs = [
        (Mechanism::SelfConsistency, "SC   ", false),
        (Mechanism::ReExecWeighted, "RXW  ", false),
        (Mechanism::Median, "méd  ", true),
        (Mechanism::TrimmedMeanNumeric, "tronq", true),
        (Mechanism::AuditWeightedMedian, "AWM  ", true),
    ];
    println!("\n── verdict @K*={k_star} (pré-spécifié sur base de coût, JAMAIS sur le pic) · puissance rapportée ──");
    for (mech, name, is_synth) in mechs {
        let v = verdict_at_k(
            &small_s, &big_s, mech, k_star, Some(&ver), Some(&weigher), delta, alpha, boot,
            0x4842_3200u64 ^ k_star as u64, price_small, price_big,
        );
        // Puissance atteinte : reconstruit (big_better, small_better) depuis la discordance McNemar.
        // diff_point = exactitude(gros) − exactitude(petit) = p_b − p_c ; psi = p_b + p_c. REVUE HB.4 :
        // `achieved_power(n, δ, α, p_b, p_c)` attend p_b = **gros mieux**, p_c = **petit mieux** (son
        // delta0 = p_b − p_c = le vrai écart gros−petit). Il faut donc passer les rôles DANS CET ORDRE —
        // l'ancien code les inversait (delta0 = −diff_point), gonflant la puissance affichée.
        let psi = v.nin.discordant as f64 / v.nin.n.max(1) as f64;
        let big_better = ((psi + v.nin.diff_point) / 2.0).max(0.0);
        let small_better = ((psi - v.nin.diff_point) / 2.0).max(0.0);
        let pw = achieved_power(v.nin.n, delta, alpha, big_better, small_better);
        let ceil = if is_synth { "plafond=env" } else { "plafond=pass@K" };
        println!(
            "  {name}@K{k_star}  {:.1}% vs gros {:.1}%  →  {:<12} puissance {:>3.0}%  ·  {ceil}  ·  coût domine {}/{}",
            v.small_acc * 100.0, v.big_acc * 100.0, v.nin.verdict.label(), pw * 100.0,
            v.cost_dominates_cache_off, v.cost_dominates_cache_on
        );
    }

    // ── sup-K PARITÉ-PURE (FWER max-t) : sélection sur K impairs (plafond pass@K), synthèse sur K pairs. ──
    let ks_odd: Vec<usize> = ks.iter().cloned().filter(|k| k % 2 == 1).collect();
    let ks_even: Vec<usize> = ks.iter().cloned().filter(|k| k % 2 == 0).collect();
    println!("\n── sup-K parité-pure (jamais mélanger les plafonds, I-HB4-PARITÉ) ──");
    for (mech, name, is_synth) in mechs {
        let kset = if is_synth { &ks_even } else { &ks_odd };
        if kset.is_empty() {
            continue;
        }
        let sup = verdict_sup_k(&small_s, &big_s, mech, kset, Some(&ver), Some(&weigher), delta, alpha, boot, 0x5402);
        println!(
            "  sup-K [{name}] sur {:?} ({}) : sup={:+.1}pts à K={} · borne inf {:+.1}pts → ∃K non-inférieur : {}",
            kset, if is_synth { "synthèse" } else { "sélection" },
            sup.sup_stat * 100.0, sup.argmax_k, sup.sup_lower * 100.0, sup.non_inferior_some_k
        );
    }

    // ── ANALYSE label-using (HORS verdict — explique POURQUOI, jamais ne décide ; I-HB4-LABELFENCE). ──
    let sym = symmetry_test(&small_s);
    println!("\n── ANALYSE (utilise le label pour EXPLIQUER, hors de tout verdict) ──");
    println!(
        "  symétrie des erreurs : biais signé {:+.2} · asymétrie {:+.2} · label∈enveloppe {:.0}% (n={})",
        sym.signed_bias, sym.skewness, sym.within_range_frac * 100.0, sym.n_items
    );
    println!("  lecture : |biais|≈0 & |asym|<0.1 ⇒ erreurs ~symétriques (la synthèse DEVRAIT payer si dispersée) ;");
    println!("            |asym|>0.5 ⇒ erreur systématique ⇒ tout gain de synthèse à un K est chance/bruit.");

    println!("\n  rappels : I-HB-COST (domination COÛT-JETONS seulement ; GPU-s → B2) ;");
    println!("  I-HB4-PUISSANCE : puissance < 70% ⇒ verdict « indéterminé », à reporter à n≥200 (jamais converti en conclusion).");
    Ok(())
}
