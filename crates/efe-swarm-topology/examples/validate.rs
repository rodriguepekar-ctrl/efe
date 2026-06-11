// SPDX-License-Identifier: UNLICENSED
#![allow(clippy::doc_overindented_list_items)] // doc d'exemple : alignement de liste intentionnel
//! # `validate` — validation Phase 2 : cost/task + win% multi-familles.
//!
//! EXÉCUTE le VRAI code des topologies (T1 `GenerateVerify`, T3 `Hierarchical`)
//! sur 3 familles de tâches, contre un monolithe. Générateurs faillibles SIMULÉS
//! (modèle honnête des petits modèles, hypothèses S1/AGG/S2) + vérificateur
//! DÉTERMINISTE (`catalog::EqualsExpected` → atteint OR). Mesure les SLO de
//! Phase 2 : # topologies validées · cost/task vs monolithe · % tâches gagnées.
//!
//! Familles :
//! - `classify`     : tâche DANS l'enveloppe de capacité → **T1** gagne (cf. F1 S2).
//! - `doc_classify` : tâche DURE mais DÉCOMPOSABLE (agrégation robuste = majorité)
//!                    → **T3** gagne en décomposant (ramène dans l'enveloppe).
//! - `hard`         : tâche DURE, NON décomposable → personne ne gagne (cf. F2 S2).
//!
//! Run :  cargo run --release --example validate -p efe-swarm-topology

use std::sync::Arc;

use async_trait::async_trait;
use efe_swarm_topology::catalog::{EqualsExpected, MajorityValue};
use efe_swarm_topology::{Decomposer, GenerateVerify, Generator, GvSolver, Hierarchical, Verifier};
use serde_json::{json, Value};

const N: usize = 60; // tâches par famille
const K: usize = 7; // générateurs T1 (tâche entière)
const K_SUB: usize = 3; // générateurs T1 par sous-tâche (faciles)
const S: usize = 5; // sous-tâches (phrases) pour la décomposition
const COST_MONO: f64 = 20.0; // coût d'un appel monolithe (unités cheap)

