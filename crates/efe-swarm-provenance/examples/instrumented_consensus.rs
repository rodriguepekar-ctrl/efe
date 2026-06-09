// SPDX-License-Identifier: UNLICENSED
//! # `instrumented_consensus` — la vraie ConsensusTopology, tracée à 100 % (Phase 4 §14.2).
//!
//! Exécute la **VRAIE** `ConsensusTopology` (Phase 3) entièrement instrumentée par les
//! 2 seams (décorateur + sink branché via `with_write_sink`), scelle la provenance et la
//! re-prouve avec `verify_swarm`. Aucun agent n'est réécrit ; la topologie ignore tout
//! de la provenance.
//!
//! Run :  cargo run --example instrumented_consensus -p efe-swarm-provenance

use std::sync::Arc;

use async_trait::async_trait;
use efe_blackboard::WriteSink;
use efe_nano_sdk::{AgentId, SwarmId, TenantId};
use efe_swarm_provenance::{
    verify_swarm, HmacKeySource, ProvenanceRecorder, ProvenanceSink, ProvenancedAgent,
};
use efe_swarm_topology::{ConsensusTopology, Generator, Judge, Pattern, SwarmContext, Topology};
use serde_json::{json, Value};
use uuid::Uuid;

struct SimGen;
#[async_trait]
impl Generator for SimGen {
    async fn generate(&self, task: &Value, variant: usize) -> Value {
        let n = task.get("n_correct").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        if variant < n {
            json!({ "value": task.get("expected").cloned().unwrap_or(Value::Null) })
        } else {
            json!({ "value": "__lure__" })
        }
    }
}

struct EqJudge(String);
#[async_trait]
impl Judge for EqJudge {
    async fn assess(&self, task: &Value, candidates: &[Value]) -> Vec<f64> {
        let exp = task.get("expected").cloned().unwrap_or(Value::Null);
        candidates.iter().map(|c| if c.get("value") == Some(&exp) { 1.0 } else { 0.0 }).collect()
    }
    fn name(&self) -> &str {
        &self.0
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let tenant = TenantId::system();
    let swarm = SwarmId::from_uuid(Uuid::from_u128(0x2222));
    let recorder = Arc::new(ProvenanceRecorder::new(swarm, tenant));
    let sink: Arc<dyn WriteSink> = Arc::new(ProvenanceSink::new(recorder.clone()));

    let (k, jn) = (7usize, 5usize);
    let gen: Arc<dyn Generator> =
        Arc::new(ProvenancedAgent::mode_a(SimGen, recorder.clone(), AgentId::new("generator").unwrap()));
    let judges: Vec<Arc<dyn Judge>> = (0..jn)
        .map(|j| {
            Arc::new(ProvenancedAgent::mode_c(
                EqJudge(format!("judge_{j:03}")),
                recorder.clone(),
                AgentId::new(format!("judge_{j:03}")).unwrap(),
            )) as Arc<dyn Judge>
        })
        .collect();

    let topo = ConsensusTopology::new(gen, k, judges).with_pattern(Pattern::Borda).with_write_sink(sink);

    println!("══════════════════════════════════════════════════════════════════════════");
    println!(" CONSENSUS INSTRUMENTÉ — la vraie ConsensusTopology, tracée à 100 % (§14.2)");
    println!("══════════════════════════════════════════════════════════════════════════");

    let task = json!({ "expected": "tech", "n_correct": 3, "difficulty": 0.7 });
    let out = topo.run(&task, &SwarmContext::standard()).await;
    println!("\n {} · sortie consensus = {}", topo.name(), out.output);
    println!(" seam B : {} appels captés · seam A : {} écritures ancrées", recorder.captured_calls(), recorder.anchored_writes());

    let key = HmacKeySource::test_key().derive(tenant).unwrap();
    let sealed = recorder.seal_with_key(key.clone());
    let report = verify_swarm(&sealed.records, &key);

    println!("\n provenance : {} enregistrements · root = {}", sealed.len(), &sealed.root_hex().unwrap()[..16]);
    println!(
        " verify_swarm : ok={} · chaîne={} · contenu={} · DAG={} · tracé={:.0}%",
        report.ok(), report.chain_intact, report.content_intact, report.dag_consistent, 100.0 * report.traced_pct
    );

    println!("\n──────────────────────────────────────────────────────────────────────────");
    println!(" → La topologie consensus de Phase 3 tourne, ses agents Mode A/C sont");
    println!("   intégralement tracés (100 %), et un tiers re-prouve le run — sans que");
    println!("   ConsensusTopology connaisse la provenance. Phase 3 ⇄ Phase 4 soudées.");
    println!("══════════════════════════════════════════════════════════════════════════");
}
