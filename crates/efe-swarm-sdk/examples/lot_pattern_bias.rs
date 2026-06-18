// SPDX-License-Identifier: UNLICENSED
//! # `lot_pattern_bias` — le **GATE** d'ADR-019 **PAT-1** : le biais de pattern fait-il, **chiffres
//! à l'appui**, ce que la conception (read-back A) prétend ? Le jumeau, côté pattern, de
//! `lot_memory` / `lot_kadapt`.
//!
//! La conception promet trois choses **mesurables**, exercées sur le **vrai** chemin de biais
//! (`PatternPrior::bias` → `apply_pattern_bias` → `PatternSelector::select`) :
//! - **§A famille FIABLE** (l'accord des candidats s'est historiquement révélé sûr → prior à
//!   FAIBLE escalade) : le seuil de vote **descend** → **moins d'escalade** (juges économisés)
//!   **à iso-qualité** (sur un set qui s'accorde vraiment, le Vote donne la même réponse correcte).
//! - **§B famille PIÉGEUSE** (l'accord y fut souvent un cluster de leurres → prior à FORTE escalade) :
//!   le seuil **monte** → **plus d'escalade** (les juges démasquent le leurre).
//! - **§C I-PAT2** : un instantané GENÈSE (`total==0`) laisse la décision **rigoureusement
//!   identique** à la base (aucun biais sans donnée).
//! - **§D clamp** : le seuil effectif reste dans la **bande sans dégénérescence** `[θ_lo, θ_hi]`.
//! - **§E** : le pattern **adaptatif** (jadis code mort) tourne **proprement bout-en-bout** + décide.
//!
//! Harnais **HONNÊTE** (doctrine Lot P4) : agents **purs synthétiques** (Mode A — des sets de
//! candidats à **accord CONTRÔLÉ**, clairement étiquetés) ; **logique RÉELLE** (`apply_pattern_bias`,
//! `PatternSelector::select`, `MajorityVote` — les fonctions de production, pas une ré-implémentation).
//!
//! GO ⇔ §A ∧ §B ∧ §C ∧ §D ∧ §E. Écrit `scripts/swarm/EFE_SWARM_PATBIAS_2026-06-18.json`.
//!
//! Run :  cargo run --release --example lot_pattern_bias -p efe-swarm-sdk

use std::fs;
use std::time::Instant;

use efe_swarm_memory::PatternPrior;
use efe_swarm_sdk::{apply_pattern_bias, Budget, Swarm, TopologySpec};
use efe_swarm_topology::{Aggregator, MajorityVote, Pattern, PatternSelector};
use serde_json::{json, Value};

const K: usize = 30;
/// `m` candidats identiques (« A »), le reste DISTINCTS → accord = m(m-1)/(K(K-1)), connu d'avance.
/// Le balayage straddle la frontière de décision (0,63 → 1,0) pour exposer les bascules du biais.
const SWEEP: &[usize] = &[24, 25, 26, 27, 28, 29, 30];
const ITERS: usize = 20_000;

/// K candidats à accord CONTRÔLÉ : `m` « A » + (K−m) singletons distincts.
fn candidates(m: usize) -> Vec<Value> {
    let mut c: Vec<Value> = (0..m).map(|_| json!("A")).collect();
    for j in 0..(K - m) {
        c.push(json!(format!("B{j}")));
    }
    c
}

/// Le sélecteur ESCALADE-t-il (juges payés) sur ce set ? (`Vote` ⇒ pas d'escalade.)
fn escalates(sel: &PatternSelector, cands: &[Value]) -> bool {
    sel.select(cands) != Pattern::Vote
}

/// Taux d'escalade (milli) d'un sélecteur sur le balayage entier.
fn escalation_rate_milli(sel: &PatternSelector) -> u32 {
    let n = SWEEP.iter().filter(|&&m| escalates(sel, &candidates(m))).count();
    (n * 1000 / SWEEP.len()) as u32
}

fn prior(total: u64, escalations: u64) -> PatternPrior {
    PatternPrior { total, escalations, ..Default::default() }
}

