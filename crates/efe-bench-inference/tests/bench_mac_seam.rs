// SPDX-License-Identifier: UNLICENSED
//! **B1.6 — la preuve que l'instrument mesure le VRAI, sur Mac, sans modèle.**
//!
//! Le test central de la frontière B1. On constitue un petit dataset à labels connus et textes
//! uniques ; on **script** le SLM pour être JUSTE sur deux lignes et FAUX sur une (via
//! `ScriptedBackend::matching`, clé-é sur le texte de chaque ligne — qui apparaît dans le prompt
//! Mode C) ; puis on lance le harnais et on **asserte** : exactitude = 2/3 (la notation marche),
//! jetons comptés et marqués estimés (la comptabilité marche), comparaison à coût égal entre deux
//! configs (I-BENCH-FAIR), bundle scellé + re-prouvé + anti-cueillette (I-BENCH-REPROVE) — le tout
//! sans aucun GPU ni réseau (I-BENCH-MAC). Quand le GPU arrivera, seul le backend changera
//! (`Scripted` → `SelfHost`) ; cet instrument est déjà prouvé juste.
//!
//! Gaté `#[cfg(feature = "test-util")]` (I-SEAM : le seam est absent du build par défaut).
//!   cargo test -p efe-bench-inference --features test-util --test bench_mac_seam
#![cfg(feature = "test-util")]

use efe_bench_inference::{
    run_config, BenchBackend, BenchBundle, BenchCase, BenchConfig, Comparison, Dataset, PriceModel, ScorerKind,
};
use efe_swarm_sdk::{KSpec, ScriptedBackend, TopologySpec};
use serde_json::json;

/// Dataset à réponse fermée (classification de sujet), textes à préfixe UNIQUE pour le scriptage.
fn dataset() -> Dataset {
    Dataset::new(vec![
        BenchCase::new("c1", json!({ "text": "alpha the central bank raised its rate" }), json!("finance"), ScorerKind::Exact),
        BenchCase::new("c2", json!({ "text": "bravo the striker scored a late goal" }), json!("sport"), ScorerKind::Exact),
        BenchCase::new("c3", json!({ "text": "charlie the new vaccine trial began" }), json!("health"), ScorerKind::Exact),
    ])
}

/// SLM scripté : JUSTE sur c1/c2, FAUX sur c3 (`tumor` ≠ `health`). Les aiguilles sont les préfixes
/// uniques ; le prompt Mode C embarque le texte de la ligne (`gen_prompt`), donc chaque règle cible
/// exactement sa ligne. Le banc DOIT donc mesurer une exactitude de 2/3.
fn scripted_slm() -> BenchBackend {
    BenchBackend::Scripted(ScriptedBackend::matching(vec![
        ("alpha".into(), "finance".into()),
        ("bravo".into(), "sport".into()),
        ("charlie".into(), "tumor".into()),
    ]))
}

