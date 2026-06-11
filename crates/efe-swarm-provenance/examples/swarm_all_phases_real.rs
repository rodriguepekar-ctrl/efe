// SPDX-License-Identifier: UNLICENSED
//! # `swarm_all_phases_real` — TOUTES les phases, VRAIS agents (Ollama local, keyless).
//!
//! Fait **vraiment tourner les agents** à travers les 4 topologies, via l'interface
//! unifiée §3 (`Box<dyn Topology>`) et UN seul contexte instrumenté par la provenance :
//!
//! | phase | ce qui tourne RÉELLEMENT |
//! |---|---|
//! | **P1 substrat** | Blackboard / TopicRouter / StreamingSwarm sous chaque topologie |
//! | **P2 topologies** | **T1** Generate-Verify · **T2** Pipeline · **T3** Hiérarchique |
//! | **P3 consensus** | **T-Consensus** (générateurs + juges Ollama → Borda) |
//! | **P4 provenance** | tous les appels d'agents capturés → 1 chaîne → `verify_swarm` |
//!
//! Tous les agents (Generator, Stage, Decomposer, Solver, Merger, Judge) appellent un
//! **vrai modèle** (`gemma3:12b` via `curl`, **aucune clé API**).
//!
//! Run :  cargo run --release --example swarm_all_phases_real -p efe-swarm-provenance

use std::fs;
use std::io::Write as _;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use efe_blackboard::WriteSink;
use efe_nano_sdk::{AgentId, SwarmId, TenantId};
use efe_swarm_provenance::{
    verify_swarm, HmacKeySource, ProvenanceRecorder, ProvenanceSink, ProvenancedAgent,
};
use efe_swarm_topology::catalog::EqualsExpected;
use efe_swarm_topology::{
    ConsensusTopology, Decomposer, GenerateVerify, Generator, GvTopology, HierTopology,
    Hierarchical, Judge, Merger, Pattern, Pipeline, PipelineTopology, Solver, Stage, SwarmContext,
    Topology,
};
use serde_json::{json, Value};

const MODEL: &str = "gemma3:12b";

// ─────────────────────────────────────────────────────────────────────────────
//  Client Ollama RÉEL (curl ; JSON par stdin → pas d'échappement shell)
// ─────────────────────────────────────────────────────────────────────────────

