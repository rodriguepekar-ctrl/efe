// SPDX-License-Identifier: UNLICENSED
//! # `selfhost_mailbox` — messagerie ciblée avec de **VRAIS agents LLM** (Mode C, ADR-015).
//!
//! La question « comment le tester avec de vrais agents, pas des bouchons ? » — réalisée.
//! Un **coordinateur** consulte un panel d'experts sur une décision d'achat autonome ;
//! il envoie à CHACUN une question **taillée pour son domaine** (sécurité · coût ·
//! conformité) — c'est l'essence du « toi, précisément » : un contenu différent à chaque
//! destinataire, jamais un broadcast. Chaque expert **draine sa boîte**, appelle un vrai
//! modèle local (`gemma3:12b` via `curl`, **aucune clé**), et **répond** avec la
//! corrélation. Le coordinateur recolle les réponses corrélées.
//!
//! Ce qui est RÉEL ici (vs `swarm_mailbox`, dont les workers calculaient n²) : le
//! **contenu** des réponses est produit par un LLM, non déterministe. Ce qui reste
//! PROUVÉ malgré ça (ADR-015 §5.2 — Mode C *auditable sans rejouable*) : qui a écrit à
//! qui, qui a lu quelle lettre (lecture scellée), quelle réponse répond à quelle
//! requête — re-prouvé hors ligne par `verify_swarm` (chaîne + contenu + DAG). Les
//! agents sont marqués `ModeC` ; leurs réponses réelles sont archivées dans la preuve.
//!
//! Pré-requis : un Ollama local (`ollama serve` + `ollama pull gemma3:12b`). Sans lui,
//! l'exemple le DIT et sort proprement (sonde de santé).
//!
//! Run : `cargo run --release --example selfhost_mailbox -p efe-swarm-provenance`
//!       (env optionnel : `EFE_OLLAMA_MODEL`)

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use efe_blackboard::{Blackboard, Mailboxes, TopicRouter};
use efe_nano_sdk::{AgentId, SwarmId, TenantId};
use efe_swarm_provenance::{
    verify_swarm, AgentKind, AgentRole, CallCapture, HmacKeySource, ProvenanceRecorder,
    ProvenanceSink,
};
use serde_json::json;

fn model() -> String {
    std::env::var("EFE_OLLAMA_MODEL").unwrap_or_else(|_| "gemma3:12b".into())
}

