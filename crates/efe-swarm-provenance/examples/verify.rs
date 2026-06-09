// SPDX-License-Identifier: UNLICENSED
//! # `verify` — le vérificateur INDÉPENDANT en action : le moat (Phase 4).
//!
//! Met en scène un **tiers** (un régulateur, un auditeur) qui reçoit les
//! enregistrements d'un run d'essaim **dans le désordre** + la clé du tenant, et
//! re-prouve **tout** avec [`verify_swarm`] — sans jamais parler au provider :
//! 1. run sain → chaîne + contenu + DAG intacts, `provenance_root` reconstruit ;
//! 2. records reçus dans un ordre arbitraire → même verdict (robuste) ;
//! 3. sortie substituée → contenu rompu ; 4. mauvaise clé → chaîne rompue.
//!
//! Run :  cargo run --example verify -p efe-swarm-provenance

use std::sync::Arc;

use async_trait::async_trait;
use efe_blackboard::{Blackboard, WriteSink};
use efe_nano_sdk::{AgentId, Blake3Hash, SwarmId, TenantId};
use efe_swarm_provenance::{
    verify_swarm, HmacKeySource, ProvenanceRecorder, ProvenanceSink, ProvenancedAgent,
};
use efe_swarm_topology::Generator;
use serde_json::{json, Value};
use uuid::Uuid;

struct SimGen;
#[async_trait]
impl Generator for SimGen {
    async fn generate(&self, _t: &Value, variant: usize) -> Value {
        json!({ "value": format!("cand-{variant}") })
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let tenant = TenantId::system();
    let swarm = SwarmId::from_uuid(Uuid::from_u128(0x2222));
    let recorder = Arc::new(ProvenanceRecorder::new(swarm, tenant));
    let bb = Blackboard::new(swarm);
    bb.set_write_sink(Arc::new(ProvenanceSink::new(recorder.clone())) as Arc<dyn WriteSink>);

    // ── Le PRODUCTEUR scelle un run (7 générateurs). ────────────────────────────
    let gen = ProvenancedAgent::mode_a(SimGen, recorder.clone(), AgentId::new("generator").unwrap());
    for v in 0..7 {
        let c = gen.generate(&json!({}), v).await;
        bb.write(&format!("/cand/{v}"), c, AgentId::new("generator").unwrap()).unwrap();
    }
    let sealed = recorder.seal_with_key(HmacKeySource::test_key().derive(tenant).unwrap());
    println!("══════════════════════════════════════════════════════════════════════════");
    println!(" VÉRIFICATEUR INDÉPENDANT — re-prouver tout sans le provider (le moat)");
    println!("══════════════════════════════════════════════════════════════════════════");
    println!("\n producteur : {} enregistrements · root = {}", sealed.len(), &sealed.root_hex().unwrap()[..16]);

    // ── Le TIERS ne reçoit QUE les records + la clé tenant (serveur). ───────────
    let key = HmacKeySource::test_key().derive(tenant).unwrap();

    // (1) tel quel.
    let r1 = verify_swarm(&sealed.records, &key);
    println!("\n [tiers · records tels quels]");
    print_report(&r1);

    // (2) reçus DANS LE DÉSORDRE (un tiers ne contrôle pas l'ordre de transmission).
    let mut scrambled = sealed.records.clone();
    scrambled.reverse();
    scrambled.rotate_left(3);
    let r2 = verify_swarm(&scrambled, &key);
    println!("\n [tiers · records reçus dans le désordre]");
    print_report(&r2);
    println!("   même root que (1) ? {}", r2.root == r1.root);

    // (3) un attaquant a substitué une sortie.
    let mut tampered = sealed.records.clone();
    tampered[4].output_hash = Blake3Hash::from_bytes([0xff; 32]);
    let r3 = verify_swarm(&tampered, &key);
    println!("\n [tiers · une sortie substituée]");
    print_report(&r3);

    // (4) mauvaise clé (un imposteur sans le secret tenant).
    let r4 = verify_swarm(&sealed.records, &vec![0u8; 32]);
    println!("\n [imposteur · sans la clé tenant]");
    print_report(&r4);

    println!("\n──────────────────────────────────────────────────────────────────────────");
    println!(" → Avec les seuls records + la clé tenant, un tiers re-prouve qui a dit quoi,");
    println!("   quand, à partir de quoi — et reconstruit le provenance_root. Robuste à");
    println!("   l'ordre, sensible à toute altération, indépendant du provider. LE MOAT.");
    println!("══════════════════════════════════════════════════════════════════════════");
}

fn print_report(r: &efe_swarm_provenance::SwarmProvenanceReport) {
    println!(
        "   ok={} · chaîne={} · contenu={} · DAG={} · tracé={:.0}% · root={}",
        r.ok(),
        r.chain_intact,
        r.content_intact,
        r.dag_consistent,
        100.0 * r.traced_pct,
        r.root_hex().map(|h| h[..16].to_string()).unwrap_or_else(|| "—".into()),
    );
    if let Some(id) = r.first_break {
        println!("   ⚠ 1ʳᵉ rupture : record {}", &id.to_string()[..8]);
    }
}
