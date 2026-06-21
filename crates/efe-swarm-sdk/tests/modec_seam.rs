// SPDX-License-Identifier: UNLICENSED
//! **B0.3 — le seam CI en action.** Ce que le seam DÉBLOQUE (architecture B0, §2.4), réalisé : le
//! chemin **Mode C COMPLET** — recall mémoire → génération LLM → agrégation → scellement → re-preuve
//! R0 — exécutable **en intégration continue, sans Ollama ni GPU**, via un `ScriptedBackend`
//! déterministe injecté par `SwarmBuilder::with_local_backend`. Jusqu'ici, le Mode C n'était couvert
//! que par des tests `#[ignore]` (Ollama) ou des tests inline ciblés ; ce fichier est le consommateur
//! d'INTÉGRATION du seam que la revue signalait manquant.
//!
//! Gaté `#[cfg(feature = "test-util")]` (I-SEAM : le seam est absent du build par défaut). Lancer :
//!   cargo test -p efe-swarm-sdk --features test-util --test modec_seam
#![cfg(feature = "test-util")]

use std::sync::Arc;

use efe_nano_sdk::TenantId;
use efe_swarm_memory::{family_key, FamilyMode, InMemoryMemoryStore, MemoryStore, MemoryUpdate, RunSummary};
use efe_swarm_sdk::{Budget, KSpec, MemoryHandle, PatternSpec, ScriptedBackend, StopRule, Swarm, TopologySpec};
use serde_json::json;
use uuid::Uuid;

