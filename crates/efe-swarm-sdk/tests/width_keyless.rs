// SPDX-License-Identifier: UNLICENSED
//! **Famille I-CP (ADR-014)** — la largeur d'exécution n'est PAS une sémantique.
//!
//! La largeur (permits des vagues) remanie l'ordre d'achèvement des agents ∥ — et rien
//! d'autre. Ces gardiens l'exigent de bout en bout : à (spec, task, graines) égales,
//! toute largeur produit le **même contenu sémantique scellé** (qui, quoi, à partir de
//! quoi, quelles cellules — l'empreinte ci-dessous), et chaque run se re-prouve.
//!
//! Note de précision (honnêteté du périmètre) : la **racine** porte aussi les
//! identités par-run (`record_id` v4, `swarm_id`) — l'entropie d'identité, voulue.
//! L'invariance sous largeur porte donc sur TOUT le contenu hors identités ; le test
//! unitaire `pairing_is_deterministic_under_capture_order` (recorder) prouve qu'à
//! identités normalisées, la racine elle-même est invariante.

use efe_swarm_sdk::{Budget, RunOutcome, Swarm, TopologySpec, WidthSpec};
use serde_json::json;

/// Empreinte SÉMANTIQUE d'un run scellé : tout ce que la largeur n'a pas le droit de
/// changer — (agent, rôle, variant, input_hash, output_hash, reads, writes), triée.
fn fingerprint(out: &RunOutcome) -> Vec<String> {
    let mut fp: Vec<String> = out
        .sealed
        .as_ref()
        .expect("provenance scellée")
        .records
        .iter()
        .map(|r| {
            let reads: Vec<String> =
                r.reads.iter().map(|c| format!("{}@{}", c.pointer, c.version)).collect();
            let writes: Vec<String> =
                r.writes.iter().map(|c| format!("{}@{}", c.pointer, c.version)).collect();
            format!(
                "{}|{:?}|{:?}|{}|{}|r={}|w={}",
                r.agent_id.as_str(),
                r.agent_role,
                r.variant,
                r.input_hash.to_hex(),
                r.output_hash.to_hex(),
                reads.join(","),
                writes.join(","),
            )
        })
        .collect();
    fp.sort();
    fp
}

async fn run_consensus_at(width: usize) -> RunOutcome {
    let swarm = Swarm::builder().width(WidthSpec::Fixed(width)).build().expect("swarm keyless");
    swarm
        .run_traced(
            json!({ "text": "width probe", "expected": "width probe" }),
            TopologySpec::consensus("echo_generator", &["consistency", "length_band", "j_non_empty"], 12),
            Budget::cents(400),
        )
        .await
        .expect("run keyless")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn semantic_content_is_invariant_under_width() {
    // I-CP1 : w ∈ {1, 4, 11, 32} — mêmes records (hors identités), même sortie,
    // même comptabilité ; et chaque run est re-prouvé.
    let mut outs = Vec::new();
    for w in [1usize, 4, 11, 32] {
        let out = run_consensus_at(w).await;
        assert!(out.run.provenance.verified, "w={w} : re-prouvé");
        assert_eq!(out.run.accounting.agents_used, 12 + 3, "w={w} : la comptabilité ignore la largeur");
        assert_eq!(out.run.budget.spent_cents, 15, "w={w} : le budget ignore la largeur (I13)");
        assert_eq!(out.run.accounting.width, w, "la largeur RÉSOLUE est rapportée par-run (§3.3)");
        outs.push(out);
    }
    let reference = fingerprint(&outs[0]);
    assert_eq!(reference.len(), 15, "12 générations + 3 ballots scellés");
    for (i, out) in outs.iter().enumerate().skip(1) {
        assert_eq!(fingerprint(out), reference, "empreinte sémantique identique (w index {i})");
        assert_eq!(out.run.output, outs[0].run.output, "sortie identique sous toute largeur");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn width_one_completes_all_topologies() {
    // I-CP2 : largeur 1 = exécution séquentielle intégrale — les SIX formes de spec
    // traversent le résolveur (qui câble la largeur partout) et terminent, re-prouvées.
    // C'est aussi le test « la molette morte est réveillée » : si un chemin n'était pas
    // câblé, ce test passerait quand même — mais `semantic_content_is_invariant_under_width`
    // et le bench de grille (`bench_width`) tiennent l'autre bout.
    let swarm = Swarm::builder().width(WidthSpec::Fixed(1)).build().expect("swarm");
    let specs: Vec<(&str, TopologySpec)> = vec![
        ("t1", TopologySpec::GenerateVerify {
            generator: "echo_generator".into(),
            verifier: "equals_expected".into(),
            k: 4.into(),
        }),
        ("t2", TopologySpec::Pipeline { stages: vec!["normalize_whitespace".into(), "uppercase".into()] }),
        ("t3", TopologySpec::Hierarchical {
            decomposer: "word_split".into(),
            solver: "uppercase".into(),
            merger: "join_lines".into(),
        }),
        ("consensus", TopologySpec::consensus("echo_generator", &["consistency"], 4)),
        ("gossip", TopologySpec::gossip("mean_pull", 4, 4, 0.1)),
        ("debate", TopologySpec::debate("echo_generator", &["j_equals_expected", "consistency"], 3, 1)),
    ];
    for (name, spec) in specs {
        let task = json!({ "text": "a b", "expected": "a b", "initial": [1.0, 2.0, 3.0, 4.0] });
        let run = swarm.run(task, spec, Budget::cents(400)).await.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(run.provenance.verified, "{name} : terminé ET re-prouvé à largeur 1 (zéro interblocage)");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn auto_width_is_the_default_and_runs() {
    // I-CP3 (résolution totale) au niveau SDK : Auto (défaut) résout vers les cœurs en
    // Mode A — le run passe sans configuration. (La résolution pure est testée
    // unitairement dans `client::tests::width_resolution_clamps_all_inputs`.)
    let swarm = Swarm::keyless(); // width = Auto par défaut
    let run = swarm
        .run(
            json!({ "text": "x", "expected": "x" }),
            TopologySpec::consensus("echo_generator", &["consistency"], 8),
            Budget::cents(100),
        )
        .await
        .expect("run");
    assert!(run.provenance.verified);
    assert_eq!(run.accounting.agents_used, 9);
}
