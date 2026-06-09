// SPDX-License-Identifier: UNLICENSED
//! # `uniform_provenance` — provenance UNIFORME via `SwarmContext` (§3 ⇄ Phase 4).
//!
//! Un **seul** `SwarmContext::with_write_sink(sink)` instrumente **toute** la couche
//! topologie via l'interface unifiée [`Topology`] : on exécute **T2 Pipeline** ET
//! **T-Consensus** à travers le MÊME contexte ; leurs écritures (`/out/*`, `/cand/*`,
//! `/ballot/*`) sont ancrées par le même `ProvenanceSink`, scellées en une chaîne, et
//! re-prouvées par `verify_swarm` — sans que les topologies connaissent la provenance.
//!
//! Run :  cargo run --example uniform_provenance -p efe-swarm-provenance

use std::sync::Arc;

use async_trait::async_trait;
use efe_blackboard::WriteSink;
use efe_nano_sdk::{AgentId, SwarmId, TenantId};
use efe_swarm_provenance::{
    verify_swarm, HmacKeySource, ProvenanceRecorder, ProvenanceSink, ProvenancedAgent,
};
use efe_swarm_topology::{
    ConsensusTopology, Generator, Judge, Pattern, Pipeline, PipelineTopology, Stage, SwarmContext,
    Topology,
};
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
struct UpStage;
#[async_trait]
impl Stage for UpStage {
    async fn process(&self, input: &Value) -> Vec<Value> {
        vec![json!({ "text": input.get("text").and_then(|v| v.as_str()).unwrap_or("").to_uppercase() })]
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let tenant = TenantId::system();
    let swarm = SwarmId::from_uuid(Uuid::from_u128(0x2222));
    let recorder = Arc::new(ProvenanceRecorder::new(swarm, tenant));
    let sink: Arc<dyn WriteSink> = Arc::new(ProvenanceSink::new(recorder.clone()));

    // UN SEUL contexte instrumenté → toute la couche topologie est tracée.
    let ctx = SwarmContext::standard().with_write_sink(sink);

    println!("══════════════════════════════════════════════════════════════════════════");
    println!(" PROVENANCE UNIFORME — un seul SwarmContext::with_write_sink instrumente §3");
    println!("══════════════════════════════════════════════════════════════════════════");

    // ── T-Consensus (sans hook local → via le contexte). ────────────────────────
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
    let t_cons: Box<dyn Topology> =
        Box::new(ConsensusTopology::new(gen, k, judges).with_pattern(Pattern::Borda));

    // ── T2 Pipeline (son StreamingSwarm interne instrumenté via le contexte). ────
    let t2: Box<dyn Topology> =
        Box::new(PipelineTopology::new(Pipeline::new().stage("upper", Arc::new(UpStage))));

    // boucle UNIFORME sur Box<dyn Topology> — provenance branchée pour toutes.
    let _ = t_cons.run(&json!({ "expected": "tech", "n_correct": 3 }), &ctx).await;
    let _ = t2.run(&json!({ "text": "hello swarm" }), &ctx).await;

    println!("\n exécutées via UNE interface (Topology) + UN contexte instrumenté :");
    println!("   {} · {}", t_cons.name(), t2.name());
    println!("   seam A (sink contexte) : {} écritures ancrées (cand+ballot+out)", recorder.anchored_writes());
    println!("   seam B (décorateurs)   : {} appels captés", recorder.captured_calls());

    let key = HmacKeySource::test_key().derive(tenant).unwrap();
    let sealed = recorder.seal_with_key(key.clone());
    let report = verify_swarm(&sealed.records, &key);
    println!("\n provenance : {} enregistrements · root = {}", sealed.len(), &sealed.root_hex().unwrap()[..16]);
    println!(
        " verify_swarm : ok={} · chaîne={} · contenu={} · DAG={} · tracé={:.0}%",
        report.ok(), report.chain_intact, report.content_intact, report.dag_consistent, 100.0 * report.traced_pct
    );

    println!("\n──────────────────────────────────────────────────────────────────────────");
    println!(" → La provenance est un SERVICE du contexte : un seul branchement instrumente");
    println!("   toute la couche §3 (T2, T-Consensus…). Phase 1→4 soudées, branchées.");
    println!("══════════════════════════════════════════════════════════════════════════");
}
