// SPDX-License-Identifier: UNLICENSED
//! # `lot_sizer_reputation_live` — le **GATE** d'ADR-019 **REP-4** : l'**OUVERTURE** de R-b/R-c.
//! L'ordonnancement réputationnel est-il, **chiffres à l'appui**, branché dans le sizer **LIVE** du
//! consensus — **sans rompre la preuve** ? C'est la précondition explicite à la bascule : *à n'ouvrir
//! qu'avec son interaction sizer/budget mesurée — et sa racine re-prouvée après permutation.*
//!
//! Contrairement à REP-3 (qui chiffrait la PRIMITIVE `profile_schedule` + `run_waves_pure_scheduled`
//! en isolation), REP-4 exerce le **vrai chemin de production** de bout en bout :
//! `recall → resolver.reputation_schedule_for → ConsensusTopology (generate_adaptive ordonnancé) →
//! attribution generator_{schedule[i]:03} → poids w[schedule[i]] → scellement → verify_swarm`.
//!
//! Ce qu'il prouve (les sections O-§* de l'architecture de l'ouverture) :
//! - **O-§A dormance (froid)** : un run **sans mémoire** ordonnance à l'**identité** (`variant =
//!   position`) — variants enregistrés `{0..k_max}` — et **re-prouve** (I-REP10).
//! - **O-§B priorisation (chaud)** : un run **chaud** (réputation favorisant les bons profils, apprise
//!   via le VRAI `fold`) atteint la cible **plus tôt** (`K_used` chaud ≪ froid) à **iso-qualité**.
//! - **O-§C attribution (I-REP7)** : la position gagnante crédite le **profil réel** `schedule[i]`
//!   (ici `generator_001`), **jamais la position** (`generator_000` reste non crédité).
//! - **O-§D RE-PREUVE après permutation (le linchpin, I-REP8)** : le run chaud, profils RÉORDONNÉS,
//!   voit sa racine **R0-re-prouvée** (`verify_swarm`) ET chaque candidat **R1-reproduit** depuis son
//!   **variant enregistré** (`replay_outputs`) — la provenance a capté `schedule[p]`, le moat tient.
//! - **O-§E cohérence poids↔ordre (I-REP6)** : le poids du profil exécuté à la position 0 est
//!   `w[schedule[0]]` (le poids haut du bon profil), pas `w[0]`.
//!
//! Harnais **HONNÊTE** : générateur Mode A pur synthétique (`profile_gen`, profils GOOD → « GOLD »,
//! étiqueté), **enregistré dans la flotte réelle** ; réputation **apprise par le vrai chemin `fold`**
//! (4 runs passés où les bons profils ont gagné, Mode A étiqueté) ; tout le reste = code de
//! production. `total = version` choisi (=4) tel que le profil d'exploration `e = 4 ∈ GOOD` isole la
//! priorisation — REP-3 §B couvre la rotation sur tous les `total`.
//!
//! GO ⇔ O-§A ∧ O-§B ∧ O-§C ∧ O-§D ∧ O-§E. Écrit `scripts/swarm/EFE_SWARM_SIZER_REPUTATION_LIVE_2026-06-18.json`.
//!
//! Run :  cargo run --release --example lot_sizer_reputation_live -p efe-swarm-sdk

use std::collections::BTreeSet;
use std::sync::Arc;
use std::fs;

use async_trait::async_trait;
use efe_nano_sdk::TenantId;
use efe_swarm_memory::{
    family_key, AgentStat, FamilyMode, InMemoryMemoryStore, MemoryStore, MemoryUpdate, RunSummary,
};
use efe_swarm_sdk::{
    replay_outputs, Budget, KSpec, MemoryHandle, Pattern, PatternSpec, StopRule, Swarm, TopologySpec,
};
use efe_swarm_topology::{AgentFleet, Generator};
use serde_json::{json, Value};
use uuid::Uuid;

const K_MIN: usize = 4;
const K_MAX: usize = 12;
const BATCH: usize = 4;
/// Les profils éprouvés-bons : ils émettent l'or (candidats qui s'accordent) ; le reste, du bruit
/// distinct. `{1,2,3,4}` (et non `{0,…}`) pour que l'ordonnancement chaud `{1,2,3,4}` se DISTINGUE
/// de l'identité-froide `{0,1,2,3}` — on lit ainsi directement que la permutation a eu lieu.
const GOOD: [usize; 4] = [1, 2, 3, 4];

