// SPDX-License-Identifier: UNLICENSED
//! `SwarmObserver` — le hook **générique** de cycle de vie d'un run.
//!
//! Le SDK **ignore** le mot « dashboard » : il expose un trait que *n'importe quel*
//! observateur (dashboard live Inc.3, journal, test) implémente. Brancher un observateur
//! (`SwarmBuilder::with_observer`) **tee** l'exutoire de provenance avec celui de
//! l'observateur (trafic stigmergique), branche son tap d'événements au `SwarmContext`
//! (trafic inter-agent), et notifie chaque appel d'agent + le scellement + la fin.
//!
//! **Non-breaking** : sans observateur, `run`/`run_traced` sont **inchangés** (aucun tee,
//! aucun tap). Toutes les méthodes ont un défaut no-op.

use std::sync::Arc;

use efe_blackboard::{BlackboardWrite, EventSink, WriteSink};
use efe_swarm_provenance::{AgentKind, AgentRole};
use serde_json::Value;

use crate::budget::BudgetSnapshot;
use crate::result::SwarmRun;

/// Données d'un **appel d'agent réellement exécuté** (facturé), passées à
/// [`SwarmObserver::on_agent_call`]. Regroupées en struct pour une signature stable.
pub struct AgentCall<'a> {
    pub agent_id: &'a str,
    pub role: AgentRole,
    pub kind: AgentKind,
    /// Variant du générateur (T1/consensus), sinon `None`.
    pub variant: Option<u64>,
    /// La sortie de l'agent (candidat, ballot de scores, résultat d'étage…).
    pub output: &'a Value,
    /// Coût facturé pour cet appel (cents).
    pub cost_cents: u32,
    /// Instantané du budget **après** cet appel (jamais au-delà du plafond — I13).
    pub budget: &'a BudgetSnapshot,
}

/// Observateur de cycle de vie d'un run. Toutes les méthodes sont **optionnelles** (no-op
/// par défaut) : un observateur n'implémente que ce qui l'intéresse.
pub trait SwarmObserver: Send + Sync {
    /// Exutoire d'écritures à **tee** avec la provenance (writes stigmergiques `/cand`,
    /// `/ballot`, `/out`…). `None` ⇒ aucun tee (la provenance reste seule, inchangée).
    fn write_sink(&self) -> Option<Arc<dyn WriteSink>> {
        None
    }
    /// Tap d'événements de bus (messages inter-agent). `None` ⇒ aucun tap.
    fn event_tap(&self) -> Option<Arc<dyn EventSink>> {
        None
    }
    /// Le run démarre : topologie résolue, étiquette de mode, plafond budget, agents estimés.
    fn on_run_start(&self, _topology: &str, _mode: &str, _budget_cap_cents: u32, _assumed_agents: usize) {}
    /// Un agent a réellement tourné (facturé).
    fn on_agent_call(&self, _call: AgentCall<'_>) {}
    /// La provenance est scellée : records, couverture, `provenance_root`, re-prouvé ?
    fn on_seal(&self, _records: usize, _traced_pct: f64, _root: Option<&str>, _verified: bool) {}
    /// Le run est terminé : le résumé complet.
    fn on_finish(&self, _run: &SwarmRun) {}
}

/// Fan-out `WriteSink` : diffuse chaque écriture à N exutoires (provenance **et** dashboard)
/// — la provenance reste byte-identique, l'observateur voit les mêmes writes.
pub(crate) struct TeeSink(pub Vec<Arc<dyn WriteSink>>);

impl WriteSink for TeeSink {
    fn on_write(&self, w: &BlackboardWrite) {
        for s in &self.0 {
            s.on_write(w);
        }
    }
    fn flush(&self) {
        for s in &self.0 {
            s.flush();
        }
    }
}