fn unit(mut z: u64) -> f64 {
    z = z.wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

/// Générateur faillible simulé : P(correct) = clip(skill − gamma·difficulté).
struct SimGen {
    skill: f64,
    gamma: f64,
    seed: u64,
}
#[async_trait]
impl Generator for SimGen {
    async fn generate(&self, task: &Value, variant: usize) -> Value {
        let diff = task.get("difficulty").and_then(|v| v.as_f64()).unwrap_or(0.5);
        let id = task.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
        let expected = task.get("expected").cloned().unwrap_or(Value::Null);
        let p = (self.skill - self.gamma * diff).clamp(0.0, 1.0);
        let r = unit(self.seed ^ id.wrapping_mul(0x100000001B3) ^ (variant as u64).wrapping_mul(0x9E37));
        if r < p {
            json!({ "value": expected })
        } else {
            json!({ "value": "__wrong__" })
        }
    }
}

/// Décomposeur : tâche → S sous-tâches (phrases) FACILES, même topic attendu.
struct DocDecomposer {
    sub_difficulty: f64,
}
#[async_trait]
impl Decomposer for DocDecomposer {
    async fn decompose(&self, task: &Value) -> Vec<Value> {
        let id = task.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
        let expected = task.get("expected").cloned().unwrap_or(Value::Null);
        (0..S)
            .map(|i| json!({ "id": id * 100 + i as u64, "expected": expected, "difficulty": self.sub_difficulty }))
            .collect()
    }
}

fn task(id: u64, expected: Value, difficulty: f64) -> Value {
    json!({ "id": id, "expected": expected, "difficulty": difficulty })
}

fn correct(candidate: &Value, expected: &Value) -> bool {
    candidate.get("value").map(|v| v == expected).unwrap_or(false)
}

/// Bootstrap : moy ± σ d'un vecteur de booléens.
fn boot(arr: &[bool], seed: u64) -> (f64, f64) {
    let n = arr.len().max(1);
    let b = 200;
    let mut means = Vec::with_capacity(b);
    for k in 0..b {
        let mut s = 0.0;
        for j in 0..n {
            let idx = (unit(seed ^ (k as u64).wrapping_mul(0x1234) ^ (j as u64)) * n as f64) as usize % n;
            if arr[idx] {
                s += 1.0;
            }
        }
        means.push(s / n as f64);
    }
    let mu = means.iter().sum::<f64>() / b as f64;
    let var = means.iter().map(|m| (m - mu).powi(2)).sum::<f64>() / b as f64;
    (mu, var.sqrt())
}

struct FamilyResult {
    name: &'static str,
    mono: (f64, f64),
    t1: (f64, f64),
    t1_cost: f64,
    t3: Option<((f64, f64), f64)>, // (acc±σ, cost) si décomposable
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let cheap = || Arc::new(SimGen { skill: 0.55, gamma: 0.6, seed: 0xC0FFEE }) as Arc<dyn Generator>;
    let mono_skill = 0.88;
    let mono_gamma = 0.25;
    let mono_seed = 0xA11CE;
    let ver: Arc<dyn Verifier> = Arc::new(EqualsExpected);
    let t1 = GenerateVerify::new(K, 8);

    // familles : (nom, difficulté entière, décomposable?)
    let families: &[(&str, f64, bool)] = &[
        ("classify", 0.30, false),
        ("doc_classify", 0.70, true),
        ("hard", 0.85, false),
    ];
    let topics = ["technology", "finance", "health", "legal", "politics", "sports"];

    let mut results = Vec::new();
    for (name, diff, decomposable) in families {
        let mut mono_c = Vec::with_capacity(N);
        let mut t1_c = Vec::with_capacity(N);
        let mut t3_c = Vec::with_capacity(N);
        for i in 0..N {
            let expected = json!(topics[i % topics.len()]);
            let tk = task(i as u64, expected.clone(), *diff);

            // monolithe : un seul tirage fort.
            let p_mono = (mono_skill - mono_gamma * diff).clamp(0.0, 1.0);
            mono_c.push(unit(mono_seed ^ (i as u64).wrapping_mul(0x100000001B3)) < p_mono);

            // T1 sur la tâche entière.
            let r1 = t1.run(&tk, cheap(), ver.as_ref()).await;
            t1_c.push(r1.selected.map(|s| correct(&s, &expected)).unwrap_or(false));

            // T3 : décompose → T1 par sous-tâche (faciles) → majorité.
            if *decomposable {
                let h = Hierarchical::new(
                    Arc::new(DocDecomposer { sub_difficulty: 0.20 }),
                    Arc::new(GvSolver {
                        gv: GenerateVerify::new(K_SUB, 4),
                        gen: cheap(),
                        ver: Arc::new(EqualsExpected),
                    }),
                    Arc::new(MajorityValue),
                );
                let out = h.run(&tk).await.output;
                t3_c.push(correct(&out, &expected));
            }
        }
        results.push(FamilyResult {
            name,
            mono: boot(&mono_c, 1),
            t1: boot(&t1_c, 2),
            t1_cost: K as f64,
            t3: if *decomposable {
                Some((boot(&t3_c, 3), (S * K_SUB) as f64))
            } else {
                None
            },
        });
    }

    // ── Rapport SLO ────────────────────────────────────────────────────────────
    println!("══════════════════════════════════════════════════════════════════════════════");
    println!(" VALIDATION PHASE 2 — topologies réelles vs monolithe · {N} tâches/famille");
    println!(" K={K} (T1) · K_sub={K_SUB}×{S} (T3) · coût monolithe={COST_MONO:.0}");
    println!("══════════════════════════════════════════════════════════════════════════════");
    println!(" {:<14} {:>14} {:>16} {:>8} {:>16} {:>8} {:>8}", "famille", "monolithe", "T1 acc", "coût", "T3 acc", "coût", "winner");
    println!(" {}", "─".repeat(86));

    let pp = |t: (f64, f64)| format!("{:.3}±{:.3}", t.0, t.1);
    let mut topo_validated = std::collections::BTreeSet::new();
    let mut wins = 0usize;
    for r in &results {
        let m = r.mono.0;
        // win = qualité ≥ monolithe (robuste ±σ) ET coût < monolithe.
        let t1_win = (r.t1.0 - r.t1.1) >= m && r.t1_cost < COST_MONO;
        let t3_win = r
            .t3
            .map(|(acc, c)| (acc.0 - acc.1) >= m && c < COST_MONO)
            .unwrap_or(false);
        let winner = if t3_win {
            topo_validated.insert("T3");
            wins += 1;
            "T3"
        } else if t1_win {
            topo_validated.insert("T1");
            wins += 1;
            "T1"
        } else {
            "—"
        };
        let (t3s, t3cost) = match r.t3 {
            Some((acc, c)) => (pp(acc), format!("{c:.0}")),
            None => ("n/a".into(), "—".into()),
        };
        println!(
            " {:<14} {:>14} {:>16} {:>8.0} {:>16} {:>8} {:>8}",
            r.name, pp(r.mono), pp(r.t1), r.t1_cost, t3s, t3cost, winner
        );
    }
    println!(" {}", "─".repeat(86));
    let win_pct = 100.0 * wins as f64 / results.len() as f64;
    println!("\n────────────────────────── SLO PHASE 2 ──────────────────────────");
    println!(" # topologies validées : {} ({:?})  [cible ≥ 2]", topo_validated.len(), topo_validated);
    println!(" % tâches/familles gagnées : {win_pct:.0}%  ({wins}/{})  [cible ≥ 50 %]", results.len());
    for r in &results {
        if let Some((_, c)) = r.t3 {
            println!("   {:<14} cost/task : T1={:.0} ({:.1}× moins cher) · T3={:.0} ({:.1}×) vs monolithe {COST_MONO:.0}",
                     r.name, r.t1_cost, COST_MONO / r.t1_cost, c, COST_MONO / c);
        } else {
            println!("   {:<14} cost/task : T1={:.0} ({:.1}× moins cher) vs monolithe {COST_MONO:.0}",
                     r.name, r.t1_cost, COST_MONO / r.t1_cost);
        }
    }
    let go = topo_validated.len() >= 2 && win_pct >= 50.0;
    println!("──────────────────────────────────────────────────────────────────");
    println!(" VERDICT : {}", if go { "GO — SLO Phase 2 atteints (≥2 topologies, ≥50 % win, moins cher)" } else { "EN-DEÇÀ — revoir l'enveloppe / K" });
    println!(" Portée : générateurs faillibles SIMULÉS ; vérificateur déterministe → T1/T3==OR.");
    println!("          Run réel = générateurs LlmClient (Mode C) + llama (cf. ADR-SWARM-002).");
    println!("══════════════════════════════════════════════════════════════════════════════");
}
