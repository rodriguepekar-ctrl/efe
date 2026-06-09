// SPDX-License-Identifier: UNLICENSED
//! # `bridge` — le pont Mode A ↔ Mode C (`llm_provenance_id` → N17) en action (Phase 4).
//!
//! Un essaim mixte : générateurs **Mode A** (Rust pur, aucun lien N17 attendu) +
//! juges **Mode C** (LLM) qui exposent leur `ProvenanceId` N17. Le décorateur **pose**
//! le lien (`with_llm_link`) ; [`verify_bridge`] le **suit** et le **vérifie** contre
//! un store N17 (ici un [`MapResolver`] de vues) : existe, même tenant, même swarm.
//!
//! Run :  cargo run --example bridge -p efe-swarm-provenance

use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use efe_blackboard::{Blackboard, WriteSink};
use efe_nano_sdk::{AgentId, Blake3Hash, ProvenanceId, SwarmId, TenantId};
use efe_swarm_provenance::{
    verify_bridge, LinkStatus, LlmProvenanceView, MapResolver, ProvenanceRecorder, ProvenanceSink,
    ProvenancedAgent,
};
use efe_swarm_topology::{Generator, Judge};
use serde_json::{json, Value};
use uuid::Uuid;

/// Générateur **Mode A** pur.
struct SimGen;
#[async_trait]
impl Generator for SimGen {
    async fn generate(&self, task: &Value, variant: usize) -> Value {
        let _ = task;
        json!({ "value": format!("cand-{variant}") })
    }
}

/// Juge **Mode C** : à chaque appel, il « fait un appel LLM » qui produit un
/// `ProvenanceId` N17, qu'il (1) range dans un store partagé et (2) expose via une
/// cellule partagée que le décorateur lira.
struct LlmJudge {
    name: String,
    swarm_req: Uuid,
    tenant: TenantId,
    store: Arc<Mutex<Vec<LlmProvenanceView>>>,
    last: Arc<Mutex<Option<ProvenanceId>>>,
}
#[async_trait]
impl Judge for LlmJudge {
    async fn assess(&self, _task: &Value, candidates: &[Value]) -> Vec<f64> {
        // simulate l'appel LLM → un record N17 avec un ProvenanceId frais.
        let prov = ProvenanceId::new();
        self.store.lock().unwrap().push(LlmProvenanceView {
            provenance_id: prov,
            tenant_id: self.tenant,
            swarm_request_id: self.swarm_req,
            output_hash: Some(Blake3Hash::from_bytes([7u8; 32])),
        });
        *self.last.lock().unwrap() = Some(prov); // surface l'id pour le décorateur
        candidates.iter().map(|_| 1.0).collect()
    }
    fn name(&self) -> &str {
        &self.name
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let tenant = TenantId::system();
    let swarm = SwarmId::from_uuid(Uuid::from_u128(0x2222));
    let recorder = Arc::new(ProvenanceRecorder::new(swarm, tenant));
    let bb = Blackboard::new(swarm);
    bb.set_write_sink(Arc::new(ProvenanceSink::new(recorder.clone())) as Arc<dyn WriteSink>);

    // store N17 partagé (alimenté par les juges « LLM »).
    let store: Arc<Mutex<Vec<LlmProvenanceView>>> = Arc::new(Mutex::new(Vec::new()));

    println!("══════════════════════════════════════════════════════════════════════════");
    println!(" PONT MODE A ↔ MODE C — llm_provenance_id → N17");
    println!("══════════════════════════════════════════════════════════════════════════");

    // ── Mode A : 3 générateurs (aucun lien attendu). ────────────────────────────
    let gen = ProvenancedAgent::mode_a(SimGen, recorder.clone(), AgentId::new("generator").unwrap());
    let mut candidates = Vec::new();
    for v in 0..3 {
        let c = gen.generate(&json!({}), v).await;
        bb.write(&format!("/cand/{v}"), c.clone(), AgentId::new("generator").unwrap()).unwrap();
        candidates.push(c);
    }

    // ── Mode C : 2 juges LLM qui exposent leur ProvenanceId → le décorateur l'attache. ─
    for j in 0..2 {
        let last = Arc::new(Mutex::new(None));
        let judge = ProvenancedAgent::mode_c(
            LlmJudge {
                name: format!("judge_{j:03}"),
                swarm_req: swarm.as_uuid(),
                tenant,
                store: store.clone(),
                last: last.clone(),
            },
            recorder.clone(),
            AgentId::new(format!("judge_{j:03}")).unwrap(),
        )
        // PONT : l'extracteur lit la cellule partagée remplie par l'appel LLM.
        .with_llm_link(move |_out| *last.lock().unwrap());
        let scores = judge.assess(&json!({}), &candidates).await;
        bb.write(&format!("/ballot/{j}"), json!({ "scores": scores }), AgentId::new(format!("judge_{j:03}")).unwrap()).unwrap();
    }

    // ── Scelle, puis vérifie le pont contre le store N17. ───────────────────────
    let sealed = recorder.seal_with_key(efe_swarm_provenance::HmacKeySource::test_key().derive(tenant).unwrap());
    let resolver = {
        let mut m = MapResolver::new();
        for v in store.lock().unwrap().iter() {
            m.insert(v.clone());
        }
        m
    };
    let report = verify_bridge(&sealed.records, &resolver);

    println!("\n {} enregistrements · résolveur N17 = {} vues", sealed.len(), resolver.len());
    for link in &report.links {
        let badge = match link.status {
            LinkStatus::ModeA => "Mode A (aucun lien)",
            LinkStatus::Consistent => "Mode C → N17 ✓ cohérent",
            LinkStatus::Unlinked => "Mode C ⚠ lacune",
            _ => "rompu",
        };
        println!("   {:<11} · {}", link.agent_id, badge);
    }
    println!(
        "\n bilan : modeA={} · liés cohérents={} · lacunes={} · rompus={}",
        report.mode_a, report.consistent, report.unlinked, report.broken
    );
    println!(" tous les Mode C liés cohérents ? {}", report.all_mode_c_consistent());

    println!("\n──────────────────────────────────────────────────────────────────────────");
    println!(" → Une seule lignée, deux granularités : les agents Mode A sont autonomes,");
    println!("   les Mode C référencent leur appel LLM (N17). Un tiers vérifie que chaque");
    println!("   lien existe, dans le bon tenant et le bon swarm — sans le provider.");
    println!("══════════════════════════════════════════════════════════════════════════");
}
