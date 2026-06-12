// SPDX-License-Identifier: UNLICENSED
//! # `mailbox` — **messagerie ciblée agent↔agent** : la lane fiable, ADRESSÉE (ADR-015).
//!
//! « Tel agent dit telle chose à tel autre » — sans trahir le keystone. Une boîte aux
//! lettres est un **namespace du Blackboard** (`/mail/<to>/<seq>`, write-once, plafonné,
//! transient d'époque) ; un envoi est une **écriture de vérité** suivie d'un
//! **réveil-indice** (`mail.<to>`, best-effort — la perte de 100 % des notifs ne perd
//! aucun courrier) ; une réception est un **drainage par curseur** (exactly-once,
//! in-order). Spécialisation de [`crate::reliable`] (WS1-F2) — AUCUNE primitive
//! nouvelle au substrat. Architecture :
//! `docs/reports/EFE_Archi_Messagerie_Ciblee_2026-06-12`.
//!
//! ## Ce qui est REJETÉ, par construction (architecture §3)
//! Pas un canal (`mpsc`) : un message en vol ne décide jamais. Pas un bus fiabilisé
//! (acks/retries/DLQ) : la perte d'indice se récupère par réconciliation contre la
//! vérité. Pas une mailbox d'acteur : le courrier vit DANS l'environnement partagé —
//! snapshot, historique, dashboard, provenance.
//!
//! ## Déviation assumée vs l'ancêtre (`reliable.rs`) — le seq SANS gap
//! `ReliableTopic` attribue le seq par `fetch_add` PUIS écrit : un échec d'écriture
//! (cap de boîte — un chemin NORMAL ici, I-MSG5) brûlerait un seq et poserait un gap
//! définitif (drain bloqué, conservateur). Ici le seq est attribué **par la vérité** :
//! boucle CAS `write_if(key(n), 0, …)` — le compteur n'est qu'un cache d'indice ; un
//! envoi refusé au cap n'avance ni la vérité ni le compteur → **jamais de gap par
//! rejet**. Le gap ne reste possible que par le crash-entre-deux de l'ancêtre, dont la
//! fenêtre n'existe plus (l'attribution EST l'écriture).
//!
//! ## Invariants (famille I-MSG)
//! - **I-MSG1** vérité d'abord : la cellule précède la notif ; pas de cellule ⇒ pas de notif.
//! - **I-MSG2** le bus ne décide jamais : 100 % des notifs perdues ⇒ 100 % du courrier
//!   livré (drain au réveil suivant, à la création du lecteur, ou à la fermeture).
//! - **I-MSG3** exactly-once, in-order, par destinataire (curseur ; arrêt au 1er gap).
//! - **I-MSG4** envoi prouvé : l'écriture est ancrée (WriteSink) ; triple concordance
//!   `from` (enveloppe) = `written_by` (cellule) = `agent_id` (record).
//! - **I-MSG5** quotas durs : boîte pleine ⇒ `Err` à l'EXPÉDITEUR (jamais d'éviction
//!   silencieuse de courrier non lu) — caps par namespace, `rejections_total` compté.
//! - **I-MSG6** transience d'époque : [`Mailboxes::new_epoch`] purge `/mail` ET remet
//!   les seq à zéro ; les lecteurs d'une époque précédente sont périmés (à recréer).
//! - **I-MSG7** corrélation prouvée : une `reply` porte la corrélation ET son
//!   producteur déclare la lecture de la cellule de requête (`Letter::as_read` →
//!   convention `_reads` d'ADR-013 → check DAG).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use dashmap::DashMap;
use efe_nano_sdk::AgentId;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::blackboard::Blackboard;
use crate::error::BlackboardError;
use crate::eventbus::{EventBusSubscription, SwarmEvent};
use crate::topic_router::TopicRouter;

/// Préfixe racine du courrier (namespace transient d'époque — parité `/gossip`, `/debate`).
pub const MAIL_ROOT: &str = "/mail";

/// Namespace de la boîte d'un destinataire : `/mail/<to>`.
pub fn mail_ns(to: &AgentId) -> String {
    format!("{MAIL_ROOT}/{}", to.as_str())
}

