// SPDX-License-Identifier: UNLICENSED
//! # `chain` — la chaîne HMAC par-swarm, en action (Phase 4, Inc.2).
//!
//! Enfile les atomes d'un mini-run d'essaim (1 générateur + 3 juges) dans une
//! [`SwarmChain`] et montre que **le `provenance_root` (un seul hash) prouve tout le
//! run** : altérer N'IMPORTE quelle entrée fait diverger la racine recalculée, et
//! [`verify_chain`] pointe l'entrée fautive — **sans le provider, sans la clé**
//! (la clé reste le secret du tenant ; le vérificateur n'a besoin que de la clé
//! tenant côté serveur).
//!
//! Run :  cargo run --example chain -p efe-swarm-provenance

use efe_nano_sdk::{AgentId, ProvenanceId, SwarmId, TenantId};
use efe_swarm_provenance::{
    verify_chain, AgentCallDraft, AgentKind, AgentRole, HmacKeySource, SwarmChain,
};
use serde_json::json;
use uuid::Uuid;

fn main() {
    let tenant = TenantId::system();
    let swarm = SwarmId::from_uuid(Uuid::from_u128(0x2222));

    // Clé dérivée par tenant (HKDF) — en prod : master serveur ; ici : test_key.
    let mut chain = SwarmChain::from_key_source(tenant, swarm, &HmacKeySource::test_key())
        .expect("dérivation clé");

    println!("══════════════════════════════════════════════════════════════════════════");
    println!(" CHAÎNE HMAC PAR-SWARM — genèse par swarm_id, tip = provenance_root");
    println!("══════════════════════════════════════════════════════════════════════════");

    // ── Un mini-run : 1 générateur Mode A produit /cand/0 ; 3 juges Mode C votent. ─
    let gen = chain.append(
        AgentCallDraft::from_io(
            swarm,
            tenant,
            AgentId::new("generator").unwrap(),
            AgentRole::Generator,
            AgentKind::ModeA,
            &json!({ "task": "classify", "difficulty": 0.7 }),
            &json!({ "value": "tech" }),
        )
        .with_variant(0)
        .wrote("/cand/0", 1),
    );

    let mut records = vec![gen.clone()];
    for j in 0..3 {
        let judge = chain.append(
            AgentCallDraft::from_io(
                swarm,
                tenant,
                AgentId::new(format!("judge_{j:03}")).unwrap(),
                AgentRole::Judge,
                AgentKind::ModeC,
                &json!({ "candidates": [{ "value": "tech" }] }),
                &json!({ "scores": [1.0] }),
            )
            .read("/cand/0", 1)
            .caused_by(gen.record_id)
            .wrote(format!("/ballot/{j}"), 2 + j as u64)
            .with_llm_provenance(ProvenanceId::from_uuid(Uuid::from_u128(0xBEEF + j as u128))),
        );
        records.push(judge);
    }

    println!("\n run : {} agents enfilés (seq 0..{})", chain.len(), chain.len() - 1);
    for r in &records {
        println!(
            "   seq {} · {:<11} · {:<10} · payload {}… · hmac {}…",
            r.seq,
            r.agent_id,
            r.agent_role.as_str(),
            &r.payload_hex()[..12],
            &r.hmac_hex()[..12],
        );
    }
    let root = chain.root_hex().unwrap();
    println!("\n ► provenance_root (UN seul hash qui prouve tout le run) :\n   {root}");

    // ── Vérification indépendante (clé tenant uniquement). ──────────────────────
    let v = verify_chain(&records, &HmacKeySource::test_key().derive(tenant).unwrap());
    println!("\n [vérification — chaîne saine]");
    println!("   ok={} · vérifiés={} · root==tip : {}", v.ok, v.checked, v.root.map(|r| efe_swarm_provenance::hex32(&r)) == Some(root.clone()));

    // ── Altération : on substitue la sortie du juge #1 (seq 2) après coup. ───────
    let mut tampered = records.clone();
    tampered[2].output_hash = efe_nano_sdk::Blake3Hash::from_bytes([0xff; 32]);
    let vt = verify_chain(&tampered, &HmacKeySource::test_key().derive(tenant).unwrap());
    println!("\n [vérification — sortie du juge #1 (seq 2) altérée]");
    match vt.first_break {
        Some(b) => println!("   ok={} · rupture à l'index {} (seq {}) : {:?}", vt.ok, b.index, b.seq, b.reason),
        None => println!("   ok={}", vt.ok),
    }

    println!("\n──────────────────────────────────────────────────────────────────────────");
    println!(" → Le provenance_root commet, par transitivité, chaque payload_hash et chaque");
    println!("   lien. Muter/supprimer/réordonner une entrée casse la chaîne — détecté par un");
    println!("   tiers avec la seule clé tenant. Inc.3 capturera ces atomes automatiquement");
    println!("   (décorateur + WriteSink) ; Inc.4 ajoutera couverture 100 % + rejeu.");
    println!("══════════════════════════════════════════════════════════════════════════");
}
