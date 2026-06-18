// SPDX-License-Identifier: UNLICENSED
//! # `lot_sizer_reputation` — le **GATE** d'ADR-019 **REP-3** : l'**ordonnancement de profils**
//! du sizer adaptatif (R-b priorisation / R-c gating, exploration déterministe keyée sur `total`)
//! fait-il, **chiffres à l'appui**, ce que la conception prétend — **et son interaction avec le
//! sizer (ADR-014) et le budget (ADR-018) est-elle mesurée** ? C'est la précondition explicite à
//! l'ouverture de cette extension : *à n'ouvrir qu'avec son interaction sizer/budget mesurée*.
//!
//! La conception promet, exercées sur le **vrai** chemin (`Reputation::profile_schedule` →
//! `run_waves_pure_scheduled`, la boucle de sizer RÉELLE) :
//! - **§A déterminisme + re-prouvabilité** : `profile_schedule` est une **fonction pure** de
//!   `(réputation, total, k_min, k_max)` — mêmes entrées ⇒ ordonnancement **byte-identique** ; et
//!   **genèse = identité** (réput vide, `total=0` ⇒ `[0,1,…)`, aucun biais sans donnée, parité I-REP2).
//! - **§B exploration déterministe (rotation keyée sur `total`)** : sur un cycle de `k_max` runs,
//!   CHAQUE profil — fût-il éprouvé-perdant relégué en queue — entre dans la région garantie-atteinte
//!   (`k_min` premières positions) au moins une fois : **couverture complète, aucune famine**.
//! - **§C priorisation R-b (interaction SIZER)** : sur un set où les bons profils s'accordent, le
//!   sizer **chaud** (ordonnancement réputationnel) atteint la cible **plus tôt** que le **froid**
//!   (identité) → `K_used` **strictement plus petit à iso-qualité** (même gagnant agrégé).
//! - **§D sûreté BUDGET (interaction ADR-018)** : l'ordonnancement est une **permutation** de
//!   `[0,k_max)` → l'**enveloppe** (`k_max`) est inchangée, le **plafond** tient (cible inatteignable
//!   ⇒ arrêt PILE à `k_max`), et le chaud ne fait **qu'économiser** (`K_used` chaud ≤ froid ≤ `k_max`).
//! - **§E non-effondrement (R-c doux)** : permutation totale ⇒ aucun profil supprimé ; l'éprouvé-
//!   perdant reste présent ET ré-atteint par la rotation. Le gating repousse, ne tue jamais.
//!
//! Harnais **HONNÊTE** (doctrine Lot P4) : générateurs **purs synthétiques** (Mode A — profils
//! `GOLD`/bruit clairement étiquetés) ; **logique RÉELLE** (`profile_schedule`,
//! `run_waves_pure_scheduled`, `MajorityVote` — les fonctions de production, pas de ré-implémentation
//! de la boucle de sizer). §C isole le bénéfice d'**exploitation** en prenant `total=6` (le profil
//! d'exploration tombe alors sur un bon profil) ; §B prouve séparément la couverture de la rotation
//! sur **tous** les `total`.
//!
//! GO ⇔ §A ∧ §B ∧ §C ∧ §D ∧ §E. Écrit `scripts/swarm/EFE_SWARM_SIZER_REPUTATION_2026-06-18.json`.
//!
//! Run :  cargo run --release --example lot_sizer_reputation -p efe-swarm-sdk

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Instant;
use std::{fs, hint};

use async_trait::async_trait;
use efe_swarm_memory::{AgentRep, Reputation};
use efe_swarm_topology::sizing::{run_waves_pure_scheduled, KPlan, StopRule};
use efe_swarm_topology::{Aggregator, Generator, MajorityVote};
use serde_json::{json, Value};

const K_MIN: usize = 4;
const K_MAX: usize = 12;
const BATCH: usize = 4;
/// Les profils éprouvés-gagnants (émettent l'or) ; tout le reste émet un bruit distinct.
const GOOD: [usize; 4] = [6, 7, 8, 9];
const ITERS: usize = 20_000;

/// Générateur Mode A pur synthétique : le profil `variant` émet `"GOLD"` s'il est éprouvé-bon,
/// sinon un bruit **distinct** `"noise{variant}"`. Fonction pure de `variant` (I-AK3).
struct GoldOrNoise;
#[async_trait]
impl Generator for GoldOrNoise {
    async fn generate(&self, _task: &Value, variant: usize) -> Value {
        if GOOD.contains(&variant) {
            json!("GOLD")
        } else {
            json!(format!("noise{variant}"))
        }
    }
}

/// Générateur tous-distincts (accord impossible) — pour prouver que le plafond `k_max` tient sous
/// ordonnancement (§D).
struct AllDistinct;
#[async_trait]
impl Generator for AllDistinct {
    async fn generate(&self, _task: &Value, variant: usize) -> Value {
        json!(format!("v{variant}"))
    }
}