/// Clé d'une lettre : `/mail/<to>/<seq:020>` (zero-pad : ordre lexical = numérique —
/// la convention `rel_key` de l'ancêtre).
pub fn mail_key(to: &AgentId, seq: u64) -> String {
    format!("{MAIL_ROOT}/{}/{seq:020}", to.as_str())
}

/// Topic de réveil d'un destinataire : `mail.<to>` (le motif topic-par-destinataire,
/// déjà pratiqué par `ballot.<j>` et `shard_topic`).
pub fn mail_topic(to: &AgentId) -> String {
    format!("mail.{}", to.as_str())
}

// ─────────────────────────────────────────────────────────────────────────────
//  MsgKind — l'enveloppe minimale (enum FERMÉ : l'extension est un acte d'ADR)
// ─────────────────────────────────────────────────────────────────────────────

/// Type d'un courrier. Volontairement fermé : pas de TTL, pas de priorités, pas
/// d'acks — les NON-features sont versionnées (ADR-015, architecture §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgKind {
    /// Ouvre une conversation : crée la corrélation, attend une [`MsgKind::Reply`].
    Request,
    /// Répond : PORTE la corrélation de la requête (et, pour être prouvée, déclare
    /// la lecture de la cellule de requête — I-MSG7).
    Reply,
    /// Information sans réponse attendue (pas de corrélation).
    Inform,
}