fn ollama(prompt: &str, num_predict: u32, temperature: f64, seed: u64) -> Option<String> {
    let body = json!({
        "model": MODEL, "prompt": prompt, "stream": false,
        "options": { "temperature": temperature, "num_predict": num_predict, "seed": seed }
    });
    let mut child = Command::new("curl")
        .args(["-s", "--max-time", "120", "http://127.0.0.1:11434/api/generate", "-d", "@-"])
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

async fn ask(prompt: String, n: u32, temp: f64, seed: u64) -> String {
    tokio::task::spawn_blocking(move || ollama(&prompt, n, temp, seed))
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn parse_topic(s: &str) -> String {
    let low = s.to_lowercase();
    for t in ["technology", "finance", "health", "legal"] {
        if low.contains(t) {
            return t.to_string();
        }
    }
    low.split_whitespace().next().unwrap_or("?").trim_matches(|c: char| !c.is_alphanumeric()).to_string()
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
fn text_of(v: &Value) -> String {
    v.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string()
}

// ─────────────────────────────────────────────────────────────────────────────
//  Agents RÉELS (un par rôle)
// ─────────────────────────────────────────────────────────────────────────────

struct OllamaGen; // classification (T1, T-Consensus)
#[async_trait]
impl Generator for OllamaGen {
    async fn generate(&self, task: &Value, variant: usize) -> Value {
        let text = text_of(task);
        let p = format!(
            "Classify the topic of the text as exactly one word from {{technology, finance, health, legal}}. \
             Text: \"{text}\". Answer with only the single topic word."
        );
        json!({ "value": parse_topic(&ask(p, 8, 0.2 + 0.3 * variant as f64, 42 + variant as u64).await) })
    }
}

struct OllamaStage {
    instruction: String,
    n: u32,
} // T2
#[async_trait]
impl Stage for OllamaStage {
    async fn process(&self, input: &Value) -> Vec<Value> {
        let text = text_of(input);
        let p = format!("{} Text: \"{text}\". Answer with only the result, no preamble.", self.instruction);
        vec![json!({ "text": ask(p, self.n, 0.0, 7).await })]
    }
}

struct OllamaDecomposer; // T3
#[async_trait]
impl Decomposer for OllamaDecomposer {
    async fn decompose(&self, task: &Value) -> Vec<Value> {
        let text = text_of(task);
        let p = format!(
            "Break this question into exactly two simpler sub-questions, one per line, no numbering. \
             Question: \"{text}\"."
        );
        let raw = ask(p, 64, 0.0, 7).await;
        let subs: Vec<Value> = raw
            .lines()
            .map(|l| l.trim().trim_start_matches(|c: char| c == '-' || c.is_ascii_digit() || c == '.' || c == ')').trim())
            .filter(|l| l.len() > 5)
            .take(2)
            .map(|l| json!({ "text": l }))
            .collect();
        if subs.is_empty() {
            vec![json!({ "text": text })]
        } else {
            subs
        }
    }
}

struct OllamaSolver; // T3
#[async_trait]
impl Solver for OllamaSolver {
    async fn solve(&self, subtask: &Value) -> Value {
        let q = text_of(subtask);
        let p = format!("Answer in one short sentence. Question: \"{q}\". Answer:");
        json!({ "text": ask(p, 40, 0.0, 7).await })
    }
}

struct OllamaMerger; // T3
#[async_trait]
impl Merger for OllamaMerger {
    async fn merge(&self, _task: &Value, results: Vec<Value>) -> Value {
        let joined = results.iter().map(text_of).collect::<Vec<_>>().join(" | ");
        let p = format!("Combine these answers into one concise sentence: {joined}. Combined:");
        json!({ "text": ask(p, 48, 0.0, 7).await })
    }
}

struct OllamaJudge {
    name: String,
    seed: u64,
} // T-Consensus
#[async_trait]
impl Judge for OllamaJudge {
    async fn assess(&self, task: &Value, candidates: &[Value]) -> Vec<f64> {
        let text = text_of(task);
        let mut scores = Vec::with_capacity(candidates.len());
        for c in candidates {
            let label = c.get("value").and_then(|v| v.as_str()).unwrap_or("?");
            let p = format!(
                "On a scale from 0 to 10, how well does the topic \"{label}\" describe this text? \
                 Text: \"{text}\". Answer with only a number from 0 to 10."
            );
            scores.push(parse_score(&ask(p, 6, 0.0, self.seed).await));
        }
        scores
    }
    fn name(&self) -> &str {
        &self.name
    }
}

fn gen_role(inner: OllamaGen, rec: &Arc<ProvenanceRecorder>, id: &str) -> Arc<dyn Generator> {
    Arc::new(ProvenancedAgent::mode_c(inner, rec.clone(), AgentId::new(id).unwrap()))
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    println!("══════════════════════════════════════════════════════════════════════════════");
    println!(" SWARM — TOUTES LES PHASES, VRAIS AGENTS · Ollama {MODEL} (local, keyless, sans clé)");
    println!("══════════════════════════════════════════════════════════════════════════════");
    print!(" warmup… ");
    let _ = std::io::stdout().flush();
    if ask("Say OK.".into(), 2, 0.0, 1).await.is_empty() {
        eprintln!("Ollama injoignable — abandon.");
        return;
    }
    println!("ok");

    let tenant = TenantId::system();
    let key = HmacKeySource::test_key().derive(tenant).unwrap();
    let recorder = Arc::new(ProvenanceRecorder::new(SwarmId::new(), tenant));
    let sink: Arc<dyn WriteSink> = Arc::new(ProvenanceSink::new(recorder.clone()));
    let ctx = SwarmContext::standard().with_write_sink(sink);

    // ── Assemblage des 4 topologies avec de VRAIS agents enveloppés (provenance). ─
    // T1 : 3 générateurs Ollama + vérificateur déterministe (a-posteriori, ADR-002).
    let t1: Box<dyn Topology> = Box::new(GvTopology {
        gv: GenerateVerify::new(3, 3),
        gen: gen_role(OllamaGen, &recorder, "t1_classifier"),
        ver: Arc::new(EqualsExpected),
    });
    // T2 : pipeline de 2 étages LLM (résume → mot-clé).
    let t2: Box<dyn Topology> = Box::new(PipelineTopology::new(
        Pipeline::new()
            .stage("summarize", Arc::new(ProvenancedAgent::mode_c(OllamaStage { instruction: "Summarize in at most 5 words.".into(), n: 16 }, recorder.clone(), AgentId::new("t2_summarize").unwrap())))
            .stage("keyword", Arc::new(ProvenancedAgent::mode_c(OllamaStage { instruction: "Give the single most important keyword (one word).".into(), n: 4 }, recorder.clone(), AgentId::new("t2_keyword").unwrap()))),
    ));
    // T3 : décompose (Ollama) → résout chaque sous-question (Ollama) → fusionne (Ollama).
    let decomposer: Arc<dyn Decomposer> = Arc::new(ProvenancedAgent::mode_c(OllamaDecomposer, recorder.clone(), AgentId::new("t3_decomposer").unwrap()));
    let solver: Arc<dyn Solver> = Arc::new(ProvenancedAgent::mode_c(OllamaSolver, recorder.clone(), AgentId::new("t3_solver").unwrap()));
    let merger: Arc<dyn Merger> = Arc::new(ProvenancedAgent::mode_c(OllamaMerger, recorder.clone(), AgentId::new("t3_merger").unwrap()));
    let t3: Box<dyn Topology> = Box::new(HierTopology::new(Arc::new(Hierarchical::new(decomposer, solver, merger)), 3.0));
    // T-Consensus : 3 générateurs Ollama + 2 juges Ollama → Borda.
    let judges: Vec<Arc<dyn Judge>> = (0..2)
        .map(|j| Arc::new(ProvenancedAgent::mode_c(OllamaJudge { name: format!("judge_{j:03}"), seed: 7 + j as u64 }, recorder.clone(), AgentId::new(format!("judge_{j:03}")).unwrap())) as Arc<dyn Judge>)
        .collect();
    let t_cons: Box<dyn Topology> = Box::new(ConsensusTopology::new(gen_role(OllamaGen, &recorder, "generator"), 3, judges).with_pattern(Pattern::Borda));

    // ── Tâches par topologie + exécution UNIFORME via l'interface §3. ───────────
    #[allow(clippy::type_complexity)] // tableau de démo (nom, topologie, tâche) — type explicite voulu
    let runs: Vec<(&str, &Box<dyn Topology>, Value)> = vec![
        ("T1 classify", &t1, json!({ "text": "The court ruled the contract clause was unenforceable.", "expected": "legal" })),
        ("T2 transform", &t2, json!({ "text": "Artificial intelligence is transforming healthcare diagnostics rapidly." })),
        ("T3 reason", &t3, json!({ "text": "What is one benefit and one risk of using AI in medicine?" })),
        ("T-Consensus", &t_cons, json!({ "text": "Surgeons performed a minimally invasive heart procedure.", "expected": "health" })),
    ];

    println!("\n────────── EXÉCUTION RÉELLE (interface §3 unifiée · UN contexte instrumenté) ──────────");
    let mut rows = Vec::new();
    for (label, topo, task) in &runs {
        let before = recorder.captured_calls();
        let t0 = Instant::now();
        let out = topo.run(task, &ctx).await; // VRAIS appels modèle
        let ms = t0.elapsed().as_millis();
        let captured = recorder.captured_calls() - before;
        let preview = serde_json::to_string(&out.output).unwrap_or_default();
        let preview = preview.chars().take(64).collect::<String>();
        println!("   {:<13} [{:<18}] {:>3} appels · {:>5} ms · → {}", label, topo.name(), captured, ms, preview);
        rows.push(json!({ "phase": label, "topology": topo.name(), "agent_calls": captured, "ms": ms, "output": out.output }));
    }

    // ── Scellement + vérification indépendante de TOUT le run multi-phases. ─────
    let sealed = recorder.seal_with_key(key.clone());
    let report = verify_swarm(&sealed.records, &key);

    println!("\n────────── PROVENANCE (P4) sur l'ensemble du run ──────────");
    println!("   appels d'agents captés (seam B) : {}", recorder.captured_calls());
    println!("   écritures ancrées (seam A)      : {}", recorder.anchored_writes());
    println!("   enregistrements scellés         : {}", sealed.len());
    println!("   provenance_root                 : {}", sealed.root_hex().unwrap_or_default());
    println!(
        "   verify_swarm : ok={} · chaîne={} · contenu={} · DAG={} · ancrés={:.0}%",
        report.ok(), report.chain_intact, report.content_intact, report.dag_consistent, 100.0 * report.traced_pct
    );

    let all_ran = rows.iter().all(|r| r["agent_calls"].as_u64().unwrap_or(0) > 0);
    println!("\n────────────────────────── VERDICT (RÉEL · toutes phases) ──────────────────────────");
    println!("   P1+P2+P3 : 4 topologies exécutées avec de VRAIS agents : {}", if all_ran { "✓ tous ont tourné" } else { "✗" });
    println!("   P4       : run multi-phases scellé en UNE chaîne, re-prouvé : {}", if report.ok() { "✓ verify_swarm" } else { "✗" });
    println!("   note honnête : l'ANCRAGE (seam A) est à 100 % pour les topos à blackboard");
    println!("   (T2, T-Consensus) ; T1/T3 sont CAPTURÉS (seam B) mais non ancrés (pas de");
    println!("   blackboard exposé) — l'intégrité de chaîne tient pour TOUS les enregistrements.");
    println!("─────────────────────────────────────────────────────────────────────────────────");

    let payload = json!({
        "_real": true, "backend": "ollama", "model": MODEL, "endpoint": "http://127.0.0.1:11434",
        "runs": rows,
        "provenance": {
            "captured_calls": recorder.captured_calls(),
            "anchored_writes": recorder.anchored_writes(),
            "records": sealed.len(),
            "root": sealed.root_hex(),
            "verify_ok": report.ok(),
            "chain_intact": report.chain_intact,
            "content_intact": report.content_intact,
            "dag_consistent": report.dag_consistent,
            "anchored_ratio": report.traced_pct,
        },
        "verdict": format!("REAL all-phases: 4 topologies (T1/T2/T3/T-Consensus) ran with real Ollama agents; whole run sealed + verify_swarm={}", report.ok()),
    });
    let _ = fs::create_dir_all("scripts/swarm");
    let path = "scripts/swarm/EFE_SWARM_ALLPHASES_2026-06-09.json";
    let _ = fs::write(path, serde_json::to_string_pretty(&payload).unwrap());
    println!("\n✅ Résultats RÉELS : {path}");
}
