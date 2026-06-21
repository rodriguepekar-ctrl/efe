// RÉGRESSION B0.2 (I-SENT) — anciennement une PROBE TEMPORAIRE qui DOCUMENTAIT le défaut :
// un générateur paniquant produit des sentinelles Null que la règle d'arrêt consommait comme de
// vrais candidats (agreement=1.0 → StopReason::Target à k_min — une FAUSSE économie). Le fix sentinel
// (un candidat ABSENT n'agrée jamais) corrige ce défaut ; ce fichier ASSERTE désormais le comportement
// CORRIGÉ et garde la régression au plus bas niveau (`run_waves_pure`), là où l'e2e SDK ne descend pas.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use efe_swarm_topology::sizing::{run_waves_pure, KPlan, StopRule};
use efe_swarm_topology::topology::{SwarmContext, Topology};
use efe_swarm_topology::{ConsensusTopology, Generator};

/// Générateur qui panique TOUJOURS — simule un bug utilisateur déterministe
/// (ex. unwrap d'un champ absent de la tâche) → sentinelle Null capturée par le harnais.
struct AlwaysPanics;
#[async_trait]
impl Generator for AlwaysPanics {
    async fn generate(&self, _task: &Value, _variant: usize) -> Value {
        panic!("bug utilisateur : champ manquant");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b0_2_all_panic_wave_does_not_falsely_converge_on_nulls() {
    // AVANT le fix : sentinelles Null toutes identiques → agreement=1.0 → arrêt au plancher k_min
    // (StopReason::Target), une fausse économie. APRÈS : un absent (Null) n'agrée jamais → agreement=0
    // → le sizer NE s'arrête PAS au plancher ; il monte jusqu'à k_max faute de vraie convergence.
    let gen: Arc<dyn Generator> = Arc::new(AlwaysPanics);
    let plan = KPlan { k_min: 4, k_max: 16, batch: 4, stop: StopRule::Agreement { threshold: 0.9 } };
    let (cands, trace) = run_waves_pure(&gen, &json!({}), &plan, 8, None, None).await;

    assert!(cands.iter().all(|c| c.is_null()), "échantillon 100% Null (essaim brisé)");
    assert_eq!(cands.len(), 16, "monte jusqu'à k_max — PLUS d'arrêt prématuré au plancher");
    assert!(trace.k_used > 4, "PAS d'arrêt au plancher k_min : les Null n'agréent plus (k_used={})", trace.k_used);
    assert!((trace.batches[0].observed - 0.0).abs() < 1e-9, "agreement=0 sur des Null (et NON 1.0)");
    assert_ne!(format!("{:?}", trace.stop_reason), "Target", "l'arrêt n'est plus 'Target' (fausse unanimité)");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b0_2_full_topology_does_not_economize_on_all_panic() {
    // Bout-en-bout topologie : un essaim tout-paniquant n'élit plus Null « target@k_min ». Le gagnant
    // reste Null (l'essaim est brisé), mais le sizer a épuisé k_max au lieu de déclarer une fausse
    // économie au plancher — la dégradation est désormais visible, pas déguisée en convergence.
    let topo = ConsensusTopology::new(Arc::new(AlwaysPanics), 16, vec![])
        .with_k_plan(KPlan { k_min: 4, k_max: 16, batch: 4, stop: StopRule::Agreement { threshold: 0.9 } });
    let out = topo.run(&json!({}), &SwarmContext::standard()).await;

    assert_eq!(out.output["value"], Value::Null, "le gagnant reste Null (essaim brisé)");
    assert_eq!(out.output["sizing"]["k_used"], json!(16), "monte à k_max — plus d'arrêt prématuré au plancher");
    assert_ne!(out.output["sizing"]["stop_reason"], json!("target"), "plus de fausse économie 'target'");
}
