// SPDX-License-Identifier: UNLICENSED
//! **Self-host RÉEL** : agents LLM keyless adossés à un backend local (`llama-server`/
//! Ollama) appelé via `curl` (**aucune clé API**, aucune dépendance réseau ajoutée).
//!
//! En mode [`crate::ClientMode::SelfHost`], le *nom* d'un agent dans `TopologySpec` est
//! interprété comme une **instruction** en langage naturel (ex. générateur
//! « classify the topic into {technology, finance, health, legal} »). Ces agents
//! produisent du **Mode C** ; ils sont prouvés bout-en-bout dans
//! `efe-swarm-provenance/examples/swarm_all_phases_real.rs`.

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::sync::Arc;

use async_trait::async_trait;
use efe_swarm_topology::{Decomposer, Generator, Judge, Merger, Solver, Stage};
use serde_json::{json, Value};

/// Backend LLM local (Ollama/llama-server) — keyless.
#[derive(Debug)]
pub(crate) struct OllamaBackend {
    endpoint: String,
    model: String,
}

impl OllamaBackend {
    pub fn new(endpoint: String, model: String) -> Arc<Self> {
        Arc::new(Self { endpoint, model })
    }

    /// Appel RÉEL bloquant (curl ; JSON par stdin → pas d'échappement shell).
    fn call_blocking(&self, prompt: &str, num_predict: u32, temperature: f64, seed: u64) -> Option<String> {
        let body = json!({
            "model": self.model, "prompt": prompt, "stream": false,
            "options": { "temperature": temperature, "num_predict": num_predict, "seed": seed }
        });
        let url = format!("{}/api/generate", self.endpoint.trim_end_matches('/'));
        let mut child = Command::new("curl")
            .args(["-s", "--max-time", "120", &url, "-d", "@-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        child.stdin.take()?.write_all(body.to_string().as_bytes()).ok()?;
        let out = child.wait_with_output().ok()?;
        let resp: Value = serde_json::from_slice(&out.stdout).ok()?;
        resp.get("response").and_then(|v| v.as_str()).map(|s| s.trim().to_string())
    }

    /// Wrapper async (curl dans un thread bloquant — compatible runtime tokio).
    async fn ask(self: &Arc<Self>, prompt: String, n: u32, temp: f64, seed: u64) -> String {
        let me = self.clone();
        tokio::task::spawn_blocking(move || me.call_blocking(&prompt, n, temp, seed))
            .await
            .ok()
            .flatten()
            .unwrap_or_default()
    }
}

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

// ─────────────────────────────────────────────────────────────────────────────
//  Agents LLM (instruction = le « nom » de l'agent dans TopologySpec)
// ─────────────────────────────────────────────────────────────────────────────

struct LlmGen {
    backend: Arc<OllamaBackend>,
    instruction: String,
}
#[async_trait]
impl Generator for LlmGen {
    async fn generate(&self, task: &Value, variant: usize) -> Value {
        let prompt = format!("{} Text: \"{}\". Answer with only the result, no preamble.", self.instruction, text_of(task));
        let out = self.backend.ask(prompt, 16, 0.2 + 0.3 * variant as f64, 42 + variant as u64).await;
        json!({ "value": out })
    }
}
pub(crate) fn llm_gen(backend: Arc<OllamaBackend>, instruction: String) -> Arc<dyn Generator> {
    Arc::new(LlmGen { backend, instruction })
}

struct LlmStage {
    backend: Arc<OllamaBackend>,
    instruction: String,
}
#[async_trait]
impl Stage for LlmStage {
    async fn process(&self, input: &Value) -> Vec<Value> {
        let prompt = format!("{} Text: \"{}\". Answer with only the result.", self.instruction, text_of(input));
        vec![json!({ "text": self.backend.ask(prompt, 24, 0.0, 7).await })]
    }
}
pub(crate) fn llm_stage(backend: Arc<OllamaBackend>, instruction: String) -> Arc<dyn Stage> {
    Arc::new(LlmStage { backend, instruction })
}

struct LlmDecomposer {
    backend: Arc<OllamaBackend>,
    instruction: String,
}
#[async_trait]
impl Decomposer for LlmDecomposer {
    async fn decompose(&self, task: &Value) -> Vec<Value> {
        let prompt = format!(
            "{} Break it into exactly two simpler sub-questions, one per line, no numbering. Input: \"{}\".",
            self.instruction, text_of(task)
        );
        let raw = self.backend.ask(prompt, 64, 0.0, 7).await;
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
pub(crate) fn llm_decomposer(backend: Arc<OllamaBackend>, instruction: String) -> Arc<dyn Decomposer> {
    Arc::new(LlmDecomposer { backend, instruction })
}

struct LlmSolver {
    backend: Arc<OllamaBackend>,
    instruction: String,
}
#[async_trait]
impl Solver for LlmSolver {
    async fn solve(&self, subtask: &Value) -> Value {
        let prompt = format!("{} Answer in one short sentence. Question: \"{}\". Answer:", self.instruction, text_of(subtask));
        json!({ "text": self.backend.ask(prompt, 48, 0.0, 7).await })
    }
}
pub(crate) fn llm_solver(backend: Arc<OllamaBackend>, instruction: String) -> Arc<dyn Solver> {
    Arc::new(LlmSolver { backend, instruction })
}

struct LlmMerger {
    backend: Arc<OllamaBackend>,
    instruction: String,
}
#[async_trait]
impl Merger for LlmMerger {
    async fn merge(&self, _task: &Value, results: Vec<Value>) -> Value {
        let joined = results.iter().map(text_of).collect::<Vec<_>>().join(" | ");
        let prompt = format!("{} Combine these into one concise sentence: {joined}. Combined:", self.instruction);
        json!({ "text": self.backend.ask(prompt, 48, 0.0, 7).await })
    }
}
pub(crate) fn llm_merger(backend: Arc<OllamaBackend>, instruction: String) -> Arc<dyn Merger> {
    Arc::new(LlmMerger { backend, instruction })
}

struct LlmJudge {
    backend: Arc<OllamaBackend>,
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
            scores.push(parse_score(&self.backend.ask(prompt, 6, 0.0, 7).await));
        }
        scores
    }
    fn name(&self) -> &str {
        &self.instruction
    }
}
pub(crate) fn llm_judge(backend: Arc<OllamaBackend>, instruction: String) -> Arc<dyn Judge> {
    Arc::new(LlmJudge { backend, instruction })
}
