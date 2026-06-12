// SPDX-License-Identifier: UNLICENSED
//! **ADR-015 — le moat conversationnel** : « qui a dit quoi à qui, et qui l'a lu »,
//! re-prouvable hors ligne depuis les seuls records — SANS un champ nouveau au format
//! gelé. Composition pure : envoi = écriture ancrée (WriteSink + appariement
//! déterministe ADR-014) ; lecture = déclaration scellée (`reads`, contrôlée par le
//! check DAG anti-version-fantôme) ; réponse = corrélation PORTÉE + lecture de la
//! cellule de requête (I-MSG7 : la causalité est dans le DAG, pas annotée).

use std::sync::Arc;
use std::time::Duration;

use efe_blackboard::{mail_key, MailReader, Mailboxes, MsgKind, TopicRouter};
use efe_nano_sdk::{AgentId, SwarmId, TenantId};
use efe_swarm_provenance::{
    verify_swarm, AgentKind, AgentRole, CallCapture, HmacKeySource, ProvenanceRecorder,
    ProvenanceSink,
};
use serde_json::json;

fn aid(s: &str) -> AgentId {
    AgentId::new(s).unwrap()
}

fn key() -> Vec<u8> {
    HmacKeySource::test_key().derive(TenantId::system()).unwrap()
}

