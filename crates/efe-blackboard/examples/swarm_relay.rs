// SPDX-License-Identifier: UNLICENSED
//! # `swarm_relay` — démo vivante de l'essaim M50 : beaucoup de petits agents,
//! en parallèle, qui se passent des informations.
//!
//! Run :
//! ```text
//! cargo run --example swarm_relay -p efe-blackboard
//! cargo run --example swarm_relay -p efe-blackboard -- 64 8   # 64 mappers, 8 reducers
//! ```
//!
//! ## Ce que la démo prouve (concept fondateur d'EFE)
//!
//! - **Grande quantité** : N mappers + R reducers + 1 aggregator + 1 tracer,
//!   tous des tâches `tokio::spawn` (montez N à 100+ en argument).
//! - **Parallélisme réel** : tous les agents tournent en même temps sur le
//!   runtime multi-thread tokio. Aucun n'attend les autres en série.
//! - **Passage d'infos entre agents** : la SEULE communication passe par les
//!   deux primitives réelles de `efe-blackboard` :
//!     * [`EventBus`] (broadcast pub/sub) — « j'ai fini, voici un événement » ;
//!     * [`Blackboard`] (store JSON versionné + CAS) — l'état partagé que tout
//!       le monde lit/écrit.
//! - **Coordination sans chef (stigmergie)** : les reducers se *volent* le
//!   travail via `write_if` (compare-and-swap) sur `/claim/<mapper>`. Exactement
//!   un reducer gagne chaque partiel ; les perdants voient un `Conflict`. Pas de
//!   scheduler central, pas de mutex global : la coordination émerge du blackboard.
//! - **Résultat émergent** : le top-mots final n'est calculé par AUCUN agent
//!   seul. Il émerge du relais mapper → (bus + blackboard) → reducer → aggregator.
//!
//! ## Topologie
//!
//! ```text
//!   corpus (shards)
//!        │
//!   ┌────┴─────┐  parallèle
//!   ▼          ▼
//! mapper_01 … mapper_NN   ── écrit /map/<id>, publie {topic:"map_done"} ─┐
//!                                                                        │ EventBus
//!   ┌──────────────────────────────────────────────────────────────────┘
//!   ▼  (work-stealing par CAS sur /claim/<id>)
//! reducer_1 … reducer_R   ── fold les partiels volés, écrit /reduce/<rid>
//!                                                                        │
//!   aggregator  ── compte les map_done, publie "shutdown", fusionne /reduce/* → top-K
//!   tracer      ── imprime CHAQUE événement inter-agent en direct
//! ```

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use efe_blackboard::{Blackboard, BlackboardError, EventBus, SwarmEvent};
use efe_nano_sdk::{AgentId, SwarmId};
use serde_json::json;

/// Petit corpus multilingue (RouterBench-like : EFE est multilingue). Chaque
/// phrase est une « observation » qu'un mapper va dépouiller en mots.
const CORPUS: &[&str] = &[
    "le swarm fait passer les informations entre les agents en parallele",
    "many tiny agents exchange information through the shared blackboard",
    "chaque agent ecrit son partiel puis publie un evenement sur le bus",
    "the event bus is a tokio broadcast channel shared across spawned tasks",
    "les reducers volent le travail via compare and swap sans chef central",
    "stigmergy means coordination emerges from the shared state not a scheduler",
    "le resultat global emerge du relais mapper reducer aggregator",
    "no single agent computes the answer the swarm computes the answer",
    "le blackboard versionne chaque champ et journalise chaque ecriture",
    "parallel agents pass information information information through the board",
    "petit modele peu de parametres grande quantite agents en essaim fluide",
    "ephemeral fluid swarm spawns dissolves agents per request",
    "the swarm passes partial results agent to agent until convergence",
    "le bus diffuse a tous les abonnes chaque evenement publie",
    "agents agents agents talking to agents through information channels",
    "consensus emerges when every shard has reported to the swarm",
];