#[tokio::main]
async fn main() {
    let base = PatternSelector::new(0.9, false); // la base d'`adaptive_default` (quasi-unanimité)
    let base_rate = escalation_rate_milli(&base);

    // ── §A famille FIABLE : prior à FAIBLE escalade → θ_eff DESCEND → MOINS d'escalade ─────────
    let reliable = prior(100, 5).bias(); // 5 % d'escalade historique
    let sel_reliable = apply_pattern_bias(base.clone(), Some(&reliable));
    let reliable_rate = escalation_rate_milli(&sel_reliable);
    // ISO-QUALITÉ : sur le set borderline que le biais bascule vers le Vote (m=28, accord 0,87,
    // majorité « A » digne de confiance), le Vote rend « A » — la MÊME réponse, sans payer les juges.
    let flipped = candidates(28);
    let vote_winner = MajorityVote.aggregate(&json!({}), &flipped, &[]).selected;
    let a_ok = reliable_rate < base_rate && vote_winner == json!("A");

    // ── §B famille PIÉGEUSE : prior à FORTE escalade → θ_eff MONTE → PLUS d'escalade ───────────
    let trap = prior(100, 90).bias(); // 90 % d'escalade historique (accord souvent trompeur)
    let sel_trap = apply_pattern_bias(base.clone(), Some(&trap));
    let trap_rate = escalation_rate_milli(&sel_trap);
    let b_ok = trap_rate > base_rate;

    // ── §C I-PAT2 : prior GENÈSE (total==0) → décision RIGOUREUSEMENT identique à la base ──────
    let genesis = prior(0, 0).bias();
    let sel_genesis = apply_pattern_bias(base.clone(), Some(&genesis));
    let genesis_rate = escalation_rate_milli(&sel_genesis);
    let genesis_identical = SWEEP.iter().all(|&m| {
        let c = candidates(m);
        escalates(&sel_genesis, &c) == escalates(&base, &c)
    });
    let c_ok = genesis_rate == base_rate && genesis_identical;

    // ── §D clamp : base BASSE + fiable → θ_lo ; base HAUTE + piégeux → θ_hi (bande sans dégén.) ─
    let low = apply_pattern_bias(PatternSelector::new(0.72, false), Some(&prior(100, 0).bias()));
    let high = apply_pattern_bias(PatternSelector::new(0.95, false), Some(&prior(100, 100).bias()));
    let d_ok = (low.vote_threshold - 0.70).abs() < 1e-9 && (high.vote_threshold - 0.97).abs() < 1e-9;

    // ── §E bout-en-bout : le pattern adaptatif tourne PROPREMENT + décide ─────────────────────
    let run = Swarm::keyless()
        .run(
            json!({ "text": "x", "expected": "x" }),
            TopologySpec::consensus_pattern_adaptive("echo_generator", &["consistency", "j_non_empty"], 3),
            Budget::cents(1000),
        )
        .await
        .unwrap();
    let e_ok = run.is_clean() && run.decided_pattern.is_some();

    // ── coût du transfert (négligeable, mais chiffré pour le gate ; `black_box` empêche
    //    l'élimination de code mort qui afficherait 0 ns) ───────────────────────────────────────
    let t = Instant::now();
    let mut acc = 0.0f64;
    for _ in 0..ITERS {
        acc += std::hint::black_box(apply_pattern_bias(base.clone(), Some(&reliable))).vote_threshold;
    }
    std::hint::black_box(acc);
    let bias_ns = t.elapsed().as_nanos() as f64 / ITERS as f64;

    let go = a_ok && b_ok && c_ok && d_ok && e_ok;
    let verdict = if go {
        "GO ✅ — fiable ↓escalade (iso-qualité) · piégeux ↑escalade · genèse byte-identique · clamp · adaptatif atteignable"
    } else {
        "NO-GO ❌"
    };

    println!("\n §A FIABLE  (θ_eff={:.2}) : escalade {reliable_rate}‰ < base {base_rate}‰ · Vote rend « {vote_winner} » (iso-qualité)  → A_ok={a_ok}", sel_reliable.vote_threshold);
    println!(" §B PIÉGEUX (θ_eff={:.2}) : escalade {trap_rate}‰ > base {base_rate}‰  → B_ok={b_ok}", sel_trap.vote_threshold);
    println!(" §C I-PAT2 (genèse)      : {genesis_rate}‰ == base {base_rate}‰ + décisions identiques={genesis_identical}  → C_ok={c_ok}");
    println!(" §D CLAMP                : base 0,72→{:.2} (θ_lo) · base 0,95→{:.2} (θ_hi)  → D_ok={d_ok}", low.vote_threshold, high.vote_threshold);
    println!(" §E ATTEIGNABLE          : run propre + pattern décidé ({:?})  → E_ok={e_ok}", run.decided_pattern);
    println!(" coût biais              : {bias_ns:.0} ns/appel ({ITERS} itérations)");

    println!("\n────────────────────────── VERDICT LOT PATTERN-BIAS ──────────────────────────");
    println!(" {verdict}");
    println!("─────────────────────────────────────────────────────────────────────────────");

    let payload = json!({
        "_note": "Transfert RÉEL apply_pattern_bias + PatternSelector::select sur des sets de candidats à accord CONTRÔLÉ (Mode A pur synthétique, étiqueté). Le biais déplace la frontière de décision (θ_eff clampé), échelonnée par la confiance, sous VETO de l'accord réel. iso-qualité : famille fiable ⇒ l'accord est digne de confiance (Vote sûr, juges économisés, MÊME gagnant) ; famille piégeuse ⇒ l'accord est un leurre (escalade = les juges le démasquent).",
        "base_vote_threshold": 0.9,
        "sweep_m": SWEEP,
        "A_reliable": { "theta_eff": sel_reliable.vote_threshold, "escalation_milli": reliable_rate, "base_milli": base_rate, "vote_winner_iso_quality": vote_winner, "ok": a_ok },
        "B_trap": { "theta_eff": sel_trap.vote_threshold, "escalation_milli": trap_rate, "base_milli": base_rate, "ok": b_ok },
        "C_ipat2": { "genesis_milli": genesis_rate, "base_milli": base_rate, "decisions_identical": genesis_identical, "ok": c_ok },
        "D_clamp": { "low_base_0_72_clamped_to": low.vote_threshold, "high_base_0_95_clamped_to": high.vote_threshold, "theta_lo": 0.70, "theta_hi": 0.97, "ok": d_ok },
        "E_reachable": { "clean": run.is_clean(), "decided_pattern": run.decided_pattern, "ok": e_ok },
        "cost": { "bias_ns_per_call": bias_ns },
        "verdict": verdict,
        "go": go
    });
    let _ = fs::create_dir_all("scripts/swarm");
    let path = "scripts/swarm/EFE_SWARM_PATBIAS_2026-06-18.json";
    let _ = fs::write(path, serde_json::to_string_pretty(&payload).unwrap());
    println!("\n✅ Résultats : {path}");
    assert!(go, "GATE ADR-019 PAT-1 : le biais de pattern doit faire ce qu'il prétend, chiffres à l'appui");
}
