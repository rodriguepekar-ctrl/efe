// SPDX-License-Identifier: UNLICENSED
//! **Famille I-AK (ADR-014)** — la taille d'essaim adaptative, à travers tout le SDK :
//! `run_traced` → sizer (vagues bornées) → Meter (budget I13 + capture) → scellement →
//! `verify_swarm`. L'économie doit être réelle, bornée, re-prouvée — et re-dérivable.

use efe_swarm_sdk::{
    Budget, ClientMode, KSpec, OnExceed, RunOutcome, StopRule, Swarm, SwarmError, TopologySpec,
};
use efe_swarm_topology::agreement;
use serde_json::{json, Value};

/// Le nombre de records GÉNÉRATEUR scellés (la preuve du K réellement dépensé).
fn generator_records(out: &RunOutcome) -> usize {
    out.sealed
        .as_ref()
        .expect("scellé")
        .records
        .iter()
        .filter(|r| r.agent_id.as_str() == "generator" || r.agent_id.as_str() == "t1_generator")
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adaptive_consensus_stops_at_k_min_when_unanimous() {
    // const_generator : candidats UNANIMES → accord 1.0 dès la première vague → le
    // sizer s'arrête au plancher. 4 + 1 juge = 5 appels au lieu de 24 + 1 (−79 %).
    let swarm = Swarm::keyless();
    let spec = TopologySpec::consensus_adaptive("const_generator", &["consistency"], 4, 24, 0.9);
    let out = swarm
        .run_traced(json!({ "text": "easy instance" }), spec, Budget::cents(100))
        .await
        .expect("run keyless");

    assert!(out.run.provenance.verified, "re-prouvé");
    assert_eq!(out.run.output["sizing"]["k_used"], json!(4), "arrêt au plancher k_min (I-AK1)");
    assert_eq!(out.run.output["sizing"]["stop_reason"], json!("target"));
    assert_eq!(out.run.accounting.agents_used, 4 + 1, "K_used + juge");
    assert_eq!(out.run.budget.spent_cents, 5, "l'économie est FACTURÉE : 5 ¢, pas 25 ¢");
    assert_eq!(generator_records(&out), 4, "la preuve porte exactement K_used générations");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adaptive_equals_fixed_when_target_is_unreachable() {
    // I-AK5 + I-AK3 : echo_generator tamponne le variant → candidats tous distincts →
    // accord ~0, cible 0.9 inatteignable → l'adaptatif déroule jusqu'à k_max — et il
    // est alors INDISTINGUABLE d'un run fixe de même K : mêmes candidats (variant =
    // seule graine), même gagnant, même dépense. Le sizer choisit COMBIEN, jamais QUOI.
    let swarm = Swarm::keyless();
    let task = json!({ "text": "hard instance", "expected": "hard instance" });
    let adaptive = TopologySpec::consensus_adaptive("echo_generator", &["consistency"], 3, 7, 0.9);
    let fixed = TopologySpec::consensus("echo_generator", &["consistency"], 7);

    let a = swarm.run_traced(task.clone(), adaptive, Budget::cents(100)).await.expect("adaptatif");
    let f = swarm.run_traced(task, fixed, Budget::cents(100)).await.expect("fixe");

    assert_eq!(a.run.output["sizing"]["k_used"], json!(7), "plafond dur atteint (I-AK1)");
    assert_eq!(a.run.output["sizing"]["stop_reason"], json!("k_max"));
    assert_eq!(a.run.output["value"], f.run.output, "même gagnant que le K fixe (I-AK5)");
    assert_eq!(a.run.accounting.agents_used, f.run.accounting.agents_used);
    assert_eq!(a.run.budget.spent_cents, f.run.budget.spent_cents, "même dépense au pire cas");
    assert!(a.run.provenance.verified && f.run.provenance.verified);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adaptive_t1_verified_quota_economises_through_the_sdk() {
    // Le cas-phare (zone vérifiable) : echo renvoie la tâche → `equals_expected`
    // passe dès le variant 0 → quota 1 atteint dans la première vague → 2 appels
    // au lieu de 8 (÷4), re-prouvés.
    let swarm = Swarm::keyless();
    let spec = TopologySpec::generate_verify_adaptive("echo_generator", "equals_expected", 2, 8, 1);
    let out = swarm
        .run_traced(json!({ "text": "x", "expected": "x" }), spec, Budget::cents(100))
        .await
        .expect("run keyless");

    assert!(out.run.provenance.verified);
    assert_eq!(out.run.output["sizing"]["k_used"], json!(2), "quota dès la 1ʳᵉ vague → k_min");
    assert_eq!(out.run.output["sizing"]["stop_reason"], json!("target"));
    assert_eq!(out.run.output["value"]["text"], json!("x"), "le candidat vérifié est rendu");
    assert_eq!(out.run.accounting.agents_used, 2);
    assert_eq!(out.run.budget.spent_cents, 2, "2 ¢ au lieu de 8 ¢");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_partial_wave_under_budget() {
    // I-AK4 : plafond 6 ¢ — la vague k_min (4) est payée ; la suivante (4 de plus)
    // dépasserait → REFUSÉE AVANT émission. Arrêt propre `budget`, zéro sentinelle
    // dans l'échantillon, puis le juge est payé (5 ¢ ≤ 6 ¢). I13 jamais percé.
    let swarm = Swarm::keyless();
    let spec = TopologySpec::consensus_adaptive("echo_generator", &["consistency"], 4, 16, 0.9);
    let out = swarm
        .run_traced(json!({ "text": "t" }), spec, Budget::cents(6).with_policy(OnExceed::Stop))
        .await
        .expect("run keyless");

    assert_eq!(out.run.output["sizing"]["stop_reason"], json!("budget"));
    assert_eq!(out.run.output["sizing"]["k_used"], json!(4), "la dernière vague entièrement payée");
    assert_eq!(out.run.budget.spent_cents, 5, "4 générations + 1 juge — jamais au-delà (I13)");
    assert_eq!(generator_records(&out), 4);
    // Revue ADR-014 : le refus COOPÉRATIF (sonde, sans charge() échoué) doit marquer
    // l'épuisement — un run tronqué par le budget n'est JAMAIS « propre ».
    assert!(out.run.budget_stopped, "politique Stop + troncature → budget_stopped");
    assert!(!out.run.is_clean(), "résultat partiel ⇒ pas propre");
    // zéro sentinelle : chaque record générateur porte une sortie non-triviale
    // (echo ⇒ objet) — le hash d'un Null serait identique pour tous, or les variants
    // tamponnés rendent les sorties toutes DISTINCTES.
    let outputs: std::collections::BTreeSet<String> = out
        .sealed
        .as_ref()
        .unwrap()
        .records
        .iter()
        .filter(|r| r.agent_id.as_str() == "generator")
        .map(|r| r.output_hash.to_hex())
        .collect();
    assert_eq!(outputs.len(), 4, "4 sorties distinctes = aucun sentinel Null dans l'échantillon");
    assert!(out.run.provenance.verified);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn t1_truncated_to_zero_sample_is_visibly_not_clean() {
    // Le pire cas du chemin coopératif (revue ADR-014) : T1 adaptatif, AUCUN agent
    // métré hors générateur, budget incapable de payer MÊME la première vague →
    // échantillon vide, value null — et le run le DIT : budget_stopped, pas propre.
    let swarm = Swarm::keyless();
    let spec = TopologySpec::generate_verify_adaptive("echo_generator", "equals_expected", 4, 16, 1);
    let out = swarm
        .run_traced(json!({ "text": "x", "expected": "x" }), spec, Budget::cents(2).with_policy(OnExceed::Stop))
        .await
        .expect("run keyless");
    assert_eq!(out.run.output["sizing"]["k_used"], json!(0), "première vague (4) impayable à 2 ¢");
    assert_eq!(out.run.output["sizing"]["stop_reason"], json!("budget"));
    assert_eq!(out.run.output["value"], serde_json::Value::Null, "aucun candidat = aucune valeur");
    assert_eq!(generator_records(&out), 0, "zéro appel facturé, zéro record fantôme");
    assert!(out.run.budget_stopped, "la sonde refusée MARQUE l'épuisement");
    assert!(!out.run.is_clean(), "un run vide n'est jamais propre");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn error_policy_prices_the_worst_case_kmax() {
    // I-AK4 (pré-vol) : la politique Error chiffre k_max (24) + 1 juge = 25 ¢ —
    // un plafond de 10 ¢ est refusé NET, avant tout appel, même si l'instance
    // se serait arrêtée à k_min. L'admission budgétaire ne spécule jamais.
    let swarm = Swarm::keyless();
    let spec = TopologySpec::consensus_adaptive("const_generator", &["consistency"], 4, 24, 0.9);
    let err = swarm
        .run_traced(json!({ "text": "t" }), spec, Budget::cents(10).with_policy(OnExceed::Error))
        .await
        .expect_err("pré-vol : refus net");
    assert!(matches!(err, SwarmError::BudgetTooLow { needed_cents: 25, max_cents: 10 }), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn degrade_policy_clamps_kmax_then_runs() {
    // Dégradation : 10 ¢ → k_max raboté (9 = 10 − 1 juge réservé), puis le run passe
    // et s'arrête de toute façon à k_min (const → unanime). I13 tenu.
    let swarm = Swarm::keyless();
    let spec = TopologySpec::consensus_adaptive("const_generator", &["consistency"], 4, 24, 0.9);
    let out = swarm
        .run_traced(json!({ "text": "t" }), spec, Budget::cents(10).with_policy(OnExceed::Degrade))
        .await
        .expect("run dégradé");
    assert_eq!(out.run.output["sizing"]["policy"]["k_max"], json!(9), "plafond raboté au budget");
    assert_eq!(out.run.output["sizing"]["k_used"], json!(4), "et l'instance facile coûte k_min");
    assert!(out.run.budget.spent_cents <= 10, "I13");
    assert!(out.run.provenance.verified);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sizing_trace_matches_offline_rederivation() {
    // I-AK2 : la trajectoire du sizer est une fonction PURE des candidats — un tiers
    // qui sait ce que produit l'agent (Mode A : fonction de (task, variant)) recompte
    // la trace entière hors ligne. Ici : echo_generator = task ⊕ {variant} — on
    // reconstruit les candidats, on recalcule l'accord par préfixe de vague, et on
    // exige l'égalité avec la trace scellée dans la sortie.
    let swarm = Swarm::keyless();
    let task = json!({ "text": "rederive me", "expected": "rederive me" });
    let spec = TopologySpec::Consensus {
        generator: "echo_generator".into(),
        judges: vec!["consistency".into()],
        k: KSpec::Adaptive { k_min: 2, k_max: 6, batch: 2, stop: StopRule::Agreement { threshold: 0.9 } },
        pattern: efe_swarm_sdk::PatternSpec::Fixed(efe_swarm_sdk::Pattern::Borda),
        escalate: false,
    };
    let out = swarm.run_traced(task.clone(), spec, Budget::cents(100)).await.expect("run");

    // ── re-dérivation hors ligne ─────────────────────────────────────────────
    let rebuild = |i: usize| -> Value {
        let mut c = task.clone();
        c.as_object_mut().unwrap().insert("variant".into(), json!(i));
        c
    };
    let k_used = out.run.output["sizing"]["k_used"].as_u64().unwrap() as usize;
    assert_eq!(k_used, 6, "distincts → jamais 0.9 → k_max (re-dérivé : aucune vague ne satisfait)");
    let batches = out.run.output["sizing"]["batches"].as_array().unwrap();
    assert_eq!(batches.len(), 3, "vagues 2, 4, 6");
    for b in batches {
        let k = b["k"].as_u64().unwrap() as usize;
        let observed = b["observed"].as_f64().unwrap();
        let cands: Vec<Value> = (0..k).map(rebuild).collect();
        let recomputed = agreement(&cands);
        assert!(
            (recomputed - observed).abs() < 1e-6,
            "vague k={k} : trace {observed} vs re-dérivé {recomputed}"
        );
        assert!(recomputed < 0.9, "aucune vague ne satisfait la cible (cohérent avec k_max)");
    }

    // ── I-AK3 (déterminisme) : le MÊME run re-exécuté produit la MÊME trace. ──
    let spec2 = TopologySpec::Consensus {
        generator: "echo_generator".into(),
        judges: vec!["consistency".into()],
        k: KSpec::Adaptive { k_min: 2, k_max: 6, batch: 2, stop: StopRule::Agreement { threshold: 0.9 } },
        pattern: efe_swarm_sdk::PatternSpec::Fixed(efe_swarm_sdk::Pattern::Borda),
        escalate: false,
    };
    let out2 = swarm.run_traced(task, spec2, Budget::cents(100)).await.expect("run 2");
    assert_eq!(out.run.output["sizing"], out2.run.output["sizing"], "trajectoire rejouée à l'identique");
}

/// **Pont N17 sous K adaptatif, RÉEL** (Ollama local). Ignoré par défaut (CI sans
/// modèle). Lancer : `cargo test -p efe-swarm-sdk --test adaptive_keyless -- --ignored`.
/// Ce que ça prouve (architecture §7.2, Mode C) : quand K varie EN VOL, chaque appel
/// LLM garde son `LlmProvenanceRecord` forensique lié à son record d'essaim — la
/// trajectoire adaptative reste auditable même non rejouable.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "nécessite un Ollama local (ollama serve + gemma3:12b)"]
async fn n17_consistent_under_adaptive_k() {
    let swarm = Swarm::builder()
        .mode(ClientMode::SelfHost { endpoint: "http://127.0.0.1:11434".into(), model: "gemma3:12b".into() })
        .require_healthy_backend(true)
        .build()
        .unwrap();
    let spec = TopologySpec::consensus_adaptive(
        "Classify the topic in one word from {technology,finance,health,legal}.",
        &["Judge how well the label fits."],
        2,
        4,
        0.9,
    );
    let out = swarm
        .run_traced(
            json!({ "text": "The court ruled the clause unenforceable.", "expected": "legal" }),
            spec,
            Budget::cents(1000),
        )
        .await
        .unwrap();

    assert!(out.run.provenance.verified, "scellé + re-prouvé sous K adaptatif");
    let k_used = out.run.output["sizing"]["k_used"].as_u64().expect("trace de sizing") as usize;
    assert!((2..=4).contains(&k_used), "I-AK1 tenu en Mode C : k_used = {k_used}");
    let n17 = out.run.n17.as_ref().expect("résumé N17 présent (Mode C)");
    assert!(n17.all_consistent, "chaque Mode C lié à un N17 cohérent : {n17:?}");
    assert_eq!(n17.unlinked, 0);
    assert_eq!(n17.broken, 0);
    assert!(n17.calls > k_used, "≥ K_used générations + ≥ 1 juge, tous forensiques");
    assert!(out.run.accounting.width >= 1, "largeur backend résolue et rapportée");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adaptive_composes_with_judge_escalation() {
    // Les deux adaptativités composent sans se voir : le sizer décide COMBIEN de
    // candidats (unanimes → k_min), l'escalade décide si on paie les juges (vote
    // unanime confiant → 0 juge). Une instance triviale traverse TOUT le système
    // pour k_min appels.
    let swarm = Swarm::keyless();
    let spec = TopologySpec::Consensus {
        generator: "const_generator".into(),
        judges: vec!["consistency".into(), "length_band".into()],
        k: KSpec::Adaptive { k_min: 4, k_max: 24, batch: 4, stop: StopRule::Agreement { threshold: 0.9 } },
        pattern: efe_swarm_sdk::PatternSpec::Fixed(efe_swarm_sdk::Pattern::Borda),
        escalate: true, // Vote gratuit d'abord ; juges seulement si hésitation
    };
    let out = swarm.run_traced(json!({ "text": "t" }), spec, Budget::cents(100)).await.expect("run");
    assert_eq!(out.run.output["sizing"]["k_used"], json!(4), "sizer : plancher");
    assert_eq!(out.run.accounting.agents_used, 4, "escalade : 0 juge payé — 4 appels au TOTAL");
    assert_eq!(out.run.budget.spent_cents, 4, "l'instance triviale coûte k_min, point");
    assert!(out.run.provenance.verified);
}