/// Sonde de santé : Ollama répond-il ? (`/api/tags`, court timeout). Aucune clé.
fn ollama_alive() -> bool {
    Command::new("curl")
        .args(["-s", "-o", "/dev/null", "--max-time", "3", "http://127.0.0.1:11434/api/tags"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Appel RÉEL au modèle local (curl ; JSON par stdin → pas d'échappement shell).
fn ollama(prompt: &str) -> Option<String> {
    let body = json!({
        "model": model(), "prompt": prompt, "stream": false,
        "options": { "temperature": 0.2, "num_predict": 80, "seed": 7 }
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
    let resp: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    resp.get("response").and_then(|v| v.as_str()).map(|s| s.trim().to_string())
}

async fn ask(prompt: String) -> String {
    tokio::task::spawn_blocking(move || ollama(&prompt))
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "(pas de réponse du modèle)".into())
}

/// Un expert du panel : son AgentId + l'angle (persona) sous lequel il lit la question.
struct Expert {
    id: &'static str,
    persona: &'static str,
    question: &'static str,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    println!("══════════════════════════════════════════════════════════════════════════");
    println!(" selfhost_mailbox — messagerie ciblée · VRAIS agents LLM (Mode C, ADR-015)");
    println!("══════════════════════════════════════════════════════════════════════════");

    if !ollama_alive() {
        println!("\n  Ollama injoignable sur http://127.0.0.1:11434 — sortie propre.");
        println!("  Lancez `ollama serve` puis `ollama pull {}` et relancez.", model());
        println!("══════════════════════════════════════════════════════════════════════════");
        return;
    }
    println!("  modèle : {} · panel de 3 experts · une question TAILLÉE par destinataire\n", model());

    // ── substrat instrumenté : chaque écriture /mail est ANCRÉE à la provenance. ──
    let swarm_id = SwarmId::new();
    let bb = Arc::new(Blackboard::new(swarm_id));
    let recorder = Arc::new(ProvenanceRecorder::new(swarm_id, TenantId::system()));
    bb.set_write_sink(Arc::new(ProvenanceSink::new(recorder.clone())));
    let router = Arc::new(TopicRouter::new(256));
    let boxes = Arc::new(Mailboxes::new());
    let coordinator = AgentId::new("coordinator").unwrap();

    let panel = [
        Expert {
            id: "expert_security",
            persona: "You are a security expert. In ONE short sentence, answer the question about autonomous procurement.",
            question: "What is the single biggest security risk when an AI agent places purchase orders unattended?",
        },
        Expert {
            id: "expert_cost",
            persona: "You are a cost-control expert. In ONE short sentence, answer the question about autonomous procurement.",
            question: "What is the single most effective budget guardrail for an AI agent that spends money?",
        },
        Expert {
            id: "expert_legal",
            persona: "You are a compliance expert. In ONE short sentence, answer the question about autonomous procurement.",
            question: "What is the one compliance record that must always be kept when an AI agent signs a contract?",
        },
    ];

    // ── le coordinateur trace son intention, puis envoie à CHACUN sa question. ────
    recorder.capture(CallCapture::new(
        coordinator.clone(),
        AgentRole::Generator,
        AgentKind::ModeA,
        &json!({ "task": "consult the procurement panel" }),
        &json!({ "questions": panel.len() }),
    ));
    let mut my_box = boxes.reader(&router, &coordinator);
    let mut pending = Vec::new();
    for e in &panel {
        let to = AgentId::new(e.id).unwrap();
        let (_seq, c) =
            boxes.request(&router, &bb, &coordinator, &to, json!({ "ask": e.question })).unwrap();
        println!("  → à {} : « {} »", e.id, e.question);
        pending.push((to, c));
    }

    // ── les experts : drainent LEUR boîte, RÉFLÉCHISSENT (LLM), répondent. ────────
    let mut handles = Vec::new();
    for e in &panel {
        let me = AgentId::new(e.id).unwrap();
        let (persona, bb, router, boxes, recorder) =
            (e.persona, bb.clone(), router.clone(), boxes.clone(), recorder.clone());
        let mut my_box = boxes.reader(&router, &me);
        handles.push(tokio::spawn(async move {
            let letters = my_box.recv_batch(&bb).await.expect("une question");
            for req in letters {
                let c = req.correlation.expect("requête corrélée");
                let question = req.body["ask"].as_str().unwrap_or("");
                // un VRAI appel modèle : la réponse est non déterministe.
                let answer = ask(format!("{persona}\nQuestion: {question}\nAnswer:")).await;
                // drainage MÉTRÉ : la lettre entre dans la tâche → lecture déclarée et scellée.
                let task = req.attach_to_task(&json!({ "persona": persona }));
                let mut cap = CallCapture::new(
                    me.clone(),
                    AgentRole::Stage,
                    AgentKind::ModeC, // VRAI agent LLM
                    &task,
                    &json!({ "answer": answer }),
                );
                for r in task["_reads"].as_array().unwrap() {
                    cap = cap.reading(r["pointer"].as_str().unwrap(), r["version"].as_u64().unwrap());
                }
                recorder.capture(cap);
                boxes.reply(&router, &bb, &me, &req.from, c, json!({ "answer": answer })).unwrap();
            }
        }));
    }

    // ── le coordinateur recolle les réponses CORRÉLÉES (chacune du bon expert). ───
    println!();
    for (to, c) in &pending {
        let reply = my_box.await_reply(&bb, *c, Duration::from_secs(180)).await.expect("réponse");
        assert_eq!(&reply.from, to, "la réponse vient bien de l'interrogé");
        println!("  ← {} répond : {}", to.as_str(), reply.body["answer"].as_str().unwrap_or(""));
    }
    for h in handles {
        h.await.unwrap();
    }

    // ── la preuve : la conversation entière se re-prouve hors ligne. ──────────────
    let key = HmacKeySource::test_key().derive(TenantId::system()).unwrap();
    let sealed = recorder.seal_with_key(key.clone());
    let report = verify_swarm(&sealed.records, &key);
    println!(
        "\n  preuve : {} records · racine {} · re-preuve tierce chaîne {} · contenu {} · DAG {} → {}",
        sealed.records.len(),
        &sealed.root_hex().unwrap()[..16],
        report.chain_intact,
        report.content_intact,
        report.dag_consistent,
        if report.ok() { "OK" } else { "ÉCHEC" }
    );
    let mode_c = sealed.records.iter().filter(|r| matches!(r.agent_kind, AgentKind::ModeC)).count();
    println!("  agents Mode C (vrais LLM) tracés : {mode_c}/3 · chacun a LU sa lettre et ÉCRIT sa réponse");
    for rec in sealed.records.iter().filter(|r| matches!(r.agent_kind, AgentKind::ModeC)) {
        println!(
            "    {} : LU {} → ÉCRIT {}",
            rec.agent_id.as_str(),
            rec.reads.first().map(|c| c.pointer.as_str()).unwrap_or("—"),
            rec.writes.first().map(|c| c.pointer.as_str()).unwrap_or("—"),
        );
    }
    assert!(report.ok(), "le moat conversationnel tient — même avec de vrais agents LLM");
    println!("══════════════════════════════════════════════════════════════════════════");
}