/// Générateur Mode A pur synthétique : profil `variant` → « GOLD » s'il est bon, sinon bruit distinct.
/// Fonction pure de `variant` (I-AK3) : reproductible à l'identique en R1 depuis le variant enregistré.
struct ProfileGen;
#[async_trait]
impl Generator for ProfileGen {
    async fn generate(&self, _task: &Value, variant: usize) -> Value {
        if GOOD.contains(&variant) {
            json!("GOLD")
        } else {
            json!(format!("noise{variant}"))
        }
    }
}

/// Reproduit la sortie déterministe du profil (R1) — la MÊME fonction pure que `ProfileGen`.
fn reproduce_profile(variant: usize) -> Value {
    if GOOD.contains(&variant) {
        json!("GOLD")
    } else {
        json!(format!("noise{variant}"))
    }
}

fn spec() -> TopologySpec {
    TopologySpec::Consensus {
        generator: "profile_gen".into(),
        judges: vec!["consistency".into(), "j_non_empty".into()],
        k: KSpec::Adaptive { k_min: K_MIN, k_max: K_MAX, batch: BATCH, stop: StopRule::Agreement { threshold: 0.9 } },
        pattern: PatternSpec::Fixed(Pattern::Borda),
        escalate: false,
    }
}

fn fleet() -> Arc<AgentFleet> {
    let mut f = AgentFleet::standard();
    f.register_generator("profile_gen", Arc::new(ProfileGen));
    Arc::new(f)
}

/// Les variants enregistrés des records GÉNÉRATEUR scellés — la PREUVE de quels profils ont tourné,
/// à quelles positions, captée par le métré (I-REP8).
fn recorded_variants(out: &efe_swarm_sdk::RunOutcome) -> BTreeSet<u64> {
    out.sealed
        .as_ref()
        .expect("scellé")
        .records
        .iter()
        .filter(|r| r.agent_id.as_str() == "generator")
        .filter_map(|r| r.variant)
        .collect()
}

