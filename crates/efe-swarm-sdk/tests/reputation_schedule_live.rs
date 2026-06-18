// SPDX-License-Identifier: UNLICENSED
//! **ADR-019 read-back B — R-b/R-c OUVERT (REP-4)** : le garde-fou AUTOMATISÉ du câblage
//! `recall → resolver.reputation_schedule_for → ConsensusTopology (sizer ordonnancé) → scellement →
//! verify_swarm`. Le gate chiffré vit dans l'exemple `lot_sizer_reputation_live` (lancé à la main) ;
//! ce fichier est sa contrepartie en `cargo test` — si quelqu'un débranche l'ordonnancement (retire
//! `with_reputation_schedule` du resolve, casse `reputation_schedule_for`, ou ré-identité
//! `generate_adaptive`), CES tests tombent.
//!
//! Ce qu'ils tiennent, sur le VRAI chemin de production :
//! - **dormance (I-REP10)** : sans mémoire ⇒ identité (`variant = position`) ⇒ re-prouvé ;
//! - **ouverture (I-REP6/7/8)** : une réputation apprise (via le vrai `fold`, Mode A étiqueté)
//!   ordonnance le sizer live — profils réordonnés, arrêt plus tôt, attribution au PROFIL, et la
//!   racine **re-prouve après permutation** (R0 `verify_swarm` + R1 `replay_outputs`).

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use efe_nano_sdk::TenantId;
use efe_swarm_memory::{
    family_key, AgentStat, FamilyMode, InMemoryMemoryStore, MemoryStore, MemoryUpdate, RunSummary,
};
use efe_swarm_sdk::{
    replay_outputs, Budget, KSpec, MemoryHandle, Pattern, PatternSpec, RunOutcome, StopRule, Swarm,
    TopologySpec,
};
use efe_swarm_topology::{AgentFleet, Generator};
use serde_json::{json, Value};
use uuid::Uuid;

const K_MIN: usize = 4;
const K_MAX: usize = 12;
const BATCH: usize = 4;
/// Profils éprouvés-bons (émettent l'or, candidats qui s'accordent). `{1,2,3,4}` ≠ `{0,1,2,3}` pour
/// que l'ordonnancement chaud se DISTINGUE de l'identité-froide par le SET de variants enregistrés.
const GOOD: [usize; 4] = [1, 2, 3, 4];

/// Générateur Mode A pur, fonction de `variant` (I-AK3) — reproductible à l'identique en R1.
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

