// SPDX-License-Identifier: UNLICENSED
//! # `recorder` — le moteur de capture qui **fusionne les 2 seams**.
//!
//! Deux flux entrent dans le [`ProvenanceRecorder`] :
//! - **Seam B (actif)** : le décorateur [`crate::decorator::ProvenancedAgent`] appelle
//!   [`ProvenanceRecorder::capture`] avec un [`CallCapture`] = la **sémantique**
//!   (entrée→sortie, agent, rôle, variant). C'est le seul endroit qui voit le lien
//!   entrée→sortie.
//! - **Seam A (passif)** : le [`crate::sink::ProvenanceSink`] appelle
//!   [`ProvenanceRecorder::anchor`] à **chaque** écriture du Blackboard = l'**ancrage
//!   durable** (cellule `pointer@version`, agent, octets). Couverture 100 % des écritures.
//!
//! [`ProvenanceRecorder::seal`] **fusionne** les deux : à chaque brouillon (capture)
//! il attache la cellule écrite (ancre du même agent, FIFO), **ordonne par la version**
//! d'écriture (ADR-SWARM-007 : ordre déterministe sur la *version*, pas l'horloge),
//! puis enfile le tout dans une [`SwarmChain`] → enregistrements scellés + `provenance_root`.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

use efe_audit_core::{AuditKeyError, HmacKeySource};
use efe_blackboard::BlackboardWrite;
use efe_nano_sdk::{AgentId, Blake3Hash, ProvenanceId, SwarmId, TenantId};
use serde_json::Value;
use uuid::Uuid;

use crate::chain::SwarmChain;
use crate::hash::blake3_value;
use crate::record::{hex32, AgentCallDraft, AgentCallRecord, AgentKind, AgentRole, CellRef};

// ─────────────────────────────────────────────────────────────────────────────
//  CallCapture — ce que le décorateur transmet par appel d'agent
// ─────────────────────────────────────────────────────────────────────────────

/// La sémantique d'un appel d'agent, captée par le décorateur (seam B).
#[derive(Debug, Clone)]
pub struct CallCapture {
    pub agent_id: AgentId,
    pub role: AgentRole,
    pub kind: AgentKind,
    pub input_hash: Blake3Hash,
    pub output_hash: Blake3Hash,
    pub variant: Option<u64>,
    pub reads: Vec<CellRef>,
    pub causal_parents: Vec<Uuid>,
    pub llm_provenance_id: Option<ProvenanceId>,
}

impl CallCapture {
    /// Construit depuis les **valeurs** d'entrée/sortie (hashées BLAKE3 canonique).
    pub fn new(agent_id: AgentId, role: AgentRole, kind: AgentKind, input: &Value, output: &Value) -> Self {
        Self {
            agent_id,
            role,
            kind,
            input_hash: blake3_value(input),
            output_hash: blake3_value(output),
            variant: None,
            reads: Vec::new(),
            causal_parents: Vec::new(),
            llm_provenance_id: None,
        }
    }
    pub fn with_variant(mut self, v: u64) -> Self {
        self.variant = Some(v);
        self
    }
    pub fn reading(mut self, pointer: impl Into<String>, version: u64) -> Self {
        self.reads.push(CellRef::new(pointer, version));
        self
    }
    pub fn caused_by(mut self, record_id: Uuid) -> Self {
        self.causal_parents.push(record_id);
        self
    }
    pub fn with_llm_provenance(mut self, id: ProvenanceId) -> Self {
        self.llm_provenance_id = Some(id);
        self
    }