/// Le banc : un substrat instrumenté (chaque écriture /mail ancrée au recorder).
fn instrumented() -> (Arc<efe_blackboard::Blackboard>, Arc<TopicRouter>, Arc<ProvenanceRecorder>, Mailboxes) {
    let swarm_id = SwarmId::new();
    let bb = Arc::new(efe_blackboard::Blackboard::new(swarm_id));
    let recorder = Arc::new(ProvenanceRecorder::new(swarm_id, TenantId::system()));
    bb.set_write_sink(Arc::new(ProvenanceSink::new(recorder.clone())));
    (bb, Arc::new(TopicRouter::new(256)), recorder, Mailboxes::new())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_is_anchored_and_from_matches_writer() {
    // **I-MSG4** : l'envoi est PROUVÉ — la cellule de courrier est ancrée au record de
    // l'expéditeur, et la triple concordance tient :
    //   from (enveloppe) == written_by (cellule) == agent_id (record qui écrit).
    let (bb, router, recorder, boxes) = instrumented();
    let (alice, bob) = (aid("alice"), aid("bob"));

    // l'appel d'agent d'alice (sémantique : ce qu'elle a calculé) PUIS son envoi
    // (l'écriture, ancrée par le sink) — le motif Meter, composé à la main.
    let task = json!({ "text": "delegate this" });
    let out = json!({ "decision": "ask bob" });
    recorder.capture(CallCapture::new(alice.clone(), AgentRole::Generator, AgentKind::ModeA, &task, &out));
    let seq = boxes
        .send(&router, &bb, &alice, &bob, MsgKind::Inform, None, json!({ "do": "shard 7" }))
        .unwrap();
    assert_eq!(seq, 1);

    let sealed = recorder.seal_with_key(key());
    assert_eq!(sealed.records.len(), 1);
    let rec = &sealed.records[0];
    // l'ancre : le record d'alice porte l'écriture de LA cellule de courrier.
    assert_eq!(rec.agent_id.as_str(), "alice");
    assert_eq!(rec.writes.len(), 1);
    assert_eq!(rec.writes[0].pointer, mail_key(&bob, 1));
    assert_eq!(rec.writes[0].version, 1, "write-once");
    // triple concordance, contrôlée depuis la VÉRITÉ + l'enveloppe + le record.
    let cell = bb.read(&mail_key(&bob, 1)).expect("la vérité");
    assert_eq!(cell.written_by.as_str(), "alice", "signature substrat");
    assert_eq!(cell.value["from"], json!("alice"), "champ d'enveloppe");
    assert_eq!(rec.agent_id, cell.written_by, "record == signature");
    // et le tout se re-prouve.
    let report = verify_swarm(&sealed.records, &key());
    assert!(report.ok(), "chaîne + contenu + DAG");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reply_correlates_and_declares_reading_the_request() {
    // **I-MSG7** : la réponse de bob PORTE la corrélation ET son record DÉCLARE la
    // lecture de la cellule de requête — la causalité conversationnelle est dans le
    // DAG. Un `reads` forgé (version fantôme) casse la re-preuve : la corrélation
    // sans la lecture déclarée est un mensonge détectable.
    let (bb, router, recorder, boxes) = instrumented();
    let (alice, bob) = (aid("alice"), aid("bob"));

    // ① alice ouvre la conversation (requête ancrée à son record).
    recorder.capture(CallCapture::new(
        alice.clone(),
        AgentRole::Generator,
        AgentKind::ModeA,
        &json!({ "text": "need an estimate" }),
        &json!({ "asked": true }),
    ));
    let (req_seq, c) = boxes.request(&router, &bb, &alice, &bob, json!({ "q": "estimate?" })).unwrap();

    // ② bob draine, traite MÉTRÉ (sa tâche porte letter.as_read() — la convention de
    //    drainage métré), répond (réponse ancrée à son record).
    let mut bob_box = boxes.reader(&router, &bob);
    let letters = bob_box.drain(&bb);
    assert_eq!(letters.len(), 1);
    let req = &letters[0];
    assert_eq!(req.kind, MsgKind::Request);
    assert_eq!(req.correlation, Some(c));
    let bob_task = req.attach_to_task(&json!({ "instruction": "estimate" }));
    let bob_out = json!({ "estimate": 42 });
    let mut cap = CallCapture::new(bob.clone(), AgentRole::Stage, AgentKind::ModeA, &bob_task, &bob_out);
    // le lift du Meter (agents.rs : declared_reads), composé à la main au même format.
    for r in bob_task["_reads"].as_array().unwrap() {
        cap = cap.reading(r["pointer"].as_str().unwrap(), r["version"].as_u64().unwrap());
    }
    recorder.capture(cap);
    boxes.reply(&router, &bb, &bob, &alice, c, bob_out.clone()).unwrap();

    // ③ alice reçoit LA réponse corrélée.
    let mut alice_box = boxes.reader(&router, &alice);
    let reply = alice_box.await_reply(&bb, c, Duration::from_millis(200)).await.expect("réponse");
    assert_eq!(reply.body, bob_out);
    assert_eq!(reply.from.as_str(), "bob");

    // ④ la preuve : scellé, re-prouvé — et le record de bob LIT la cellule de requête.
    let sealed = recorder.seal_with_key(key());
    assert_eq!(sealed.records.len(), 2, "alice (requête) + bob (réponse)");
    let bob_rec = sealed.records.iter().find(|r| r.agent_id.as_str() == "bob").expect("record bob");
    assert_eq!(bob_rec.reads.len(), 1, "la lecture de la requête est SCELLÉE");
    assert_eq!(bob_rec.reads[0].pointer, mail_key(&bob, req_seq));
    assert_eq!(bob_rec.reads[0].version, 1);
    assert_eq!(bob_rec.writes[0].pointer, mail_key(&alice, 1), "et sa réponse est ANCRÉE");
    let report = verify_swarm(&sealed.records, &key());
    assert!(report.ok() && report.dag_consistent, "le DAG voit la causalité conversationnelle");

    // ⑤ contre-épreuve : une lecture FORGÉE (version fantôme d'une cellule SCELLÉE)
    //    est détectée par le check DAG. Sémantique exacte du check (verify.rs :
    //    check_dag (b)) : il contrôle les versions des pointeurs que les records ont
    //    ÉCRITS — l'expéditeur doit donc être capturé (son écriture scellée) pour que
    //    le mensonge du lecteur soit jugeable ; une lecture d'un pointeur que personne
    //    n'a scellé reste une « lecture externe », tolérée.
    let (bb2, router2, recorder2, boxes2) = instrumented();
    let forger_sender = aid("sender");
    recorder2.capture(CallCapture::new(
        forger_sender.clone(),
        AgentRole::Generator,
        AgentKind::ModeA,
        &json!({}),
        &json!({}),
    ));
    boxes2.send(&router2, &bb2, &forger_sender, &bob, MsgKind::Inform, None, json!({})).unwrap();
    let mut forged = CallCapture::new(
        bob.clone(),
        AgentRole::Stage,
        AgentKind::ModeA,
        &json!({ "x": 1 }),
        &json!({ "y": 2 }),
    );
    forged = forged.reading(mail_key(&bob, 1), 99); // la cellule scellée est @1 — 99 est un fantôme
    recorder2.capture(forged);
    let sealed2 = recorder2.seal_with_key(key());
    let report2 = verify_swarm(&sealed2.records, &key());
    assert!(!report2.dag_consistent, "version fantôme → DAG inconsistant (le mensonge se voit)");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_reply_end_to_end_reproves_offline() {
    // Le motif COMPLET, en conditions réelles de concurrence : alice interroge deux
    // workers en parallèle, chacun draine sa boîte par RÉVEIL, traite métré, répond ;
    // alice corrèle les deux réponses. Puis un TIERS, muni des seuls records + clé,
    // re-prouve la conversation entière — l'argument du moat, conversationnel.
    let (bb, router, recorder, boxes) = instrumented();
    let boxes = Arc::new(boxes);
    let alice = aid("alice");
    let workers = [aid("worker_a"), aid("worker_b")];

    // les workers, EN ATTENTE (chemin réveil, pas amorçage).
    let mut handles = Vec::new();
    for w in &workers {
        let (bb, router, boxes, recorder, w) =
            (bb.clone(), router.clone(), boxes.clone(), recorder.clone(), w.clone());
        let mut my_box: MailReader = boxes.reader(&router, &w);
        handles.push(tokio::spawn(async move {
            let letters = my_box.recv_batch(&bb).await.expect("réveillé par la requête");
            for req in letters {
                let c = req.correlation.expect("requête corrélée");
                let reply_to = req.from.clone();
                let task = req.attach_to_task(&json!({ "work": "double it" }));
                let n = req.body["n"].as_u64().unwrap();
                let out = json!({ "n2": n * 2 });
                let mut cap =
                    CallCapture::new(w.clone(), AgentRole::Stage, AgentKind::ModeA, &task, &out);
                for r in task["_reads"].as_array().unwrap() {
                    cap = cap.reading(r["pointer"].as_str().unwrap(), r["version"].as_u64().unwrap());
                }
                recorder.capture(cap);
                boxes.reply(&router, &bb, &w, &reply_to, c, out).unwrap();
            }
        }));
    }
    tokio::time::sleep(Duration::from_millis(20)).await; // les workers attendent

    // alice interroge les deux, puis corrèle chaque réponse.
    let mut alice_box = boxes.reader(&router, &alice);
    recorder.capture(CallCapture::new(
        alice.clone(),
        AgentRole::Generator,
        AgentKind::ModeA,
        &json!({ "text": "fan-out" }),
        &json!({ "sent": 2 }),
    ));
    let (_s1, c1) = boxes.request(&router, &bb, &alice, &workers[0], json!({ "n": 21 })).unwrap();
    let (_s2, c2) = boxes.request(&router, &bb, &alice, &workers[1], json!({ "n": 100 })).unwrap();
    let r1 = alice_box.await_reply(&bb, c1, Duration::from_secs(2)).await.expect("réponse A");
    let r2 = alice_box.await_reply(&bb, c2, Duration::from_secs(2)).await.expect("réponse B");
    assert_eq!(r1.body["n2"], json!(42));
    assert_eq!(r2.body["n2"], json!(200));
    assert_eq!(r1.from, workers[0]);
    assert_eq!(r2.from, workers[1]);
    for h in handles {
        h.await.unwrap();
    }

    // LE TIERS : records + clé, rien d'autre — la conversation se re-prouve.
    let sealed = recorder.seal_with_key(key());
    assert_eq!(sealed.records.len(), 3, "alice + 2 workers");
    let report = verify_swarm(&sealed.records, &key());
    assert!(report.ok(), "chaîne intacte, contenus intacts, DAG cohérent");
    assert!(report.dag_consistent);
    // et chaque worker prouve « j'ai lu MA requête » : 1 lecture, la bonne cellule.
    for w in &workers {
        let rec = sealed.records.iter().find(|r| &r.agent_id == w).expect("record worker");
        assert_eq!(rec.reads.len(), 1);
        assert_eq!(rec.reads[0].pointer, mail_key(w, 1), "qui a lu quoi : scellé");
        assert!(!rec.writes.is_empty(), "et sa réponse est ancrée");
    }
}
