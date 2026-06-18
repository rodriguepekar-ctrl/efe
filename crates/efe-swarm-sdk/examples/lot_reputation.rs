// SPDX-License-Identifier: UNLICENSED
//! # `lot_reputation` — le **GATE** d'ADR-019 **REP-2** : le routage par réputation (R-a,
//! agrégation pondérée) fait-il, **chiffres à l'appui**, ce que le read-back B prétend ? Le
//! pendant, côté réputation, de `lot_pattern_bias` (PAT-1).
//!
//! La conception promet, exercées sur le **vrai** chemin (`Reputation::routing_weights` →
//! `Aggregator::aggregate_weighted`) :
//! - **§A uniforme = byte-identique (I-REP2)** : une réputation vide ⇒ poids tous neutres ⇒ la
//!   décision pondérée est **rigoureusement identique** à la décision non pondérée.
//! - **§B le routage déplace la décision AMBIGUË** : une réputation nette (les variantes « B »
//!   gagnantes) + des poids **CLAMPÉS de production** bascule un consensus **peu confiant** A → B.
//! - **§C un gagnant FRANC est IMMUNE (le veto)** : même sous des poids **extrêmes** (au-delà du
//!   clamp production, pour ISOLER le veto), une décision **confiante** ne bascule PAS.
//! - **§D les poids ne s'effondrent JAMAIS** : `routing_weights` ∈ `[500,1500]` (jamais zéro),
//!   gagnant éprouvé > 1000, perdant éprouvé < 1000, **cold-start neutre** = 1000 (UCB).
//! - **§E bout-en-bout** : un vrai consensus apprend une réputation par-variante non dégénérée,
//!   d'où des poids de routage **non uniformes** — la boucle apprendre→router se ferme.
//!
//! Harnais **HONNÊTE** (doctrine Lot P4) : agents **purs synthétiques** (Mode A — candidats +
//! réputations construits, clairement étiquetés) ; **logique RÉELLE** (`routing_weights`,
//! `aggregate_weighted`, `MajorityVote` — les fonctions de production, pas une ré-implémentation).
//!
//! GO ⇔ §A ∧ §B ∧ §C ∧ §D ∧ §E. Écrit `scripts/swarm/EFE_SWARM_REPUTATION_2026-06-18.json`.
//!
//! Run :  cargo run --release --example lot_reputation -p efe-swarm-sdk

use std::sync::Arc;
use std::time::Instant;
use std::{fs, hint};

use efe_nano_sdk::TenantId;
use efe_swarm_memory::{
    family_key, AgentRep, FamilyMode, InMemoryMemoryStore, MemoryStore, Reputation,
};
use efe_swarm_sdk::{Budget, MemoryHandle, Swarm, TopologySpec};
use efe_swarm_topology::{Aggregator, MajorityVote};
use serde_json::json;

/// Le seuil de veto, miroir de `REPUTATION_VETO_FLOOR` (consensus/mod.rs) — sous lui, la décision
/// est AMBIGUË (la réputation a voix) ; au-dessus, le gagnant est FRANC (immune).
const VETO_FLOOR: f64 = 0.34;
const ITERS: usize = 20_000;

/// Poids `[0,1500]` milli → facteurs f64 (comme le SDK : `w/1000.0`).
fn weights_f64(rep: &Reputation, ids: &[String]) -> Vec<f64> {
    rep.routing_weights(ids).iter().map(|w| *w as f64 / 1000.0).collect()
}

fn ids(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("generator_{i:03}")).collect()
}