    fn into_draft(self, swarm_id: SwarmId, tenant: TenantId) -> AgentCallDraft {
        let mut d = AgentCallDraft::new(
            swarm_id, tenant, self.agent_id, self.role, self.kind, self.input_hash, self.output_hash,
        );
        if let Some(v) = self.variant {
            d = d.with_variant(v);
        }
        for r in self.reads {
            d = d.read(r.pointer, r.version);
        }
        for p in self.causal_parents {
            d = d.caused_by(p);
        }
        if let Some(l) = self.llm_provenance_id {
            d = d.with_llm_provenance(l);
        }
        d
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  WriteAnchor — ce que le sink transmet par écriture du Blackboard
// ─────────────────────────────────────────────────────────────────────────────

/// L'ancrage durable d'une écriture (seam A). Pas de valeur (le `WriteSink` ne la
/// reçoit pas) : la valeur est hashée côté décorateur ; l'ancre apporte la **cellule**
/// (`pointer@version`) et la **couverture**.
#[derive(Debug, Clone)]
pub struct WriteAnchor {
    pub pointer: String,
    pub version: u64,
    pub agent_id: AgentId,
    pub at: SystemTime,
    pub bytes: usize,
}

impl WriteAnchor {
    fn from_write(w: &BlackboardWrite) -> Self {
        Self {
            pointer: w.pointer.clone(),
            version: w.version,
            agent_id: w.agent_id.clone(),
            at: w.at,
            bytes: w.bytes,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  ProvenanceRecorder — collecte + fusion
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Default)]
struct Recorded {
    drafts: Vec<AgentCallDraft>,
    anchors: Vec<WriteAnchor>,
}

/// Moteur de capture thread-safe. Partagé via `Arc` entre les agents décorés (seam B)
/// et le sink (seam A). Les deux méthodes d'entrée (`capture`, `anchor`) sont **rapides
/// et non bloquantes** (un bref verrou + push) — compatibles avec le chemin chaud.
pub struct ProvenanceRecorder {
    swarm_id: SwarmId,
    tenant: TenantId,
    inner: Mutex<Recorded>,
    captured: AtomicU64,
    anchored: AtomicU64,
}

impl ProvenanceRecorder {
    pub fn new(swarm_id: SwarmId, tenant: TenantId) -> Self {
        Self {
            swarm_id,
            tenant,
            inner: Mutex::new(Recorded::default()),
            captured: AtomicU64::new(0),
            anchored: AtomicU64::new(0),
        }
    }

    /// **Seam B** : enregistre la sémantique d'un appel d'agent (entrée→sortie).
    pub fn capture(&self, cap: CallCapture) {
        let draft = cap.into_draft(self.swarm_id, self.tenant);
        if let Ok(mut g) = self.inner.lock() {
            g.drafts.push(draft);
        }
        self.captured.fetch_add(1, Ordering::Relaxed);
    }

    /// **Seam A** : ancre une écriture du Blackboard (couverture durable).
    pub fn anchor(&self, write: &BlackboardWrite) {
        if let Ok(mut g) = self.inner.lock() {
            g.anchors.push(WriteAnchor::from_write(write));
        }
        self.anchored.fetch_add(1, Ordering::Relaxed);
    }

    /// Nombre d'appels d'agent captés (seam B).
    pub fn captured_calls(&self) -> u64 {
        self.captured.load(Ordering::Relaxed)
    }
    /// Nombre d'écritures ancrées (seam A).
    pub fn anchored_writes(&self) -> u64 {
        self.anchored.load(Ordering::Relaxed)
    }

    pub fn swarm_id(&self) -> SwarmId {
        self.swarm_id
    }
    pub fn tenant(&self) -> TenantId {
        self.tenant
    }

    /// **Fusionne + scelle** : attache à chaque brouillon la cellule écrite (ancre du
    /// même agent, FIFO), ordonne par version d'écriture (ADR-007), enfile dans une
    /// [`SwarmChain`]. Dérive la clé tenant depuis `src` (HKDF).
    pub fn seal(&self, src: &HmacKeySource) -> Result<SealedProvenance, AuditKeyError> {
        let key = src.derive(self.tenant)?;
        Ok(self.seal_with_key(key))
    }

    /// Idem [`Self::seal`] avec une **clé déjà dérivée** (32 o serveur).
    pub fn seal_with_key(&self, key: Vec<u8>) -> SealedProvenance {
        let recorded = self.inner.lock().expect("recorder mutex");

        // anchors regroupées par agent (FIFO par ordre de version croissant).
        let mut by_agent: HashMap<String, VecDeque<WriteAnchor>> = HashMap::new();
        let mut anchors_sorted = recorded.anchors.clone();
        anchors_sorted.sort_by_key(|a| a.version);
        for a in anchors_sorted {
            by_agent.entry(a.agent_id.as_str().to_string()).or_default().push_back(a);
        }

        // **Appariement DÉTERMINISTE (ADR-014 / I-CP1)** : les brouillons arrivent en
        // ordre de CAPTURE (= ordre d'achèvement des agents ∥ — non déterministe sous
        // largeur variable), les ancres en ordre d'écriture. Apparier en ordre de
        // capture lierait `variant i` à une cellule qui dépend de l'ordonnanceur → le
        // root varierait d'un run à l'autre. On apparie donc en ordre de CONTENU :
        // (agent, variant — None en dernier, input_hash, output_hash). Pour les rôles
        // à variant (générateurs), c'est AUSSI l'appariement sémantique exact
        // (variant i ↔ i-ème écriture de l'agent) ; pour les autres, c'est un ordre
        // stable de contenu — la largeur d'exécution ne peut plus toucher la racine.
        let mut drafts_sorted: Vec<AgentCallDraft> = recorded.drafts.clone();
        drafts_sorted.sort_by(|a, b| {
            a.agent_id
                .as_str()
                .cmp(b.agent_id.as_str())
                .then(a.variant.unwrap_or(u64::MAX).cmp(&b.variant.unwrap_or(u64::MAX)))
                .then(a.input_hash.to_hex().cmp(&b.input_hash.to_hex()))
                .then(a.output_hash.to_hex().cmp(&b.output_hash.to_hex()))
        });

        // enrichit chaque brouillon : attache la prochaine ancre (cellule écrite) de SON
        // agent, FIFO par version. La clé d'ordre (ADR-SWARM-007) est calculée APRÈS, sur
        // la cellule écrite + payload_hash — JAMAIS sur wall_at. Partagée avec `replay` ⇒
        // le root vivant == le root rejoué.
        let mut enriched: Vec<(crate::replay::OrderKey, AgentCallDraft)> =
            Vec::with_capacity(drafts_sorted.len());
        let mut anchored_records = 0usize;
        for d in drafts_sorted.iter() {
            let mut d = d.clone();
            if let Some(a) = by_agent.get_mut(d.agent_id.as_str()).and_then(|q| q.pop_front()) {
                d = d.wrote(a.pointer, a.version);
                anchored_records += 1;
            }
            let k = crate::replay::key_of(&d.writes, d.payload_hash());
            enriched.push((k, d));
        }
        // ordre DÉTERMINISTE (ADR-SWARM-007).
        enriched.sort_by(|a, b| a.0.cmp(&b.0));

        let mut chain = SwarmChain::new(self.tenant, self.swarm_id, key);
        let records = chain.append_all(enriched.into_iter().map(|(_, d)| d));

        SealedProvenance {
            records,
            root: chain.root(),
            swarm_id: self.swarm_id,
            captured_calls: self.captured_calls(),
            anchored_writes: self.anchored_writes(),
            anchored_records,
        }
    }
}

impl std::fmt::Debug for ProvenanceRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProvenanceRecorder")
            .field("swarm_id", &self.swarm_id)
            .field("captured_calls", &self.captured_calls())
            .field("anchored_writes", &self.anchored_writes())
            .finish()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  SealedProvenance — le résultat scellé d'un run
// ─────────────────────────────────────────────────────────────────────────────

/// Résultat scellé : enregistrements chaînés + `provenance_root` + compteurs de couverture.
#[derive(Debug, Clone)]
pub struct SealedProvenance {
    pub records: Vec<AgentCallRecord>,
    pub root: Option<[u8; 32]>,
    pub swarm_id: SwarmId,
    /// Appels d'agent captés (seam B).
    pub captured_calls: u64,
    /// Écritures ancrées (seam A).
    pub anchored_writes: u64,
    /// Enregistrements ayant reçu au moins une cellule écrite (fusion réussie).
    pub anchored_records: usize,
}

impl SealedProvenance {
    pub fn len(&self) -> usize {
        self.records.len()
    }
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
    pub fn root_hex(&self) -> Option<String> {
        self.root.as_ref().map(hex32)
    }
    /// Fraction d'enregistrements ancrés à une cellule durable (proxy de couverture ;
    /// la métrique `traced_pct` complète arrive en Inc.4 avec `verify_swarm`).
    pub fn anchored_record_ratio(&self) -> f64 {
        if self.records.is_empty() {
            return 1.0;
        }
        self.anchored_records as f64 / self.records.len() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify_chain;
    use efe_blackboard::BlackboardWrite;

    fn swarm() -> SwarmId {
        SwarmId::from_uuid(Uuid::from_u128(0x2222))
    }
    fn key() -> Vec<u8> {
        HmacKeySource::test_key().derive(TenantId::system()).unwrap()
    }
    fn write(pointer: &str, version: u64, agent: &str) -> BlackboardWrite {
        BlackboardWrite {
            pointer: pointer.into(),
            version,
            agent_id: AgentId::new(agent).unwrap(),
            at: SystemTime::UNIX_EPOCH,
            bytes: 10,
        }
    }
    fn cap(agent: &str, role: AgentRole, variant: u64) -> CallCapture {
        CallCapture::new(
            AgentId::new(agent).unwrap(),
            role,
            AgentKind::ModeA,
            &serde_json::json!({ "in": variant }),
            &serde_json::json!({ "out": variant }),
        )
        .with_variant(variant)
    }

    #[test]
    fn capture_and_anchor_increment_counters() {
        let r = ProvenanceRecorder::new(swarm(), TenantId::system());
        r.capture(cap("generator", AgentRole::Generator, 0));
        r.capture(cap("generator", AgentRole::Generator, 1));
        r.anchor(&write("/cand/0", 1, "generator"));
        assert_eq!(r.captured_calls(), 2);
        assert_eq!(r.anchored_writes(), 1);
    }

    #[test]
    fn seal_fuses_writes_and_orders_by_version() {
        let r = ProvenanceRecorder::new(swarm(), TenantId::system());
        // capturés dans un ordre (1 puis 0), ancrés dans l'ordre des versions (0→v1, 1→v2).
        r.capture(cap("generator", AgentRole::Generator, 1));
        r.capture(cap("generator", AgentRole::Generator, 0));
        r.anchor(&write("/cand/0", 1, "generator"));
        r.anchor(&write("/cand/1", 2, "generator"));

        let sealed = r.seal_with_key(key());
        assert_eq!(sealed.len(), 2);
        // ordonné par version d'écriture : /cand/0 (v1) avant /cand/1 (v2).
        assert_eq!(sealed.records[0].writes, vec![CellRef::new("/cand/0", 1)]);
        assert_eq!(sealed.records[1].writes, vec![CellRef::new("/cand/1", 2)]);
        assert_eq!(sealed.records[0].seq, 0);
        assert_eq!(sealed.records[1].seq, 1);
        assert_eq!(sealed.anchored_records, 2);
        assert!((sealed.anchored_record_ratio() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn pairing_is_deterministic_under_capture_order() {
        // **I-CP1 (ADR-014)** : l'ordre de CAPTURE (= achèvement des agents ∥, que la
        // largeur d'exécution remanie) ne doit toucher AUCUN contenu sémantique.
        // Deux recorders, mêmes contenus, ordres de capture OPPOSÉS :
        // ① l'appariement est sémantique (variant i ↔ /cand/i — plus jamais « le
        //   premier fini prend la première cellule ») ;
        // ② à IDENTITÉS normalisées (record_id est l'entropie PAR-RUN de l'atome,
        //   volontairement aléatoire), la chaîne re-scellée donne le MÊME root.
        let reseal_normalized = |records: &[crate::record::AgentCallRecord]| -> String {
            let mut drafts: Vec<AgentCallDraft> = records
                .iter()
                .map(|r| {
                    let mut d = r.to_draft();
                    // identité déterministe : une par variant (l'entropie v4 neutralisée).
                    d.record_id = Uuid::from_u128(0xC0FFEE + u128::from(d.variant.unwrap_or(0)));
                    d
                })
                .collect();
            // même ordre canonique que seal/replay (ADR-007).
            drafts.sort_by_key(|d| crate::replay::key_of(&d.writes, d.payload_hash()));
            let mut chain = SwarmChain::new(TenantId::system(), swarm(), key());
            let _ = chain.append_all(drafts);
            hex32(&chain.root().expect("root"))
        };

        let roots: Vec<String> = [[0u64, 1, 2, 3], [3, 2, 1, 0]]
            .into_iter()
            .map(|order| {
                let r = ProvenanceRecorder::new(swarm(), TenantId::system());
                for v in order {
                    r.capture(cap("generator", AgentRole::Generator, v));
                }
                // les écritures, elles, arrivent en ordre d'index (le motif consensus).
                for i in 0..4u64 {
                    r.anchor(&write(&format!("/cand/{i}"), 1, "generator"));
                }
                let sealed = r.seal_with_key(key());
                // ① appariement sémantique : le record du variant i écrit /cand/i.
                for rec in &sealed.records {
                    let v = rec.variant.expect("variant posé");
                    assert_eq!(
                        rec.writes,
                        vec![CellRef::new(format!("/cand/{v}"), 1)],
                        "variant {v} ↔ /cand/{v}"
                    );
                }
                reseal_normalized(&sealed.records)
            })
            .collect();
        // ② contenu ⇒ racine : l'ordre de capture n'existe plus nulle part.
        assert_eq!(roots[0], roots[1], "à identités normalisées, racine invariante sous l'ordre de capture");
    }

    #[test]
    fn sealed_chain_verifies() {
        let r = ProvenanceRecorder::new(swarm(), TenantId::system());
        for j in 0..3 {
            r.capture(cap(&format!("judge_{j:03}"), AgentRole::Judge, j));
            r.anchor(&write(&format!("/ballot/{j}"), j + 1, &format!("judge_{j:03}")));
        }
        let sealed = r.seal_with_key(key());
        assert_eq!(sealed.len(), 3);
        assert!(verify_chain(&sealed.records, &key()).ok, "chaîne scellée intègre");
        assert_eq!(sealed.root_hex().unwrap().len(), 64);
    }

    #[test]
    fn draft_without_anchor_orders_last_but_is_recorded() {
        let r = ProvenanceRecorder::new(swarm(), TenantId::system());
        r.capture(cap("merger", AgentRole::Merger, 0)); // pas d'écriture ancrée
        r.capture(cap("generator", AgentRole::Generator, 0));
        r.anchor(&write("/cand/0", 1, "generator"));
        let sealed = r.seal_with_key(key());
        assert_eq!(sealed.len(), 2);
        // le générateur (ancré v1) passe avant le merger (sans ancre → u64::MAX).
        assert_eq!(sealed.records[0].agent_id.as_str(), "generator");
        assert_eq!(sealed.records[1].agent_id.as_str(), "merger");
        assert_eq!(sealed.anchored_records, 1);
        assert!((sealed.anchored_record_ratio() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn empty_recorder_seals_to_empty() {
        let r = ProvenanceRecorder::new(swarm(), TenantId::system());
        let sealed = r.seal_with_key(key());
        assert!(sealed.is_empty());
        assert_eq!(sealed.root, None);
        assert_eq!(sealed.anchored_record_ratio(), 1.0);
    }

    #[test]
    fn seal_derives_key_from_source() {
        let r = ProvenanceRecorder::new(swarm(), TenantId::system());
        r.capture(cap("generator", AgentRole::Generator, 0));
        let sealed = r.seal(&HmacKeySource::test_key()).unwrap();
        assert_eq!(sealed.len(), 1);
    }
}
