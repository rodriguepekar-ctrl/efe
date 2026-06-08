// SPDX-License-Identifier: UNLICENSED
//! # `swarm_verifier` — l'agrégateur A-POSTERIORI (le pari de la Phase 3).
//!
//! Contrepartie de `swarm_slm` (qui votait). Ici on démontre le MÉCANISME que
//! le gate H-SWARM a désigné comme LE pari (ADR-SWARM-001) :
//!
//! > L'essaim bon marché CONTIENT la bonne réponse (plafond OR ≫ monolithe), mais
//! > le VOTE majoritaire ne l'extrait pas. La valeur vient d'un **vérificateur
//! > a-posteriori** qui LIT les candidats et SÉLECTIONNE — pas d'un routage.
//!
//! Forme du swarm : K agents produisent chacun un candidat pour la MÊME question ;
//! les candidats circulent via le **Blackboard** ; un agrégateur compare 4 stratégies :
//!   * `single`   — un agent seul (proxy du monolithe bon marché) ;
//!   * `vote`     — vote majoritaire (le PLANCHER, ce que fait `swarm_slm`) ;
//!   * `verifier` — sélection a-posteriori par un [`CandidateVerifier`] (LE pari) ;
//!   * `OR`       — plafond (un candidat correct existe-t-il ?).
//!
//! Le vérificateur est **paramétré par sa précision `v`** (le bouton `v*` de
//! `swarm_aggregator_eval.py`) : à `v=1.0` il est parfait (= OR) ; en deçà il
//! retombe sur le vote. On démontre ainsi : `verifier(v) ≈ v·OR + (1−v)·vote`,
//! et le **seuil `v*`** au-delà duquel l'essaim bat le monolithe.
//!
//! ## Honnêteté (portée)
//! Les agents-solveurs sont ici des **stand-ins déterministes faillibles** (PRNG
//! seedé), car le backend par défaut est keyless. Le **mécanisme** (substrat +
//! trait `CandidateVerifier` + sélection a-posteriori) est RÉEL et backend-
//! agnostique : remplacez le solveur par un agent `LlmClient` (Mode C) et le
//! vérificateur exact par `efe-gold-test-gate` / un jury `efe-multi-llm`, le flux
//! d'essaim ne change pas. Tâche verifiable (somme) = vérificateur déterministe
//! (≈ OR) ; tâche non-vérifiable = jury LLM de précision `v` à mesurer.
//!
//! Run :
//! ```text
//! cargo run --example swarm_verifier -p efe-blackboard                  # v=1.0 (parfait)
//! cargo run --example swarm_verifier -p efe-blackboard -- 300 7 0.87    # M=300, K=7, v=0.87 (la spec)
//! ```

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use efe_blackboard::{Blackboard, EventBus, SwarmEvent};
use efe_nano_sdk::{AgentId, SwarmId};
use serde_json::json;

// ─────────────────────────────────────────────────────────────────────────────
//  PRNG déterministe (splitmix64) — pas de dépendance externe, reproductible.
// ─────────────────────────────────────────────────────────────────────────────
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_add(0x9E3779B97F4A7C15))
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn f01(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next_u64() % ((hi - lo) as u64)) as i64
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Le trait RÉUTILISABLE : un vérificateur a-posteriori de candidats.
//  (Style gold-test-gate / output_validator : lit un candidat, dit s'il passe.)
// ─────────────────────────────────────────────────────────────────────────────
trait CandidateVerifier: Send + Sync {
    /// Vrai si le candidat satisfait le critère vérifiable de la question.
    fn passes(&self, q: &Question, candidate: i64) -> bool;
}

/// Vérificateur EXACT pour une tâche vérifiable (somme) : déterministe → ≈ plafond OR.
/// Analogue d'un test gold qui passe/échoue (code qui compile, schéma valide, etc.).
struct ExactSumVerifier;
impl CandidateVerifier for ExactSumVerifier {
    fn passes(&self, q: &Question, candidate: i64) -> bool {
        candidate == q.a + q.b
    }
}

