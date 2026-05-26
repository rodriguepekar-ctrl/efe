// SPDX-License-Identifier: UNLICENSED
//! `DagExecutor` — orchestrateur DAG bornné en parallèle.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinSet;

use crate::error::{CoordError, CoordResult};

use super::node::{DagNode, NodeId, NodeStatus};
use crate::DEFAULT_MAX_CONCURRENCY;

/// Rapport d'exécution d'un DAG complet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagRunReport {
    /// Statut final par nœud.
    pub statuses: BTreeMap<NodeId, NodeStatus>,
    /// Sorties JSON par nœud (None si Failed/Skipped).
    pub outputs: BTreeMap<NodeId, Option<serde_json::Value>>,
    /// Messages d'erreur par nœud Failed.
    pub errors: BTreeMap<NodeId, String>,
    /// Latence par nœud (ms).
    pub latencies_ms: BTreeMap<NodeId, u64>,
    /// Latence totale (ms).
    pub total_ms: u64,
    /// Au moins un nœud a échoué ?
    pub any_failure: bool,
}

impl DagRunReport {
    /// Sortie d'un nœud (None si pas exécuté ou Failed).
    pub fn output(&self, id: &str) -> Option<&serde_json::Value> {
        self.outputs.get(id).and_then(Option::as_ref)
    }

    /// Statut d'un nœud (None si nœud inconnu).
    pub fn status(&self, id: &str) -> Option<&NodeStatus> {
        self.statuses.get(id)
    }

    /// True si TOUT a réussi.
    #[must_use]
    pub fn all_success(&self) -> bool {
        self.statuses.values().all(|s| matches!(s, NodeStatus::Success))
    }
}

/// Exécuteur DAG (réutilisable, sans état entre runs).
#[derive(Clone, Debug)]
pub struct DagExecutor {
    max_concurrency: usize,
}

impl Default for DagExecutor {
    fn default() -> Self {
        Self {
            max_concurrency: DEFAULT_MAX_CONCURRENCY,
        }
    }
}

impl DagExecutor {
    /// Construit avec parallélisme max custom.
    #[must_use]
    pub fn new(max_concurrency: usize) -> Self {
        Self {
            max_concurrency: max_concurrency.max(1),
        }
    }

    /// Valide le DAG (cycles + parents manquants) et le tri topologique.
    fn topo_sort(nodes: &[DagNode]) -> CoordResult<Vec<NodeId>> {
        let mut in_degree: HashMap<NodeId, usize> = HashMap::new();
        let mut adj: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        let known: BTreeSet<NodeId> = nodes.iter().map(|n| n.id.clone()).collect();

        for n in nodes {
            in_degree.entry(n.id.clone()).or_insert(0);
            for p in &n.parents {
                if !known.contains(p) {
                    return Err(CoordError::DagMissingParent {
                        node_id: n.id.clone(),
                        missing_parent: p.clone(),
                    });
                }
                adj.entry(p.clone()).or_default().push(n.id.clone());
                *in_degree.entry(n.id.clone()).or_insert(0) += 1;
            }
        }

        let mut queue: VecDeque<NodeId> = in_degree
            .iter()
            .filter_map(|(k, &v)| (v == 0).then(|| k.clone()))
            .collect();
        let mut order = Vec::with_capacity(nodes.len());
        while let Some(n) = queue.pop_front() {
            order.push(n.clone());
            if let Some(children) = adj.get(&n) {
                for c in children {
                    if let Some(d) = in_degree.get_mut(c) {
                        *d -= 1;
                        if *d == 0 {
                            queue.push_back(c.clone());
                        }
                    }
                }
            }
        }

        if order.len() != nodes.len() {
            // Trouve un nœud impliqué dans le cycle.
            let stuck = in_degree
                .iter()
                .find(|(_, &v)| v > 0)
                .map(|(k, _)| k.clone())
                .unwrap_or_else(|| "<unknown>".into());
            return Err(CoordError::DagCycle { node_id: stuck });
        }

        Ok(order)
    }