/// La spec ADAPTATIVE du test #[ignore] modec_memory_active, ici exercée via le seam : k_min 4 →
/// warm-start visé 6, k_max 8. Pattern adaptatif. Générateur + juge = des instructions LLM (en Mode
/// C, le resolver les bâtit en LlmGen/LlmJudge par-dessus le backend injecté).
fn adaptive_spec() -> TopologySpec {
    TopologySpec::Consensus {
        generator: "Identify the single most relevant topic word for this text.".into(),
        judges: vec!["Judge how well the one-word topic label fits the text.".into()],
        k: KSpec::Adaptive { k_min: 4, k_max: 8, batch: 2, stop: StopRule::Agreement { threshold: 0.9 } },
        pattern: PatternSpec::adaptive_default(),
        escalate: false,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn modec_full_path_warm_start_fires_via_seam_no_ollama() {
    // Le chemin Mode C COMPLET en CI : seed taille K=6 → recall warm-start (k_min 4→6) → génération
    // LLM scriptée → agrégation → scellement → R0 re-prouve. La CAUSALITÉ du warm-start est prouvée
    // par un CONTRÔLE sans mémoire (k_min=4). Jumelle déterministe de modec_memory_active (#[ignore]).
    let tenant = TenantId::system().to_string();
    let task = json!({ "text": "The central bank raised its benchmark interest rate.", "expected": "finance" });

    // SEED : 5 runs propres à K=6 → le SizePrior warm-startera k_min vers 6 (déterministe).
    let store = Arc::new(InMemoryMemoryStore::new());
    let (kind, agents) = adaptive_spec().family_signature();
    let family = family_key(&task, &kind, &agents, FamilyMode::Structural);
    for n in 0..5u128 {
        store
            .fold(&tenant, MemoryUpdate { run_id: Uuid::from_u128(0xC0DE_5EA0 + n), family: family.clone(), summary: RunSummary::clean(6) })
            .await
            .expect("seed fold");
    }

    // CONTRÔLE — Mode C (seam) SANS mémoire : k_min = défaut du spec (4). Prouve la causalité.
    let cold = Swarm::builder().with_local_backend(ScriptedBackend::responding("finance")).build().unwrap();
    let cold_out = cold.run_traced(task.clone(), adaptive_spec(), Budget::cents(1_000_000)).await.expect("run contrôle (seam)");
    let cold_k_min = cold_out.run.output["sizing"]["policy"]["k_min"].as_u64().expect("trace sizing (contrôle)");
    assert_eq!(cold_k_min, 4, "CONTRÔLE : sans mémoire, k_min = défaut du spec (4)");

    // CHAUD — Mode C (seam) AVEC la mémoire branchée : warm-start k_min = 6, APPLIQUÉ dans le pipeline.
    let warm = Swarm::builder()
        .with_local_backend(ScriptedBackend::responding("finance"))
        .memory(MemoryHandle::new(store.clone()))
        .build()
        .unwrap();
    let out = warm.run_traced(task, adaptive_spec(), Budget::cents(1_000_000)).await.expect("run chaud (seam)");

    // ① la lecture taille a FIRÉ dans le pipeline Mode C : warm-start de k_min, causal.
    let k_min_eff = out.run.output["sizing"]["policy"]["k_min"].as_u64().expect("trace de sizing présente");
    assert_eq!(k_min_eff, 6, "WARM-START EN MODE C (seam) : contrôle={cold_k_min} → chaud={k_min_eff} (la lecture taille a CAUSÉ l'écart)");
    let k_used = out.run.output["sizing"]["k_used"].as_u64().unwrap();
    assert!(k_used >= 6, "I-AK1 : K_used ≥ k_min warm-starté (= {k_used})");

    // ② le moat Mode C tient SOUS le seam : R0 re-prouve + N17 cohérent. Le chemin scellé est exercé
    //    de bout en bout (le ScriptedBackend pousse une PendingLlmCall par appel, comme un vrai backend).
    assert!(out.run.provenance.verified, "R0 : la chaîne re-prouve (chemin Mode C COMPLET via le seam)");
    let n17 = out.run.n17.as_ref().expect("résumé N17 (run Mode C)");
    assert!(n17.all_consistent && n17.unlinked == 0 && n17.broken == 0, "N17 cohérent : {n17:?}");
    assert!(n17.calls >= k_used as usize, "≥ K_used générations LLM, toutes forensiques (scriptées)");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn modec_noisy_llm_outputs_are_parsed_robustly_via_seam() {
    // §2.4 (item 4) — un backend qui renvoie des sorties MAL FORMÉES (texte arbitraire, JSON cassé,
    // score non-numérique) ne fait PAS paniquer le pipeline Mode C. Avec un consensus Borda (les juges
    // tournent TOUJOURS), le juge « Score… » reçoit une sortie non-numérique → le parsing dégrade
    // gracieusement (score → défaut), le gagnant est la sortie présente, et le run se SCELLE +
    // RE-PROUVE (R0). C'est la robustesse de parsing que rien n'exerçait avant le seam.
    let task = json!({ "text": "q", "expected": "x" });
    // consensus() ⇒ pattern Fixed(Borda) ⇒ les juges sont TOUJOURS consultés (Borda agrège leurs
    // ballots), donc le parsing du juge sur une sortie bruyante est réellement exercé.
    let spec = TopologySpec::consensus("Identify the topic in one word.", &["Score the candidate from 0 to 10."], 3);
    let garbage = "⊘ this is not a number and { not valid json — arbitrary LLM noise ]]";
    let noisy = Swarm::builder().with_local_backend(ScriptedBackend::responding(garbage)).build().unwrap();

    let out = noisy
        .run_traced(task, spec, Budget::cents(1_000_000))
        .await
        .expect("le pipeline Mode C ne panique PAS sur des sorties LLM bruyantes");

    // Le run se scelle et re-prouve malgré le bruit : le parsing (générateur + juge Borda) a dégradé
    // sans casser. C'est la garantie de robustesse cherchée.
    assert!(out.run.provenance.verified, "R0 : le run re-prouve malgré des sorties LLM bruyantes");
    let n17 = out.run.n17.as_ref().expect("résumé N17 présent");
    assert!(n17.all_consistent && n17.broken == 0, "N17 cohérent même sous bruit : {n17:?}");
    // Le gagnant est la sortie (présente, non-vide) : un bruit non-vide reste une « réponse présente ».
    assert!(out.run.quality.candidates >= 3, "les 3 candidats bruyants ont bien été générés + scellés");
}
