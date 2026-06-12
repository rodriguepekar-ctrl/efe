// SPDX-License-Identifier: UNLICENSED
//! # Lot K-ADAPT — le gate de la **taille d'essaim adaptative** (ADR-014).
//!
//! Question posée : *l'échantillonnage séquentiel borné (vagues + arrêt sur cible)
//! atteint-il la qualité du K fixe en dépensant moins — et bat-il un K fixe recalibré
//! sur SA propre dépense ?* Trois familles, trois bras, exécutés AVEC le build (la
//! discipline des lots S2-bis/S3-bis) :
//!
//! - **F1 numérique** (mélange facile/difficile) : estimation bruitée, arrêt sur
//!   erreur-type (`StdError`). C'est le MÉLANGE que l'adaptatif exploite, pas la
//!   moyenne : π = fraction d'instances faciles, balayé {0.2, 0.5, 0.8}.
//! - **F2 catégorielle** : labels avec taux de leurre par instance, arrêt sur accord
//!   (`Agreement 0.9`).
//! - **F3 vérifiable** (le cas-phare) : candidat correct avec proba p ∈ {0.9, 0.5, 0.2},
//!   arrêt au quota vérifié — coût attendu ~1/p contre K fixe qui paie tout.
//!
//! Bras : `adaptive` · `fixed@k_max` (témoin **iso-qualité**) · `fixed@⌈E[K_used]⌉`
//! (témoin **iso-budget**, recalibré APRÈS COUP sur la dépense réalisée de l'adaptatif
//! — le comparateur honnête). Splitmix64 ensemencé : chaque candidat est une fonction
//! pure de (famille, π, instance, variant) — les bras voient les MÊMES candidats.
//!
//! Écrit `scripts/swarm/EFE_SWARM_KADAPT_<date>.json`.
//! Run : `cargo run --release --example lot_kadapt -p efe-swarm-sdk`

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use efe_swarm_topology::sizing::{run_waves, run_waves_pure, KPlan, StopRule};
use efe_swarm_topology::Generator;
use serde_json::{json, Value};

const DATE: &str = "2026-06-12";
const INSTANCES: usize = 200;
const TRUTH: f64 = 100.0;

// ─────────────────────────────────────────────────────────────────────────────
//  PRNG déterministe (splitmix64) + gaussienne (Box-Muller) — zéro dépendance.
// ─────────────────────────────────────────────────────────────────────────────

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn unit(seed: u64) -> f64 {
    (splitmix64(seed) >> 11) as f64 / (1u64 << 53) as f64
}

