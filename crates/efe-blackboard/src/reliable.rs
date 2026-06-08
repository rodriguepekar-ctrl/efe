// SPDX-License-Identifier: UNLICENSED
//! **Phase 1 · WS1** — lane de livraison FIABLE (corrige F2 : perte best-effort).
//!
//! La baseline `swarm_load` a montré que le bus best-effort perd ~99 % des
//! événements sous charge. La lane fiable applique le **keystone** d'EFE :
//!
//! > **Le bus est un INDICE** (best-effort, peut tout perdre) — **le Blackboard
//! > est la VÉRITÉ** (durable in-proc). On ne rend pas le bus fiable ; on rend la
//! > perte d'indice RÉCUPÉRABLE par réconciliation contre la vérité.
//!
//! ## Modèle
//! - **Producteur** ([`ReliableTopic::publish`]) : (1) attribue un `seq` monotone
//!   partagé ; (2) écrit le payload au Blackboard sous `"<ns>/<seq>"` — la VÉRITÉ,
//!   d'ABORD ; (3) publie une notif légère `{seq}` sur le topic — l'INDICE.
//! - **Consommateur** ([`ReliableReader`]) : maintient un `cursor` (dernier `seq`
//!   CONTIGU livré). À chaque réveil (notif) — ou à la fermeture du bus — il
//!   **draine** par POINT-READS : `read("<ns>/<cursor+1>")`, `cursor+2`, … tant
//!   que présent. O(livrés), pas O(total).
//!
//! ## Garanties
//! - **At-least-once** : même si TOUTES les notifs sont perdues, un seul réveil
//!   (ou la fermeture) suffit à tout drainer depuis le Blackboard.
//! - **Exactly-once par lecteur** : `cursor` monotone → aucun doublon livré.
//! - **In-order + sûr aux réordonnancements** : le drain s'arrête au 1er `seq`
//!   ABSENT (gap) → un `seq` publié avant que son écriture concurrente soit
//!   committée n'est jamais « sauté » ; il sera livré au réveil suivant.
//!
//! ## Limite (documentée)
//! Un `seq` DÉFINITIVEMENT manquant (producteur crashé après `fetch_add` mais
//! avant `write`) bloque la livraison de la suite (conservateur = ne rien perdre).
//! Une variante prod ajouterait un saut de gap sur timeout. Hors scope WS1.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use efe_nano_sdk::AgentId;
use serde_json::{json, Value};

use crate::blackboard::Blackboard;
use crate::error::BlackboardError;
use crate::eventbus::{EventBusSubscription, SwarmEvent};
use crate::topic_router::TopicRouter;

/// Clé Blackboard d'un item : `"<ns>/<seq zero-paddé>"` (ordre lexical = numérique).
fn rel_key(ns: &str, seq: u64) -> String {
    format!("{ns}/{seq:020}")
}

/// Un item livré de façon fiable.
#[derive(Debug, Clone, PartialEq)]
pub struct ReliableItem {
    pub seq: u64,
    pub agent: AgentId,
    pub payload: Value,
}

/// **WS1** : un flux fiable, ordonné, at-least-once sur un namespace Blackboard,
/// avec le [`TopicRouter`] comme simple réveil best-effort. Clonable (le compteur
/// de séquence est partagé entre tous les producteurs).
#[derive(Clone)]
pub struct ReliableTopic {
    ns: String,
    topic: String,
    seq: Arc<AtomicU64>,
}

impl std::fmt::Debug for ReliableTopic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReliableTopic")
            .field("ns", &self.ns)
            .field("topic", &self.topic)
            .field("seq", &self.seq.load(Ordering::Relaxed))
            .finish()
    }
}

