// SPDX-License-Identifier: UNLICENSED
//! # Intégration — la **vraie** `ConsensusTopology` instrumentée de bout en bout.
//!
//! Critère §14.2 de l'architecture Phase 4 : « ConsensusTopology instrumentée ;
//! `traced_pct == 100 %` sur l'exemple ». On exécute la VRAIE topologie consensus
//! (Phase 3) avec :
//! - **seam B** : générateur + juges enveloppés par `ProvenancedAgent` (capture entrée→sortie) ;
//! - **seam A** : `ProvenanceSink` branché via `ConsensusTopology::with_write_sink` (ancrage 100 %).
//!
//! Puis on scelle et on **re-prouve** le run avec `verify_swarm` — sans le provider.

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

/// Générateur Mode A : variants 0..n_correct corrects, le reste un leurre.
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

/// Juge Mode C (simulé) : 1.0 si candidat == attendu, sinon 0.0.
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_consensus_topology_is_fully_traced_and_reproven() {
    let tenant = TenantId::system();
    let swarm = SwarmId::from_uuid(Uuid::from_u128(0x2222));
    let recorder = Arc::new(ProvenanceRecorder::new(swarm, tenant));
    let sink: Arc<dyn WriteSink> = Arc::new(ProvenanceSink::new(recorder.clone()));

    let k = 7usize;
    let jn = 5usize;

    // seam B : agents enveloppés (les agent_id MATCHENT ceux qu'écrit ConsensusTopology :
    // "generator" pour /cand/*, "judge_{j:03}" pour /ballot/*).
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

    // seam A : sink branché sur le Blackboard interne via le hook Phase 4.
    let topo = ConsensusTopology::new(gen, k, judges)
        .with_pattern(Pattern::Borda) // force la phase juges
        .with_write_sink(sink);

    let task = json!({ "expected": "tech", "n_correct": 3, "difficulty": 0.7 });
    let out = topo.run(&task, &SwarmContext::standard()).await;

    // la topologie a bien produit un consensus.
    assert_eq!(out.output, json!({ "value": "tech" }), "Borda extrait le correct");

    // les 2 seams ont tout capté : K générations + J évaluations ; K cand + J ballots.
    assert_eq!(recorder.captured_calls(), (k + jn) as u64, "seam B : K gen + J juges");
    assert_eq!(recorder.anchored_writes(), (k + jn) as u64, "seam A : K cand + J ballots");

    // scelle + re-prouve indépendamment.
    let key = HmacKeySource::test_key().derive(tenant).unwrap();
    let sealed = recorder.seal_with_key(key.clone());
    assert_eq!(sealed.len(), k + jn, "{} enregistrements", k + jn);

    let report = verify_swarm(&sealed.records, &key);
    assert!((report.traced_pct - 1.0).abs() < 1e-9, "traced_pct == 100 % (§14.2)");
    assert!(report.chain_intact && report.content_intact && report.dag_consistent, "{report:?}");
    assert!(report.ok(), "run instrumenté entièrement re-prouvé");
    assert!(report.root.is_some(), "provenance_root reconstruit");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn uninstrumented_topology_is_unchanged() {
    // sans sink ni agents enveloppés : la topologie se comporte EXACTEMENT comme avant
    // (la provenance est opt-in, coût nul par défaut).
    let gen: Arc<dyn Generator> = Arc::new(SimGen);
    let judges: Vec<Arc<dyn Judge>> = (0..3)
        .map(|j| Arc::new(EqJudge(format!("j{j}"))) as Arc<dyn Judge>)
        .collect();
    let topo = ConsensusTopology::new(gen, 7, judges).with_pattern(Pattern::Borda);
    let out = topo.run(&json!({ "expected": "tech", "n_correct": 3 }), &SwarmContext::standard()).await;
    assert_eq!(out.output, json!({ "value": "tech" }));
}