#[tokio::main]
async fn main() {
    let task = json!({ "text": "open R-b/R-c", "expected": "GOLD" });
    let tenant = TenantId::system().to_string();
    let budget = Budget::cents(1000);

    // ═══ O-§A — DORMANCE : froid (sans mémoire) → identité + re-preuve ═══════════════════════
    let cold_swarm = Swarm::builder().fleet(fleet()).build().unwrap(); // PAS de mémoire ⇒ recall off
    let cold = cold_swarm.run_traced(task.clone(), spec(), budget).await.expect("run froid");
    let cold_k = cold.run.output["sizing"]["k_used"].as_u64().unwrap() as usize;
    let cold_variants = recorded_variants(&cold);
    let cold_winner = cold.run.output["value"].clone();
    let identity_set: BTreeSet<u64> = (0..K_MAX as u64).collect();
    let a_ok = cold.run.provenance.verified            // re-prouvé (I-REP10)
        && cold_variants == identity_set               // variants {0..11} = IDENTITÉ (variant=position)
        && cold_k == K_MAX;                            // pas d'accord précoce (échantillon mixte) → k_max

    // ═══ SEED — réputation apprise par le VRAI chemin `fold` (4 runs passés, Mode A étiqueté) ═══
    // 4 replis où chacun des bons profils a gagné une fois → version=4. e = 4 mod 12 = 4 ∈ GOOD :
    // le profil d'exploration tombe sur un bon profil, isolant la priorisation (REP-3 §B couvre la rotation).
    let store = Arc::new(InMemoryMemoryStore::new());
    let (kind, agents) = spec().family_signature();
    let family = family_key(&task, &kind, &agents, FamilyMode::Structural);
    for (n, &p) in GOOD.iter().enumerate() {
        let summary = RunSummary {
            clean: true,
            // Les bons profils s'accordent → ces runs passés se sont ARRÊTÉS TÔT (k_used = k_min).
            // Honnête ET nécessaire : un k_used=k_max entraînerait le SizePrior à warm-starter k_min
            // vers k_max, ce qui ferait tourner TOUS les profils (ordre permuté mais set complet) et
            // masquerait la priorisation. Des bons profils qui gagnent S'ARRÊTENT à k_min — cohérent.
            k_used: K_MIN as u32,
            winning_pattern: Some("borda".into()),
            escalated: false,
            exemplars: Vec::new(),
            agents: vec![AgentStat { agent_id: format!("generator_{p:03}"), verified: true, won: true }],
            failed: false,
            failure_key: None,
        };
        let update = MemoryUpdate { run_id: Uuid::from_u128(0xC0FFEE + n as u128), family: family.clone(), summary };
        store.fold(&tenant, update).await.expect("seed fold");
    }

    // ── état pinné (ce que le recall chaud lira) : schedule non-identité + poids (O-§E data-level) ──
    let seed_snap = store.snapshot(&tenant, &family).await.expect("snapshot seed");
    let rep = &seed_snap.cells.reputation;
    let schedule = rep.profile_schedule(seed_snap.version, K_MIN, K_MAX);
    let ids: Vec<String> = (0..K_MAX).map(|i| format!("generator_{i:03}")).collect();
    let weights = rep.routing_weights(&ids);
    let schedule_non_identity = schedule != (0..K_MAX).collect::<Vec<_>>();
    // O-§E : le poids appliqué à la position 0 est w[schedule[0]] (profil bon → poids HAUT), pas w[0].
    let w_at_pos0 = weights[schedule[0]];
    let e_ok = GOOD.contains(&schedule[0]) && w_at_pos0 > 1000 && schedule_non_identity;

    // ═══ O-§B+C+D — CHAUD : la mémoire ordonnance le sizer LIVE, et la racine re-prouve ══════════
    let warm_swarm = Swarm::builder().fleet(fleet()).memory(MemoryHandle::new(store.clone())).build().unwrap();
    let warm = warm_swarm.run_traced(task.clone(), spec(), budget).await.expect("run chaud");
    let warm_k = warm.run.output["sizing"]["k_used"].as_u64().unwrap() as usize;
    let warm_variants = recorded_variants(&warm);
    let warm_winner = warm.run.output["value"].clone();
    let good_set: BTreeSet<u64> = GOOD.iter().map(|&p| p as u64).collect();

    // O-§B : priorisation — chaud s'arrête plus tôt, MÊME gagnant (iso-qualité).
    let b_ok = warm_k < cold_k && warm_winner == cold_winner && warm_winner == json!("GOLD");

    // O-§D : RE-PREUVE après permutation. R0 = verify_swarm (la racine du run). R1 = replay_outputs
    //        sur les records générateur, reproduits depuis le VARIANT ENREGISTRÉ (= schedule[p]).
    let gen_recs: Vec<_> = warm
        .sealed
        .as_ref()
        .expect("scellé")
        .records
        .iter()
        .filter(|r| r.agent_id.as_str() == "generator")
        .cloned()
        .collect();
    let r1 = replay_outputs(&gen_recs, |r| reproduce_profile(r.variant.unwrap_or(0) as usize));
    let d_ok = warm.run.provenance.verified          // R0 : racine re-prouvée APRÈS permutation
        && r1.mismatched == 0 && r1.matched == warm_k // R1 : chaque candidat reproduit depuis son variant
        && warm_variants == good_set;                 // variants {1,2,3,4} = l'ORDONNANCEMENT (≠ identité {0,1,2,3})

    // O-§C : attribution. La position gagnante (0) a exécuté schedule[0]=1 → crédit à generator_001,
    //        JAMAIS à generator_000 (la position). Le run chaud a auto-replié (learn) : on relit.
    let after = store.snapshot(&tenant, &family).await.expect("snapshot post-chaud");
    let won_profile1 = after.cells.reputation.of("generator_001").map(|r| r.won).unwrap_or(0);
    let pos0_uncredited = after.cells.reputation.of("generator_000").is_none();
    let c_ok = won_profile1 >= 2 && pos0_uncredited; // 1 (seed) + 1 (chaud) ; la position n'est pas créditée

    let go = a_ok && b_ok && c_ok && d_ok && e_ok;
    let verdict = if go {
        "GO ✅ — froid=identité re-prouvé · chaud ↓K_used (iso-qualité) · crédit au PROFIL pas à la position · RACINE re-prouvée après permutation (R0+R1) · poids w[schedule[i]]"
    } else {
        "NO-GO ❌"
    };

    println!("\n O-§A DORMANCE (froid)   : verified={} · variants {:?}=identité · k_used={cold_k}  → A_ok={a_ok}", cold.run.provenance.verified, cold_variants.iter().take(4).collect::<Vec<_>>());
    println!(" O-§B PRIORISATION       : K_used chaud {warm_k} < froid {cold_k} · gagnant « {} » (iso-qualité)  → B_ok={b_ok}", warm_winner.as_str().unwrap_or("?"));
    println!(" O-§C ATTRIBUTION (I-REP7): generator_001.won={won_profile1} (profil de la pos. gagnante) · generator_000 non crédité={pos0_uncredited}  → C_ok={c_ok}");
    println!(" O-§D RE-PREUVE (I-REP8) : R0 verify_swarm={} · R1 replay {}/{} matched · variants {:?}=ordonnancement  → D_ok={d_ok}", warm.run.provenance.verified, r1.matched, warm_k, warm_variants.iter().collect::<Vec<_>>());
    println!(" O-§E POIDS↔ORDRE (I-REP6): schedule[0]={} (bon profil) · w[schedule[0]]={w_at_pos0}‰ (haut) · non-identité={schedule_non_identity}  → E_ok={e_ok}", schedule[0]);
    println!(" schedule chaud (v={})    : {schedule:?}", seed_snap.version);

    println!("\n──────────────────── VERDICT LOT SIZER-REPUTATION-LIVE (OUVERTURE) ────────────────────");
    println!(" {verdict}");
    println!("───────────────────────────────────────────────────────────────────────────────────────");

    let payload = json!({
        "_note": "OUVERTURE de R-b/R-c sur le VRAI chemin de production : recall (réputation apprise via fold, Mode A étiqueté) → resolver.reputation_schedule_for → ConsensusTopology generate_adaptive ORDONNANCÉ → attribution generator_{schedule[i]:03} → poids w[schedule[i]] → scellement → verify_swarm + replay_outputs. Le moat est PRÉSERVÉ car (I-REP8) le métré enregistre le variant RÉEL (schedule[p]) et l'auteur d'écriture reste l'unique 'generator' : R0 re-hash inchangé, R1 reproduit depuis le variant enregistré. total=version=4 → exploration e=4 ∈ GOOD (isole la priorisation ; REP-3 §B couvre la rotation). Dormance : sans mémoire → identité → byte-identique (I-REP10).",
        "policy": { "k_min": K_MIN, "k_max": K_MAX, "batch": BATCH, "good_profiles": GOOD, "seed_version": seed_snap.version },
        "schedule_warm": schedule,
        "A_dormancy_cold": { "verified": cold.run.provenance.verified, "variants_identity": cold_variants == identity_set, "k_used": cold_k, "ok": a_ok },
        "B_prioritization": { "k_used_warm": warm_k, "k_used_cold": cold_k, "winner_warm": warm_winner, "winner_cold": cold_winner, "ok": b_ok },
        "C_attribution_irep7": { "generator_001_won": won_profile1, "generator_000_uncredited": pos0_uncredited, "ok": c_ok },
        "D_reproof_irep8": { "R0_verify_swarm": warm.run.provenance.verified, "R1_replay_matched": r1.matched, "R1_replay_mismatched": r1.mismatched, "warm_variants": warm_variants.iter().collect::<Vec<_>>(), "schedule_applied_live": warm_variants == good_set, "ok": d_ok },
        "E_weight_order_irep6": { "schedule_pos0": schedule[0], "weight_at_pos0_milli": w_at_pos0, "schedule_non_identity": schedule_non_identity, "ok": e_ok },
        "verdict": verdict,
        "go": go
    });
    let _ = fs::create_dir_all("scripts/swarm");
    let path = "scripts/swarm/EFE_SWARM_SIZER_REPUTATION_LIVE_2026-06-18.json";
    let _ = fs::write(path, serde_json::to_string_pretty(&payload).unwrap());
    println!("\n✅ Résultats : {path}");
    assert!(go, "GATE ADR-019 REP-4 (ouverture) : le sizer live ordonnancé doit faire ce qu'il prétend ET re-prouver après permutation");
}
