// SPDX-License-Identifier: UNLICENSED
//! # `atom` — l'atome de provenance par-agent, en action (Phase 4, Inc.1).
//!
//! Montre qu'un agent **Mode A** (Rust pur, jusqu'ici MUET) produit désormais un
//! enregistrement « qui a dit quoi, quand, à partir de quoi » — hashé, content-
//! addressable, et **vérifiable seul**. Puis démontre la **tamper-evidence** de
//! l'atome : altérer la sortie après coup casse `verify_payload_hash`.
//!
//! Run :  cargo run --example atom -p efe-swarm-provenance

use efe_nano_sdk::{AgentId, ProvenanceId, SwarmId, TenantId};
use efe_swarm_provenance::{AgentCallDraft, AgentKind, AgentRole};
use serde_json::json;
use uuid::Uuid;

fn main() {
    let swarm = SwarmId::from_uuid(Uuid::from_u128(0x2222));
    let tenant = TenantId::system();

    println!("══════════════════════════════════════════════════════════════════════════");
    println!(" ATOME DE PROVENANCE PAR-AGENT — « qui a dit quoi, quand, à partir de quoi »");
    println!("══════════════════════════════════════════════════════════════════════════");

    // ── 1. Un GÉNÉRATEUR Mode A (jusqu'ici muet) produit le candidat 3. ──────────
    let gen = AgentCallDraft::from_io(
        swarm,
        tenant,
        AgentId::new("generator").unwrap(),
        AgentRole::Generator,
        AgentKind::ModeA,
        &json!({ "task": "classify", "difficulty": 0.7 }), // à partir de quoi
        &json!({ "value": "tech" }),                        // quoi
    )
    .with_variant(3)
    .wrote("/cand/3", 3)
    .seal(0, None, [0u8; 32]); // genèse de la chaîne (hmac réel posé en Inc.2)

    println!("\n [generator · Mode A]");
    println!("   qui          : {} ({:?}, {:?})", gen.agent_id, gen.agent_role, gen.agent_kind);
    println!("   à partir de  : input_hash={}", gen.input_hash);
    println!("   quoi         : output_hash={}  writes={:?}", gen.output_hash, gen.writes);
    println!("   quand        : seq={}", gen.seq);
    println!("   payload_hash : {}", gen.payload_hex());
    println!("   re-prouvable : {}", gen.verify_payload_hash());

    // ── 2. Un JUGE Mode C lit /cand/3 et produit un ballot — lié à un appel LLM. ─
    let judge = AgentCallDraft::from_io(
        swarm,
        tenant,
        AgentId::new("judge_002").unwrap(),
        AgentRole::Judge,
        AgentKind::ModeC,
        &json!({ "candidates": [{ "value": "tech" }] }),
        &json!({ "scores": [1.0] }),
    )
    .read("/cand/3", 3)
    .caused_by(gen.record_id) // le ballot DÉPEND du candidat du générateur
    .wrote("/ballot/2", 4)
    .with_llm_provenance(ProvenanceId::from_uuid(Uuid::from_u128(0xBEEF)))
    .seal(1, Some(gen.hmac), [0u8; 32]);

    println!("\n [judge_002 · Mode C]");
    println!("   à partir de  : reads={:?}  causal_parents={:?}", judge.reads, judge.causal_parents);
    println!("   pont N17     : llm_provenance_id={:?}", judge.llm_provenance_id.map(|p| p.as_uuid()));
    println!("   payload_hash : {}", judge.payload_hex());
    println!("   re-prouvable : {}", judge.verify_payload_hash());

    // ── 3. Tamper-evidence : on substitue la sortie du juge APRÈS scellement. ────
    let mut forged = judge.clone();
    forged.output_hash = efe_nano_sdk::Blake3Hash::from_bytes([0xff; 32]);
    println!("\n [altération de la sortie du juge après coup]");
    println!("   verify_payload_hash (sain)  : {}", judge.verify_payload_hash());
    println!("   verify_payload_hash (forgé) : {}  ← détecté", forged.verify_payload_hash());

    // ── 4. Sérialisation (forme hex, cross-vérifiable). ─────────────────────────
    let wire = serde_json::to_string_pretty(&gen).unwrap();
    println!("\n [enregistrement sérialisé — extrait]");
    for line in wire.lines().take(6) {
        println!("   {line}");
    }
    println!("   …");

    println!("\n──────────────────────────────────────────────────────────────────────────");
    println!(" → Les agents Mode A ne sont plus muets : chaque appel devient un atome");
    println!("   hashé, causalement relié, et re-prouvable. Inc.2 chaînera ces atomes");
    println!("   (HMAC par-swarm) ; Inc.4 les rejouera et les vérifiera sans le provider.");
    println!("══════════════════════════════════════════════════════════════════════════");
}