/// N(0,1) déterministe par graine (Box-Muller, u1 borné loin de 0).
fn gauss(seed: u64) -> f64 {
    let u1 = unit(seed).max(1e-12);
    let u2 = unit(seed.wrapping_add(0x5851_F42D_4C95_7F2D));
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

fn seed_of(family: u64, pi_idx: u64, instance: u64, variant: u64) -> u64 {
    splitmix64(family.wrapping_mul(0x9E37).wrapping_add(pi_idx) ^ (instance << 20) ^ (variant << 1))
}

// ─────────────────────────────────────────────────────────────────────────────
//  Générateurs Mode A par famille — fonctions pures de (graine, variant) (I-AK3).
// ─────────────────────────────────────────────────────────────────────────────

/// F1 : estimateur bruité — `truth + σ·N(0,1)` (σ = difficulté de l'instance).
struct NoisyEstimator {
    sigma: f64,
    base: u64,
}
#[async_trait]
impl Generator for NoisyEstimator {
    async fn generate(&self, _task: &Value, variant: usize) -> Value {
        json!({ "value": TRUTH + self.sigma * gauss(self.base.wrapping_add(variant as u64)) })
    }
}

/// F2 : labelliseur — correct « A » avec proba p, sinon le leurre « B ».
struct Labeler {
    p_correct: f64,
    base: u64,
}
#[async_trait]
impl Generator for Labeler {
    async fn generate(&self, _task: &Value, variant: usize) -> Value {
        let ok = unit(self.base.wrapping_add(variant as u64)) < self.p_correct;
        json!({ "value": if ok { "A" } else { "B" } })
    }
}

/// F3 : candidat vérifiable — passe avec proba p (le vérificateur lit `ok`).
struct Solver {
    p_pass: f64,
    base: u64,
}
#[async_trait]
impl Generator for Solver {
    async fn generate(&self, _task: &Value, variant: usize) -> Value {
        let ok = unit(self.base.wrapping_add(variant as u64)) < self.p_pass;
        json!({ "ok": ok, "id": variant })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Bras
// ─────────────────────────────────────────────────────────────────────────────

/// Exécute un bras FIXE : k candidats (variants 0..k), mêmes graines que l'adaptatif.
async fn fixed_arm(gen: &Arc<dyn Generator>, k: usize) -> Vec<Value> {
    let plan = KPlan { k_min: k, k_max: k, batch: k, stop: StopRule::Agreement { threshold: 2.0 } };
    let (cands, _) = run_waves_pure(gen, &json!({}), &plan, 8, None).await;
    cands
}

fn mean_value(cands: &[Value]) -> f64 {
    let xs: Vec<f64> = cands.iter().filter_map(|c| c.get("value").and_then(Value::as_f64)).collect();
    xs.iter().sum::<f64>() / xs.len().max(1) as f64
}

fn majority_is_correct(cands: &[Value]) -> bool {
    let a = cands.iter().filter(|c| c["value"] == json!("A")).count();
    a * 2 > cands.len()
}

fn any_verified(cands: &[Value]) -> bool {
    cands.iter().any(|c| c["ok"] == json!(true))
}

/// Moyenne ± IC 95 % (approximation normale, écart-type échantillon).
fn ci(xs: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    let m = xs.iter().sum::<f64>() / n;
    let var = xs.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (n - 1.0);
    (m, 1.96 * (var / n).sqrt())
}

#[derive(Debug, Clone)]
struct ArmStats {
    name: String,
    quality_mean: f64,
    quality_ci: f64,
    cost_mean: f64,
}

fn arm(name: &str, quality: &[f64], costs: &[f64]) -> ArmStats {
    let (qm, qci) = ci(quality);
    ArmStats { name: name.into(), quality_mean: qm, quality_ci: qci, cost_mean: costs.iter().sum::<f64>() / costs.len() as f64 }
}

fn arm_json(a: &ArmStats) -> Value {
    json!({
        "arm": a.name,
        "quality_mean": (a.quality_mean * 1e4).round() / 1e4,
        "quality_ci95": (a.quality_ci * 1e4).round() / 1e4,
        "cost_mean_calls": (a.cost_mean * 100.0).round() / 100.0,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
//  Familles
// ─────────────────────────────────────────────────────────────────────────────

/// F1 — numérique : qualité = |mean − truth| (PLUS BAS = mieux).
async fn family_numeric(pi_idx: usize, pi: f64) -> (Vec<ArmStats>, f64) {
    let plan = KPlan { k_min: 4, k_max: 12, batch: 4, stop: StopRule::StdError { target: 0.6 } };
    let (mut q_ad, mut c_ad, mut q_max, mut q_iso, mut c_iso) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut gens: Vec<Arc<dyn Generator>> = Vec::with_capacity(INSTANCES);
    for i in 0..INSTANCES {
        let easy = unit(seed_of(1, pi_idx as u64, i as u64, 999_983)) < pi;
        let sigma = if easy { 0.5 } else { 3.0 };
        gens.push(Arc::new(NoisyEstimator { sigma, base: seed_of(1, pi_idx as u64, i as u64, 0) }));
    }
    // passe 1 : adaptatif + témoin iso-qualité (k_max).
    for g in &gens {
        let (cands, trace) = run_waves_pure(g, &json!({}), &plan, 8, None).await;
        q_ad.push((mean_value(&cands) - TRUTH).abs());
        c_ad.push(trace.k_used as f64);
        q_max.push((mean_value(&fixed_arm(g, plan.k_max).await) - TRUTH).abs());
    }
    // passe 2 : témoin ISO-BUDGET — K fixe recalibré sur la dépense réalisée.
    let k_iso = (c_ad.iter().sum::<f64>() / c_ad.len() as f64).round().max(1.0) as usize;
    for g in &gens {
        q_iso.push((mean_value(&fixed_arm(g, k_iso).await) - TRUTH).abs());
        c_iso.push(k_iso as f64);
    }
    let kmax_cost = vec![plan.k_max as f64; INSTANCES];
    (
        vec![
            arm("adaptive(se<=0.6)", &q_ad, &c_ad),
            arm("fixed@k_max=12", &q_max, &kmax_cost),
            arm(&format!("fixed@iso={k_iso}"), &q_iso, &c_iso),
        ],
        pi,
    )
}

/// F2 — catégorielle : qualité = taux de majorité correcte (PLUS HAUT = mieux).
async fn family_categorical(pi_idx: usize, pi: f64) -> (Vec<ArmStats>, f64) {
    let plan = KPlan { k_min: 4, k_max: 12, batch: 4, stop: StopRule::Agreement { threshold: 0.9 } };
    let (mut q_ad, mut c_ad, mut q_max, mut q_iso, mut c_iso) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut gens: Vec<Arc<dyn Generator>> = Vec::with_capacity(INSTANCES);
    for i in 0..INSTANCES {
        let easy = unit(seed_of(2, pi_idx as u64, i as u64, 999_983)) < pi;
        let p = if easy { 0.95 } else { 0.55 };
        gens.push(Arc::new(Labeler { p_correct: p, base: seed_of(2, pi_idx as u64, i as u64, 0) }));
    }
    for g in &gens {
        let (cands, trace) = run_waves_pure(g, &json!({}), &plan, 8, None).await;
        q_ad.push(if majority_is_correct(&cands) { 1.0 } else { 0.0 });
        c_ad.push(trace.k_used as f64);
        q_max.push(if majority_is_correct(&fixed_arm(g, plan.k_max).await) { 1.0 } else { 0.0 });
    }
    let k_iso = (c_ad.iter().sum::<f64>() / c_ad.len() as f64).round().max(1.0) as usize;
    for g in &gens {
        q_iso.push(if majority_is_correct(&fixed_arm(g, k_iso).await) { 1.0 } else { 0.0 });
        c_iso.push(k_iso as f64);
    }
    let kmax_cost = vec![plan.k_max as f64; INSTANCES];
    (
        vec![
            arm("adaptive(agree>=0.9)", &q_ad, &c_ad),
            arm("fixed@k_max=12", &q_max, &kmax_cost),
            arm(&format!("fixed@iso={k_iso}"), &q_iso, &c_iso),
        ],
        pi,
    )
}

/// F3 — vérifiable (cas-phare) : qualité = taux de « ≥ 1 vérifié » (PLUS HAUT = mieux).
async fn family_verifiable(p_idx: usize, p: f64) -> (Vec<ArmStats>, f64) {
    let plan = KPlan { k_min: 2, k_max: 12, batch: 2, stop: StopRule::VerifiedQuota { quota: 1 } };
    let observe = |cands: &[Value]| cands.iter().filter(|c| c["ok"] == json!(true)).count() as f64;
    let (mut q_ad, mut c_ad, mut q_max, mut q_iso, mut c_iso) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut gens: Vec<Arc<dyn Generator>> = Vec::with_capacity(INSTANCES);
    for i in 0..INSTANCES {
        gens.push(Arc::new(Solver { p_pass: p, base: seed_of(3, p_idx as u64, i as u64, 0) }));
    }
    for g in &gens {
        let (cands, trace) = run_waves(g, &json!({}), &plan, 8, None, &observe).await;
        q_ad.push(if any_verified(&cands) { 1.0 } else { 0.0 });
        c_ad.push(trace.k_used as f64);
        q_max.push(if any_verified(&fixed_arm(g, plan.k_max).await) { 1.0 } else { 0.0 });
    }
    let k_iso = (c_ad.iter().sum::<f64>() / c_ad.len() as f64).round().max(1.0) as usize;
    for g in &gens {
        q_iso.push(if any_verified(&fixed_arm(g, k_iso).await) { 1.0 } else { 0.0 });
        c_iso.push(k_iso as f64);
    }
    let kmax_cost = vec![plan.k_max as f64; INSTANCES];
    (
        vec![
            arm("adaptive(quota=1)", &q_ad, &c_ad),
            arm("fixed@k_max=12", &q_max, &kmax_cost),
            arm(&format!("fixed@iso={k_iso}"), &q_iso, &c_iso),
        ],
        p,
    )
}

// ─────────────────────────────────────────────────────────────────────────────
//  Verdict par famille : iso-qualité à moindre coût ET ≥ témoin iso-budget.
// ─────────────────────────────────────────────────────────────────────────────

/// `lower_is_better` : F1 (erreur) ; F2/F3 : taux (plus haut = mieux).
fn verdict_row(arms: &[ArmStats], lower_is_better: bool) -> (bool, bool, f64) {
    let (ad, kmax, iso) = (&arms[0], &arms[1], &arms[2]);
    let margin = ad.quality_ci + kmax.quality_ci;
    let iso_quality_vs_kmax = if lower_is_better {
        ad.quality_mean <= kmax.quality_mean + margin
    } else {
        ad.quality_mean >= kmax.quality_mean - margin
    };
    let margin_iso = ad.quality_ci + iso.quality_ci;
    let beats_iso_budget = if lower_is_better {
        ad.quality_mean <= iso.quality_mean + margin_iso
    } else {
        ad.quality_mean >= iso.quality_mean - margin_iso
    };
    let saving = 1.0 - ad.cost_mean / kmax.cost_mean;
    (iso_quality_vs_kmax && saving > 0.0, beats_iso_budget, saving)
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    println!("══════════════════════════════════════════════════════════════════════════════");
    println!(" Lot K-ADAPT — taille d'essaim adaptative vs 2 témoins (ADR-014) · {INSTANCES} inst./cellule");
    println!("══════════════════════════════════════════════════════════════════════════════");

    let mut report_rows: Vec<Value> = Vec::new();
    let mut all_iso_quality = true;
    let mut all_beat_iso_budget = true;
    let mut conditional_notes: Vec<String> = Vec::new();

    let print_block = |label: &str, arms: &[ArmStats], lower: bool, rows: &mut Vec<Value>,
                       all_q: &mut bool, all_b: &mut bool, notes: &mut Vec<String>| {
        let (iso_q, beats_iso, saving) = verdict_row(arms, lower);
        println!("\n  ── {label} ──");
        for a in arms {
            println!(
                "    {:24} qualité {:.4} ± {:.4} · coût moyen {:>5.2} appels",
                a.name, a.quality_mean, a.quality_ci, a.cost_mean
            );
        }
        println!(
            "    → iso-qualité vs k_max : {} · bat l'iso-budget : {} · économie : {:.0} %",
            if iso_q { "OUI" } else { "NON" },
            if beats_iso { "OUI" } else { "NON" },
            saving * 100.0
        );
        *all_q &= iso_q;
        *all_b &= beats_iso;
        if !iso_q || !beats_iso {
            notes.push(format!("{label}: iso_q={iso_q} beats_iso={beats_iso}"));
        }
        rows.push(json!({
            "cell": label,
            "arms": arms.iter().map(arm_json).collect::<Vec<_>>(),
            "iso_quality_vs_kmax": iso_q,
            "beats_iso_budget": beats_iso,
            "saving_vs_kmax": (saving * 1e4).round() / 1e4,
        }));
    };

    println!("\n F1 — numérique (StdError ≤ 0.6, k∈[4,12], batch 4) · qualité = |err| (plus bas = mieux)");
    for (idx, pi) in [0.2f64, 0.5, 0.8].iter().enumerate() {
        let (arms, _) = family_numeric(idx, *pi).await;
        print_block(&format!("F1 π_facile={pi}"), &arms, true, &mut report_rows,
                    &mut all_iso_quality, &mut all_beat_iso_budget, &mut conditional_notes);
    }

    println!("\n F2 — catégorielle (Agreement ≥ 0.9, k∈[4,12], batch 4) · qualité = majorité correcte");
    for (idx, pi) in [0.2f64, 0.5, 0.8].iter().enumerate() {
        let (arms, _) = family_categorical(idx, *pi).await;
        print_block(&format!("F2 π_facile={pi}"), &arms, false, &mut report_rows,
                    &mut all_iso_quality, &mut all_beat_iso_budget, &mut conditional_notes);
    }

    println!("\n F3 — vérifiable (quota 1, k∈[2,12], batch 2) · qualité = ≥1 vérifié trouvé");
    for (idx, p) in [0.9f64, 0.5, 0.2].iter().enumerate() {
        let (arms, _) = family_verifiable(idx, *p).await;
        print_block(&format!("F3 p_succès={p}"), &arms, false, &mut report_rows,
                    &mut all_iso_quality, &mut all_beat_iso_budget, &mut conditional_notes);
    }

    // ── Verdict global ───────────────────────────────────────────────────────
    let verdict = if all_iso_quality && all_beat_iso_budget {
        "GO"
    } else if all_iso_quality {
        "CONDITIONNEL"
    } else {
        "NO-GO"
    };
    println!("\n  VERDICT K-ADAPT : {verdict}");
    if !conditional_notes.is_empty() {
        println!("    cellules en défaut : {conditional_notes:?}");
    }
    println!("    (iso-qualité partout : {all_iso_quality} · bat l'iso-budget partout : {all_beat_iso_budget})");

    let report = json!({
        "lot": "K-ADAPT",
        "date": DATE,
        "adr": "ADR-SWARM-014",
        "mode": "Mode A déterministe (splitmix64 — bras à candidats partagés)",
        "instances_per_cell": INSTANCES,
        "verdict": verdict,
        "criteria": {
            "iso_quality_vs_kmax_everywhere": all_iso_quality,
            "beats_iso_budget_everywhere": all_beat_iso_budget,
            "note": "iso-budget = K fixe recalibré APRÈS COUP sur ⌈E[K_used]⌉ de l'adaptatif (le comparateur honnête)"
        },
        "cells": report_rows,
    });

    if let Some(root) = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2) {
        let dir = root.join("scripts/swarm");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("EFE_SWARM_KADAPT_{DATE}.json"));
        match std::fs::write(&path, format!("{}\n", serde_json::to_string_pretty(&report).unwrap())) {
            Ok(_) => println!("\n  rapport écrit : {}", path.display()),
            Err(e) => println!("\n  (écriture du rapport échouée : {e})"),
        }
    }
    println!("══════════════════════════════════════════════════════════════════════════════");
}
