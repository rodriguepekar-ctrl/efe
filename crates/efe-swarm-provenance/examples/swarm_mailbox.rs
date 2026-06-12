// SPDX-License-Identifier: UNLICENSED
//! # `swarm_mailbox` — la **messagerie ciblée prouvée** (ADR-015), de bout en bout.
//!
//! Démo vivante du moat conversationnel : un coordinateur interroge K workers PAR
//! COURRIER ADRESSÉ (boîtes aux lettres sur la vérité, réveil-indice), chaque worker
//! traite sa requête en déclarant sa LECTURE (`Letter::attach_to_task` → convention
//! `_reads`), répond avec la corrélation — puis un TIERS, muni des seuls records et
//! d'une clé, re-prouve la conversation entière : qui a dit quoi à qui, qui l'a lu,
//! quelle réponse répond à quelle requête. Le bus est volontairement HOSTILE
//! (capacité 1) : la perte d'indice est massive, la livraison est totale (I-MSG2).
//!
//! Run : `cargo run --release --example swarm_mailbox -p efe-swarm-provenance`

use std::sync::Arc;
use std::time::Duration;

use efe_blackboard::{Blackboard, Mailboxes, TopicRouter};
use efe_nano_sdk::{AgentId, SwarmId, TenantId};
use efe_swarm_provenance::{
    verify_swarm, AgentKind, AgentRole, CallCapture, HmacKeySource, ProvenanceRecorder,
    ProvenanceSink,
};
use serde_json::json;

const WORKERS: usize = 4;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    println!("══════════════════════════════════════════════════════════════════════════");
    println!(" swarm_mailbox — messagerie ciblée PROUVÉE (ADR-015) · {WORKERS} workers · bus cap 1");
    println!("══════════════════════════════════════════════════════════════════════════");

    // ── le substrat instrumenté : chaque écriture /mail est ANCRÉE. ────────────
    let swarm_id = SwarmId::new();
    let bb = Arc::new(Blackboard::new(swarm_id));
    let recorder = Arc::new(ProvenanceRecorder::new(swarm_id, TenantId::system()));
    bb.set_write_sink(Arc::new(ProvenanceSink::new(recorder.clone())));
    // bus HOSTILE : capacité 1 — l'indice peut tout perdre, la vérité livre tout.
    let router = Arc::new(TopicRouter::new(1));
    let boxes = Arc::new(Mailboxes::new());

    let coordinator = AgentId::new("coordinator").unwrap();

    // ── les workers : en attente sur LEUR boîte, traitement MÉTRÉ, réponse. ────
    let mut handles = Vec::new();
    for i in 0..WORKERS {
        let me = AgentId::new(format!("worker_{i}")).unwrap();
        let (bb, router, boxes, recorder) =
            (bb.clone(), router.clone(), boxes.clone(), recorder.clone());
        let mut my_box = boxes.reader(&router, &me);
        handles.push(tokio::spawn(async move {
            let letters = my_box.recv_batch(&bb).await.expect("du courrier");
            for req in letters {
                let c = req.correlation.expect("requête corrélée");
                // drainage MÉTRÉ : la lettre entre dans la tâche — sa lecture sera scellée.
                let task = req.attach_to_task(&json!({ "instruction": "square the number" }));
                let n = req.body["n"].as_u64().unwrap();
                let out = json!({ "n_squared": n * n });
                let mut cap =
                    CallCapture::new(me.clone(), AgentRole::Stage, AgentKind::ModeA, &task, &out);
                for r in task["_reads"].as_array().unwrap() {
                    cap = cap.reading(r["pointer"].as_str().unwrap(), r["version"].as_u64().unwrap());
                }
                recorder.capture(cap);
                boxes.reply(&router, &bb, &me, &req.from, c, out).expect("réponse écrite");
            }
        }));
    }
    tokio::time::sleep(Duration::from_millis(30)).await;

    // ── le coordinateur : K requêtes ADRESSÉES, K réponses CORRÉLÉES. ──────────
    recorder.capture(CallCapture::new(
        coordinator.clone(),
        AgentRole::Generator,
        AgentKind::ModeA,
        &json!({ "text": "fan-out squares" }),
        &json!({ "requests": WORKERS }),
    ));
    let mut my_box = boxes.reader(&router, &coordinator);
    let mut correlations = Vec::new();
    for i in 0..WORKERS {
        let to = AgentId::new(format!("worker_{i}")).unwrap();
        let (seq, c) = boxes.request(&router, &bb, &coordinator, &to, json!({ "n": i + 3 })).unwrap();
        println!("  → requête #{seq} vers {} (corrélation {})", to.as_str(), &c.to_string()[..8]);
        correlations.push((to, c, (i as u64 + 3).pow(2)));
    }
    for (to, c, expected) in &correlations {
        let reply = my_box
            .await_reply(&bb, *c, Duration::from_secs(2))
            .await
            .expect("réponse corrélée");
        assert_eq!(reply.body["n_squared"], json!(expected));
        assert_eq!(&reply.from, to, "la réponse vient bien de l'interrogé");
        println!(
            "  ← réponse de {} : {} (corrélation {} confirmée)",
            reply.from.as_str(),
            reply.body["n_squared"],
            &c.to_string()[..8]
        );
    }
    for h in handles {
        h.await.unwrap();
    }
    println!(
        "\n  bus hostile : {} notifs publiées, {} perdues — et 100 % du courrier livré (I-MSG2)",
        router.published_total(),
        router.dropped_total()
    );

    // ── LE TIERS : records + clé, rien d'autre — la conversation se re-prouve. ──
    let key = HmacKeySource::test_key().derive(TenantId::system()).unwrap();
    let sealed = recorder.seal_with_key(key.clone());
    let report = verify_swarm(&sealed.records, &key);
    println!("\n  scellé : {} records · racine {}", sealed.records.len(), &sealed.root_hex().unwrap()[..16]);
    println!(
        "  re-preuve tierce : chaîne {} · contenus {} · DAG {} → {}",
        report.chain_intact, report.content_intact, report.dag_consistent,
        if report.ok() { "OK" } else { "ÉCHEC" }
    );
    for rec in &sealed.records {
        if rec.agent_id.as_str().starts_with("worker") {
            println!(
                "    {} a LU {} (scellé) et a ÉCRIT {} — qui a dit quoi à qui : prouvé",
                rec.agent_id.as_str(),
                rec.reads.first().map(|c| c.pointer.as_str()).unwrap_or("—"),
                rec.writes.first().map(|c| c.pointer.as_str()).unwrap_or("—"),
            );
        }
    }
    assert!(report.ok(), "le moat conversationnel tient");
    println!("══════════════════════════════════════════════════════════════════════════");
}
