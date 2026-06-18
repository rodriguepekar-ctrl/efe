// SPDX-License-Identifier: UNLICENSED
//! **ADR-019, Mode C RÉEL — les lectures-retour AGNOSTIQUES sont-elles vivantes sur un vrai LLM ?**
//! (Architecture du Mode C réel, §5 + §12 étape 3 — la confirmation d'intégration.)
//!
//! Quatre des cinq lectures-retour (taille / pattern / routage R-a / ordonnancement R-b-R-c) sont
//! **agnostiques au mode** : elles opèrent sur la couche consensus/sizer, qui traite les candidats
//! comme **opaques** — elle ignore s'ils viennent d'une fonction Mode A ou d'un LLM. Leur
//! *correction* est déjà acquise (gates Mode A : MEMORY, PATBIAS, REPUTATION, SIZER_REPUTATION).
//! Ce qui n'était PAS exercé : un run **Mode C réel AVEC mémoire** — les autres harnais Ollama
//! (`replay_real`, `adaptive_keyless`, `n17_persist_real`) tournent **sans** mémoire. Ce test ferme
//! ce point d'intégration : une mémoire riche (taille + pattern + réputation + exemplaires), semée
//! via le vrai `fold`, est-elle recall-ée ET appliquée dans le pipeline LLM, **en re-prouvant** ?
//!
//! **Périmètre EXACT des assertions (revue adversariale — ne pas surévaluer).** Ce test ASSERTE la
//! seule lecture à la fois **déterministe** ET **indépendamment observable** : le **warm-start de
//! `k_min`** (lecture taille), calculé par le resolver depuis le prior pinné — il FIRE ou non, sans
//! rien devoir au LLM. Un **run de CONTRÔLE sans mémoire** prouve la causalité : `k_min` y vaut 4
//! (le défaut du spec) ; avec la mémoire, 6 — l'écart ne peut venir que du prior de taille. Les
//! autres lectures (pattern, réputation R-a/R-b-R-c, exemplaires) sont **SEMÉES pour exercer le
//! chemin de recall** dans le pipeline LLM, mais leur *correction* n'est PAS re-démontrée ici —
//! elle reste acquise par les gates Mode A (PATBIAS, REPUTATION, SIZER_REPUTATION) : ce harnais les
//! montre **vivantes**, pas correctes. Et le run **re-prouve** (R0) avec une chaîne N17 **cohérente**
//! — le moat Mode C tient sous mémoire active.
//!
//! Ignoré par défaut (CI sans modèle). Lancer :
//!   cargo test -p efe-swarm-sdk --test modec_memory_active -- --ignored --nocapture

use std::sync::Arc;

use efe_nano_sdk::TenantId;
use efe_swarm_memory::{
    family_key, AgentStat, Exemplar, FamilyMode, InMemoryMemoryStore, MemoryStore, MemoryUpdate, RunSummary,
};
use efe_swarm_sdk::{Budget, ClientMode, KSpec, MemoryHandle, PatternSpec, StopRule, Swarm, TopologySpec};
use serde_json::json;
use uuid::Uuid;

