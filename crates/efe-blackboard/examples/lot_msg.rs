// SPDX-License-Identifier: UNLICENSED
//! # Lot MSG — le gate de la **messagerie ciblée** (ADR-015).
//!
//! Question instruite : sur la **délégation 1-parmi-K à choix dynamique**, l'adressage
//! (mailbox) bat-il le broadcast-claim (le motif historique, dont la baseline a mesuré
//! 90→99,9 % de CAS gaspillés) ET le shard statique WS4 (le champion actuel, aveugle à
//! la charge) ? Trois bras, MÊMES items et MÊMES coûts (splitmix64), mécanique RÉELLE
//! (CAS, notifs, drains à travers le vrai substrat), temps de travail VIRTUEL (le
//! harnais mesure la QUALITÉ D'AFFECTATION + les coûts de coordination, pas un sleep).
//!
//! - `broadcast-claim` : notif à tous, K workers tentent `write_if(/claim/i)` — témoin
//!   « ce qu'on fuit » ;
//! - `sharded` (WS4) : owner = `shard_owner(i)`, CAS sans contention — témoin
//!   « le champion », ici SANS vol (le vol est temporel, hors harnais analytique :
//!   dit honnêtement dans l'artefact) ;
//! - `targeted` (mailbox) : un dispatcher affecte chaque item au worker le MOINS
//!   chargé (argmin du travail virtuel affecté) — la boîte EST l'attribution
//!   (write-once, pas de claim).
//!
//! Verdict (architecture §7.1) : GO si, à skew fort (σ=1.5), `targeted` réduit le p99
//! d'au moins 25 % vs `sharded` à coût de coordination comparable (±10 %) ET élimine
//! les CAS gaspillés vs `broadcast` ; CONDITIONNEL si le gain n'apparaît qu'à fort
//! skew (σ* chiffré) ; NO-GO sinon — versionné quoi qu'il dise.
//!
//! Écrit `scripts/swarm/EFE_SWARM_MSG_<date>.json`.
//! Run : `cargo run --release --example lot_msg -p efe-blackboard`

use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use efe_blackboard::{
    shard_owner, shard_topic, Blackboard, Mailboxes, MsgKind, SwarmEvent, TopicRouter,
};
use efe_nano_sdk::{AgentId, SwarmId};
use serde_json::{json, Value};

const DATE: &str = "2026-06-12";
const ITEMS: usize = 400;
const WORKERS: usize = 8;
const SKEWS: &[f64] = &[0.0, 0.5, 1.0, 1.5, 2.0, 2.5];

// ── PRNG déterministe (splitmix64) + lognormal — les MÊMES coûts pour les 3 bras. ──
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
fn unit(seed: u64) -> f64 {
    (splitmix64(seed) >> 11) as f64 / (1u64 << 53) as f64
}
fn gauss(seed: u64) -> f64 {
    let u1 = unit(seed).max(1e-12);
    let u2 = unit(seed.wrapping_add(0x5851_F42D_4C95_7F2D));
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}
/// Coût (travail virtuel) de l'item i au skew σ : lognormal(0, σ).
fn cost_of(skew_idx: usize, i: usize) -> impl Fn(f64) -> f64 {
    let seed = splitmix64((skew_idx as u64) << 32 ^ i as u64);
    move |sigma: f64| (sigma * gauss(seed)).exp()
}

fn aid(s: &str) -> AgentId {
    AgentId::new(s).unwrap()
}

/// Résultat d'un bras : files d'items (coûts) par worker + coûts de coordination.
struct ArmOutcome {
    queues: Vec<Vec<f64>>, // coûts, en ordre d'arrivée par worker
    cas_lost: u64,
    events_published: u64,
    bb_writes: usize,
}