/// Les variants enregistrés des records générateur — la preuve de quels profils ont tourné (I-REP8).
fn recorded_variants(out: &RunOutcome) -> BTreeSet<u64> {
    out.sealed
        .as_ref()
        .expect("scellé")
        .records
        .iter()
        .filter(|r| r.agent_id.as_str() == "generator")
        .filter_map(|r| r.variant)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cold_start_is_identity_and_reproves() {
    // I-REP10 : sans mémoire, le sizer ordonnance à l'identité (variant = position) ; l'échantillon
    // mixte (4 or + 8 bruits) n'atteint jamais 0,9 → k_max ; et la racine re-prouve.
    let swarm = Swarm::builder().fleet(fleet()).build().unwrap(); // aucune mémoire ⇒ recall off
    let out = swarm
        .run_traced(json!({ "text": "cold", "expected": "GOLD" }), spec(), Budget::cents(1000))
        .await
        .expect("run froid");

    assert!(out.run.provenance.verified, "froid : re-prouvé (I-REP10)");
    assert_eq!(out.run.output["sizing"]["k_used"], json!(K_MAX), "mixte → jamais d'accord → k_max");
    let variants = recorded_variants(&out);
    assert_eq!(variants, (0..K_MAX as u64).collect(), "variants {{0..11}} = IDENTITÉ (variant = position)");
    assert_eq!(out.run.output["value"], json!("GOLD"), "Borda extrait l'or");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn warm_reputation_opens_rbrc_on_the_live_path() {
    let tenant = TenantId::system().to_string();
    let store = Arc::new(InMemoryMemoryStore::new());
    let task = json!({ "text": "warm", "expected": "GOLD" });

    // SEED via le VRAI chemin `fold` : 4 runs passés où chaque bon profil a gagné, ARRÊTÉS TÔT
    // (k_used = k_min — sinon le SizePrior warm-starterait k_min vers k_max et tous les profils
    // tourneraient, masquant la priorisation). Mode A étiqueté. version = 4 ⇒ rotation e = 4 ∈ GOOD.
    let (kind, agents) = spec().family_signature();
    let family = family_key(&task, &kind, &agents, FamilyMode::Structural);
    for (n, &p) in GOOD.iter().enumerate() {
        let summary = RunSummary {
            clean: true,
            k_used: K_MIN as u32,
            winning_pattern: Some("borda".into()),
            escalated: false,
            exemplars: Vec::new(),
            agents: vec![AgentStat { agent_id: format!("generator_{p:03}"), verified: true, won: true }],
            failed: false,
            failure_key: None,
        };
        store
            .fold(&tenant, MemoryUpdate { run_id: Uuid::from_u128(0xA11CE + n as u128), family: family.clone(), summary })
            .await
            .expect("seed fold");
    }

    // CHAUD : la mémoire ordonnance le sizer live.
    let swarm = Swarm::builder().fleet(fleet()).memory(MemoryHandle::new(store.clone())).build().unwrap();
    let out = swarm.run_traced(task.clone(), spec(), Budget::cents(1000)).await.expect("run chaud");

    // O-§B priorisation : les bons profils {1,2,3,4} en tête → accord dès k_min → arrêt anticipé.
    assert_eq!(out.run.output["sizing"]["k_used"], json!(K_MIN), "chaud : arrêt à k_min (bons profils d'abord)");
    assert_eq!(out.run.output["value"], json!("GOLD"), "iso-qualité : même gagnant que froid");

    // O-§D (linchpin) : ordonnancement APPLIQUÉ live + racine RE-PROUVÉE après permutation.
    let variants = recorded_variants(&out);
    assert_eq!(variants, GOOD.iter().map(|&p| p as u64).collect(), "variants {{1,2,3,4}} = l'ORDONNANCEMENT (≠ identité {{0,1,2,3}})");
    assert!(out.run.provenance.verified, "R0 : verify_swarm re-prouve APRÈS permutation (I-REP8)");
    // R1 : chaque candidat reproduit depuis son VARIANT ENREGISTRÉ (= schedule[p]).
    let gen_recs: Vec<_> = out
        .sealed
        .as_ref()
        .expect("scellé")
        .records
        .iter()
        .filter(|r| r.agent_id.as_str() == "generator")
        .cloned()
        .collect();
    let r1 = replay_outputs(&gen_recs, |r| {
        let v = r.variant.unwrap_or(0) as usize;
        if GOOD.contains(&v) { json!("GOLD") } else { json!(format!("noise{v}")) }
    });
    assert_eq!(r1.mismatched, 0, "R1 : zéro divergence — reproduit depuis le variant enregistré");
    assert_eq!(r1.matched, K_MIN, "R1 : les k_min candidats reproduits");

    // O-§C attribution (I-REP7) : la position gagnante (0) a exécuté schedule[0]=1 → crédit au PROFIL
    // generator_001, JAMAIS à la position generator_000 (qui n'a pas tourné). Le run chaud a auto-replié.
    let after = store.snapshot(&tenant, &family).await.expect("snapshot post-chaud");
    let rep = &after.cells.reputation;
    assert_eq!(rep.of("generator_001").map(|r| r.won), Some(2), "profil 1 (pos. gagnante) crédité : 1 seed + 1 chaud");
    assert!(rep.of("generator_000").is_none(), "la POSITION 0 n'est pas créditée — c'est le profil qui l'est (I-REP7)");
}
