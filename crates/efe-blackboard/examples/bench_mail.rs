// SPDX-License-Identifier: UNLICENSED
//! # `bench_mail` — l'**éprouvette** de la messagerie ciblée (ADR-015).
//!
//! Ne compare pas — ÉPROUVE, et chiffre :
//! 1. **I-MSG2 sous brutalité** : bus de capacité 1, 8 expéditeurs × 500 lettres vers
//!    UNE boîte chaude, drains espacés — 100 % livré, exactly-once, in-order, quel que
//!    soit le taux de perte de notifs (imprimé : il sera massif, c'est le point) ;
//! 2. **le coût NU** d'un courrier : µs par `send` (écriture + notif), µs par lettre
//!    drainée — pour que la notice dise « un courrier coûte X » ;
//! 3. **le requête/réponse de bout en bout** : RTT p50/p99 du motif complet
//!    (2 écritures + 2 notifs + 2 drains), à 16 conversations CROISÉES — les
//!    corrélations ne se mélangent jamais (asserté à chaque tour).
//!
//! Écrit `scripts/swarm/EFE_SWARM_MAILBENCH_<date>.json`.
//! Run : `cargo run --release --example bench_mail -p efe-blackboard`

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use efe_blackboard::{Blackboard, Mailboxes, MsgKind, TopicRouter};
use efe_nano_sdk::{AgentId, SwarmId};
use serde_json::{json, Value};

const DATE: &str = "2026-06-12";
const SENDERS: usize = 8;
const LETTERS_PER_SENDER: usize = 500;
const COST_PROBE: usize = 10_000;
const CONVERSATIONS: usize = 16;
const ROUNDS_PER_CONVERSATION: usize = 50;

fn aid(s: &str) -> AgentId {
    AgentId::new(s).unwrap()
}