/// p99 des temps de complétion (somme préfixe dans la file de chaque worker) + makespan.
fn schedule_stats(queues: &[Vec<f64>]) -> (f64, f64, f64) {
    let mut completions = Vec::new();
    let mut makespan: f64 = 0.0;
    let mut totals = Vec::new();
    for q in queues {
        let mut t = 0.0;
        for c in q {
            t += c;
            completions.push(t);
        }
        makespan = makespan.max(t);
        totals.push(t);
    }
    completions.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p99 = completions[((completions.len() as f64 * 0.99).ceil() as usize - 1).min(completions.len() - 1)];
    let mean = totals.iter().sum::<f64>() / totals.len() as f64;
    let sd = (totals.iter().map(|t| (t - mean) * (t - mean)).sum::<f64>() / totals.len() as f64).sqrt();
    (p99, makespan, sd)
}

/// ── Bras 1 : broadcast-claim — tout le monde reçoit, tout le monde tente. ──────────
async fn arm_broadcast(costs: Arc<Vec<f64>>) -> ArmOutcome {
    let bb = Arc::new(Blackboard::new(SwarmId::new()));
    let router = Arc::new(TopicRouter::new(4096));
    let claimed = Arc::new(AtomicUsize::new(0));
    let cas_lost = Arc::new(AtomicU64::new(0));
    let mut handles = Vec::new();
    for w in 0..WORKERS {
        let (bb, claimed, cas_lost, costs) = (bb.clone(), claimed.clone(), cas_lost.clone(), costs.clone());
        let mut sub = router.subscribe("work");
        let me = aid(&format!("worker_{w}"));
        handles.push(tokio::spawn(async move {
            let mut queue = Vec::new();
            while let Some(ev) = sub.recv().await {
                let i = ev.payload["item"].as_u64().unwrap() as usize;
                match bb.write_if(&format!("/claim/{i}"), 0, json!(w), me.clone()) {
                    Ok(_) => {
                        queue.push(costs[i]);
                        claimed.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(_) => {
                        cas_lost.fetch_add(1, Ordering::Relaxed); // le gaspillage mesuré
                    }
                }
            }
            queue
        }));
    }
    let dispatcher = aid("dispatcher");
    for i in 0..ITEMS {
        let _ = router.publish(SwarmEvent {
            swarm_id: bb.swarm_id(),
            published_by: dispatcher.clone(),
            topic: "work".into(),
            payload: json!({ "item": i }),
            published_at: SystemTime::now(),
        });
    }
    while claimed.load(Ordering::SeqCst) < ITEMS {
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    let events = router.published_total();
    drop(router); // ferme les canaux → les workers rendent leurs files
    let mut queues = Vec::new();
    for h in handles {
        queues.push(h.await.unwrap());
    }
    ArmOutcome { queues, cas_lost: cas_lost.load(Ordering::Relaxed), events_published: events, bb_writes: bb.write_count() }
}

/// ── Bras 2 : sharded (WS4) — owner par hachage, CAS sans contention, sans vol. ─────
async fn arm_sharded(costs: Arc<Vec<f64>>) -> ArmOutcome {
    let bb = Arc::new(Blackboard::new(SwarmId::new()));
    let router = Arc::new(TopicRouter::new(4096));
    let claimed = Arc::new(AtomicUsize::new(0));
    let cas_lost = Arc::new(AtomicU64::new(0));
    let mut handles = Vec::new();
    for w in 0..WORKERS {
        let (bb, claimed, cas_lost, costs) = (bb.clone(), claimed.clone(), cas_lost.clone(), costs.clone());
        let mut sub = router.subscribe(&shard_topic("work", w));
        let me = aid(&format!("worker_{w}"));
        handles.push(tokio::spawn(async move {
            let mut queue = Vec::new();
            while let Some(ev) = sub.recv().await {
                let i = ev.payload["item"].as_u64().unwrap() as usize;
                match bb.write_if(&format!("/claim/{i}"), 0, json!(w), me.clone()) {
                    Ok(_) => {
                        queue.push(costs[i]);
                        claimed.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(_) => {
                        cas_lost.fetch_add(1, Ordering::Relaxed); // attendu : 0
                    }
                }
            }
            queue
        }));
    }
    let dispatcher = aid("dispatcher");
    for i in 0..ITEMS {
        let owner = shard_owner(i as u64, WORKERS);
        let _ = router.publish(SwarmEvent {
            swarm_id: bb.swarm_id(),
            published_by: dispatcher.clone(),
            topic: shard_topic("work", owner),
            payload: json!({ "item": i }),
            published_at: SystemTime::now(),
        });
    }
    while claimed.load(Ordering::SeqCst) < ITEMS {
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    let events = router.published_total();
    drop(router);
    let mut queues = Vec::new();
    for h in handles {
        queues.push(h.await.unwrap());
    }
    ArmOutcome { queues, cas_lost: cas_lost.load(Ordering::Relaxed), events_published: events, bb_writes: bb.write_count() }
}

/// ── Bras 3 : targeted (mailbox) — le dispatcher affecte au MOINS chargé. ───────────
async fn arm_targeted(costs: Arc<Vec<f64>>) -> ArmOutcome {
    let bb = Arc::new(Blackboard::new(SwarmId::new()));
    let router = Arc::new(TopicRouter::new(4096));
    let boxes = Arc::new(Mailboxes::new());
    let mut handles = Vec::new();
    for w in 0..WORKERS {
        let bb = bb.clone();
        let mut my_box = boxes.reader(&router, &aid(&format!("worker_{w}")));
        handles.push(tokio::spawn(async move {
            let mut queue = Vec::new();
            while let Some(letters) = my_box.recv_batch(&bb).await {
                for l in letters {
                    queue.push(l.body["cost"].as_f64().unwrap());
                }
            }
            queue // router droppé + boîte vide → None → fin
        }));
    }
    // le dispatcher : affectation argmin du travail virtuel (list scheduling).
    let dispatcher = aid("dispatcher");
    let mut loads = [0.0f64; WORKERS];
    for i in 0..ITEMS {
        let w = (0..WORKERS).min_by(|a, b| loads[*a].partial_cmp(&loads[*b]).unwrap()).unwrap();
        boxes
            .send(&router, &bb, &dispatcher, &aid(&format!("worker_{w}")), MsgKind::Inform, None,
                  json!({ "item": i, "cost": costs[i] }))
            .unwrap();
        loads[w] += costs[i];
    }
    let events = router.published_total();
    drop(router);
    drop(boxes);
    let mut queues = Vec::new();
    for h in handles {
        queues.push(h.await.unwrap());
    }
    let delivered: usize = queues.iter().map(|q| q.len()).sum();
    assert_eq!(delivered, ITEMS, "100 % du courrier livré (I-MSG2, même à la fermeture)");
    ArmOutcome { queues, cas_lost: 0, events_published: events, bb_writes: bb.write_count() }
}

fn arm_json(name: &str, o: &ArmOutcome) -> (Value, f64, f64, u64) {
    let (p99, makespan, sd) = schedule_stats(&o.queues);
    let coord = o.events_published + o.bb_writes as u64;
    (
        json!({
            "arm": name,
            "p99": (p99 * 1e4).round() / 1e4,
            "makespan": (makespan * 1e4).round() / 1e4,
            "load_stddev": (sd * 1e4).round() / 1e4,
            "cas_lost": o.cas_lost,
            "events_published": o.events_published,
            "bb_writes": o.bb_writes,
            "coordination_cost": coord,
        }),
        p99,
        makespan,
        coord,
    )
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    println!("══════════════════════════════════════════════════════════════════════════════");
    println!(" Lot MSG — délégation 1-parmi-K (ADR-015) · {ITEMS} items · {WORKERS} workers · 3 bras");
    println!("══════════════════════════════════════════════════════════════════════════════");
    println!(" (sharded : SANS vol — le vol est temporel, hors harnais analytique ; dit dans l'artefact)");

    let mut cells = Vec::new();
    let mut go_at_high_skew = false;
    let mut sigma_star: Option<f64> = None;

    for (s_idx, &sigma) in SKEWS.iter().enumerate() {
        let costs: Arc<Vec<f64>> = Arc::new((0..ITEMS).map(|i| cost_of(s_idx, i)(sigma)).collect());
        let b = arm_broadcast(costs.clone()).await;
        let s = arm_sharded(costs.clone()).await;
        let t = arm_targeted(costs.clone()).await;
        let (bj, _bp99, _bm, _bc) = arm_json("broadcast-claim", &b);
        let (sj, sp99, sm, sc) = arm_json("sharded(ws4,sans-vol)", &s);
        let (tj, tp99, tm, tc) = arm_json("targeted(mailbox)", &t);

        let p99_gain = 1.0 - tp99 / sp99;
        let cost_ratio = tc as f64 / sc as f64;
        let cas_eliminated = t.cas_lost == 0 && b.cas_lost > 0;
        let cell_go = p99_gain >= 0.25 && (0.9..=1.1).contains(&cost_ratio) && cas_eliminated;
        if cell_go && sigma_star.is_none() {
            sigma_star = Some(sigma);
        }
        if sigma >= 1.5 && cell_go {
            go_at_high_skew = true;
        }

        println!("\n  ── σ = {sigma} ──");
        println!(
            "    broadcast : p99 {:.2} · CAS perdus {} ({:.1} % des tentatives) · {} événements",
            schedule_stats(&b.queues).0, b.cas_lost,
            100.0 * b.cas_lost as f64 / (b.cas_lost as f64 + ITEMS as f64), b.events_published
        );
        println!(
            "    sharded   : p99 {sp99:.2} · makespan {sm:.2} · σ_charge {:.2} · CAS perdus {} · coord {sc}",
            schedule_stats(&s.queues).2, s.cas_lost
        );
        println!(
            "    targeted  : p99 {tp99:.2} · makespan {tm:.2} · σ_charge {:.2} · CAS perdus 0 · coord {tc}",
            schedule_stats(&t.queues).2
        );
        println!(
            "    → gain p99 vs sharded : {:+.1} % · coût coord ×{:.2} · cellule {}",
            p99_gain * 100.0, cost_ratio, if cell_go { "GO" } else { "non" }
        );
        cells.push(json!({
            "sigma": sigma,
            "arms": [bj, sj, tj],
            "p99_gain_vs_sharded": (p99_gain * 1e4).round() / 1e4,
            "coordination_ratio_vs_sharded": (cost_ratio * 1e4).round() / 1e4,
            "cell_go": cell_go,
        }));
    }

    let verdict = if go_at_high_skew {
        if sigma_star.map(|s| s <= SKEWS[1]).unwrap_or(false) { "GO" } else { "CONDITIONNEL" }
    } else {
        "NO-GO"
    };
    println!("\n  VERDICT Lot MSG : {verdict}");
    if let Some(s) = sigma_star {
        println!("    σ* de bascule (premier skew où l'adressage gagne) : {s}");
    }

    let report = json!({
        "lot": "MSG",
        "date": DATE,
        "adr": "ADR-SWARM-015",
        "items": ITEMS, "workers": WORKERS,
        "honesty": "bras sharded SANS vol (le vol WS4 est déclenché au timeout temporel — hors harnais analytique) ; temps de travail VIRTUEL (le harnais mesure l'affectation + la coordination réelles, pas un sleep)",
        "verdict": verdict,
        "sigma_star": sigma_star,
        "criteria": { "p99_gain_min": 0.25, "coordination_ratio_band": [0.9, 1.1], "cas_eliminated_vs_broadcast": true },
        "cells": cells,
    });
    if let Some(root) = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2) {
        let dir = root.join("scripts/swarm");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("EFE_SWARM_MSG_{DATE}.json"));
        match std::fs::write(&path, format!("{}\n", serde_json::to_string_pretty(&report).unwrap())) {
            Ok(_) => println!("\n  rapport écrit : {}", path.display()),
            Err(e) => println!("\n  (écriture du rapport échouée : {e})"),
        }
    }
    println!("══════════════════════════════════════════════════════════════════════════════");
}
