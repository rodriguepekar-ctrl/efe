// SPDX-License-Identifier: UNLICENSED
//! # `sink` — **seam A (passif)** : ancrage 100 % des écritures du Blackboard.
//!
//! [`ProvenanceSink`] implémente [`efe_blackboard::WriteSink`] : à **chaque** écriture
//! committée du Blackboard, il transmet l'ancre (`pointer@version`, agent, octets) au
//! [`ProvenanceRecorder`]. Branché via `Blackboard::set_write_sink`, il capture **toute**
//! écriture — y compris d'agents **non décorés** (filet de sécurité / mesure de couverture).
//!
//! Contrat du seam (cf. `efe-blackboard::sink`) : `on_write` est appelé en synchrone
//! sur la tâche écrivaine, hors tout verrou du Blackboard ; il doit être **rapide et
//! non bloquant**. [`ProvenanceRecorder::anchor`] ne fait qu'un bref `push` — conforme.

use std::sync::Arc;

use efe_blackboard::{BlackboardWrite, WriteSink};

use crate::recorder::ProvenanceRecorder;

/// Sink de provenance : ancre chaque écriture dans le [`ProvenanceRecorder`].
#[derive(Debug)]
pub struct ProvenanceSink {
    recorder: Arc<ProvenanceRecorder>,
}

impl ProvenanceSink {
    pub fn new(recorder: Arc<ProvenanceRecorder>) -> Self {
        Self { recorder }
    }

    /// Construit le sink **et** renvoie le recorder partagé (à passer aussi aux
    /// agents décorés). Pratique pour câbler les deux seams d'un coup.
    pub fn with_recorder(recorder: Arc<ProvenanceRecorder>) -> Arc<Self> {
        Arc::new(Self::new(recorder))
    }

    pub fn recorder(&self) -> &Arc<ProvenanceRecorder> {
        &self.recorder
    }
}

impl WriteSink for ProvenanceSink {
    fn on_write(&self, write: &BlackboardWrite) {
        self.recorder.anchor(write);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_blackboard::Blackboard;
    use efe_nano_sdk::{AgentId, SwarmId, TenantId};
    use uuid::Uuid;

    #[test]
    fn sink_anchors_every_blackboard_write() {
        let swarm = SwarmId::from_uuid(Uuid::from_u128(0x2222));
        let recorder = Arc::new(ProvenanceRecorder::new(swarm, TenantId::system()));
        let sink: Arc<dyn WriteSink> = Arc::new(ProvenanceSink::new(recorder.clone()));

        let bb = Blackboard::new(swarm);
        bb.set_write_sink(sink);

        let agent = AgentId::new("generator").unwrap();
        bb.write("/cand/0", serde_json::json!({ "value": "a" }), agent.clone()).unwrap();
        bb.write("/cand/1", serde_json::json!({ "value": "b" }), agent.clone()).unwrap();
        bb.write("/cand/2", serde_json::json!({ "value": "c" }), agent).unwrap();

        // 100 % des écritures ancrées (seam passif).
        assert_eq!(recorder.anchored_writes(), 3);
    }

    #[test]
    fn sink_is_a_writesink_trait_object() {
        let recorder = Arc::new(ProvenanceRecorder::new(
            SwarmId::from_uuid(Uuid::from_u128(1)),
            TenantId::system(),
        ));
        // se passe comme Arc<dyn WriteSink> (interface du substrat).
        let _sink: Arc<dyn WriteSink> = ProvenanceSink::with_recorder(recorder);
    }
}