const ENDPOINT: &str = "http://127.0.0.1:11434";
const MODEL: &str = "gemma3:12b";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "nécessite un Ollama local (ollama serve + gemma3:12b)"]
async fn memory_readbacks_are_active_on_a_real_mode_c_run() {
    let tenant = TenantId::system().to_string();
    let task = json!({ "text": "The central bank raised its benchmark interest rate.", "expected": "finance" });

    // Spec ADAPTATIVE (pour que la taille ET R-b/R-c soient en jeu) : k_min 4 → warm-start visé 6,
    // k_max 8 (borné pour rester rapide en Mode C). Pattern adaptatif ⇒ le biais de pattern s'applique.
    let spec = || TopologySpec::Consensus {
        generator: "Identify the single most relevant topic word for this text.".into(),
        judges: vec!["Judge how well the one-word topic label fits the text.".into()],
        k: KSpec::Adaptive { k_min: 4, k_max: 8, batch: 2, stop: StopRule::Agreement { threshold: 0.9 } },
        pattern: PatternSpec::adaptive_default(),
        escalate: false,
    };

    // SEED — une mémoire RICHE, via le vrai `fold` (Mode A étiqueté) : 5 runs passés à K=6 (→ le
    // SizePrior warm-startera k_min vers 6), gagnés par les profils {0,1,2} (→ réputation pour R-a /
    // R-b-R-c), pattern « borda », et des exemplaires propres (→ few-shot). TOUTES les lectures sont
    // donc disponibles au recall : on exerce le chemin agnostique COMPLET dans le pipeline LLM.
    let store = Arc::new(InMemoryMemoryStore::new());
    let (kind, agents) = spec().family_signature();
    let family = family_key(&task, &kind, &agents, FamilyMode::Structural);
    let exemplars: Vec<Exemplar> = ["finance", "technology", "health"]
        .iter()
        .enumerate()
        .map(|(i, l)| Exemplar { content_hash: format!("{i:064x}"), preview: (*l).to_string(), quality_milli: 1000 })
        .collect();
    for n in 0..5u128 {
        let summary = RunSummary {
            clean: true,
            k_used: 6, // → le prior de taille suggère k_min ≈ 6 (warm-start déterministe)
            winning_pattern: Some("borda".into()),
            escalated: false,
            exemplars: exemplars.clone(),
            agents: vec![
                AgentStat { agent_id: "generator_000".into(), verified: true, won: true },
                AgentStat { agent_id: "generator_001".into(), verified: true, won: true },
                AgentStat { agent_id: "generator_002".into(), verified: true, won: true },
            ],
            failed: false,
            failure_key: None,
        };
        store
            .fold(&tenant, MemoryUpdate { run_id: Uuid::from_u128(0xC0DE_C000 + n), family: family.clone(), summary })
            .await
            .expect("seed fold");
    }

    // CONTRÔLE — Mode C réel SANS mémoire : prouve la CAUSALITÉ du warm-start. `k_min` doit y valoir
    // le DÉFAUT du spec (4) ; s'il valait déjà 6 ici, le 6 du run chaud ne prouverait rien.
    let cold = Swarm::builder()
        .mode(ClientMode::SelfHost { endpoint: ENDPOINT.into(), model: MODEL.into() })
        .require_healthy_backend(true)
        .build()
        .expect("backend Ollama sain (contrôle)");
    let cold_out = cold.run_traced(task.clone(), spec(), Budget::cents(100_000)).await.expect("run contrôle");
    let cold_k_min = cold_out.run.output["sizing"]["policy"]["k_min"].as_u64().expect("trace sizing (contrôle)");
    assert_eq!(cold_k_min, 4, "CONTRÔLE : sans mémoire, k_min = défaut du spec (4) — aucun warm-start");

    // RUN — Mode C réel AVEC la mémoire riche branchée (recall ON).
    let swarm = Swarm::builder()
        .mode(ClientMode::SelfHost { endpoint: ENDPOINT.into(), model: MODEL.into() })
        .require_healthy_backend(true)
        .memory(MemoryHandle::new(store.clone()))
        .build()
        .expect("backend Ollama sain");
    let out = swarm.run_traced(task, spec(), Budget::cents(100_000)).await.expect("run Mode C avec mémoire");

    // ── ① la lecture AGNOSTIQUE déterministe a FIRÉ dans le pipeline LLM : warm-start de k_min. ──
    let k_min_eff = out.run.output["sizing"]["policy"]["k_min"].as_u64().expect("trace de sizing présente");
    assert_eq!(k_min_eff, 6, "warm-start k_min EN MODE C : contrôle={cold_k_min} → chaud={k_min_eff} (la lecture taille a CAUSÉ l'écart)");
    let k_used = out.run.output["sizing"]["k_used"].as_u64().unwrap();
    assert!(k_used >= 6, "I-AK1 : K_used ≥ k_min warm-starté (= {k_used})");

    // ── ② le moat Mode C tient SOUS mémoire active : R0 re-prouve + N17 cohérent. ──
    assert!(out.run.provenance.verified, "R0 : la chaîne re-prouve (mémoire active n'altère pas la preuve)");
    let n17 = out.run.n17.as_ref().expect("résumé N17 (run Mode C)");
    assert!(n17.all_consistent && n17.unlinked == 0 && n17.broken == 0, "N17 cohérent : {n17:?}");
    assert!(n17.calls >= k_used as usize, "≥ K_used générations LLM, toutes forensiques");

    // ── ③ un pattern a été décidé (la lecture pattern + l'agrégation ont tourné sur des candidats LLM). ──
    assert!(out.run.decided_pattern.is_some(), "un pattern d'agrégation a été décidé en Mode C");

    println!(
        "✅ Mode C + mémoire : warm-start k_min contrôle={cold_k_min}→chaud={k_min_eff} (causal, déterministe) · \
         K_used={k_used} · R0 re-prouve · N17 cohérent ({} appels forensiques) · pattern {:?}. Les lectures \
         agnostiques tournent dans le pipeline LLM (semées, vivantes) ; leur correction reste acquise par les gates Mode A.",
        n17.calls, out.run.decided_pattern
    );
}
