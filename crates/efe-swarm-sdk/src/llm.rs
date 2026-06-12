// SPDX-License-Identifier: UNLICENSED
//! **Self-host RÉEL** : agents LLM keyless adossés à un [`LocalBackend`] (Ollama natif ou
//! serveur OpenAI-compatible local), appelé via `curl` (**aucune clé API**, aucune
//! dépendance réseau ajoutée).
//!
//! En mode [`crate::ClientMode::SelfHost`], le *nom* d'un agent dans `TopologySpec` est
//! interprété comme une **instruction** en langage naturel (ex. générateur
//! « classify the topic into {technology, finance, health, legal} »). Ces agents
//! produisent du **Mode C** ; ils sont prouvés bout-en-bout dans
//! `efe-swarm-provenance/examples/swarm_all_phases_real.rs`.

use std::sync::Arc;

use async_trait::async_trait;
use efe_swarm_topology::{Decomposer, Generator, Judge, Merger, Refiner, Solver, Stage};
use serde_json::{json, Value};

use crate::backend::{LocalBackend, SamplingParams};

fn text_of(v: &Value) -> String {
    v.get("text")
        .and_then(|t| t.as_str())
        .map(String::from)
        .unwrap_or_else(|| v.as_str().map(String::from).unwrap_or_else(|| v.to_string()))
}

fn parse_score(s: &str) -> f64 {
    let mut num = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() || (c == '.' && !num.is_empty()) {
            num.push(c);
        } else if !num.is_empty() {
            break;
        }
    }
    num.parse::<f64>().map(|x| (x / 10.0).clamp(0.0, 1.0)).unwrap_or(0.5)
}

/// Raccourci : appelle le backend, sentinel vide si échec (la topologie dégrade).
async fn ask(backend: &Arc<dyn LocalBackend>, prompt: String, max_tokens: u32, temperature: f64, seed: u64) -> String {
    backend
        .complete(&prompt, SamplingParams { max_tokens, temperature, seed })
        .await
        .unwrap_or_default()
}

// ─────────────────────────────────────────────────────────────────────────────
//  Agents LLM (instruction = le « nom » de l'agent dans TopologySpec)
// ─────────────────────────────────────────────────────────────────────────────

struct LlmRefiner {
    backend: Arc<dyn LocalBackend>,
    instruction: String,
}
#[async_trait]
impl Refiner for LlmRefiner {
    async fn refine(&self, task: &Value, own: Option<&Value>, peers: &[Value]) -> Value {
        let own_txt = own.map(|v| v.to_string()).unwrap_or_else(|| "none".into());
        let peers_txt =
            peers.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(" | ");
        // Révision ARGUMENTÉE, pas alignement (anti-chambre d'écho, archi §7) :
        let prompt = format!(
            "{} Task: \"{}\". Your previous estimate: {}. Peer estimates: [{}]. \
             Revise ONLY if the peers convince you; otherwise keep your estimate. \
             Answer with only the result, no preamble.",
            self.instruction,
            text_of(task),
            own_txt,
            peers_txt
        );
        let round = task.pointer("/gossip/round").and_then(Value::as_u64).unwrap_or(0);
        let idx = task.pointer("/gossip/agent_index").and_then(Value::as_u64).unwrap_or(0);
        let out = ask(&self.backend, prompt, 16, 0.2 + 0.1 * (idx % 4) as f64, 42 + round * 31 + idx).await;
        out.trim().parse::<f64>().map(|x| json!({ "value": x })).unwrap_or_else(|_| json!({ "value": out }))
    }
}
/// Refiner LLM (Mode C) : l'instruction = le « nom » de l'agent dans la spec.
pub(crate) fn llm_refiner(backend: Arc<dyn LocalBackend>, instruction: String) -> Arc<dyn Refiner> {
    Arc::new(LlmRefiner { backend, instruction })
}

struct LlmGen {
    backend: Arc<dyn LocalBackend>,
    instruction: String,
}
#[async_trait]
impl Generator for LlmGen {
    async fn generate(&self, task: &Value, variant: usize) -> Value {
        let prompt = format!("{} Text: \"{}\". Answer with only the result, no preamble.", self.instruction, text_of(task));
        let out = ask(&self.backend, prompt, 16, 0.2 + 0.3 * variant as f64, 42 + variant as u64).await;
        json!({ "value": out })
    }
}
pub(crate) fn llm_gen(backend: Arc<dyn LocalBackend>, instruction: String) -> Arc<dyn Generator> {
    Arc::new(LlmGen { backend, instruction })
}

struct LlmStage {
    backend: Arc<dyn LocalBackend>,
    instruction: String,
}
#[async_trait]
impl Stage for LlmStage {
    async fn process(&self, input: &Value) -> Vec<Value> {
        let prompt = format!("{} Text: \"{}\". Answer with only the result.", self.instruction, text_of(input));
        vec![json!({ "text": ask(&self.backend, prompt, 24, 0.0, 7).await })]
    }
}
pub(crate) fn llm_stage(backend: Arc<dyn LocalBackend>, instruction: String) -> Arc<dyn Stage> {
    Arc::new(LlmStage { backend, instruction })
}