/// Mots vides ignorés (pour que le top-K montre du signal, pas du bruit).
const STOP: &[&str] = &[
    "le", "la", "les", "un", "une", "de", "des", "du", "et", "en", "a", "the",
    "to", "of", "is", "no", "not", "per", "son", "puis", "sur",
];

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    // ── Config : nombre d'agents (montez-les pour « grande quantité ») ──────
    let args: Vec<String> = std::env::args().collect();
    let n_mappers: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(16);
    let n_reducers: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(4);

    let swarm_id = SwarmId::new();

    // Les DEUX primitives partagées, en Arc, exactement comme prévu par le crate.
    let bb = Arc::new(Blackboard::new(swarm_id));
    // Capacité large = aucun abonné lent ne perd un événement même à N élevé.
    let bus = Arc::new(EventBus::with_capacity((n_mappers * 4).max(256)));

    println!("══════════════════════════════════════════════════════════════");
    println!(" ESSAIM M50 — {n_mappers} mappers ∥ {n_reducers} reducers ∥ 1 aggregator ∥ 1 tracer");
    println!(" swarm_id = {swarm_id}");
    println!(" communication = EventBus (broadcast) + Blackboard (CAS versionné)");
    println!("══════════════════════════════════════════════════════════════\n");

    // ── TRACER : s'abonne AVANT toute publication (le broadcast ne rejoue pas
    //    l'historique). Imprime chaque message inter-agent en direct. ─────────
    let tracer = {
        let mut sub = bus.subscribe();
        tokio::spawn(async move {
            let mut seen = 0u64;
            while let Some(ev) = sub.recv().await {
                seen += 1;
                if ev.topic == "swarm_complete" {
                    break;
                }
                // On n'imprime pas les 100+ map_done un par un quand N est grand :
                // un échantillon suffit à montrer le trafic vivant.
                if ev.topic != "map_done" || seen <= 12 || seen % 17 == 0 {
                    println!(
                        "   ┊ [{:>4}] {:<11} ⇒ topic={:<9} {}",
                        seen,
                        ev.published_by,
                        ev.topic,
                        compact(&ev.payload),
                    );
                }
            }
            seen
        })
    };

    // ── AGGREGATOR : s'abonne AVANT les mappers. Compte les map_done distincts ;
    //    quand tous ont parlé, publie "shutdown" pour libérer les reducers. ────
    let aggregator = {
        let mut sub = bus.subscribe();
        let bus = bus.clone();
        let bb = bb.clone();
        let agg_id = AgentId::new("aggregator").unwrap();
        tokio::spawn(async move {
            use std::collections::BTreeSet;
            let mut done: BTreeSet<String> = BTreeSet::new();
            while let Some(ev) = sub.recv().await {
                if ev.topic == "map_done" {
                    if let Some(m) = ev.payload.get("mapper").and_then(|v| v.as_str()) {
                        done.insert(m.to_string());
                    }
                    if done.len() == n_mappers {
                        break; // tous les shards ont été dépouillés
                    }
                }
            }
            // Libère tous les reducers (ils bloquent sur recv en attendant ceci).
            let _ = bus.publish(mk(swarm_id, &agg_id, "shutdown", json!({})));

            // Attend que les R reducers aient écrit leur /reduce/<rid> (poll
            // coopératif, sans dépendre de la feature `tokio::time`).
            loop {
                let n = bb
                    .snapshot()
                    .fields
                    .keys()
                    .filter(|k| k.starts_with("/reduce/"))
                    .count();
                if n >= n_reducers {
                    break;
                }
                tokio::task::yield_now().await;
            }

            // FUSION FINALE : agrège tous les /reduce/* → compteur global émergent.
            let mut global: BTreeMap<String, u64> = BTreeMap::new();
            for (k, v) in bb.snapshot().fields {
                if !k.starts_with("/reduce/") {
                    continue;
                }
                if let Some(obj) = v.value.as_object() {
                    for (word, cnt) in obj {
                        *global.entry(word.clone()).or_default() += cnt.as_u64().unwrap_or(0);
                    }
                }
            }
            let _ = bb.write("/global", json!(global), agg_id.clone());
            global
        })
    };

    // ── REDUCERS : se volent les partiels par CAS. Tous parallèles. ──────────
    let mut reducer_tasks = Vec::new();
    for r in 0..n_reducers {
        let mut sub = bus.subscribe();
        let bb = bb.clone();
        let rid = AgentId::new(format!("reducer_{r}")).unwrap();
        reducer_tasks.push(tokio::spawn(async move {
            let mut local: BTreeMap<String, u64> = BTreeMap::new();
            let mut claimed = 0u64;
            let mut lost = 0u64;
            while let Some(ev) = sub.recv().await {
                match ev.topic.as_str() {
                    "map_done" => {
                        let Some(mapper) = ev.payload.get("mapper").and_then(|v| v.as_str()) else {
                            continue;
                        };
                        // WORK-STEALING : CAS sur /claim/<mapper>, version attendue 0
                        // (= absent). Le PREMIER reducer gagne ; les autres → Conflict.
                        let claim_key = format!("/claim/{mapper}");
                        match bb.write_if(&claim_key, 0, json!(rid.as_str()), rid.clone()) {
                            Ok(_) => {
                                claimed += 1;
                                // Lit le partiel du mapper et le fold dans l'accu local.
                                if let Some(bv) = bb.read(&format!("/map/{mapper}")) {
                                    if let Some(obj) = bv.value.as_object() {
                                        for (w, c) in obj {
                                            *local.entry(w.clone()).or_default() +=
                                                c.as_u64().unwrap_or(0);
                                        }
                                    }
                                }
                            }
                            Err(BlackboardError::Conflict { .. }) => lost += 1, // peer a gagné
                            Err(_) => {}
                        }
                    }
                    "shutdown" => break,
                    _ => {}
                }
            }
            // Publie son partiel agrégé au blackboard pour l'aggregator.
            let _ = bb.write(&format!("/reduce/{r}"), json!(local), rid.clone());
            println!(
                "   ✓ {:<11} a volé {claimed:>3} partiels (perdu {lost:>3} courses CAS)",
                rid.to_string()
            );
            (claimed, lost)
        }));
    }

    // ── MAPPERS : le gros de l'essaim. Tous en parallèle. ────────────────────
    let mut mapper_tasks = Vec::new();
    for m in 0..n_mappers {
        let bb = bb.clone();
        let bus = bus.clone();
        let id = AgentId::new(format!("mapper_{m:02}")).unwrap();
        // Shard round-robin du corpus : chaque mapper voit un sous-ensemble.
        let shard: Vec<String> = CORPUS
            .iter()
            .enumerate()
            .filter(|(i, _)| i % n_mappers == m)
            .map(|(_, s)| s.to_string())
            .collect();
        mapper_tasks.push(tokio::spawn(async move {
            // Travail local « peu de paramètres » : un comptage de mots trivial.
            let mut counts: BTreeMap<String, u64> = BTreeMap::new();
            let mut tokens = 0u64;
            for line in &shard {
                for word in line.split_whitespace() {
                    if STOP.contains(&word) {
                        continue;
                    }
                    *counts.entry(word.to_string()).or_default() += 1;
                    tokens += 1;
                }
            }
            // Passe l'info à l'essaim : (1) état dans le blackboard,
            //                          (2) signal sur le bus.
            let _ = bb.write(&format!("/map/{id}"), json!(counts), id.clone());
            let _ = bus.publish(mk(
                swarm_id,
                &id,
                "map_done",
                json!({ "mapper": id.as_str(), "tokens": tokens }),
            ));
            tokens
        }));
    }

    // ── Join : on attend que tout l'essaim ait fini ──────────────────────────
    let mut total_tokens = 0u64;
    for t in mapper_tasks {
        total_tokens += t.await.unwrap_or(0);
    }
    let mut total_claimed = 0u64;
    let mut total_lost = 0u64;
    for t in reducer_tasks {
        let (c, l) = t.await.unwrap_or((0, 0));
        total_claimed += c;
        total_lost += l;
    }
    let global = aggregator.await.unwrap_or_default();

    // Clôture le tracer proprement.
    let closer = AgentId::new("aggregator").unwrap();
    let _ = bus.publish(mk(swarm_id, &closer, "swarm_complete", json!({})));
    let traced = tracer.await.unwrap_or(0);

    // ── Rapport : ce que l'essaim a fait émerger ─────────────────────────────
    let mut top: Vec<(String, u64)> = global.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    println!("\n──────────────────────── RÉSULTAT ÉMERGENT ────────────────────────");
    println!(" Top mots (calculé par AUCUN agent seul — émergé du relais) :");
    for (w, c) in top.iter().take(10) {
        let bar = "█".repeat((*c as usize).min(30));
        println!("   {w:<14} {c:>3}  {bar}");
    }

    println!("\n──────────────────────── PREUVE DE TRAFIC ─────────────────────────");
    println!(" agents totaux         : {}", n_mappers + n_reducers + 2);
    println!(" tokens dépouillés     : {total_tokens}");
    println!(" partiels volés (CAS)  : {total_claimed}  ·  courses CAS perdues : {total_lost}");
    println!(" événements publiés    : {}", bus.published_total());
    println!(" événements vus/tracer : {traced}");
    println!(" écritures blackboard  : {}", bb.write_count());
    println!(" champs blackboard      : {}", bb.len());
    // Qui a le plus écrit ? (montre la participation distribuée)
    let busiest = AgentId::new("reducer_0").unwrap();
    println!(
        " écritures reducer_0    : {}  (ex. de participation par agent)",
        bb.writes_by_agent(&busiest)
    );
    println!("════════════════════════════════════════════════════════════════════");
    println!(
        "\n→ Aucun chef d'orchestre : la coordination (qui réduit quoi) a émergé\n  \
         du CAS sur le blackboard ; le résultat a émergé du passage d'infos\n  \
         mapper → bus/blackboard → reducer → aggregator. C'est l'essaim."
    );
}

// ─────────────────────────── helpers ────────────────────────────────────────

/// Construit un `SwarmEvent` (le format d'info qui circule entre agents).
fn mk(swarm_id: SwarmId, by: &AgentId, topic: &str, payload: serde_json::Value) -> SwarmEvent {
    SwarmEvent {
        swarm_id,
        published_by: by.clone(),
        topic: topic.to_string(),
        payload,
        published_at: SystemTime::now(),
    }
}

/// Rend un payload JSON compact pour la trace live.
fn compact(v: &serde_json::Value) -> String {
    let s = v.to_string();
    if s.len() > 48 {
        format!("{}…", &s[..47])
    } else {
        s
    }
}
