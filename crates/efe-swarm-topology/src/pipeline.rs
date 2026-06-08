// SPDX-License-Identifier: UNLICENSED
//! **T2 — Pipeline streaming** : étages chaînés exécutés sur le substrat, sans
//! barrière de couche, terminaison par quiescence (`efe_blackboard::StreamingSwarm`).
//!
//! Sert à DÉCOMPOSER une tâche dure (hors enveloppe de capacité des petits modèles,
//! cf. F2 de Lot S2) en sous-étapes cheap qui, elles, sont dans l'enveloppe.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use efe_blackboard::{AgentCtx, Blackboard, StreamingSwarm, SwarmEvent, TopicRouter};
use efe_nano_sdk::{AgentId, SwarmId};
use serde_json::Value;

use crate::traits::Stage;

/// **T2** : pipeline d'étages, exécuté en streaming sur un batch d'items.
pub struct Pipeline {
    stages: Vec<(String, Arc<dyn Stage>)>,
    max_concurrency: usize,
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Pipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pipeline")
            .field("stages", &self.stages.iter().map(|(n, _)| n).collect::<Vec<_>>())
            .field("max_concurrency", &self.max_concurrency)
            .finish()
    }
}

impl Pipeline {
    pub fn new() -> Self {
        Self {
            stages: Vec::new(),
            max_concurrency: 16,
        }
    }

    /// Concurrence par étage (appels async en vol). Backpressure pour les SLM lents.
    pub fn with_concurrency(mut self, c: usize) -> Self {
        self.max_concurrency = c.max(1);
        self
    }

    /// Ajoute une étape nommée (chaînage builder).
    pub fn stage(mut self, name: impl Into<String>, stage: Arc<dyn Stage>) -> Self {
        self.stages.push((name.into(), stage));
        self
    }

    pub fn len(&self) -> usize {
        self.stages.len()
    }
    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }

    /// Exécute un BATCH d'items à travers le pipeline streaming. Renvoie tous les
    /// items produits par le DERNIER étage (ordre non garanti — flux concurrent).
    pub async fn run_batch(&self, inputs: Vec<Value>, timeout: Duration) -> Vec<Value> {
        if self.stages.is_empty() {
            return inputs;
        }
        let bb = Arc::new(Blackboard::with_recent_cap(SwarmId::new(), 256_000_000, 4096));
        let cap = (inputs.len() * self.stages.len() * 4).max(1024);
        let router = Arc::new(TopicRouter::new(cap));
        let mut swarm = StreamingSwarm::new(bb.clone(), router);
        let out_seq = Arc::new(AtomicU64::new(0));
        let n = self.stages.len();

        for (i, (name, stage)) in self.stages.iter().enumerate() {
            let in_topic = format!("stage_{i}");
            let is_last = i + 1 == n;
            let out_topic = format!("stage_{}", i + 1);
            let stage = stage.clone();
            let out_seq = out_seq.clone();
            let agent_id = AgentId::new(sanitize(name, i)).expect("agent id valide");
            swarm.async_agent(
                agent_id,
                &in_topic,
                self.max_concurrency,
                Arc::new(move |ctx: Arc<AgentCtx>, ev: SwarmEvent| {
                    let stage = stage.clone();
                    let out_topic = out_topic.clone();
                    let out_seq = out_seq.clone();
                    Box::pin(async move {
                        let outs = stage.process(&ev.payload).await;
                        for o in outs {
                            if is_last {
                                let seq = out_seq.fetch_add(1, Ordering::Relaxed);
                                let _ = ctx.blackboard().write(
                                    &format!("/out/{seq:020}"),
                                    o,
                                    ctx.agent_id().clone(),
                                );
                            } else {
                                ctx.emit(&out_topic, o);
                            }
                        }
                    })
                }),
            );
        }

        for input in inputs {
            swarm.inject("stage_0", input);
        }
        let _ = swarm.run_until_quiescent(timeout).await;

        // Collecte les sorties finales (clés /out/*).
        bb.snapshot()
            .value_map()
            .into_iter()
            .filter(|(k, _)| k.starts_with("/out/"))
            .map(|(_, v)| v)
            .collect()
    }
}