impl MsgKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            MsgKind::Request => "request",
            MsgKind::Reply => "reply",
            MsgKind::Inform => "inform",
        }
    }
    fn parse(s: &str) -> MsgKind {
        match s {
            "request" => MsgKind::Request,
            "reply" => MsgKind::Reply,
            _ => MsgKind::Inform, // conservateur : un kind inconnu se lit, ne casse pas
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Letter — une cellule de courrier DRAINÉE, avec son adresse exacte
// ─────────────────────────────────────────────────────────────────────────────

/// Une lettre livrée : le contenu ET son adresse exacte (`pointer@version`) — c'est ce
/// qui permet au traitement de **déclarer sa lecture** (convention `_reads`, ADR-013),
/// donc d'entrer dans le DAG de provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct Letter {
    pub seq: u64,
    /// `/mail/<to>/<seq>` — l'adresse de vérité.
    pub pointer: String,
    /// write-once ⇒ 1 — mais PORTÉ depuis la cellule, jamais supposé.
    pub version: u64,
    /// L'expéditeur selon l'ENVELOPPE (champ métier).
    pub from: AgentId,
    /// L'écrivain selon le SUBSTRAT (`written_by` — la signature). I-MSG4 : la triple
    /// concordance `from == signed_by == agent_id(record)` est contrôlable ; un
    /// désaccord ici est un courrier forgé, détectable par le consommateur même.
    pub signed_by: AgentId,
    pub kind: MsgKind,
    pub correlation: Option<Uuid>,
    pub body: Value,
}

impl Letter {
    /// La référence de lecture `{pointer, version}` — l'élément de `_reads` que le
    /// Meter du SDK scelle dans le record du lecteur (agents.rs : `declared_reads`).
    pub fn as_read(&self) -> Value {
        json!({ "pointer": self.pointer, "version": self.version })
    }

    /// La tâche d'un agent métré, AUGMENTÉE de la déclaration de lecture de cette
    /// lettre (`_reads` ajouté ou étendu à la racine). C'est la convention de
    /// **drainage métré** : une lettre entre dans la tâche, jamais à côté de la preuve.
    pub fn attach_to_task(&self, task: &Value) -> Value {
        let mut m = match task {
            Value::Object(o) => o.clone(),
            other => {
                let mut m = Map::new();
                m.insert("task".into(), other.clone());
                m
            }
        };
        let mut reads = match m.remove("_reads") {
            Some(Value::Array(a)) => a,
            _ => Vec::new(),
        };
        reads.push(self.as_read());
        m.insert("_reads".into(), Value::Array(reads));
        Value::Object(m)
    }

    /// Parse une cellule de boîte en lettre. Une cellule qui n'est pas une enveloppe
    /// (misuse du namespace) se lit en `Inform` conservateur — `from` retombe sur la
    /// signature substrat, le corps brut est conservé : rien ne se perd, rien ne panique.
    fn from_cell(seq: u64, pointer: String, version: u64, signed_by: AgentId, value: Value) -> Letter {
        let from = value
            .get("from")
            .and_then(Value::as_str)
            .and_then(|s| AgentId::new(s).ok())
            .unwrap_or_else(|| signed_by.clone());
        let kind = value.get("kind").and_then(Value::as_str).map(MsgKind::parse).unwrap_or(MsgKind::Inform);
        let correlation =
            value.get("correlation").and_then(Value::as_str).and_then(|s| Uuid::parse_str(s).ok());
        let body = value.get("body").cloned().unwrap_or(value);
        Letter { seq, pointer, version, from, signed_by, kind, correlation, body }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Mailboxes — le registre (un seq par boîte, multi-expéditeurs)
// ─────────────────────────────────────────────────────────────────────────────

/// Registre des boîtes aux lettres d'un substrat. Clonable à travers `Arc` ; le seq de
/// chaque boîte est un cache partagé entre TOUS les expéditeurs — la vérité (CAS
/// write-once) reste l'arbitre de l'attribution.
#[derive(Default)]
pub struct Mailboxes {
    seqs: DashMap<String, Arc<AtomicU64>>,
}

impl std::fmt::Debug for Mailboxes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mailboxes").field("boxes", &self.seqs.len()).finish()
    }
}

impl Mailboxes {
    pub fn new() -> Self {
        Self::default()
    }

    fn seq_for(&self, to: &AgentId) -> Arc<AtomicU64> {
        self.seqs.entry(to.as_str().to_string()).or_insert_with(|| Arc::new(AtomicU64::new(0))).clone()
    }

    /// **L'envoi** — VÉRITÉ d'abord, INDICE ensuite (I-MSG1).
    ///
    /// Le seq est attribué par la vérité : boucle `write_if(key(n), 0, …)` — un
    /// `Conflict` signifie qu'un expéditeur concurrent a pris `n` (on retente `n+1`) ;
    /// toute autre erreur (cap de boîte — I-MSG5) est rendue à l'expéditeur SANS
    /// avancer quoi que ce soit : jamais de gap par rejet. Renvoie le seq attribué.
    #[allow(clippy::too_many_arguments)] // l'enveloppe est explicite : qui, à qui, quoi, corrélé comment
    pub fn send(
        &self,
        router: &TopicRouter,
        bb: &Blackboard,
        from: &AgentId,
        to: &AgentId,
        kind: MsgKind,
        correlation: Option<Uuid>,
        body: Value,
    ) -> Result<u64, BlackboardError> {
        let seq = self.seq_for(to);
        let mut envelope = Map::new();
        envelope.insert("from".into(), json!(from.as_str()));
        envelope.insert("kind".into(), json!(kind.as_str()));
        if let Some(c) = correlation {
            envelope.insert("correlation".into(), json!(c.to_string()));
        }
        if kind == MsgKind::Request {
            // où répondre : l'AgentId — le chemin (/mail/<from>) se DÉRIVE, jamais stocké.
            envelope.insert("reply_to".into(), json!(from.as_str()));
        }
        envelope.insert("body".into(), body);
        let envelope = Value::Object(envelope);

        let assigned = loop {
            let candidate = seq.load(Ordering::Relaxed) + 1;
            match bb.write_if(&mail_key(to, candidate), 0, envelope.clone(), from.clone()) {
                Ok(_) => {
                    seq.fetch_max(candidate, Ordering::Relaxed);
                    break candidate;
                }
                Err(BlackboardError::Conflict { .. }) => {
                    // un expéditeur concurrent a pris `candidate` : avancer le cache, retenter.
                    seq.fetch_max(candidate, Ordering::Relaxed);
                }
                Err(e) => return Err(e), // cap/pointeur : l'expéditeur est INFORMÉ (I-MSG5)
            }
        };

        // INDICE ensuite — best-effort : 0 abonné, canal plein, tout est tolérable (I-MSG2).
        let _ = router.publish(SwarmEvent {
            swarm_id: bb.swarm_id(),
            published_by: from.clone(),
            topic: mail_topic(to),
            payload: json!({ "seq": assigned, "ns": mail_ns(to) }),
            published_at: SystemTime::now(),
        });
        Ok(assigned)
    }

    /// Sucre : ouvre une conversation — `send(kind=Request)` avec une corrélation
    /// fraîche, rendue à l'appelant pour [`MailReader::await_reply`].
    #[allow(clippy::too_many_arguments)] // miroir de send : paramètres explicites
    pub fn request(
        &self,
        router: &TopicRouter,
        bb: &Blackboard,
        from: &AgentId,
        to: &AgentId,
        body: Value,
    ) -> Result<(u64, Uuid), BlackboardError> {
        let c = Uuid::new_v4();
        self.send(router, bb, from, to, MsgKind::Request, Some(c), body).map(|seq| (seq, c))
    }

    /// Sucre : répond — `send(kind=Reply)` portant la corrélation de la requête.
    #[allow(clippy::too_many_arguments)] // miroir de send
    pub fn reply(
        &self,
        router: &TopicRouter,
        bb: &Blackboard,
        from: &AgentId,
        to: &AgentId,
        correlation: Uuid,
        body: Value,
    ) -> Result<u64, BlackboardError> {
        self.send(router, bb, from, to, MsgKind::Reply, Some(correlation), body)
    }

    /// Le lecteur de SA boîte : curseur à zéro, abonné à `mail.<me>` — à créer AVANT
    /// d'attendre des réveils (un courrier antérieur est rattrapé par le drain
    /// d'amorçage de [`MailReader::recv_batch`]).
    pub fn reader(&self, router: &TopicRouter, me: &AgentId) -> MailReader {
        MailReader {
            ns: mail_ns(me),
            sub: router.subscribe(&mail_topic(me)),
            cursor: 0,
            finished: false,
            set_aside: VecDeque::new(),
        }
    }

    /// **I-MSG6** : nouvelle époque — purge `/mail` (contrat des namespaces
    /// transients) ET remet les seq à zéro. Les lecteurs existants sont PÉRIMÉS
    /// (leur curseur précède une boîte vide) : à recréer, comme tout état transient.
    pub fn new_epoch(&self, bb: &Blackboard) {
        bb.new_epoch(&[MAIL_ROOT]);
        self.seqs.clear();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  MailReader — le drainage par curseur (exactly-once, in-order), adressé
// ─────────────────────────────────────────────────────────────────────────────

/// Lecteur d'une boîte : réconcilie contre la vérité par point-reads (O(livrés)) ;
/// la notification ne sert que de réveil (sa perte est sans conséquence — I-MSG2).
pub struct MailReader {
    ns: String,
    sub: EventBusSubscription,
    cursor: u64,
    finished: bool,
    /// **Mise de côté** ([`MailReader::await_reply`]) : lettres déjà drainées mais non
    /// réclamées — re-livrées EN TÊTE du prochain drain, ordre préservé. L'exactly-once
    /// survit à l'attente sélective : rien n'est relu du Blackboard.
    set_aside: VecDeque<Letter>,
}

impl std::fmt::Debug for MailReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MailReader")
            .field("ns", &self.ns)
            .field("cursor", &self.cursor)
            .field("set_aside", &self.set_aside.len())
            .field("finished", &self.finished)
            .finish()
    }
}

impl MailReader {
    /// Dernier seq contigu livré.
    pub fn cursor(&self) -> u64 {
        self.cursor
    }

    /// Draine : la mise de côté d'abord (ordre préservé), puis tous les seq contigus
    /// présents au-delà du curseur. S'arrête au premier gap (I-MSG3, conservateur).
    pub fn drain(&mut self, bb: &Blackboard) -> Vec<Letter> {
        let mut out: Vec<Letter> = self.set_aside.drain(..).collect();
        loop {
            let next = self.cursor + 1;
            let key = format!("{}/{next:020}", self.ns);
            match bb.read(&key) {
                Some(cell) => {
                    out.push(Letter::from_cell(next, key, cell.version, cell.written_by, cell.value));
                    self.cursor = next;
                }
                None => break,
            }
        }
        out
    }

    /// Attend du courrier : drain d'amorçage immédiat (rattrape les lettres
    /// antérieures à l'abonnement — pas de réveil perdu), puis réveil → drain.
    /// À la fermeture du bus : dernier drain (la queue), puis `None`.
    pub async fn recv_batch(&mut self, bb: &Blackboard) -> Option<Vec<Letter>> {
        if self.finished {
            return None;
        }
        let bootstrap = self.drain(bb);
        if !bootstrap.is_empty() {
            return Some(bootstrap);
        }
        loop {
            match self.sub.recv().await {
                Some(_wakeup) => {
                    let batch = self.drain(bb);
                    if !batch.is_empty() {
                        return Some(batch);
                    } // réveil sans nouveauté (notif en doublon) → réattendre.
                }
                None => {
                    self.finished = true;
                    let batch = self.drain(bb);
                    return if batch.is_empty() { None } else { Some(batch) };
                }
            }
        }
    }

    /// **Le motif requête/réponse** : attend LA réponse corrélée `c`. Les autres
    /// lettres sont mises de côté (jamais perdues — re-livrées au prochain drain).
    /// `deadline` est OBLIGATOIRE : le contrat ne promet jamais qu'une réponse
    /// existe — seulement que si elle est écrite, elle est livrée. `None` au délai.
    pub async fn await_reply(&mut self, bb: &Blackboard, c: Uuid, deadline: Duration) -> Option<Letter> {
        let until = tokio::time::Instant::now() + deadline;
        loop {
            // draine ce qui est là (mise de côté comprise), cherche LA réponse.
            let batch = self.drain(bb);
            let mut found: Option<Letter> = None;
            for letter in batch {
                if found.is_none() && letter.kind == MsgKind::Reply && letter.correlation == Some(c) {
                    found = Some(letter);
                } else {
                    self.set_aside.push_back(letter); // ordre préservé
                }
            }
            if found.is_some() {
                return found;
            }
            if self.finished {
                return None;
            }
            // attend un réveil, borné par la deadline.
            match tokio::time::timeout_at(until, self.sub.recv()).await {
                Ok(Some(_wakeup)) => continue,
                Ok(None) => {
                    // bus fermé : un DERNIER drain à l'itération suivante, puis None.
                    self.finished = true;
                }
                Err(_elapsed) => {
                    // délai échu : un dernier drain (la réponse a pu arriver notif perdue).
                    let batch = self.drain(bb);
                    let mut found = None;
                    for letter in batch {
                        if found.is_none()
                            && letter.kind == MsgKind::Reply
                            && letter.correlation == Some(c)
                        {
                            found = Some(letter);
                        } else {
                            self.set_aside.push_back(letter);
                        }
                    }
                    return found; // Some si in extremis, sinon None — honnête.
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Tests — les gardiens I-MSG (keyless, sans réseau)
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::streaming::QuiescenceTracker;
    use efe_nano_sdk::SwarmId;

    fn aid(s: &str) -> AgentId {
        AgentId::new(s).unwrap()
    }
    fn substrate() -> (Arc<Blackboard>, Arc<TopicRouter>) {
        (Arc::new(Blackboard::new(SwarmId::new())), Arc::new(TopicRouter::new(256)))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mail_is_truth_first_no_cell_no_notify() {
        // I-MSG1/I-MSG5 : boîte plafonnée à ~1 lettre — le 2e envoi échoue au cap :
        // l'expéditeur reçoit Err, AUCUNE notif ne part, AUCUN gap n'est créé.
        let bb = Arc::new(Blackboard::with_namespaces(
            SwarmId::new(),
            1 << 20,
            vec![(mail_ns(&aid("bob")), 60)], // ~UNE enveloppe courte (la 2e dépasse)
        ));
        let router = Arc::new(TopicRouter::new(16));
        let boxes = Mailboxes::new();
        let (alice, bob) = (aid("alice"), aid("bob"));

        let s1 = boxes.send(&router, &bb, &alice, &bob, MsgKind::Inform, None, json!({ "n": 1 }));
        assert_eq!(s1.unwrap(), 1);
        assert_eq!(router.published_total(), 1);

        let s2 = boxes.send(&router, &bb, &alice, &bob, MsgKind::Inform, None, json!({ "n": 2 }));
        assert!(matches!(s2, Err(BlackboardError::SizeLimitExceeded { .. })), "cap → Err à l'expéditeur");
        assert_eq!(router.published_total(), 1, "pas de cellule ⇒ PAS de notif (I-MSG1)");
        assert!(bb.rejections() >= 1, "le refus est compté");

        // le courrier non lu est INTACT, et l'échec n'a brûlé aucun seq : après
        // drainage (la place se libère à l'époque, pas avant — on vérifie juste
        // qu'un envoi vers une AUTRE boîte démarre bien à 1, sans pollution).
        let mut reader = boxes.reader(&router, &bob);
        let letters = reader.drain(&bb);
        assert_eq!(letters.len(), 1, "jamais d'éviction silencieuse (I-MSG5)");
        assert_eq!(letters[0].body, json!({ "n": 1 }));
        let s3 = boxes.send(&router, &bb, &alice, &aid("carol"), MsgKind::Inform, None, json!({}));
        assert_eq!(s3.unwrap(), 1, "boîtes indépendantes, seq par destinataire");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn all_mail_delivered_despite_total_notify_loss() {
        // I-MSG2 : routeur de capacité 1, AUCUN abonné au moment des envois — la
        // quasi-totalité des notifs part dans le vide. 100 % du courrier est livré.
        let (bb, _big) = substrate();
        let router = Arc::new(TopicRouter::new(1)); // le bus le plus hostile possible
        let boxes = Mailboxes::new();
        let (alice, bob) = (aid("alice"), aid("bob"));
        for i in 0..200u64 {
            boxes.send(&router, &bb, &alice, &bob, MsgKind::Inform, None, json!({ "i": i })).unwrap();
        }
        // lecteur créé APRÈS coup : il n'a vu AUCUNE notif — le drain d'amorçage suffit.
        let mut reader = boxes.reader(&router, &bob);
        let letters = reader.recv_batch(&bb).await.expect("amorçage");
        assert_eq!(letters.len(), 200, "tout est livré malgré la perte totale d'indice");
        for (i, l) in letters.iter().enumerate() {
            assert_eq!(l.seq, i as u64 + 1, "in-order");
            assert_eq!(l.body["i"], json!(i as u64));
        }
        assert_eq!(reader.cursor(), 200);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cursor_is_exactly_once_and_gap_conservative() {
        // I-MSG3 : un gap manufacturé (écriture directe hors Mailboxes) bloque le
        // drain — conservateur ; combler le gap reprend la livraison, sans doublon.
        let (bb, router) = substrate();
        let bob = aid("bob");
        let writer = aid("writer");
        bb.write(&mail_key(&bob, 1), json!({ "from": "writer", "kind": "inform", "body": 1 }), writer.clone()).unwrap();
        bb.write(&mail_key(&bob, 3), json!({ "from": "writer", "kind": "inform", "body": 3 }), writer.clone()).unwrap();
        let boxes = Mailboxes::new();
        let mut reader = boxes.reader(&router, &bob);
        let first = reader.drain(&bb);
        assert_eq!(first.len(), 1, "s'arrête au gap (seq 2 absent)");
        assert_eq!(first[0].seq, 1);
        // comble le gap → le drain reprend 2 PUIS 3, exactement une fois chacun.
        bb.write(&mail_key(&bob, 2), json!({ "from": "writer", "kind": "inform", "body": 2 }), writer).unwrap();
        let rest = reader.drain(&bb);
        assert_eq!(rest.iter().map(|l| l.seq).collect::<Vec<_>>(), vec![2, 3]);
        assert!(reader.drain(&bb).is_empty(), "exactly-once : rien à relivrer");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn multi_sender_single_box_is_seq_monotone() {
        // 8 expéditeurs concurrents × 50 lettres vers UNE boîte : l'attribution par la
        // vérité (CAS) sérialise — 400 seq exacts, contigus, chacun une fois.
        let (bb, router) = substrate();
        let boxes = Arc::new(Mailboxes::new());
        let bob = aid("bob");
        let mut handles = Vec::new();
        for s in 0..8 {
            let (bb, router, boxes, bob) = (bb.clone(), router.clone(), boxes.clone(), bob.clone());
            handles.push(tokio::spawn(async move {
                let me = aid(&format!("sender_{s}"));
                for i in 0..50 {
                    boxes
                        .send(&router, &bb, &me, &bob, MsgKind::Inform, None, json!({ "s": s, "i": i }))
                        .unwrap();
                }
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        let mut reader = boxes.reader(&router, &bob);
        let letters = reader.drain(&bb);
        assert_eq!(letters.len(), 400);
        for (i, l) in letters.iter().enumerate() {
            assert_eq!(l.seq, i as u64 + 1, "seq contigus, monotones, sans trou ni doublon");
            assert_eq!(l.version, 1, "write-once");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn epoch_purges_mail_and_resets_seq() {
        // I-MSG6 : new_epoch purge /mail et remet les seq à zéro.
        let (bb, router) = substrate();
        let boxes = Mailboxes::new();
        let (alice, bob) = (aid("alice"), aid("bob"));
        for _ in 0..3 {
            boxes.send(&router, &bb, &alice, &bob, MsgKind::Inform, None, json!({})).unwrap();
        }
        boxes.new_epoch(&bb);
        assert!(bb.read(&mail_key(&bob, 1)).is_none(), "courrier purgé");
        let s = boxes.send(&router, &bb, &alice, &bob, MsgKind::Inform, None, json!({ "ep": 2 })).unwrap();
        assert_eq!(s, 1, "seq remis à zéro");
        // lecteur FRAIS (les anciens sont périmés par contrat) : il lit l'époque neuve.
        let mut reader = boxes.reader(&router, &bob);
        let letters = reader.drain(&bb);
        assert_eq!(letters.len(), 1);
        assert_eq!(letters[0].body["ep"], json!(2));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn await_reply_sets_aside_other_letters_without_loss() {
        // I-MSG : la mise de côté — await_reply rend LA réponse corrélée ; les autres
        // lettres re-livrées au prochain drain, ordre préservé, exactly-once.
        let (bb, router) = substrate();
        let boxes = Mailboxes::new();
        let (alice, bob) = (aid("alice"), aid("bob"));
        let mut alice_box = boxes.reader(&router, &alice);
        let c = Uuid::new_v4();
        boxes.send(&router, &bb, &bob, &alice, MsgKind::Inform, None, json!("A")).unwrap();
        boxes.send(&router, &bb, &bob, &alice, MsgKind::Inform, None, json!("B")).unwrap();
        boxes.reply(&router, &bb, &bob, &alice, c, json!("LA réponse")).unwrap();
        boxes.send(&router, &bb, &bob, &alice, MsgKind::Inform, None, json!("C")).unwrap();

        let reply = alice_box.await_reply(&bb, c, Duration::from_millis(200)).await.expect("réponse");
        assert_eq!(reply.body, json!("LA réponse"));
        assert_eq!(reply.kind, MsgKind::Reply);
        let rest = alice_box.drain(&bb);
        assert_eq!(
            rest.iter().map(|l| l.body.clone()).collect::<Vec<_>>(),
            vec![json!("A"), json!("B"), json!("C")],
            "mises de côté re-livrées en ordre, rien de perdu, rien de relu"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn await_reply_times_out_honestly_none() {
        // le contrat ne promet JAMAIS qu'une réponse existe : délai échu → None,
        // et le courrier non-réclamé reste livrable.
        let (bb, router) = substrate();
        let boxes = Mailboxes::new();
        let (alice, bob) = (aid("alice"), aid("bob"));
        let mut alice_box = boxes.reader(&router, &alice);
        boxes.send(&router, &bb, &bob, &alice, MsgKind::Inform, None, json!("pendant ce temps")).unwrap();
        let t0 = std::time::Instant::now();
        let none = alice_box.await_reply(&bb, Uuid::new_v4(), Duration::from_millis(60)).await;
        assert!(none.is_none(), "pas de réponse → None, honnêtement");
        assert!(t0.elapsed() >= Duration::from_millis(55), "la deadline a réellement borné");
        assert_eq!(alice_box.drain(&bb).len(), 1, "la mise de côté a survécu au timeout");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn drain_after_router_dropped_recovers_tail() {
        // fermeture du bus : recv_batch fait un DERNIER drain (la queue), puis None.
        let (bb, router) = substrate();
        let boxes = Mailboxes::new();
        let (alice, bob) = (aid("alice"), aid("bob"));
        let mut reader = boxes.reader(&router, &bob);
        // amorçage à vide d'abord (personne n'a écrit) — puis le bus meurt APRÈS des envois.
        boxes.send(&router, &bb, &alice, &bob, MsgKind::Inform, None, json!(1)).unwrap();
        boxes.send(&router, &bb, &alice, &bob, MsgKind::Inform, None, json!(2)).unwrap();
        drop(router);
        drop(boxes);
        let batch = reader.recv_batch(&bb).await.expect("amorçage rattrape avant la fermeture");
        assert_eq!(batch.len(), 2);
        assert!(reader.recv_batch(&bb).await.is_none(), "bus fermé + boîte vide → None");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wakeup_path_delivers_while_reader_waits() {
        // le chemin RÉVEIL (pas seulement l'amorçage) : un lecteur en attente est
        // réveillé par la notif et draine la lettre.
        let (bb, router) = substrate();
        let boxes = Arc::new(Mailboxes::new());
        let (alice, bob) = (aid("alice"), aid("bob"));
        let mut reader = boxes.reader(&router, &bob);
        let waiter = tokio::spawn({
            let bb = bb.clone();
            async move { reader.recv_batch(&bb).await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await; // le lecteur attend
        boxes.send(&router, &bb, &alice, &bob, MsgKind::Inform, None, json!("réveil")).unwrap();
        let batch = waiter.await.unwrap().expect("réveillé");
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].body, json!("réveil"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn quiescence_counts_addressed_mail() {
        // intégration WS2 : le courrier se crédite comme tout événement — on_emit à
        // l'envoi, on_done au traitement ; la quiescence reste UN seul mécanisme.
        let (bb, router) = substrate();
        let boxes = Mailboxes::new();
        let tracker = QuiescenceTracker::new();
        let (alice, bob) = (aid("alice"), aid("bob"));
        for i in 0..5 {
            boxes.send(&router, &bb, &alice, &bob, MsgKind::Inform, None, json!(i)).unwrap();
            tracker.on_emit(1);
        }
        tracker.mark_sources_done();
        assert!(!tracker.is_quiescent(), "du courrier en vol : pas quiescent");
        let mut reader = boxes.reader(&router, &bob);
        for letter in reader.drain(&bb) {
            let _ = letter; // « traitement »
            tracker.on_done();
        }
        assert!(tracker.is_quiescent(), "tout courrier traité + sources finies = quiescent");
    }

    #[test]
    fn letter_attach_to_task_builds_the_reads_convention() {
        // la convention de drainage MÉTRÉ : _reads ajouté/étendu à la racine — la
        // forme exacte que le Meter du SDK scelle (agents.rs : declared_reads).
        let l = Letter {
            seq: 7,
            pointer: "/mail/bob/00000000000000000007".into(),
            version: 1,
            from: aid("alice"),
            signed_by: aid("alice"),
            kind: MsgKind::Request,
            correlation: Some(Uuid::from_u128(0xC0DE)),
            body: json!({ "q": "?" }),
        };
        let t1 = l.attach_to_task(&json!({ "text": "x" }));
        assert_eq!(t1["_reads"][0]["pointer"], json!("/mail/bob/00000000000000000007"));
        assert_eq!(t1["_reads"][0]["version"], json!(1));
        assert_eq!(t1["text"], json!("x"));
        // s'ÉTEND sans écraser une déclaration existante.
        let t2 = l.attach_to_task(&t1);
        assert_eq!(t2["_reads"].as_array().unwrap().len(), 2);
        // une tâche non-objet est enveloppée, jamais perdue.
        let t3 = l.attach_to_task(&json!("nu"));
        assert_eq!(t3["task"], json!("nu"));
        assert_eq!(t3["_reads"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn malformed_cell_reads_as_conservative_inform() {
        // une cellule /mail qui n'est pas une enveloppe (misuse) se lit sans paniquer :
        // Inform, from = signature substrat, corps brut conservé.
        let l = Letter::from_cell(1, "/mail/bob/x".into(), 1, AgentId::new("writer").unwrap(), json!([1, 2]));
        assert_eq!(l.kind, MsgKind::Inform);
        assert_eq!(l.from.as_str(), "writer");
        assert_eq!(l.body, json!([1, 2]));
    }
}
