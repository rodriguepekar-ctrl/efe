// SPDX-License-Identifier: UNLICENSED
//! **Phase 1 · WS2** — exécution STREAMING + détection de quiescence (corrige F3).
//!
//! Le dispatcher d'origine exécute par COUCHES (barrière par couche). WS2 offre un
//! modèle où des agents LONG-VIVANTS réagissent à leurs événements d'entrée dès
//! qu'ils arrivent, émettent des enfants, SANS barrière globale — « les infos
//! circulent vraiment ».
//!
//! ## Terminaison = détection de quiescence (le point subtil)
//! Schéma crédit/compteur (Dijkstra-Scholten simplifié) via [`QuiescenceTracker`] :
//! - `inflight` (AtomicI64). Émettre un événement → `+1` ; l'avoir CONSOMMÉ → `−1`.
//! - **Sûreté** : tout émis (`on_emit`) PRÉCÈDE le `−1` (`on_done`) du parent → pas
//!   de quiescence prématurée (l'enfant est crédité avant que le parent soit décrémenté).
//! - Quiescent ⇔ `sources_done` ET `inflight == 0`.
//!
//! ## Composition
//! Utilise [`crate::TopicRouter`] (WS1) : chaque agent s'abonne à SON topic d'entrée
//! (fan-out réduit). À l'arrêt, on dépose le routeur → les canaux se ferment → les
//! boucles d'agents se terminent (les agents ne tiennent qu'une référence `Weak`).

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant, SystemTime};

use efe_nano_sdk::AgentId;
use serde_json::Value;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::blackboard::Blackboard;
use crate::eventbus::SwarmEvent;
use crate::topic_router::TopicRouter;

// ─────────────────────────────────────────────────────────────────────────────
//  QuiescenceTracker — le cœur testable
// ─────────────────────────────────────────────────────────────────────────────

/// Détecteur de quiescence par crédit. Partagé (`Arc`) entre sources et agents.
#[derive(Debug)]
pub struct QuiescenceTracker {
    inflight: AtomicI64,
    produced: AtomicU64,
    consumed: AtomicU64,
    sources_done: AtomicBool,
    notify: Notify,
}

impl QuiescenceTracker {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inflight: AtomicI64::new(0),
            produced: AtomicU64::new(0),
            consumed: AtomicU64::new(0),
            sources_done: AtomicBool::new(false),
            notify: Notify::new(),
        })
    }

    /// Crédite `n` événements émis (inflight += n). À appeler AVANT le `on_done`
    /// du parent qui les a produits.
    pub fn on_emit(&self, n: u64) {
        if n == 0 {
            return;
        }
        self.inflight.fetch_add(n as i64, Ordering::AcqRel);
        self.produced.fetch_add(n, Ordering::Relaxed);
    }

    /// Marque un événement comme CONSOMMÉ (inflight −= 1). Notifie si l'on atteint 0.
    pub fn on_done(&self) {
        let prev = self.inflight.fetch_sub(1, Ordering::AcqRel);
        self.consumed.fetch_add(1, Ordering::Relaxed);
        if prev - 1 <= 0 {
            self.maybe_notify();
        }
    }

    /// Signale que toutes les sources ont fini d'injecter.
    pub fn mark_sources_done(&self) {
        self.sources_done.store(true, Ordering::Release);
        self.maybe_notify();
    }

    fn maybe_notify(&self) {
        if self.is_quiescent() {
            // notify_one STOCKE un permis même sans waiter → pas de wakeup perdu.
            self.notify.notify_one();
        }
    }

    /// Quiescent ⇔ sources finies ET aucun événement en vol.
    pub fn is_quiescent(&self) -> bool {
        self.sources_done.load(Ordering::Acquire) && self.inflight.load(Ordering::Acquire) <= 0
    }

    pub fn inflight(&self) -> i64 {
        self.inflight.load(Ordering::Acquire)
    }
    pub fn produced(&self) -> u64 {
        self.produced.load(Ordering::Relaxed)
    }
    pub fn consumed(&self) -> u64 {
        self.consumed.load(Ordering::Relaxed)
    }

    /// Attend la quiescence. Robuste aux wakeups perdus (notify_one stocke un permis).
    pub async fn wait_quiescent(&self) {
        while !self.is_quiescent() {
            self.notify.notified().await;
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  AgentCtx — passé au handler, permet d'ÉMETTRE (avec crédit automatique)
// ─────────────────────────────────────────────────────────────────────────────

/// Contexte d'un agent streaming : émission (créditée) + accès Blackboard.
pub struct AgentCtx {
    id: AgentId,
    router: Weak<TopicRouter>,
    bb: Arc<Blackboard>,
    tracker: Arc<QuiescenceTracker>,
    emitted: AtomicU64,
}

impl std::fmt::Debug for AgentCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentCtx")
            .field("id", &self.id)
            .field("emitted", &self.emitted.load(Ordering::Relaxed))
            .finish()
    }
}