/// Construit un `AgentId` valide (lowercase `[a-z0-9_-]`, 3..=64) depuis un nom d'étage.
fn sanitize(name: &str, idx: usize) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let mut s = format!("stage_{idx}_{cleaned}");
    s.truncate(64);
    if s.len() < 3 {
        s.push_str("_st");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::json;

    struct Double;
    #[async_trait]
    impl Stage for Double {
        async fn process(&self, input: &Value) -> Vec<Value> {
            let v = input.get("v").and_then(|x| x.as_i64()).unwrap_or(0);
            vec![json!({ "v": v * 2 })]
        }
    }
    struct AddOne;
    #[async_trait]
    impl Stage for AddOne {
        async fn process(&self, input: &Value) -> Vec<Value> {
            let v = input.get("v").and_then(|x| x.as_i64()).unwrap_or(0);
            vec![json!({ "v": v + 1 })]
        }
    }
    struct FanOut2;
    #[async_trait]
    impl Stage for FanOut2 {
        async fn process(&self, input: &Value) -> Vec<Value> {
            let v = input.get("v").and_then(|x| x.as_i64()).unwrap_or(0);
            vec![json!({ "v": v }), json!({ "v": v })]
        }
    }
    struct DropOdd;
    #[async_trait]
    impl Stage for DropOdd {
        async fn process(&self, input: &Value) -> Vec<Value> {
            let v = input.get("v").and_then(|x| x.as_i64()).unwrap_or(0);
            if v % 2 == 0 {
                vec![input.clone()]
            } else {
                vec![] // filtre 1→0
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn t2_transforms_batch_through_stages() {
        let pipe = Pipeline::new()
            .stage("double", Arc::new(Double))
            .stage("addone", Arc::new(AddOne));
        let outs = pipe
            .run_batch(vec![json!({"v":1}), json!({"v":2}), json!({"v":3})], Duration::from_secs(5))
            .await;
        let mut vals: Vec<i64> = outs.iter().map(|o| o["v"].as_i64().unwrap()).collect();
        vals.sort_unstable();
        assert_eq!(vals, vec![3, 5, 7]); // (v*2)+1
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn t2_supports_stage_fanout() {
        let pipe = Pipeline::new()
            .stage("fan", Arc::new(FanOut2))
            .stage("addone", Arc::new(AddOne));
        let outs = pipe
            .run_batch(vec![json!({"v":10}), json!({"v":20})], Duration::from_secs(5))
            .await;
        assert_eq!(outs.len(), 4); // chaque input → 2 sorties
        let mut vals: Vec<i64> = outs.iter().map(|o| o["v"].as_i64().unwrap()).collect();
        vals.sort_unstable();
        assert_eq!(vals, vec![11, 11, 21, 21]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn t2_supports_filtering_stage() {
        let pipe = Pipeline::new().stage("drop_odd", Arc::new(DropOdd));
        let outs = pipe
            .run_batch(vec![json!({"v":1}), json!({"v":2}), json!({"v":3}), json!({"v":4})], Duration::from_secs(5))
            .await;
        let mut vals: Vec<i64> = outs.iter().map(|o| o["v"].as_i64().unwrap()).collect();
        vals.sort_unstable();
        assert_eq!(vals, vec![2, 4], "les impairs sont filtrés (1→0)");
    }

    #[tokio::test]
    async fn t2_empty_pipeline_is_identity() {
        let pipe = Pipeline::new();
        let outs = pipe.run_batch(vec![json!(1), json!(2)], Duration::from_secs(1)).await;
        assert_eq!(outs, vec![json!(1), json!(2)]);
    }
}