impl ReliableTopic {
    /// `ns` = préfixe de namespace Blackboard (ex. `"/rel/cand"`) ; `topic` = topic
    /// de notification sur le routeur.
    pub fn new(ns: impl Into<String>, topic: impl Into<String>) -> Self {
        Self {
            ns: ns.into(),
            topic: topic.into(),
            seq: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn ns(&self) -> &str {
        &self.ns
    }
    pub fn topic(&self) -> &str {
        &self.topic
    }

    /// **WS1** : publie un item de façon fiable. Écrit la VÉRITÉ (Blackboard)
    /// AVANT l'INDICE (notif). Renvoie le `seq` attribué.
    pub fn publish(
        &self,
        router: &TopicRouter,
        bb: &Blackboard,
        agent: &AgentId,
        payload: Value,
    ) -> Result<u64, BlackboardError> {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1; // 1-based, monotone
        // 1) VÉRITÉ d'abord : si l'écriture échoue (cap), on ne notifie pas.
        bb.write(&rel_key(&self.ns, seq), payload, agent.clone())?;
        // 2) INDICE ensuite : best-effort, perte tolérée (réconciliable).
        let _ = router.publish(SwarmEvent {
            swarm_id: bb.swarm_id(),
            published_by: agent.clone(),
            topic: self.topic.clone(),
            payload: json!({ "seq": seq, "ns": self.ns }),
            published_at: SystemTime::now(),
        });
        Ok(seq)
    }

    /// Crée un lecteur fiable abonné aux notifs de ce topic.
    pub fn reader(&self, router: &TopicRouter) -> ReliableReader {
        ReliableReader {
            ns: self.ns.clone(),
            sub: router.subscribe(&self.topic),
            cursor: 0,
            finished: false,
        }
    }
}

/// **WS1** : lecteur fiable — réconcilie contre le Blackboard par point-reads.
pub struct ReliableReader {
    ns: String,
    sub: EventBusSubscription,
    cursor: u64,
    finished: bool,
}

impl std::fmt::Debug for ReliableReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReliableReader")
            .field("ns", &self.ns)
            .field("cursor", &self.cursor)
            .field("finished", &self.finished)
            .finish()
    }
}

impl ReliableReader {
    /// Dernier `seq` contigu livré.
    pub fn cursor(&self) -> u64 {
        self.cursor
    }

    /// **WS1** : draine (réconciliation) tous les items contigus présents au-delà
    /// du curseur, par point-reads. O(nombre livré). S'arrête au 1er gap.
    pub fn drain(&mut self, bb: &Blackboard) -> Vec<ReliableItem> {
        let mut out = Vec::new();
        loop {
            let next = self.cursor + 1;
            match bb.read(&rel_key(&self.ns, next)) {
                Some(v) => {
                    out.push(ReliableItem {
                        seq: next,
                        agent: v.written_by,
                        payload: v.value,
                    });
                    self.cursor = next;
                }
                None => break,
            }
        }
        out
    }