struct LlmDecomposer {
    backend: Arc<dyn LocalBackend>,
    instruction: String,
}
#[async_trait]
impl Decomposer for LlmDecomposer {
    async fn decompose(&self, task: &Value) -> Vec<Value> {
        let prompt = format!(
            "{} Break it into exactly two simpler sub-questions, one per line, no numbering. Input: \"{}\".",
            self.instruction,
            text_of(task)
        );
        let raw = ask(&self.backend, prompt, 64, 0.0, 7).await;
        let subs: Vec<Value> = raw
            .lines()
            .map(|l| l.trim().trim_start_matches(|c: char| c == '-' || c.is_ascii_digit() || c == '.' || c == ')').trim())
            .filter(|l| l.len() > 5)
            .take(2)
            .map(|l| json!({ "text": l }))
            .collect();
        if subs.is_empty() {
            vec![json!({ "text": text_of(task) })]
        } else {
            subs
        }
    }
}
pub(crate) fn llm_decomposer(backend: Arc<dyn LocalBackend>, instruction: String) -> Arc<dyn Decomposer> {
    Arc::new(LlmDecomposer { backend, instruction })
}

struct LlmSolver {
    backend: Arc<dyn LocalBackend>,
    instruction: String,
}
#[async_trait]
impl Solver for LlmSolver {
    async fn solve(&self, subtask: &Value) -> Value {
        let prompt = format!("{} Answer in one short sentence. Question: \"{}\". Answer:", self.instruction, text_of(subtask));
        json!({ "text": ask(&self.backend, prompt, 48, 0.0, 7).await })
    }
}
pub(crate) fn llm_solver(backend: Arc<dyn LocalBackend>, instruction: String) -> Arc<dyn Solver> {
    Arc::new(LlmSolver { backend, instruction })
}

struct LlmMerger {
    backend: Arc<dyn LocalBackend>,
    instruction: String,
}
#[async_trait]
impl Merger for LlmMerger {
    async fn merge(&self, _task: &Value, results: Vec<Value>) -> Value {
        let joined = results.iter().map(text_of).collect::<Vec<_>>().join(" | ");
        let prompt = format!("{} Combine these into one concise sentence: {joined}. Combined:", self.instruction);
        json!({ "text": ask(&self.backend, prompt, 48, 0.0, 7).await })
    }
}
pub(crate) fn llm_merger(backend: Arc<dyn LocalBackend>, instruction: String) -> Arc<dyn Merger> {
    Arc::new(LlmMerger { backend, instruction })
}

struct LlmJudge {
    backend: Arc<dyn LocalBackend>,
    instruction: String,
}
#[async_trait]
impl Judge for LlmJudge {
    async fn assess(&self, task: &Value, candidates: &[Value]) -> Vec<f64> {
        let text = text_of(task);
        let mut scores = Vec::with_capacity(candidates.len());
        for c in candidates {
            let label = c.get("value").and_then(|v| v.as_str()).unwrap_or("?");
            let prompt = format!(
                "{} On a scale from 0 to 10, how well does \"{label}\" fit this text? \
                 Text: \"{text}\". Answer with only a number from 0 to 10.",
                self.instruction
            );
            scores.push(parse_score(&ask(&self.backend, prompt, 6, 0.0, 7).await));
        }
        scores
    }
    fn name(&self) -> &str {
        &self.instruction
    }
}
pub(crate) fn llm_judge(backend: Arc<dyn LocalBackend>, instruction: String) -> Arc<dyn Judge> {
    Arc::new(LlmJudge { backend, instruction })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{BackendDescriptor, BackendKind};
    use serde_json::json;
    use std::sync::Mutex;

    /// Backend SCRIPTÉ : rejoue une réponse fixe et capture les prompts émis —
    /// l'unité Mode C sans réseau (le motif des mocks de `backend/mod.rs`).
    struct Scripted {
        reply: String,
        prompts: Mutex<Vec<String>>,
    }
    #[async_trait]
    impl LocalBackend for Scripted {
        async fn complete(&self, prompt: &str, _p: SamplingParams) -> Option<String> {
            self.prompts.lock().unwrap().push(prompt.to_string());
            Some(self.reply.clone())
        }
        fn descriptor(&self) -> BackendDescriptor {
            BackendDescriptor { kind: BackendKind::OpenAiLocal, model: "mock".into(), endpoint: "mock".into() }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn llm_refiner_anti_echo_prompt_and_numeric_parse() {
        let be = Arc::new(Scripted { reply: "42".into(), prompts: Mutex::new(Vec::new()) });
        let r = llm_refiner(be.clone(), "Estimate the value.".into());

        // Tour 0 : pas d'état propre, pas de pairs — le refiner EST le générateur.
        let task = json!({ "text": "t", "gossip": { "round": 0, "agent_index": 1 } });
        let out = r.refine(&task, None, &[]).await;
        assert_eq!(out, json!({ "value": 42.0 }), "réponse numérique parsée en valeur");

        // Tour 1 : le prompt porte SON estimation, LES pairs, et la consigne
        // anti-chambre-d'écho (réviser seulement si convaincu — archi §7).
        let own = json!({ "value": 40.0 });
        let peers = vec![json!({ "value": 50.0 }), json!({ "value": 44.0 })];
        let _ = r.refine(&task, Some(&own), &peers).await;

        let prompts = be.prompts.lock().unwrap();
        assert!(prompts[0].contains("none"), "tour 0 : estimation propre absente, dite");
        let p1 = &prompts[1];
        assert!(p1.contains("40"), "l'estimation propre est dans le prompt");
        assert!(p1.contains("50") && p1.contains("44"), "les pairs sont dans le prompt");
        assert!(
            p1.contains("Revise ONLY if the peers convince you"),
            "la consigne anti-écho est présente : {p1}"
        );
    }
}