#[tokio::main]
async fn main() {
    let task = json!({});
    let vote = MajorityVote;

    // ═══ §A — UNIFORME = BYTE-IDENTIQUE (I-REP2) ═════════════════════════════════════════════
    // candidats à décision AMBIGUË (A=3, B=2, C=1) ; réputation VIDE → poids tous neutres (1000).
    let ambiguous = vec![json!("A"), json!("A"), json!("A"), json!("B"), json!("B"), json!("C")];
    let uniform = weights_f64(&Reputation::default(), &ids(ambiguous.len()));
    let plain = vote.aggregate(&task, &ambiguous, &[]);
    let weighted_uniform = vote.aggregate_weighted(&task, &ambiguous, &[], &uniform);
    let a_ok = uniform.iter().all(|&w| (w - 1.0).abs() < 1e-9)
        && plain.selected_index == weighted_uniform.selected_index
        && plain.selected == weighted_uniform.selected
        && plain.scores == weighted_uniform.scores
        && (plain.confidence - weighted_uniform.confidence).abs() < 1e-12;

    // ═══ §B — LE ROUTAGE DÉPLACE LA DÉCISION AMBIGUË (poids CLAMPÉS de production) ════════════
    // réputation NETTE : les variantes « B » (indices 3,4) gagnantes ; « A » (0,1,2) perdantes.
    let mut rep_b = Reputation::default();
    for i in [0, 1, 2] {
        rep_b.agents.insert(format!("generator_{i:03}"), AgentRep { verified: 20, won: 1, total: 20 });
    }
    for i in [3, 4] {
        rep_b.agents.insert(format!("generator_{i:03}"), AgentRep { verified: 20, won: 18, total: 20 });
    }
    let w_b = weights_f64(&rep_b, &ids(ambiguous.len()));
    let weighted_b = vote.aggregate_weighted(&task, &ambiguous, &[], &w_b);
    // sans réputation → A (index 0) ; avec → B (index 3) ; et c'était bien AMBIGU (confidence < floor).
    let b_ok = plain.selected_index == 0
        && plain.confidence < VETO_FLOOR
        && weighted_b.selected_index == 3
        && weighted_b.selected == json!("B")
        && w_b.iter().all(|&w| (0.5..=1.5).contains(&w)); // poids RESTÉS clampés (production)

    // ═══ §C — UN GAGNANT FRANC EST IMMUNE (le VETO, isolé par des poids EXTRÊMES) ═════════════
    // A=6, B=2 → confidence 0,5 ≥ floor (FRANC). Poids EXTRÊMES pro-B (0,1 vs 10,0, hors clamp
    // production) : le veto — pas le clamp — est ce qui bloque la bascule.
    let mut franc = vec![json!("A"); 6];
    franc.extend(vec![json!("B"); 2]);
    let plain_c = vote.aggregate(&task, &franc, &[]);
    let extreme: Vec<f64> = (0..franc.len()).map(|i| if i < 6 { 0.1 } else { 10.0 }).collect();
    let weighted_c = vote.aggregate_weighted(&task, &franc, &[], &extreme);
    let c_ok = plain_c.confidence >= VETO_FLOOR
        && plain_c.selected_index == 0
        && weighted_c.selected_index == 0 // A reste, malgré des poids extrêmes pro-B
        && weighted_c.selected == json!("A");

    // ═══ §D — LES POIDS NE S'EFFONDRENT JAMAIS (clamp + cold-start UCB) ═══════════════════════
    let mut rep_d = Reputation::default();
    rep_d.agents.insert("generator_000".into(), AgentRep { verified: 20, won: 18, total: 20 }); // éprouvé gagnant
    rep_d.agents.insert("generator_001".into(), AgentRep { verified: 20, won: 1, total: 20 }); // éprouvé perdant
    let dids = ids(3); // generator_002 jamais vu → cold-start
    let wd = rep_d.routing_weights(&dids);
    let d_ok = wd[0] > 1000
        && wd[1] < 1000
        && wd[2] == 1000 // cold-start NEUTRE (jamais pénalisé, jamais boosté)
        && wd.iter().all(|&w| (500..=1500).contains(&w)); // jamais zéro, jamais hors bande

    // ═══ §E — BOUT-EN-BOUT : apprendre → router (la boucle se ferme) ═════════════════════════
    let store = Arc::new(InMemoryMemoryStore::new());
    let tenant = TenantId::system().to_string();
    let task_e = json!({ "text": "x", "expected": "x" });
    let judges = ["consistency", "length_band", "j_non_empty"];
    let mut agents = vec!["echo_generator".to_string()];
    agents.extend(judges.iter().map(|s| s.to_string()));
    let family = family_key(&task_e, "consensus", &agents, FamilyMode::Structural);
    let swarm = Swarm::builder().memory(MemoryHandle::new(store.clone())).build().unwrap();
    let run = swarm
        .run(task_e, TopologySpec::consensus("echo_generator", &judges, 3), Budget::cents(1000))
        .await
        .unwrap();
    let snap = store.snapshot(&tenant, &family).await.unwrap();
    let learned = &snap.cells.reputation;
    let learned_w = learned.routing_weights(&ids(3));
    let e_ok = run.is_clean()
        && learned.agents.len() >= 2 // réputation par-variante NON dégénérée
        && learned_w.iter().any(|&w| w != 1000); // → poids de routage NON uniformes (la boucle ferme)

    // ── coût (routing_weights, chiffré pour le gate ; black_box anti-DCE) ─────────────────────
    let t = Instant::now();
    let mut acc = 0u64;
    for _ in 0..ITERS {
        acc += hint::black_box(rep_b.routing_weights(&ids(6))).iter().map(|&w| w as u64).sum::<u64>();
    }
    hint::black_box(acc);
    let route_ns = t.elapsed().as_nanos() as f64 / ITERS as f64;

    let go = a_ok && b_ok && c_ok && d_ok && e_ok;
    let verdict = if go {
        "GO ✅ — uniforme byte-identique · routage bascule l'ambigu · veto protège le franc · jamais d'effondrement · boucle bout-en-bout"
    } else {
        "NO-GO ❌"
    };

    println!("\n §A UNIFORME (I-REP2)    : poids tous neutres + décision IDENTIQUE  → A_ok={a_ok}");
    println!(" §B ROUTAGE (ambigu)     : sans réput A(0) → avec réput B(3) · poids clampés {:?}  → B_ok={b_ok}", w_b.iter().map(|w| (w*1000.0).round()/1000.0).collect::<Vec<_>>());
    println!(" §C VETO (franc immune)  : confidence {:.2} ≥ {VETO_FLOOR} → A reste malgré poids extrêmes pro-B  → C_ok={c_ok}", plain_c.confidence);
    println!(" §D NON-EFFONDREMENT     : gagnant {} · perdant {} · cold-start {} (∈[500,1500])  → D_ok={d_ok}", wd[0], wd[1], wd[2]);
    println!(" §E BOUT-EN-BOUT         : run propre · {} variantes apprises · poids {:?} non uniformes  → E_ok={e_ok}", learned.agents.len(), learned_w);
    println!(" coût routing_weights    : {route_ns:.0} ns/appel (k=6, {ITERS} itérations)");

    println!("\n────────────────────────── VERDICT LOT REPUTATION ──────────────────────────");
    println!(" {verdict}");
    println!("────────────────────────────────────────────────────────────────────────────");

    let payload = json!({
        "_note": "Chemin RÉEL Reputation::routing_weights + Aggregator::aggregate_weighted (MajorityVote) sur des candidats + réputations construits (Mode A pur synthétique, étiqueté). R-a pondère les candidats par la réputation de leur variante (generator_{i:03} = candidat i), SOUS VETO : ne re-décide que si la décision d'origine est AMBIGUË (confidence < veto_floor) ; un gagnant FRANC est immune. Poids UCB clampés [500,1500], jamais zéro (anti-effondrement), cold-start neutre.",
        "veto_floor": VETO_FLOOR,
        "A_uniform_byte_identical": { "weights_all_neutral": uniform.iter().all(|&w| (w-1.0).abs() < 1e-9), "decision_identical": plain.selected_index == weighted_uniform.selected_index, "ok": a_ok },
        "B_routing_shifts_ambiguous": { "without_rep_index": plain.selected_index, "with_rep_index": weighted_b.selected_index, "with_rep_value": weighted_b.selected, "confidence": plain.confidence, "weights": w_b, "ok": b_ok },
        "C_veto_protects_franc": { "confidence": plain_c.confidence, "without_index": plain_c.selected_index, "with_extreme_weights_index": weighted_c.selected_index, "extreme_weights_beyond_clamp": true, "ok": c_ok },
        "D_no_collapse": { "proven_winner_milli": wd[0], "proven_loser_milli": wd[1], "cold_start_milli": wd[2], "band": [500, 1500], "ok": d_ok },
        "E_end_to_end": { "clean": run.is_clean(), "variants_learned": learned.agents.len(), "routing_weights_milli": learned_w, "ok": e_ok },
        "cost": { "routing_weights_ns_per_call_k6": route_ns },
        "verdict": verdict,
        "go": go
    });
    let _ = fs::create_dir_all("scripts/swarm");
    let path = "scripts/swarm/EFE_SWARM_REPUTATION_2026-06-18.json";
    let _ = fs::write(path, serde_json::to_string_pretty(&payload).unwrap());
    println!("\n✅ Résultats : {path}");
    assert!(go, "GATE ADR-019 REP-2 : le routage par réputation doit faire ce qu'il prétend, chiffres à l'appui");
}