#[derive(Clone, Copy)]
struct Question {
    a: i64,
    b: i64,
    difficulty: f64, // facteur partagé → erreurs CORRÉLÉES entre agents (réalisme)
    lure: i64,       // mauvaise réponse "plausible" commune (fait échouer le vote)
}
impl Question {
    fn truth(&self) -> i64 {
        self.a + self.b
    }
}

fn mk_event(swarm_id: SwarmId, by: &AgentId, topic: &str, payload: serde_json::Value) -> SwarmEvent {
    SwarmEvent {
        swarm_id,
        published_by: by.clone(),
        topic: topic.to_string(),
        payload,
        published_at: SystemTime::now(),
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let m: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(300);
    let k: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(7);
    let v: f64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let seed: u64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(42);

    let swarm_id = SwarmId::new();
    let bb = Arc::new(Blackboard::new(swarm_id));
    let bus = Arc::new(EventBus::with_capacity((m * 2).max(256)));

    // ── Génère M questions vérifiables, avec difficulté partagée + leurre ──────
    let mut qrng = Rng::new(seed);
    let questions: Vec<Question> = (0..m)
        .map(|_| {
            let a = qrng.range(10, 90);
            let b = qrng.range(10, 90);
            let difficulty = qrng.f01();
            // leurre = une faute "plausible" partagée (a*b modulo, ou a-b) → vote piégé.
            let lure = if qrng.f01() < 0.5 { a + b + qrng.range(1, 4) } else { a - b };
            Question { a, b, difficulty, lure }
        })
        .collect();
    let questions = Arc::new(questions);

    // ── K agents-solveurs FAILLIBLES, en parallèle, écrivent leurs candidats au
    //    Blackboard et publient un événement. P(correct) baisse avec la difficulté
    //    (erreurs corrélées). Quand faux → souvent le LEURRE (→ vote majoritaire faux).
    println!("══════════════════════════════════════════════════════════════════════");
    println!(" ESSAIM VÉRIFICATEUR (Phase 3) — {k} agents ∥ {m} questions · précision vérif v={v:.2}");
    println!(" swarm_id={swarm_id} · substrat=Blackboard+EventBus · vérif=ExactSumVerifier");
    println!("══════════════════════════════════════════════════════════════════════");

    let mut solvers = Vec::new();
    for i in 0..k {
        let bb = bb.clone();
        let bus = bus.clone();
        let questions = questions.clone();
        let aid = AgentId::new(format!("solver_{i:02}")).unwrap();
        let skill = 0.45 + 0.10 * (i as f64 / k.max(1) as f64); // skills hétérogènes ~0.45-0.55
        solvers.push(tokio::spawn(async move {
            let mut rng = Rng::new(seed ^ ((i as u64).wrapping_mul(0x100000001B3)));
            for (q_idx, q) in questions.iter().enumerate() {
                let p_correct = (skill - 0.5 * q.difficulty).clamp(0.0, 1.0);
                let cand = if rng.f01() < p_correct {
                    q.truth()
                } else if rng.f01() < 0.6 {
                    q.lure // faute corrélée → piège le vote
                } else {
                    q.truth() + rng.range(1, 6) // faute idiosyncratique
                };
                // L'info circule par le substrat.
                let _ = bb.write(&format!("/cand/{q_idx}/{i}"), json!(cand), aid.clone());
            }
            let _ = bus.publish(mk_event(swarm_id, &aid, "solver_done", json!({ "agent": aid.as_str() })));
        }));
    }
    for s in solvers {
        let _ = s.await;
    }

    // ── Reconstruit les candidats par question DEPUIS le Blackboard (substrat) ──
    let snap = bb.snapshot();
    let mut by_q: Vec<Vec<i64>> = vec![Vec::new(); m];
    for (key, val) in &snap.fields {
        // clé = "/cand/<q>/<i>"
        let parts: Vec<&str> = key.trim_start_matches("/cand/").split('/').collect();
        if parts.len() == 2 {
            if let Ok(q) = parts[0].parse::<usize>() {
                if let Some(c) = val.value.as_i64() {
                    by_q[q].push(c);
                }
            }
        }
    }

    // ── Agrégateurs ────────────────────────────────────────────────────────────
    let verifier = ExactSumVerifier;
    let mut arng = Rng::new(seed ^ 0xAB);
    let mut single_correct = vec![0usize; k]; // accuracy par agent (proxy monolithe = le meilleur)
    let (mut vote_ok, mut verif_ok, mut or_ok) = (0usize, 0usize, 0usize);

    for (q_idx, q) in questions.iter().enumerate() {
        let cands = &by_q[q_idx];
        if cands.is_empty() {
            continue;
        }
        // single : accuracy de chaque agent (index i = position dans la liste écrite).
        for (i, &c) in cands.iter().enumerate() {
            if c == q.truth() {
                single_correct[i.min(k - 1)] += 1;
            }
        }
        // vote majoritaire.
        let mut counts: BTreeMap<i64, usize> = BTreeMap::new();
        for &c in cands {
            *counts.entry(c).or_default() += 1;
        }
        let vote = *counts.iter().max_by_key(|(_, n)| **n).map(|(c, _)| c).unwrap();
        if vote == q.truth() {
            vote_ok += 1;
        }
        // OR (plafond) : un candidat passe-t-il le vérificateur ?
        let any_pass = cands.iter().any(|&c| verifier.passes(q, c));
        if any_pass {
            or_ok += 1;
        }
        // verifier(v) a-posteriori : si un candidat vérifié existe, avec prob v on le
        // sélectionne (correct) ; sinon on retombe sur le vote (vérificateur faillible).
        let verified: Vec<i64> = cands.iter().copied().filter(|&c| verifier.passes(q, c)).collect();
        let pick = if !verified.is_empty() && arng.f01() < v {
            verified[0] // un candidat vérifié-correct
        } else {
            vote
        };
        if pick == q.truth() {
            verif_ok += 1;
        }
    }

    let m_eff = by_q.iter().filter(|c| !c.is_empty()).count().max(1);
    let best_single = *single_correct.iter().max().unwrap() as f64 / m_eff as f64;
    let vote_acc = vote_ok as f64 / m_eff as f64;
    let verif_acc = verif_ok as f64 / m_eff as f64;
    let or_acc = or_ok as f64 / m_eff as f64;

    println!(" {:<22} {:>10}", "agrégateur", "accuracy");
    println!(" {:-<34}", "");
    println!(" {:<22} {:>9.1}%   (proxy monolithe bon marché)", "single (meilleur agent)", best_single * 100.0);
    println!(" {:<22} {:>9.1}%   (PLANCHER — ce que fait swarm_slm)", "vote majoritaire", vote_acc * 100.0);
    println!(" {:<22} {:>9.1}%   ◀ LE PARI (a-posteriori, v={v:.2})", "verifier-select", verif_acc * 100.0);
    println!(" {:<22} {:>9.1}%   (PLAFOND — vérificateur parfait)", "OR", or_acc * 100.0);
    println!(" {:-<34}", "");

    // v* observé : interpolation verifier ≈ v·OR + (1−v)·vote ; seuil pour battre le single.
    let denom = or_acc - vote_acc;
    let v_star = if denom > 1e-9 { ((best_single - vote_acc) / denom).clamp(0.0, 2.0) } else { f64::INFINITY };
    println!(
        " Écart vote→OR = {:.1} pts = la valeur qu'un vérificateur débloque.",
        (or_acc - vote_acc) * 100.0
    );
    println!(
        " v* (précision vérif. pour battre le proxy monolithe) ≈ {:.0}%  ·  ici v={:.0}% → {}",
        v_star * 100.0,
        v * 100.0,
        if verif_acc >= best_single { "BAT le monolithe ✅" } else { "sous le monolithe ✗" }
    );
    println!(
        " événements substrat : {} · écritures Blackboard : {}",
        bus.published_total(),
        bb.write_count()
    );
    println!("══════════════════════════════════════════════════════════════════════");
    println!(
        "\n→ Le vote stagne au plancher (faute corrélée = leurre majoritaire) ; le\n  \
         vérificateur a-posteriori LIT les candidats et sélectionne le correct →\n  \
         approche le plafond OR. C'est le mécanisme du pari Phase 3, sur le substrat réel."
    );
}