fn plan() -> KPlan {
    KPlan { k_min: K_MIN, k_max: K_MAX, batch: BATCH, stop: StopRule::Agreement { threshold: 0.9 } }
}

/// La réputation de référence : `GOOD` éprouvés-gagnants, le reste éprouvé-perdant.
fn reputation() -> Reputation {
    let mut rep = Reputation::default();
    for i in 0..K_MAX {
        let r = if GOOD.contains(&i) {
            AgentRep { verified: 20, won: 18, total: 20 } // gagnant
        } else {
            AgentRep { verified: 1, won: 1, total: 20 } // perdant éprouvé
        };
        rep.agents.insert(format!("generator_{i:03}"), r);
    }
    rep
}

#[tokio::main]
async fn main() {
    let task = json!({});
    let rep = reputation();
    let vote = MajorityVote;

    // ═══ §A — DÉTERMINISME + RE-PROUVABILITÉ (genèse = identité) ══════════════════════════════
    let s1 = rep.profile_schedule(6, K_MIN, K_MAX);
    let s2 = rep.profile_schedule(6, K_MIN, K_MAX);
    let genesis = Reputation::default().profile_schedule(0, K_MIN, K_MAX);
    let a_ok = s1 == s2 // pur : re-dérivable hors-ligne (R1)
        && genesis == (0..K_MAX).collect::<Vec<_>>(); // aucun biais sans donnée (parité I-REP2)

    // ═══ §B — EXPLORATION DÉTERMINISTE : la rotation couvre TOUS les profils (aucune famine) ═══
    let mut covered = BTreeSet::new();
    let mut rotation_always_guaranteed = true;
    for total in 0..K_MAX as u64 {
        let sched = rep.profile_schedule(total, K_MIN, K_MAX);
        let e = (total % K_MAX as u64) as usize; // le profil que la rotation force ce run
        if !sched[..K_MIN].contains(&e) {
            rotation_always_guaranteed = false;
        }
        covered.extend(sched[..K_MIN].iter().copied());
    }
    let b_ok = rotation_always_guaranteed && covered == (0..K_MAX).collect();

    // ═══ §C — PRIORISATION R-b (interaction SIZER) : chaud atteint la cible plus tôt ═════════
    // total=6 → profil d'exploration 6 ∈ GOOD : isole le bénéfice d'EXPLOITATION (§B couvre la rotation).
    let schedule = rep.profile_schedule(6, K_MIN, K_MAX);
    let gen: Arc<dyn Generator> = Arc::new(GoldOrNoise);
    // FROID : identité (None) — le sizer historique, sans mémoire.
    let (cold_c, cold_t) = run_waves_pure_scheduled(&gen, &task, &plan(), 8, None, None, None).await;
    // CHAUD : ordonnancement réputationnel — les bons profils d'abord.
    let (warm_c, warm_t) = run_waves_pure_scheduled(&gen, &task, &plan(), 8, None, None, Some(&schedule)).await;
    let cold_winner = vote.aggregate(&task, &cold_c, &[]).selected;
    let warm_winner = vote.aggregate(&task, &warm_c, &[]).selected;
    let c_ok = warm_t.k_used < cold_t.k_used // moins de candidats : accord plus tôt
        && warm_winner == cold_winner // iso-qualité : MÊME gagnant agrégé
        && warm_winner == json!("GOLD");

    // ═══ §D — SÛRETÉ BUDGET (interaction ADR-018) : permutation ⇒ enveloppe inchangée, plafond tient ═══
    let envelope_unchanged = schedule.len() == K_MAX; // couvre exactement l'enveloppe, jamais au-delà
    // cible inatteignable (tous distincts) → arrêt PILE à k_max, MÊME sous ordonnancement (I-AK1).
    let distinct: Arc<dyn Generator> = Arc::new(AllDistinct);
    let (_dc, dt) = run_waves_pure_scheduled(&distinct, &task, &plan(), 8, None, None, Some(&schedule)).await;
    let ceiling_holds = dt.k_used == K_MAX;
    let only_saves = warm_t.k_used <= cold_t.k_used && warm_t.k_used <= K_MAX; // ne dépasse jamais
    let d_ok = envelope_unchanged && ceiling_holds && only_saves;

    // ═══ §E — NON-EFFONDREMENT (R-c doux) : permutation totale, perdant présent ET ré-atteint ═══
    let mut sorted = schedule.clone();
    sorted.sort_unstable();
    let is_permutation = sorted == (0..K_MAX).collect::<Vec<_>>(); // aucun profil supprimé
    let loser = 0usize; // profil éprouvé-perdant
    let loser_present = schedule.contains(&loser);
    // la rotation l'atteint quand total ≡ loser (ici total=0) : entré dans la région garantie.
    let loser_reached = rep.profile_schedule(loser as u64, K_MIN, K_MAX)[..K_MIN].contains(&loser);
    let e_ok = is_permutation && loser_present && loser_reached;

    // ── coût (profile_schedule, chiffré pour le gate ; black_box anti-DCE) ────────────────────
    let t = Instant::now();
    let mut acc = 0usize;
    for n in 0..ITERS {
        acc += hint::black_box(rep.profile_schedule((n as u64) % K_MAX as u64, K_MIN, K_MAX))[0];
    }
    hint::black_box(acc);
    let sched_ns = t.elapsed().as_nanos() as f64 / ITERS as f64;

    let go = a_ok && b_ok && c_ok && d_ok && e_ok;
    let verdict = if go {
        "GO ✅ — déterministe+genèse-identité · rotation couvre tout · chaud ↓K_used (iso-qualité) · enveloppe/plafond intacts · jamais d'effondrement"
    } else {
        "NO-GO ❌"
    };

    println!("\n §A DÉTERMINISME       : pur (s1==s2) + genèse=identité {:?}  → A_ok={a_ok}", &genesis[..K_MIN.min(genesis.len())]);
    println!(" §B EXPLORATION        : rotation garantie ∀total + couverture {}/{} profils  → B_ok={b_ok}", covered.len(), K_MAX);
    println!(" §C PRIORISATION (R-b) : K_used chaud {} < froid {} · gagnant « {} » (iso-qualité)  → C_ok={c_ok}", warm_t.k_used, cold_t.k_used, warm_winner.as_str().unwrap_or("?"));
    println!(" §D BUDGET (ADR-018)   : enveloppe k_max={K_MAX} inchangée · cible inatteignable→{} (plafond) · chaud≤froid≤k_max  → D_ok={d_ok}", dt.k_used);
    println!(" §E NON-EFFONDREMENT   : permutation={is_permutation} · perdant(0) présent={loser_present} + ré-atteint={loser_reached}  → E_ok={e_ok}");
    println!(" schedule chaud        : {schedule:?}");
    println!(" coût profile_schedule : {sched_ns:.0} ns/appel (k_max={K_MAX}, {ITERS} itérations)");

    println!("\n────────────────────── VERDICT LOT SIZER-REPUTATION ──────────────────────");
    println!(" {verdict}");
    println!("──────────────────────────────────────────────────────────────────────────");

    let payload = json!({
        "_note": "Chemin RÉEL Reputation::profile_schedule + run_waves_pure_scheduled (la boucle de sizer ADR-014 RÉELLE) sur des profils Mode A pur synthétique (étiquetés). R-b ordonne les profils par poids de routage décroissant (mêmes poids UCB que R-a) ; R-c gate par arrêt anticipé (le bas de classement n'est pas atteint, JAMAIS supprimé) ; l'exploration force le profil total mod k_max dans la région garantie (rotation déterministe keyée sur total — re-prouvable, ADR-007). L'ordonnancement est une PERMUTATION de [0,k_max) : enveloppe budgétaire (ADR-018) et plafond k_max (I-AK1) intacts ; l'ordre ne fait que converger plus tôt. §C prend total=6 (exploration ∈ GOOD) pour isoler l'exploitation ; §B couvre la rotation sur tous les total. Live: le sizer de prod reste en IDENTITÉ (extension bornée, à ouvrir une fois cette interaction mesurée).",
        "policy": { "k_min": K_MIN, "k_max": K_MAX, "batch": BATCH, "stop_agreement": 0.9, "good_profiles": GOOD },
        "A_determinism_genesis": { "pure_stable": s1 == s2, "genesis_identity": genesis == (0..K_MAX).collect::<Vec<_>>(), "ok": a_ok },
        "B_exploration_rotation": { "rotation_always_guaranteed": rotation_always_guaranteed, "profiles_covered": covered.len(), "k_max": K_MAX, "ok": b_ok },
        "C_prioritization_sizer": { "k_used_cold": cold_t.k_used, "k_used_warm": warm_t.k_used, "cold_stop": cold_t.stop_reason.as_str(), "warm_stop": warm_t.stop_reason.as_str(), "winner_cold": cold_winner, "winner_warm": warm_winner, "ok": c_ok },
        "D_budget_safety": { "envelope_unchanged": envelope_unchanged, "ceiling_unreachable_k_used": dt.k_used, "k_max": K_MAX, "warm_le_cold_le_kmax": only_saves, "ok": d_ok },
        "E_no_collapse": { "is_permutation": is_permutation, "loser_present": loser_present, "loser_reached_by_rotation": loser_reached, "ok": e_ok },
        "warm_schedule": schedule,
        "cost": { "profile_schedule_ns_per_call": sched_ns },
        "verdict": verdict,
        "go": go
    });
    let _ = fs::create_dir_all("scripts/swarm");
    let path = "scripts/swarm/EFE_SWARM_SIZER_REPUTATION_2026-06-18.json";
    let _ = fs::write(path, serde_json::to_string_pretty(&payload).unwrap());
    println!("\n✅ Résultats : {path}");
    assert!(go, "GATE ADR-019 REP-3 : l'ordonnancement du sizer doit faire ce qu'il prétend, son interaction sizer/budget mesurée");
}