    /// **WS1** : attend un réveil (notif) puis draine. À la fermeture du bus,
    /// effectue un dernier drain (récupère la queue) puis renvoie `None`.
    ///
    /// At-least-once : la valeur livrée vient TOUJOURS du Blackboard (vérité), la
    /// notif ne sert que de réveil — sa perte est sans conséquence.
    pub async fn recv_batch(&mut self, bb: &Blackboard) -> Option<Vec<ReliableItem>> {
        if self.finished {
            return None;
        }
        loop {
            match self.sub.recv().await {
                Some(_notify) => {
                    let batch = self.drain(bb);
                    if !batch.is_empty() {
                        return Some(batch);
                    }
                    // réveil sans nouveauté (doublon / gap non encore comblé) → réattendre.
                }
                None => {
                    // bus fermé : dernier drain pour la queue.
                    self.finished = true;
                    let batch = self.drain(bb);
                    return if batch.is_empty() { None } else { Some(batch) };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::SwarmId;
    use std::sync::Arc;

    fn agent() -> AgentId {
        AgentId::new("producer").unwrap()
    }

    #[tokio::test]
    async fn in_order_exactly_once_delivery() {
        let bb = Blackboard::new(SwarmId::new());
        let router = TopicRouter::new(64);
        let topic = ReliableTopic::new("/rel/t", "t");
        let mut reader = topic.reader(&router);

        for i in 0..5 {
            topic.publish(&router, &bb, &agent(), json!({ "i": i })).unwrap();
        }
        let batch = reader.drain(&bb);
        let seqs: Vec<u64> = batch.iter().map(|x| x.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3, 4, 5]);
        assert_eq!(reader.cursor(), 5);
        // re-drain → rien (exactly-once par lecteur).
        assert!(reader.drain(&bb).is_empty());
    }

    #[tokio::test]
    async fn at_least_once_despite_all_notifications_lost() {
        // Capacité 1 : en publiant beaucoup sans drainer, les notifs LAGguent et
        // sont perdues. Le drain final récupère TOUT depuis le Blackboard.
        let bb = Blackboard::new(SwarmId::new());
        let router = TopicRouter::new(1);
        let topic = ReliableTopic::new("/rel/c", "c");
        let mut reader = topic.reader(&router); // abonné, mais ne draine pas en flux

        let produced = 200u64;
        for i in 0..produced {
            topic.publish(&router, &bb, &agent(), json!({ "i": i })).unwrap();
        }
        // malgré ~99 % de notifs perdues, le drain récupère 100 %.
        let recovered = reader.drain(&bb);
        assert_eq!(recovered.len() as u64, produced, "at-least-once : aucune perte");
        assert_eq!(reader.cursor(), produced);
    }

    #[tokio::test]
    async fn drain_stops_at_gap_then_resumes() {
        let bb = Blackboard::new(SwarmId::new());
        let router = TopicRouter::new(16);
        let topic = ReliableTopic::new("/rel/g", "g");
        let mut reader = topic.reader(&router);

        // écrit seq 1 et 3 directement (simule un réordonnancement : 2 pas encore là).
        bb.write(&rel_key("/rel/g", 1), json!("a"), agent()).unwrap();
        bb.write(&rel_key("/rel/g", 3), json!("c"), agent()).unwrap();
        let b1 = reader.drain(&bb);
        assert_eq!(b1.iter().map(|x| x.seq).collect::<Vec<_>>(), vec![1], "s'arrête au gap (2 absent)");
        // 2 arrive → drain reprend et livre 2 puis 3.
        bb.write(&rel_key("/rel/g", 2), json!("b"), agent()).unwrap();
        let b2 = reader.drain(&bb);
        assert_eq!(b2.iter().map(|x| x.seq).collect::<Vec<_>>(), vec![2, 3]);
    }

    #[tokio::test]
    async fn recv_batch_terminates_on_bus_close_with_tail() {
        let bb = Arc::new(Blackboard::new(SwarmId::new()));
        let router = Arc::new(TopicRouter::new(64));
        let topic = ReliableTopic::new("/rel/x", "x");
        let mut reader = topic.reader(&router);

        for i in 0..10 {
            topic.publish(&router, &bb, &agent(), json!({ "i": i })).unwrap();
        }
        // ferme le bus → recv_batch fait un dernier drain puis None.
        drop(router);
        let mut total = 0usize;
        while let Some(b) = reader.recv_batch(&bb).await {
            total += b.len();
        }
        assert_eq!(total, 10, "la queue est récupérée à la fermeture");
    }

    #[tokio::test]
    async fn concurrent_producers_shared_sequence_no_loss() {
        // 8 producteurs partagent le même ReliableTopic (compteur seq partagé) ;
        // le drain final récupère exactement N×M items, aucun doublon.
        let bb = Arc::new(Blackboard::new(SwarmId::new()));
        let router = Arc::new(TopicRouter::new(8)); // petit → notifs perdues
        let topic = ReliableTopic::new("/rel/p", "p");
        let mut reader = topic.reader(&router);

        let n = 8u64;
        let per = 100u64;
        let mut handles = Vec::new();
        for p in 0..n {
            let bb = bb.clone();
            let router = router.clone();
            let topic = topic.clone();
            let id = AgentId::new(format!("prod_{p}")).unwrap();
            handles.push(tokio::spawn(async move {
                for i in 0..per {
                    topic.publish(&router, &bb, &id, json!({ "i": i })).unwrap();
                }
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        let recovered = reader.drain(&bb);
        assert_eq!(recovered.len() as u64, n * per, "tous les items, aucune perte");
        // seqs uniques et contigus 1..=N*M.
        let mut seqs: Vec<u64> = recovered.iter().map(|x| x.seq).collect();
        seqs.sort_unstable();
        assert_eq!(seqs.first(), Some(&1));
        assert_eq!(seqs.last(), Some(&(n * per)));
    }
}