const CLASSIFY: &str = "Classify the topic in one word.";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn harness_measures_accuracy_truthfully_via_seam() {
    let ds = dataset();
    let backend = scripted_slm();
    let cfg = BenchConfig::new("swarm-k4", TopologySpec::consensus(CLASSIFY, &["consistency"], 4));

    let report = run_config(&ds, &cfg, &backend, 1_000_000, 4).await.expect("run config (seam)");

    // ① La JUSTESSE est mesurée FIDÈLEMENT : 2 corrects sur 3 (c3 scripté faux). C'est la preuve que
    //    la notation distingue le juste du faux — pas un 0/3 ni un 3/3 trivial.
    assert_eq!(report.cases, 3);
    assert_eq!(report.correct, 2, "c1+c2 corrects, c3 faux (tumor≠health)");
    assert!((report.accuracy - 2.0 / 3.0).abs() < 1e-9, "exactitude = 2/3, eu {}", report.accuracy);
    let c3 = report.records.iter().find(|r| r.case_id == "c3").expect("c3");
    assert!(!c3.correct, "c3 = faux, correctement identifié");
    assert_eq!(c3.winner, json!({ "value": "tumor" }), "le gagnant scoré est bien la réponse scriptée");

    // ② Le COÛT-jetons est compté (depuis les records N17 par-appel) et marqué ESTIMÉ (scripté →
    //    proxy, aucun compte fournisseur). I-BENCH-PROXY.
    assert!(report.total_tokens > 0, "jetons sommés depuis llm_records");
    assert!(report.tokens_estimated, "scripté → estimé (le drapeau remonte au rapport)");

    // ③ La PREUVE tient SOUS le seam : chaque run R0-re-prouve, porte ses racines, et son barreau
    //    d'attestation est R2 (loopback témoin). Le moat s'applique à la mesure elle-même.
    assert_eq!(report.verified, 3, "R0 re-prouve chaque run");
    assert!((report.verified_pct - 1.0).abs() < 1e-9);
    for rec in &report.records {
        assert!(rec.provenance_root.is_some(), "racine de provenance présente ({})", rec.case_id);
        assert!(rec.n17_root.is_some(), "racine N17 présente ({})", rec.case_id);
        assert!(rec.n17_calls >= 1, "≥ 1 appel LLM forensique ({})", rec.case_id);
        assert_eq!(rec.attestation.as_deref(), Some("R2-LOCAL-TEMOIN"), "barreau loopback témoin");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn comparison_is_fair_and_bundle_reproves_via_seam() {
    let ds = dataset();
    let backend = scripted_slm();

    // La thèse en miniature : l'essaim (K=4 petits) vs un gros modèle de référence (K=1), MÊME
    // dataset, MÊME scorer, MÊME backend → I-BENCH-FAIR.
    let swarm = BenchConfig::new("swarm-k4", TopologySpec::consensus(CLASSIFY, &["consistency"], 4));
    let baseline = BenchConfig::new(
        "baseline-k1",
        TopologySpec::GenerateVerify { generator: CLASSIFY.into(), verifier: "non_empty_text".into(), k: KSpec::Fixed(1) },
    );

    let a = run_config(&ds, &swarm, &backend, 1_000_000, 4).await.expect("swarm");
    let b = run_config(&ds, &baseline, &backend, 1_000_000, 4).await.expect("baseline");

    // Même justesse sous le même backend (les deux voient les mêmes réponses scriptées) : 2/3.
    assert_eq!(a.correct, 2, "essaim : 2/3");
    assert_eq!(b.correct, 2, "baseline : 2/3");

    // L'axe COÛT, mesuré : l'essaim (4 générations/ligne) consomme STRICTEMENT plus de jetons que la
    // référence (1 génération/ligne). C'est exactement le compromis que la thèse met en balance.
    assert!(a.total_tokens > b.total_tokens, "essaim {} jetons > baseline {} jetons", a.total_tokens, b.total_tokens);
    assert!(a.mean_agents > b.mean_agents, "essaim {} agents/run > baseline {}", a.mean_agents, b.mean_agents);

    // Le rapport comparatif liste les deux configs + un coût pondéré (modèle de prix externe).
    let cmp = Comparison::new(vec![a.clone(), b.clone()])
        .with_price(PriceModel::new().with("swarm-k4", 0.2).with("baseline-k1", 4.0));
    let table = cmp.table();
    assert!(table.contains("swarm-k4") && table.contains("baseline-k1"), "les deux configs tabulées :\n{table}");

    // I-BENCH-REPROVE : le bundle scelle un benchmark re-prouvable et infalsifiable.
    let bundle = BenchBundle::seal(&ds, vec![a, b]);
    let verdict = bundle.verify();
    assert!(verdict.ok, "bundle re-prouvable : {verdict:?}");
    assert!(verdict.all_verified, "tous les runs R0-vérifiés");
    assert!(verdict.counts_consistent, "2 cas × 2 configs, aucun manquant");
    assert!(verdict.roots_present, "racines présentes partout");
    assert!(verdict.manifest_root_matches, "racine de manifeste cohérente");
    assert_eq!(bundle.manifest_root.len(), 64, "racine blake3 (hex 32 octets)");
    assert_eq!(bundle.cases, 3, "le bundle connaît le compte de cas du dataset");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_saturated_backend_does_not_fake_correctness() {
    // Le scénario « GPU saturé » (B0.2) côté banc : un backend qui ÉCHOUE → sentinels vides → aucune
    // réponse présente → exactitude 0, et le banc le dit honnêtement (pas une fausse réussite).
    let ds = dataset();
    let backend = BenchBackend::Scripted(ScriptedBackend::failing());
    let cfg = BenchConfig::new("swarm-k4", TopologySpec::consensus(CLASSIFY, &["consistency"], 4));

    let report = run_config(&ds, &cfg, &backend, 1_000_000, 4).await.expect("run (backend défaillant)");
    assert_eq!(report.correct, 0, "aucune réponse présente → 0 correct (pas de fausse réussite)");
    assert!((report.accuracy - 0.0).abs() < 1e-9);
    // Le run reste SCELLÉ et re-prouvable même sur échec d'inférence (le moat tient).
    assert_eq!(report.verified, 3, "R0 re-prouve même un run sans réponse");
}