impl AgentCtx {
    pub fn agent_id(&self) -> &AgentId {
        &self.id
    }
    pub fn blackboard(&self) -> &Blackboard {
        &self.bb
    }

    /// Émet un événement enfant sur `topic`. Le crédit (`on_emit`) est posé AVANT
    /// la publication → la quiescence ne peut pas se déclencher prématurément. Si
    /// le routeur a disparu (swarm en arrêt), c'est un no-op (aucun crédit).
    pub fn emit(&self, topic: impl Into<String>, payload: Value) {
        if let Some(router) = self.router.upgrade() {
            self.tracker.on_emit(1);
            self.emitted.fetch_add(1, Ordering::Relaxed);
            let _ = router.publish(SwarmEvent {
                swarm_id: self.bb.swarm_id(),
                published_by: self.id.clone(),
                topic: topic.into(),
                payload,
                published_at: SystemTime::now(),
            });
        }
    }
}

/// Handler d'agent streaming : appelé pour chaque événement reçu ; peut émettre
/// via le `AgentCtx`. Synchrone (agents type Mode A) — suffisant pour WS2.
pub type AgentHandler = Arc<dyn Fn(&AgentCtx, &SwarmEvent) + Send + Sync + 'static>;

/// Rapport par agent.
#[derive(Debug, Clone)]
pub struct AgentReport {
    pub agent: AgentId,
    pub processed: u64,
    pub emitted: u64,
}

/// Rapport global d'un run streaming.
#[derive(Debug, Clone)]
pub struct SwarmReport {
    /// true si la quiescence a été atteinte avant le timeout.
    pub quiesced: bool,
    pub produced: u64,
    pub consumed: u64,
    /// Temps écoulé entre le début de `run_until_quiescent` et la quiescence
    /// (ou le timeout) — la « durée de quiescence » de la table SLO WS5.
    pub elapsed: Duration,
    pub agents: Vec<AgentReport>,
}

// ─────────────────────────────────────────────────────────────────────────────
//  StreamingSwarm — l'orchestrateur ergonomique
// ─────────────────────────────────────────────────────────────────────────────

/// Essaim streaming : enregistrez des agents (long-vivants), injectez des sources,
/// puis `run_until_quiescent`.
pub struct StreamingSwarm {
    router: Arc<TopicRouter>,
    bb: Arc<Blackboard>,
    tracker: Arc<QuiescenceTracker>,
    handles: Vec<JoinHandle<AgentReport>>,
}

impl std::fmt::Debug for StreamingSwarm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamingSwarm")
            .field("agents", &self.handles.len())
            .field("inflight", &self.tracker.inflight())
            .finish()
    }
}

impl StreamingSwarm {
    pub fn new(bb: Arc<Blackboard>, router: Arc<TopicRouter>) -> Self {
        Self {
            router,
            bb,
            tracker: QuiescenceTracker::new(),
            handles: Vec::new(),
        }
    }

    /// Accès au tracker (pour observer inflight/produced/consumed en cours de route).
    pub fn tracker(&self) -> &Arc<QuiescenceTracker> {
        &self.tracker
    }