fn pct(sorted_us: &[u128], p: f64) -> f64 {
    if sorted_us.is_empty() {
        return 0.0;
    }
    let idx = (((sorted_us.len() as f64) * p).ceil() as usize).saturating_sub(1).min(sorted_us.len() - 1);
    sorted_us[idx] as f64 / 1000.0
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    println!("══════════════════════════════════════════════════════════════════════════════");
    println!(" bench_mail — l'éprouvette ADR-015 : brutalité I-MSG2 · coût nu · RTT requête/réponse");
    println!("══════════════════════════════════════════════════════════════════════════════");

    // ── ① I-MSG2 sous brutalité : bus cap 1, boîte chaude, drains espacés. ────────
    let bb = Arc::new(Blackboard::new(SwarmId::new()));
    let router = Arc::new(TopicRouter::new(1)); // l'indice le plus hostile possible
    let boxes = Arc::new(Mailboxes::new());
    let hot = aid("hot_box");
    let mut reader = boxes.reader(&router, &hot);
    let mut handles = Vec::new();
    for s in 0..SENDERS {
        let (bb, router, boxes, hot) = (bb.clone(), router.clone(), boxes.clone(), hot.clone());
        handles.push(tokio::spawn(async move {
            let me = aid(&format!("sender_{s}"));
            for i in 0..LETTERS_PER_SENDER {
                boxes
                    .send(&router, &bb, &me, &hot, MsgKind::Inform, None, json!({ "s": s, "i": i }))
                    .unwrap();
            }
        }));
    }
    // drains ESPACÉS pendant la tempête (le lecteur vit sa vie, l'indice se perd).
    let mut delivered = 0usize;
    let mut last_seq = 0u64;
    let total = SENDERS * LETTERS_PER_SENDER;
    while delivered < total {
        tokio::time::sleep(Duration::from_millis(5)).await;
        for l in reader.drain(&bb) {
            assert_eq!(l.seq, last_seq + 1, "in-order, exactly-once");
            last_seq = l.seq;
            delivered += 1;
        }
    }
    for h in handles {
        h.await.unwrap();
    }
    let published = router.published_total();
    println!("\n  ① brutalité : {total} lettres · bus cap 1 · {published} notifs publiées, 0 réveil CONSOMMÉ (drains à froid) → livré 100 %, in-order — la livraison n'a tenu qu'à la VÉRITÉ");
    let brutality_ok = delivered == total;
    assert!(brutality_ok);

    // ── ② le coût NU (bus normal, boîte fraîche). ─────────────────────────────────
    let bb2 = Arc::new(Blackboard::with_cap(SwarmId::new(), 64 << 20)); // la sonde dépasse le cap par défaut (256 KiB)
    let router2 = Arc::new(TopicRouter::new(256));
    let boxes2 = Mailboxes::new();
    let (alice, bob) = (aid("alice"), aid("bob"));
    let t0 = Instant::now();
    for i in 0..COST_PROBE {
        boxes2.send(&router2, &bb2, &alice, &bob, MsgKind::Inform, None, json!({ "i": i })).unwrap();
    }
    let send_us = t0.elapsed().as_micros() as f64 / COST_PROBE as f64;
    let mut reader2 = boxes2.reader(&router2, &bob);
    let t1 = Instant::now();
    let n = reader2.drain(&bb2).len();
    let drain_us = t1.elapsed().as_micros() as f64 / n as f64;
    println!("  ② coût nu : send {send_us:.2} µs · drain {drain_us:.2} µs/lettre ({n} lettres)");

    // ── ③ RTT requête/réponse, 16 conversations CROISÉES. ────────────────────────
    let bb3 = Arc::new(Blackboard::new(SwarmId::new()));
    let router3 = Arc::new(TopicRouter::new(1024));
    let boxes3 = Arc::new(Mailboxes::new());
    let mut rtts_us: Vec<u128> = Vec::with_capacity(CONVERSATIONS * ROUNDS_PER_CONVERSATION);
    let mut convs = Vec::new();
    for k in 0..CONVERSATIONS {
        let (bb, router, boxes) = (bb3.clone(), router3.clone(), boxes3.clone());
        let responder = aid(&format!("resp_{k}"));
        let mut resp_box = boxes.reader(&router, &responder);
        // le répondeur : écho corrélé, n tours puis fin.
        let resp = tokio::spawn({
            let (bb, router, boxes) = (bb.clone(), router.clone(), boxes.clone());
            let me = responder.clone();
            async move {
                let mut served = 0usize;
                while served < ROUNDS_PER_CONVERSATION {
                    let letters = resp_box.recv_batch(&bb).await.expect("requête");
                    for req in letters {
                        let c = req.correlation.unwrap();
                        boxes.reply(&router, &bb, &me, &req.from, c, json!({ "echo": req.body["n"] })).unwrap();
                        served += 1;
                    }
                }
            }
        });
        // le demandeur : n tours de RTT mesurés, corrélation VÉRIFIÉE à chaque tour.
        let ask = tokio::spawn({
            let (bb, router, boxes) = (bb.clone(), router.clone(), boxes.clone());
            let me = aid(&format!("ask_{k}"));
            let responder = responder.clone();
            async move {
                let mut my_box = boxes.reader(&router, &me);
                let mut rtts = Vec::with_capacity(ROUNDS_PER_CONVERSATION);
                for r in 0..ROUNDS_PER_CONVERSATION {
                    let t = Instant::now();
                    let (_seq, c) =
                        boxes.request(&router, &bb, &me, &responder, json!({ "n": k * 1000 + r })).unwrap();
                    let reply = my_box
                        .await_reply(&bb, c, Duration::from_secs(5))
                        .await
                        .expect("réponse corrélée");
                    rtts.push(t.elapsed().as_micros());
                    assert_eq!(reply.body["echo"], json!(k * 1000 + r), "les corrélations ne se croisent JAMAIS");
                }
                rtts
            }
        });
        convs.push((resp, ask));
    }
    for (resp, ask) in convs {
        resp.await.unwrap();
        rtts_us.extend(ask.await.unwrap());
    }
    rtts_us.sort_unstable();
    let (p50, p99) = (pct(&rtts_us, 0.50), pct(&rtts_us, 0.99));
    println!(
        "  ③ requête/réponse : {} RTT mesurés · p50 {p50:.3} ms · p99 {p99:.3} ms · 0 corrélation croisée",
        rtts_us.len()
    );

    let go = brutality_ok;
    println!("\n  VERDICT bench_mail : {}", if go { "GO ✓ — les invariants tiennent sous brutalité, les coûts sont chiffrés" } else { "NO-GO ✗" });

    let report: Value = json!({
        "bench": "mail",
        "date": DATE,
        "adr": "ADR-SWARM-015",
        "verdict": if go { "GO" } else { "NO-GO" },
        "brutality_IMSG2": {
            "letters": total, "router_capacity": 1,
            "notifs_published": published, "wakeups_consumed": 0,
            "delivered_pct": 100.0, "in_order": true, "exactly_once": true,
            "note": "le lecteur n'a jamais consommé UN réveil — toute la livraison vient des drains contre la vérité"
        },
        "raw_cost": {
            "send_us": (send_us * 100.0).round() / 100.0,
            "drain_us_per_letter": (drain_us * 100.0).round() / 100.0,
            "probe": COST_PROBE
        },
        "request_reply": {
            "conversations": CONVERSATIONS, "rounds_each": ROUNDS_PER_CONVERSATION,
            "rtt_p50_ms": (p50 * 1000.0).round() / 1000.0,
            "rtt_p99_ms": (p99 * 1000.0).round() / 1000.0,
            "crossed_correlations": 0
        },
    });
    if let Some(root) = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2) {
        let dir = root.join("scripts/swarm");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("EFE_SWARM_MAILBENCH_{DATE}.json"));
        match std::fs::write(&path, format!("{}\n", serde_json::to_string_pretty(&report).unwrap())) {
            Ok(_) => println!("  rapport écrit : {}", path.display()),
            Err(e) => println!("  (écriture du rapport échouée : {e})"),
        }
    }
    println!("══════════════════════════════════════════════════════════════════════════════");
}
