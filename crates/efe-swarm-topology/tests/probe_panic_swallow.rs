// PROBE TEMPORAIRE — vérifie empiriquement l'affirmation de la revue ADR-014 :
// un générateur paniquant produit des sentinelles Null que la règle d'arrêt
// consomme comme de vrais candidats (agreement=1.0 → StopReason::Target à k_min).
// À SUPPRIMER après vérification.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use efe_swarm_topology::sizing::{run_waves_pure, KPlan, StopRule};
use efe_swarm_topology::topology::{SwarmContext, Topology};
use efe_swarm_topology::{ConsensusTopology, Generator};

/// Générateur qui panique TOUJOURS — simule un bug utilisateur déterministe
/// (ex. unwrap d'un champ absent de la tâche).
struct AlwaysPanics;
#[async_trait]
impl Generator for AlwaysPanics {
    async fn generate(&self, _task: &Value, _variant: usize) -> Value {
        panic!("bug utilisateur : champ manquant");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn claim_step1_all_panic_wave_stops_at_kmin_with_target_and_all_nulls() {
    let gen: Arc<dyn Generator> = Arc::new(AlwaysPanics);
    let plan = KPlan { k_min: 4, k_max: 16, batch: 4, stop: StopRule::Agreement { threshold: 0.9 } };
    let (cands, trace) = run_waves_pure(&gen, &json!({}), &plan, 8, None).await;

    eprintln!("PROBE1: k_used={} stop={:?} cands={:?}", trace.k_used, trace.stop_reason, cands);
    // L'affirmation : arrêt au plancher, raison Target, échantillon 100 % Null,
    // observable agreement = 1.0.
    assert_eq!(cands.len(), 4, "arrêt à k_min");
    assert!(cands.iter().all(|c| c.is_null()), "échantillon 100% Null");
    assert_eq!(format!("{:?}", trace.stop_reason), "Target", "StopReason::Target via Null==Null");
    assert!((trace.batches[0].observed - 1.0).abs() < 1e-9, "agreement=1.0 sur des Null");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn claim_step2_full_topology_elects_null_with_full_confidence() {
    let topo = ConsensusTopology::new(Arc::new(AlwaysPanics), 16, vec![])
        .with_k_plan(KPlan { k_min: 4, k_max: 16, batch: 4, stop: StopRule::Agreement { threshold: 0.9 } });
    let out = topo.run(&json!({}), &SwarmContext::standard()).await;

    eprintln!("PROBE2: output={}", out.output);
    // L'affirmation : MajorityVote élit Null, sizing dit target@k_min.
    assert_eq!(out.output["value"], Value::Null, "le gagnant élu est Null");
    assert_eq!(out.output["sizing"]["k_used"], json!(4));
    assert_eq!(out.output["sizing"]["stop_reason"], json!("target"));
}