    /// Enregistre un agent long-vivant abonné à `input_topic`. La souscription est
    /// créée SYNCHRONEMENT ici (avant tout `inject`) → aucun événement raté.
    pub fn agent(&mut self, id: AgentId, input_topic: &str, handler: AgentHandler) {
        let mut sub = self.router.subscribe(input_topic);
        let ctx = AgentCtx {
            id: id.clone(),
            router: Arc::downgrade(&self.router),
            bb: self.bb.clone(),
            tracker: self.tracker.clone(),
            emitted: AtomicU64::new(0),
        };
        let handle = tokio::spawn(async move {
            let mut processed = 0u64;
            while let Some(ev) = sub.recv().await {
                handler(&ctx, &ev);
                ctx.tracker.on_done(); // l'événement courant est consommé APRÈS d'éventuels émis
                processed += 1;
            }
            AgentReport {
                agent: ctx.id.clone(),
                processed,
                emitted: ctx.emitted.load(Ordering::Relaxed),
            }
        });
        self.handles.push(handle);
    }

    /// Injecte un événement source (crédité). À appeler APRÈS avoir enregistré les
    /// agents (sinon l'événement est perdu — broadcast sans abonné).
    pub fn inject(&self, topic: &str, payload: Value) {
        self.tracker.on_emit(1);
        let _ = self.router.publish(SwarmEvent {
            swarm_id: self.bb.swarm_id(),
            published_by: AgentId::new("source").unwrap(),
            topic: topic.to_string(),
            payload,
            published_at: SystemTime::now(),
        });
    }

