// SPDX-License-Identifier: UNLICENSED
//! # `replay` — rejeu DÉTERMINISTE clé sur la version, pas l'horloge (Phase 4, Inc.4 · ADR-007).
//!
//! Démontre que le `provenance_root` est **reproductible** :
//! 1. un run produit un root ;
//! 2. les enregistrements **réordonnés** (simulant un autre ordre d'exécution parallèle)
//!    rejouent le **MÊME** root ;
//! 3. décaler tous les `wall_at` ne change **rien** au root (advisory, ADR-007) ;
//! 4. altérer une sortie casse l'intégrité de contenu ;
//! 5. rejeu d'exécution : re-exécuter l'agent **pur** reproduit les `output_hash`.
//!
//! Run :  cargo run --example replay -p efe-swarm-provenance

use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use async_trait::async_trait;
use efe_blackboard::{Blackboard, WriteSink};
use efe_nano_sdk::{AgentId, Blake3Hash, SwarmId, TenantId};
use efe_swarm_provenance::{
    replay, replay_outputs, AgentRole, HmacKeySource, ProvenanceRecorder, ProvenanceSink,
    ProvenancedAgent,
};
use efe_swarm_topology::Generator;
use serde_json::{json, Value};
use uuid::Uuid;

/// Générateur Mode A PUR : sortie = fonction déterministe de (task, variant).
struct PureGen;
#[async_trait]
impl Generator for PureGen {
    async fn generate(&self, task: &Value, variant: usize) -> Value {
        let base = task.get("base").and_then(|v| v.as_i64()).unwrap_or(0);
        json!({ "value": base + variant as i64 })
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let tenant = TenantId::system();
    let swarm = SwarmId::from_uuid(Uuid::from_u128(0x2222));
    let key = HmacKeySource::test_key().derive(tenant).unwrap();

    let recorder = Arc::new(ProvenanceRecorder::new(swarm, tenant));
    let bb = Blackboard::new(swarm);
    bb.set_write_sink(Arc::new(ProvenanceSink::new(recorder.clone())) as Arc<dyn WriteSink>);

    let gen = ProvenancedAgent::mode_a(PureGen, recorder.clone(), AgentId::new("generator").unwrap());

    println!("══════════════════════════════════════════════════════════════════════════");
    println!(" REJEU DÉTERMINISTE — clé sur la version du Blackboard, pas l'horloge (ADR-007)");
    println!("══════════════════════════════════════════════════════════════════════════");

    // ── Run : 5 générations (séquentielles ici ; en réel : JoinSet parallèle). ──
    let task = json!({ "base": 100 });
    for variant in 0..5 {
        let cand = gen.generate(&task, variant).await;
        bb.write(&format!("/cand/{variant}"), cand, AgentId::new("generator").unwrap()).unwrap();
    }
    let sealed = recorder.seal_with_key(key.clone());
    let live_root = sealed.root_hex().unwrap();
    println!("\n run scellé : {} enregistrements", sealed.len());
    println!(" ► provenance_root (vivant) : {live_root}");

    // ── (1+2) Réordonner les enregistrements → MÊME root. ───────────────────────
    let mut shuffled = sealed.records.clone();
    shuffled.reverse();
    shuffled.rotate_left(2);
    let rep = replay(&shuffled, &key);
    println!("\n [rejeu depuis records RÉORDONNÉS (simulateur d'exécution parallèle)]");
    println!("   root rejoué : {}", rep.root_hex().unwrap());
    println!("   == root vivant ? {}", rep.matches(sealed.root));

    // ── (3) Décaler tous les wall_at → root INCHANGÉ (advisory). ────────────────
    let mut shifted = sealed.records.clone();
    for (i, r) in shifted.iter_mut().enumerate() {
        r.wall_at = UNIX_EPOCH + Duration::from_secs(9_000 + i as u64);
    }
    let rep_shifted = replay(&shifted, &key);
    println!("\n [wall_at décalés de plusieurs heures]");
    println!("   root rejoué : {}", rep_shifted.root_hex().unwrap());
    println!("   == root vivant ? {}  ← wall_at est ADVISORY (hors hash)", rep_shifted.matches(sealed.root));

    // ── (4) Altérer une sortie → intégrité de contenu rompue. ───────────────────
    let mut tampered = sealed.records.clone();
    tampered[2].output_hash = Blake3Hash::from_bytes([0xff; 32]);
    let rep_t = replay(&tampered, &key);
    println!("\n [sortie d'un enregistrement substituée]");
    println!("   content_intact={} · rupture à l'index {:?}", rep_t.content_intact, rep_t.first_content_break);

    // ── (5) Rejeu d'EXÉCUTION : re-exécuter l'agent pur reproduit les output_hash. ─
    // On reconstruit les sorties en re-exécutant PureGen (l'entrée task+variant est connue).
    let fresh = PureGen;
    let mut repro: std::collections::HashMap<Uuid, Value> = std::collections::HashMap::new();
    for r in &sealed.records {
        if r.agent_role == AgentRole::Generator {
            if let Some(v) = r.variant {
                repro.insert(r.record_id, fresh.generate(&task, v as usize).await);
            }
        }
    }
    let out_rep = replay_outputs(&sealed.records, |r| repro.get(&r.record_id).cloned().unwrap_or(Value::Null));
    println!("\n [rejeu d'EXÉCUTION — re-dérive les sorties de l'agent pur]");
    println!(
        "   vérifiés={} · reproduits={} · divergents={} · déterministe={}",
        out_rep.checked, out_rep.matched, out_rep.mismatched, out_rep.deterministic()
    );

    println!("\n──────────────────────────────────────────────────────────────────────────");
    println!(" → Le provenance_root ne dépend QUE de l'ensemble d'enregistrements + la clé");
    println!("   tenant — jamais de l'horloge ni de l'ordre d'exécution. Un tiers rejoue");
    println!("   l'essaim et retrouve le même root, sans le provider. ADR-SWARM-007.");
    println!("══════════════════════════════════════════════════════════════════════════");
}
