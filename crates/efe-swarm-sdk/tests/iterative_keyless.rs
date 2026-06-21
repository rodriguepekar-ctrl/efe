// SPDX-License-Identifier: UNLICENSED
//! Intégration KEYLESS des topologies itératives (T4 Gossip · Debate) à travers
//! tout le SDK : `run_traced` → Meter (budget I13 + capture) → scellement →
//! `verify_swarm` — la preuve doit tenir sur l'itératif comme sur le one-shot.

use efe_swarm_sdk::{Budget, ConvergenceMetric, OnExceed, Pattern, Swarm, TopologySpec};
use serde_json::json;

fn gossip_spec(k: usize, rounds: usize) -> TopologySpec {
    TopologySpec::Gossip {
        refiner: "mean_pull".into(),
        k,
        max_rounds: rounds,
        epsilon: 0.1,
        metric: ConvergenceMetric::Variance,
    }
}

fn debate_spec(k: usize, rounds: usize) -> TopologySpec {
    TopologySpec::Debate {
        generator: "echo_generator".into(),
        judges: vec!["j_equals_expected".into(), "consistency".into(), "length_band".into()],
        pro: "argue for".into(),
        contra: "argue against".into(),
        k,
        pattern: Pattern::Borda,
        rounds,
        escalate: false,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gossip_run_is_sealed_and_reproven() {
    let swarm = Swarm::keyless();
    let task = json!({ "initial": [90.0, 110.0, 95.0, 105.0], "text": "estimate" });
    let out = swarm
        .run_traced(task, gossip_spec(4, 10), Budget::cents(400))
        .await
        .expect("run keyless");

    // La preuve tient sur l'itératif : scellé + re-prouvé + 100 % ancré.
    assert!(out.run.provenance.verified, "verify_swarm re-prouve le gossip");
    assert!((out.run.provenance.traced_pct - 1.0).abs() < 1e-9, "chaque refine ancré à sa cellule");
    assert!(out.run.provenance.root.is_some());

    // La sortie converge vers la moyenne, et le DIT.
    let v = out.run.output.pointer("/value").and_then(|x| x.as_f64()).expect("valeur");
    assert!((v - 100.0).abs() < 0.5, "converge vers 100, eu {v}");
    assert_eq!(out.run.output.pointer("/gossip/converged"), Some(&json!(true)));

    // Comptabilité : K × tours, au centime (I13 en dessous du plafond).
    let rounds = out.run.output.pointer("/gossip/rounds").and_then(|x| x.as_u64()).unwrap() as usize;
    assert_eq!(out.run.accounting.agents_used, 4 * rounds);
    assert_eq!(out.run.budget.spent_cents, (4 * rounds) as u32, "keyless = 1 ¢/appel");
    assert!(out.run.budget.spent_cents <= out.run.budget.max_cost_cents);
    assert_eq!(out.sealed.as_ref().map(|s| s.records.len()), Some(4 * rounds), "1 record par refine");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn debate_run_is_sealed_and_reproven() {
    let swarm = Swarm::keyless();
    // echo_generator renvoie la tâche → tous les candidats identiques : le débat et
    // le jury tournent, Borda tranche trivialement — on teste la PLOMBERIE prouvée.
    let task = json!({ "text": "t", "expected": "t" });
    let out = swarm
        .run_traced(task, debate_spec(3, 2), Budget::cents(400))
        .await
        .expect("run keyless");

    assert!(out.run.provenance.verified, "verify_swarm re-prouve le débat");
    assert!((out.run.provenance.traced_pct - 1.0).abs() < 1e-9, "cand + arguments + ballots ancrés");
    // K + 2K×rounds + J = 3 + 12 + 3.
    assert_eq!(out.run.accounting.agents_used, 3 + 2 * 3 * 2 + 3);
    assert_eq!(out.sealed.as_ref().map(|s| s.records.len()), Some(3 + 12 + 3));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn degrade_cuts_rounds_first_to_fit_budget() {
    let swarm = Swarm::keyless();
    // Demandé : K=4 × 10 tours = 40 appels. Budget : 12 ¢ → r dégradé à 3 (I-IT4).
    let task = json!({ "initial": [0.0, 0.0, 0.0, 0.0] }); // déjà convergé → 1 tour suffit
    let out = swarm
        .run_traced(
            task,
            gossip_spec(4, 10),
            Budget::cents(12).with_policy(OnExceed::Degrade),
        )
        .await
        .expect("run dégradé");
    assert!(out.run.budget.spent_cents <= 12, "jamais au-delà du plafond (I13)");
    assert!(out.run.provenance.verified);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn error_policy_rejects_an_undersized_budget_upfront() {
    let swarm = Swarm::keyless();
    let err = swarm
        .run_traced(
            json!({ "initial": [1.0, 2.0] }),
            gossip_spec(4, 10), // estimé 40 appels = 40 ¢
            Budget::cents(5).with_policy(OnExceed::Error),
        )
        .await
        .expect_err("pré-vol : refus net");
    let msg = format!("{err}");
    assert!(msg.contains("budget"), "erreur de budget explicite, eu : {msg}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn debate_escalation_economises_through_the_sdk() {
    let swarm = Swarm::keyless();
    let spec = TopologySpec::Debate {
        generator: "const_generator".into(),
        judges: vec!["j_equals_expected".into()],
        pro: "for".into(),
        contra: "against".into(),
        k: 4,
        pattern: Pattern::Borda,
        rounds: 3,
        escalate: true,
    };
    // const_generator : candidats UNANIMES → le Vote gratuit suffit (étage ①).
    // (echo_generator tamponne le variant → candidats distincts → escalade complète.)
    let out = swarm
        .run_traced(json!({ "text": "t", "expected": "t" }), spec, Budget::cents(100))
        .await
        .expect("run keyless");
    assert!(out.run.provenance.verified);
    assert_eq!(out.run.accounting.agents_used, 4, "escalade : K seulement, débat économisé");
    assert_eq!(out.run.budget.spent_cents, 4);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn influence_graph_is_sealed_as_reads_and_dag_checked() {
    // §4.3 de l'architecture, réalisé de bout en bout : chaque refine du tour r ≥ 1
    // DÉCLARE ses lectures (sa cellule + les pairs, pointer@version exacte) ; le
    // Meter les scelle dans le record ; verify_swarm contrôle le DAG (anti-version-
    // fantôme) — et le run reste intégralement re-prouvé.
    let swarm = Swarm::keyless();
    let task = json!({ "initial": [10.0, 20.0, 30.0] });
    let out = swarm
        .run_traced(task, gossip_spec(3, 3), Budget::cents(100))
        .await
        .expect("run keyless");
    assert!(out.run.provenance.verified, "le DAG (reads compris) se re-prouve");

    let records = &out.sealed.as_ref().expect("scellé").records;
    // Tour 0 : aucune lecture (pas de pairs). Tours ≥ 1 : 3 lectures par record
    // (sa cellule + 2 pairs), toutes à version 1 (write-once), pointant /gossip/<r−1>/.
    let with_reads: Vec<_> = records.iter().filter(|r| !r.reads.is_empty()).collect();
    let rounds = out.run.output.pointer("/gossip/rounds").and_then(|x| x.as_u64()).unwrap() as usize;
    assert_eq!(with_reads.len(), 3 * (rounds - 1), "K lectures déclarées par tour ≥ 1");
    for rec in &with_reads {
        assert_eq!(rec.reads.len(), 3, "sa cellule + 2 pairs");
        for cell in &rec.reads {
            assert!(cell.pointer.starts_with("/gossip/"), "lecture de cellule de tour : {}", cell.pointer);
            assert_eq!(cell.version, 1, "write-once ⇒ version exacte 1");
        }
    }

    // Le débat déclare aussi : l'arguer lit le CANDIDAT (+ l'adverse aux tours ≥ 1).
    let out = swarm
        .run_traced(json!({ "text": "t", "expected": "t" }), debate_spec(2, 2), Budget::cents(100))
        .await
        .expect("run keyless");
    assert!(out.run.provenance.verified);
    let records = &out.sealed.as_ref().expect("scellé").records;
    let arguer_reads: Vec<_> = records
        .iter()
        .filter(|r| r.agent_id.as_str() == "pro" || r.agent_id.as_str() == "contra")
        .collect();
    assert_eq!(arguer_reads.len(), 2 * 2 * 2, "2K arguers × 2 tours");
    assert!(
        arguer_reads.iter().all(|r| r.reads.iter().any(|c| c.pointer.starts_with("/cand/"))),
        "chaque argument déclare la lecture de SON candidat"
    );
    let round2: Vec<_> = arguer_reads.iter().filter(|r| r.reads.len() == 2).collect();
    assert_eq!(round2.len(), 2 * 2, "au tour 1, chaque arguer lit AUSSI l'adverse du tour 0");
    assert!(
        round2.iter().all(|r| r.reads.iter().any(|c| c.pointer.starts_with("/debate/0/"))),
        "la lecture adverse pointe le tour précédent"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fleet_stage_serves_as_refiner_fallback() {
    // Un nom NON réservé en keyless tombe sur l'adaptation d'une Stage de flotte (StageRefiner) :
    // "trim" reçoit le wrapper gossip {task, own, peers} — qui n'a pas de champ `text` au top-niveau,
    // donc sa sortie est {text:""}, un candidat ABSENT. Depuis le fix sentinel (B0.2/I-SENT), des
    // candidats absents N'AGRÉENT PLUS : pas de fausse convergence au tour 1 → le gossip épuise
    // max_rounds. Le chemin de repli StageRefiner est néanmoins exercé de bout en bout, preuve comprise.
    let swarm = Swarm::keyless();
    let spec = TopologySpec::Gossip {
        refiner: "trim".into(), // stage de flotte, pas un builtin
        k: 3,
        max_rounds: 4,
        epsilon: 0.0,
        metric: ConvergenceMetric::Agreement,
    };
    let out = swarm
        .run_traced(json!({ "text": "  x  " }), spec, Budget::cents(50))
        .await
        .expect("fallback StageRefiner");
    assert!(out.run.provenance.verified, "le chemin de repli StageRefiner se re-prouve");
    // Sorties dégénérées (vides) ⇒ pas de fausse convergence (I-SENT) ⇒ épuise max_rounds.
    assert_eq!(out.run.output.pointer("/gossip/converged"), Some(&json!(false)), "candidats vides → PAS de fausse convergence");
    assert_eq!(out.run.output.pointer("/gossip/rounds"), Some(&json!(4)), "épuise max_rounds faute de vraie convergence");
    assert_eq!(out.run.accounting.agents_used, 12, "3 refiners × 4 tours (plus d'arrêt prématuré au tour 1)");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unknown_refiner_is_a_clean_typed_error() {
    let swarm = Swarm::keyless();
    let spec = TopologySpec::gossip("definitely_not_an_agent", 3, 2, 0.1);
    let err = swarm
        .run_traced(json!({}), spec, Budget::cents(50))
        .await
        .expect_err("nom inconnu → erreur nette");
    let msg = format!("{err}");
    assert!(msg.contains("refiner") && msg.contains("definitely_not_an_agent"),
            "l'erreur nomme le rôle ET le nom : {msg}");
}