    /// Exécute jusqu'à quiescence (ou `timeout`). Marque les sources finies, attend
    /// la quiescence, ferme le routeur (→ les agents terminent), joint, renvoie le rapport.
    pub async fn run_until_quiescent(mut self, timeout: Duration) -> SwarmReport {
        let tracker = self.tracker.clone();
        let start = Instant::now();
        tracker.mark_sources_done();

        let quiesced = tokio::time::timeout(timeout, tracker.wait_quiescent())
            .await
            .is_ok();
        let elapsed = start.elapsed();

        // Ferme les canaux : les agents ne tiennent qu'un Weak<TopicRouter> → drop
        // de l'unique Arc fort ferme tous les topics → recv() renvoie None → fin.
        drop(self.router);

        let mut agents = Vec::with_capacity(self.handles.len());
        for h in self.handles.drain(..) {
            if let Ok(r) = h.await {
                agents.push(r);
            }
        }
        SwarmReport {
            quiesced,
            produced: tracker.produced(),
            consumed: tracker.consumed(),
            elapsed,
            agents,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::SwarmId;

    // ── QuiescenceTracker (cœur) ───────────────────────────────────────────────

    #[test]
    fn tracker_accounting_balances() {
        let t = QuiescenceTracker::new();
        t.on_emit(3);
        assert_eq!(t.inflight(), 3);
        assert!(!t.is_quiescent(), "sources pas finies");
        t.on_done();
        t.on_done();
        t.on_done();
        assert_eq!(t.inflight(), 0);
        t.mark_sources_done();
        assert!(t.is_quiescent());
        assert_eq!(t.produced(), 3);
        assert_eq!(t.consumed(), 3);
    }

    #[test]
    fn not_quiescent_until_sources_done_even_at_zero_inflight() {
        let t = QuiescenceTracker::new();
        // inflight 0 dès le départ MAIS sources pas finies → pas quiescent.
        assert!(!t.is_quiescent());
        t.mark_sources_done();
        assert!(t.is_quiescent());
    }

    #[test]
    fn cascade_no_premature_quiescence() {
        // Émule un traitement : on émet l'enfant AVANT le on_done du parent.
        let t = QuiescenceTracker::new();
        t.on_emit(1); // source
        t.mark_sources_done();
        assert!(!t.is_quiescent(), "1 en vol");
        // l'agent traite l'événement source : émet 1 enfant PUIS done.
        t.on_emit(1); // enfant crédité d'abord
        t.on_done(); // parent consommé
        assert!(!t.is_quiescent(), "l'enfant est en vol → pas quiescent");
        // l'agent traite l'enfant : pas d'émission, done.
        t.on_done();
        assert!(t.is_quiescent(), "cascade terminée");
    }

    #[tokio::test]
    async fn wait_quiescent_returns_when_drained() {
        let t = QuiescenceTracker::new();
        t.on_emit(2);
        t.mark_sources_done();
        let t2 = t.clone();
        let waiter = tokio::spawn(async move { t2.wait_quiescent().await });
        // pas encore quiescent.
        t.on_done();
        t.on_done(); // → 0 → notify
        // le waiter doit se débloquer.
        waiter.await.unwrap();
        assert!(t.is_quiescent());
    }

    // ── StreamingSwarm (intégration légère) ────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pipeline_streams_to_quiescence() {
        let bb = Arc::new(Blackboard::new(SwarmId::new()));
        let router = Arc::new(TopicRouter::new(256));
        let mut swarm = StreamingSwarm::new(bb.clone(), router);

        // étape 1 → émet étape 2 ; étape 2 → terminal (écrit au Blackboard).
        swarm.agent(
            AgentId::new("stage_one").unwrap(),
            "s1",
            Arc::new(|ctx, ev| {
                let i = ev.payload.get("i").and_then(|v| v.as_i64()).unwrap_or(0);
                ctx.emit("s2", serde_json::json!({ "i": i }));
            }),
        );
        swarm.agent(
            AgentId::new("stage_two").unwrap(),
            "s2",
            Arc::new(|ctx, ev| {
                let i = ev.payload.get("i").and_then(|v| v.as_i64()).unwrap_or(0);
                let _ = ctx.blackboard().write(
                    &format!("/out/{i}"),
                    serde_json::json!(i * 2),
                    ctx.agent_id().clone(),
                );
            }),
        );

        for i in 0..50 {
            swarm.inject("s1", serde_json::json!({ "i": i }));
        }
        let report = swarm.run_until_quiescent(Duration::from_secs(5)).await;

        assert!(report.quiesced, "doit atteindre la quiescence");
        // 50 sources + 50 enfants = 100 produits ET consommés.
        assert_eq!(report.produced, 100);
        assert_eq!(report.consumed, 100);
        // 50 sorties écrites au Blackboard.
        assert_eq!(bb.snapshot().value_map().keys().filter(|k| k.starts_with("/out/")).count(), 50);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn fanout_credits_all_children() {
        let bb = Arc::new(Blackboard::new(SwarmId::new()));
        let router = Arc::new(TopicRouter::new(256));
        let mut swarm = StreamingSwarm::new(bb.clone(), router);

        // 1 source → émet 5 enfants ; enfants terminaux.
        swarm.agent(
            AgentId::new("fanout").unwrap(),
            "in",
            Arc::new(|ctx, _ev| {
                for k in 0..5 {
                    ctx.emit("leaf", serde_json::json!({ "k": k }));
                }
            }),
        );
        swarm.agent(
            AgentId::new("leaf_sink").unwrap(),
            "leaf",
            Arc::new(|_ctx, _ev| {}),
        );

        swarm.inject("in", serde_json::json!({}));
        let report = swarm.run_until_quiescent(Duration::from_secs(5)).await;
        assert!(report.quiesced);
        // 1 source + 5 enfants = 6 produits/consommés.
        assert_eq!(report.produced, 6);
        assert_eq!(report.consumed, 6);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn timeout_when_event_has_no_consumer() {
        let bb = Arc::new(Blackboard::new(SwarmId::new()));
        let router = Arc::new(TopicRouter::new(64));
        let swarm = StreamingSwarm::new(bb, router);
        // injecte sur un topic SANS agent → jamais consommé → timeout.
        swarm.inject("orphan", serde_json::json!({}));
        let report = swarm.run_until_quiescent(Duration::from_millis(150)).await;
        assert!(!report.quiesced, "doit timeouter (événement orphelin)");
    }
}
