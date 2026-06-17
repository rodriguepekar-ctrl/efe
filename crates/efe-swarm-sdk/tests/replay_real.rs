// SPDX-License-Identifier: UNLICENSED
//! **ADR-017, Mode C RÉEL** — le rejeu déterministe (R0) tient-il sur un run LLM **authentique** ?
//!
//! Le harnais `lot_replay` et les gardiens de la passerelle prouvent la mécanique avec des
//! agents purs synthétiques. Ici, le run est **réel** (Ollama `gemma3:12b`, Mode C). On éprouve
//! la propriété R0 — la plus forte qui tienne en Mode C : **re-sceller les records persistés**
//! (ordre canonique ADR-007) re-prouve **exactement** la `provenance_root` du run, et la chaîne
//! forensique N17 se **re-vérifie** (sans la recomputer). On NE re-exécute PAS le provider (la
//! sortie LLM n'est pas reproductible bit-à-bit — c'est R1, hors-périmètre pour le Mode C, §10).
//!
//! Ignoré par défaut (CI sans modèle). Lancer :
//!   cargo test -p efe-swarm-sdk --test replay_real -- --ignored --nocapture

use efe_nano_sdk::TenantId;
use efe_swarm_sdk::{replay, verify_run_offline, Budget, ClientMode, HmacKeySource, Swarm, TopologySpec};
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "nécessite un Ollama local (ollama serve + gemma3:12b)"]
async fn replay_r0_reproves_root_of_real_mode_c_run() {
    // ── 1) un run Mode C RÉEL (Ollama), avec sa graine déterministe posée par le SDK. ──
    let swarm = Swarm::builder()
        .mode(ClientMode::SelfHost {
            endpoint: "http://127.0.0.1:11434".into(),
            model: "gemma3:12b".into(),
        })
        .require_healthy_backend(true)
        .build()
        .expect("backend Ollama sain");

    let spec = TopologySpec::consensus(
        "Classify the topic in one word from {technology,finance,health,legal}.",
        &["Judge how well the label fits the text."],
        3,
    );
    let out = swarm
        .run_traced(
            json!({ "text": "The court ruled the contract clause unenforceable.", "expected": "legal" }),
            spec,
            Budget::cents(1000),
        )
        .await
        .expect("run Mode C réel");

    assert!(!out.llm_records.is_empty(), "≥ 1 LlmProvenanceRecord (run Mode C réel)");
    assert!(out.run.provenance.verified, "scellé + vérifié en vol");
    let sealed_root = out.run.provenance.root.clone();
    assert!(sealed_root.is_some(), "le run réel a une provenance_root");

    // ── 2) R0 — re-sceller les records persistés (ordre canonique) → MÊME racine. ──
    let key = HmacKeySource::test_key().derive(TenantId::system()).expect("clé tenant");
    let rep = replay(out.records(), &key);
    assert!(rep.content_intact, "tous les payload_hash se recalculent");
    assert_eq!(rep.root_hex(), sealed_root, "R0 : la racine se reproduit depuis les records scellés");

    // ── 2bis) l'ordre de transmission est neutre : records mélangés → MÊME racine. ──
    let mut shuffled = out.records().to_vec();
    shuffled.reverse();
    assert_eq!(replay(&shuffled, &key).root_hex(), sealed_root, "désordre de transmission neutre (ADR-007)");

    // ── 3) la chaîne forensique N17 se RE-VÉRIFIE (pas recomputée — I-RP7). ──
    let v = verify_run_offline(
        out.records(),
        &out.llm_records,
        &out.n17_sealed.leaf_hashes,
        out.n17_sealed.n17_root,
        &key,
    );
    assert!(v.fully_proven, "essaim + chaîne N17 + pont re-prouvés hors ligne");
    assert!(v.n17_chain.intact, "I-RP7 : chaîne N17 re-vérifiée");

    // ── 4) honnêteté Mode C : la racine LIE l'identité par-run (swarm_id, record_id) — elle
    //    est re-PROUVÉE par R0, pas REPRODUITE par re-exécution (le provider n'est pas rejoué).
    println!(
        "✅ Mode C réel : R0 re-prouve la racine ({} records, {} forensiques N17) ; \
         re-exécution provider NON tentée (sortie LLM non reproductible bit-à-bit).",
        out.records().len(),
        out.llm_records.len()
    );
}