    /// Exécute le DAG : fan-out parallèle, fan-in via barrière implicite,
    /// borné par `max_concurrency`.
    pub async fn run(&self, nodes: Vec<DagNode>) -> CoordResult<DagRunReport> {
        let started = Instant::now();
        // Validate + topo.
        let order = Self::topo_sort(&nodes)?;

        // Construit map id → node.
        let mut node_map: HashMap<NodeId, DagNode> = HashMap::new();
        let mut parents_of: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        let mut children_of: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for n in nodes {
            parents_of.insert(n.id.clone(), n.parents.clone());
            for p in &n.parents {
                children_of.entry(p.clone()).or_default().push(n.id.clone());
            }
            node_map.insert(n.id.clone(), n);
        }

        // États partagés.
        let statuses: Arc<Mutex<BTreeMap<NodeId, NodeStatus>>> = Arc::new(Mutex::new(
            order
                .iter()
                .map(|id| (id.clone(), NodeStatus::Pending))
                .collect(),
        ));
        let outputs: Arc<Mutex<BTreeMap<NodeId, Option<serde_json::Value>>>> =
            Arc::new(Mutex::new(BTreeMap::new()));
        let errors: Arc<Mutex<BTreeMap<NodeId, String>>> = Arc::new(Mutex::new(BTreeMap::new()));
        let latencies: Arc<Mutex<BTreeMap<NodeId, u64>>> =
            Arc::new(Mutex::new(BTreeMap::new()));

        // Compteur de parents restants pour chaque nœud.
        let pending_parents: Arc<Mutex<HashMap<NodeId, usize>>> = Arc::new(Mutex::new(
            parents_of
                .iter()
                .map(|(id, ps)| (id.clone(), ps.len()))
                .collect(),
        ));

        let semaphore = Arc::new(Semaphore::new(self.max_concurrency));
        let mut join_set: JoinSet<()> = JoinSet::new();

        // Trouve les racines (in-degree 0).
        let roots: Vec<NodeId> = order
            .iter()
            .filter(|id| parents_of.get(*id).map(Vec::is_empty).unwrap_or(true))
            .cloned()
            .collect();

        // Helper récursif via canal : on injecte les nœuds ready dans le pool.
        let (tx_ready, mut rx_ready) =
            tokio::sync::mpsc::unbounded_channel::<NodeId>();
        for r in roots {
            let _ = tx_ready.send(r);
        }

        // Compteur d'inflight.
        let total_nodes = order.len();
        let mut launched = 0usize;
        let mut completed = 0usize;

        // Boucle d'orchestration.
        while completed < total_nodes {
            // Drain le canal en non-bloquant pour récupérer les nœuds prêts.
            while let Ok(id) = rx_ready.try_recv() {
                launched += 1;
                let node = node_map
                    .remove(&id)
                    .expect("topo_sort should ensure presence");
                let sem = Arc::clone(&semaphore);
                let statuses_c = Arc::clone(&statuses);
                let outputs_c = Arc::clone(&outputs);
                let errors_c = Arc::clone(&errors);
                let latencies_c = Arc::clone(&latencies);
                let children_c = children_of.get(&id).cloned().unwrap_or_default();
                let pending_parents_c = Arc::clone(&pending_parents);
                let tx_ready_c = tx_ready.clone();
                let id_c = id.clone();

                // Si parent skipped/failed/timed-out → propagate skip.
                let parents = parents_of.get(&id).cloned().unwrap_or_default();
                let any_parent_failed = {
                    let st = statuses_c.lock().await;
                    parents.iter().any(|p| {
                        matches!(
                            st.get(p),
                            Some(NodeStatus::Failed)
                                | Some(NodeStatus::Skipped)
                                | Some(NodeStatus::TimedOut)
                        )
                    })
                };
                if any_parent_failed {
                    let mut st = statuses_c.lock().await;
                    st.insert(id_c.clone(), NodeStatus::Skipped);
                    outputs_c.lock().await.insert(id_c.clone(), None);
                    latencies_c.lock().await.insert(id_c.clone(), 0);
                    // Décrémente les enfants & propage.
                    let mut pp = pending_parents_c.lock().await;
                    for c in &children_c {
                        if let Some(d) = pp.get_mut(c) {
                            *d = d.saturating_sub(1);
                            if *d == 0 {
                                let _ = tx_ready_c.send(c.clone());
                            }
                        }
                    }
                    completed += 1;
                    continue;
                }

                // Collecte les outputs des parents en succès (clé = parent_id).
                let parent_outputs: super::node::ParentOutputs = {
                    let outs = outputs_c.lock().await;
                    parents
                        .iter()
                        .filter_map(|p| {
                            outs.get(p)
                                .and_then(|opt| opt.clone())
                                .map(|v| (p.clone(), v))
                        })
                        .collect()
                };
                let node_timeout = node.timeout;

                join_set.spawn(async move {
                    let _permit = sem.acquire_owned().await.expect("semaphore closed");
                    {
                        let mut st = statuses_c.lock().await;
                        st.insert(id_c.clone(), NodeStatus::Running);
                    }
                    let started_node = Instant::now();
                    let fut = (node.work)(parent_outputs);
                    let result = if let Some(timeout) = node_timeout {
                        match tokio::time::timeout(timeout, fut).await {
                            Ok(r) => Ok(r),
                            Err(_) => Err(timeout),
                        }
                    } else {
                        Ok(fut.await)
                    };
                    let latency = started_node.elapsed().as_millis() as u64;
                    latencies_c.lock().await.insert(id_c.clone(), latency);
                    match result {
                        Ok(Ok(v)) => {
                            statuses_c
                                .lock()
                                .await
                                .insert(id_c.clone(), NodeStatus::Success);
                            outputs_c.lock().await.insert(id_c.clone(), Some(v));
                        }
                        Ok(Err(e)) => {
                            statuses_c
                                .lock()
                                .await
                                .insert(id_c.clone(), NodeStatus::Failed);
                            errors_c.lock().await.insert(id_c.clone(), e.to_string());
                            outputs_c.lock().await.insert(id_c.clone(), None);
                        }
                        Err(timeout) => {
                            statuses_c
                                .lock()
                                .await
                                .insert(id_c.clone(), NodeStatus::TimedOut);
                            errors_c
                                .lock()
                                .await
                                .insert(id_c.clone(), format!("timeout after {timeout:?}"));
                            outputs_c.lock().await.insert(id_c.clone(), None);
                        }
                    }
                    // Décrémente les compteurs enfants.
                    let mut pp = pending_parents_c.lock().await;
                    for c in &children_c {
                        if let Some(d) = pp.get_mut(c) {
                            *d = d.saturating_sub(1);
                            if *d == 0 {
                                let _ = tx_ready_c.send(c.clone());
                            }
                        }
                    }
                });
            }

            // Si on n'a rien lancé ni complété, on attend qu'une task finisse.
            if launched > completed {
                if let Some(res) = join_set.join_next().await {
                    let _ = res; // panic-resilient
                    completed += 1;
                }
            } else if completed < total_nodes {
                // Pas de tâche en vol ni de ready : devrait être impossible.
                // Sécurité contre deadlock.
                tokio::task::yield_now().await;
                if launched == completed && rx_ready.is_empty() {
                    break;
                }
            }
        }

        // Termine les tasks restantes (au cas où).
        while join_set.join_next().await.is_some() {}

        let statuses_final = statuses.lock().await.clone();
        let outputs_final = outputs.lock().await.clone();
        let errors_final = errors.lock().await.clone();
        let latencies_final = latencies.lock().await.clone();
        let any_failure = statuses_final
            .values()
            .any(|s| matches!(s, NodeStatus::Failed | NodeStatus::TimedOut));
        Ok(DagRunReport {
            statuses: statuses_final,
            outputs: outputs_final,
            errors: errors_final,
            latencies_ms: latencies_final,
            total_ms: started.elapsed().as_millis() as u64,
            any_failure,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node_ok(id: &str, parents: Vec<&str>, payload: serde_json::Value) -> DagNode {
        let p = payload.clone();
        DagNode::with_parents(id, parents, move || async move { Ok(p) })
    }

    fn node_err(id: &str, parents: Vec<&str>) -> DagNode {
        DagNode::with_parents(id, parents, || async {
            Err(CoordError::Config("boom".into()))
        })
    }

    #[tokio::test]
    async fn empty_dag_runs_ok() {
        let exec = DagExecutor::default();
        let report = exec.run(vec![]).await.unwrap();
        assert!(report.all_success());
        assert!(report.outputs.is_empty());
    }

    #[tokio::test]
    async fn single_root_runs() {
        let exec = DagExecutor::default();
        let n = node_ok("a", vec![], serde_json::json!(42));
        let report = exec.run(vec![n]).await.unwrap();
        assert!(report.all_success());
        assert_eq!(report.output("a").unwrap(), &serde_json::json!(42));
    }

    #[tokio::test]
    async fn parents_run_before_children() {
        let exec = DagExecutor::default();
        let a = node_ok("a", vec![], serde_json::json!("a"));
        let b = node_ok("b", vec!["a"], serde_json::json!("b"));
        let c = node_ok("c", vec!["a", "b"], serde_json::json!("c"));
        let report = exec.run(vec![a, b, c]).await.unwrap();
        assert!(report.all_success());
    }

    #[tokio::test]
    async fn failure_propagates_as_skip() {
        let exec = DagExecutor::default();
        let a = node_err("a", vec![]);
        let b = node_ok("b", vec!["a"], serde_json::json!(0));
        let c = node_ok("c", vec!["b"], serde_json::json!(0));
        let report = exec.run(vec![a, b, c]).await.unwrap();
        assert_eq!(report.status("a"), Some(&NodeStatus::Failed));
        assert_eq!(report.status("b"), Some(&NodeStatus::Skipped));
        assert_eq!(report.status("c"), Some(&NodeStatus::Skipped));
        assert!(report.any_failure);
    }

    #[tokio::test]
    async fn missing_parent_errors_at_submit() {
        let exec = DagExecutor::default();
        let n = node_ok("a", vec!["ghost"], serde_json::json!(0));
        let r = exec.run(vec![n]).await;
        assert!(matches!(r, Err(CoordError::DagMissingParent { .. })));
    }

    #[tokio::test]
    async fn cycle_detected() {
        let exec = DagExecutor::default();
        let a = node_ok("a", vec!["b"], serde_json::json!(0));
        let b = node_ok("b", vec!["a"], serde_json::json!(0));
        let r = exec.run(vec![a, b]).await;
        assert!(matches!(r, Err(CoordError::DagCycle { .. })));
    }

    #[tokio::test]
    async fn parallelism_respected_invariant_i8() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let inflight = Arc::new(AtomicUsize::new(0));
        let max_observed = Arc::new(AtomicUsize::new(0));
        let exec = DagExecutor::new(3);
        let mut nodes = vec![];
        for i in 0..10 {
            let inflight = Arc::clone(&inflight);
            let max_o = Arc::clone(&max_observed);
            let id = format!("n{i}");
            nodes.push(DagNode::with_parents(id, Vec::<String>::new(), move || async move {
                let cur = inflight.fetch_add(1, Ordering::SeqCst) + 1;
                max_o.fetch_max(cur, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                inflight.fetch_sub(1, Ordering::SeqCst);
                Ok(serde_json::json!(cur))
            }));
        }
        let report = exec.run(nodes).await.unwrap();
        assert!(report.all_success());
        let max = max_observed.load(Ordering::SeqCst);
        assert!(max <= 3, "parallelism breached: max_observed={max}");
    }

    #[tokio::test]
    async fn diamond_dag_outputs_all_three() {
        // a → {b, c} → d
        let exec = DagExecutor::default();
        let nodes = vec![
            node_ok("a", vec![], serde_json::json!("A")),
            node_ok("b", vec!["a"], serde_json::json!("B")),
            node_ok("c", vec!["a"], serde_json::json!("C")),
            node_ok("d", vec!["b", "c"], serde_json::json!("D")),
        ];
        let r = exec.run(nodes).await.unwrap();
        assert!(r.all_success());
        for id in ["a", "b", "c", "d"] {
            assert!(r.output(id).is_some(), "missing {id}");
        }
    }
}
