// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21** : Blackboard — concurrent JSON-pointer-keyed
//! store with per-field versioning.
//!
//! Architecture § 7.2 + ADR-2 + I3 + I17.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;

use dashmap::DashMap;
use efe_nano_sdk::{AgentId, SwarmId};
use serde::{Deserialize, Serialize};

use crate::error::BlackboardError;
use crate::metrics::SubstrateMetrics;
use crate::observer::BlackboardObserver;
use crate::sink::WriteSink;
use crate::snapshot::BlackboardSnapshot;

/// **WS3** : capacité par défaut du ring d'historique récent (nombre d'écritures).
/// Les compteurs EXACTS (`write_count`, `writes_by_agent`) ne sont PAS bornés ;
/// seul l'historique détaillé récent l'est, pour borner la mémoire (corrige F4).
pub const DEFAULT_RECENT_HISTORY: usize = 8192;

/// **N21** : a single versioned blackboard cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlackboardValue {
    pub value: serde_json::Value,
    pub version: u64,
    pub written_by: AgentId,
    pub written_at: SystemTime,
}

/// **N21** : a single write event in the audit log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlackboardWrite {
    pub pointer: String,
    pub version: u64,
    pub agent_id: AgentId,
    pub at: SystemTime,
    pub bytes: usize,
}

/// **N21** : the shared blackboard.
///
/// Concurrency model :
/// - `DashMap` shard-locked for per-key fine-grained access.
/// - Each `BlackboardValue.version` increments monotonically on
///   each write (per JSON pointer).
/// - `write_if` is the CAS primitive : write iff stored version
///   matches `expected_version`.
/// - `write_history` is a `Mutex<Vec<...>>` ; only `write()` touches it
///   and the lock is held only over the push (no `.await` between
///   lock + drop — I17).
/// - `total_bytes` is an atomic running sum of serialised value
///   sizes — used to enforce `MAX_BLACKBOARD_BYTES`.
pub struct Blackboard {
    fields: DashMap<String, BlackboardValue>,
    swarm_id: SwarmId,
    /// **WS3** : ring BORNÉ des N dernières écritures (debug). Remplace l'ancien
    /// `Mutex<Vec<…>>` non borné (corrige F4 : croissance mémoire).
    recent: Mutex<VecDeque<BlackboardWrite>>,
    recent_cap: usize,
    /// **WS3** : compteur EXACT d'écritures (jamais borné, O(1)).
    writes_total: AtomicU64,
    /// **WS3** : compteur EXACT par agent (O(1), remplace le scan O(historique)).
    writes_per_agent: DashMap<AgentId, AtomicU64>,
    /// **WS3** : écritures rejetées pour dépassement de cap.
    rejections_total: AtomicU64,
    total_bytes: AtomicUsize,
    max_bytes: usize,
    /// **WS3** : caps par namespace (prefixe → octets max), triés longest-first.
    namespace_caps: Vec<(String, usize)>,
    /// **WS3** : octets cumulés par namespace (suivi seulement si caps configurés).
    ns_bytes: DashMap<String, AtomicUsize>,
    /// **WS3** : exutoire d'historique (provenance / journal de replay).
    sink: RwLock<Option<Arc<dyn WriteSink>>>,
    /// **WS5** : registre de métriques (optionnel).
    metrics: RwLock<Option<Arc<SubstrateMetrics>>>,
    /// **WS5** : observateurs — le hook est désormais CÂBLÉ dans write/write_if.
    observers: RwLock<Vec<Arc<dyn BlackboardObserver>>>,
}

impl std::fmt::Debug for Blackboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Blackboard")
            .field("swarm_id", &self.swarm_id)
            .field("fields", &self.fields.len())
            .field("total_bytes", &self.total_bytes.load(Ordering::Relaxed))
            .field("max_bytes", &self.max_bytes)
            .finish()
    }
}

impl Blackboard {
    /// **N21** : Construct with the default 256 KB cap.
    pub fn new(swarm_id: SwarmId) -> Self {
        Self::with_cap(swarm_id, crate::MAX_BLACKBOARD_BYTES)
    }

    /// **N21 — test helper** : custom cap global.
    pub fn with_cap(swarm_id: SwarmId, max_bytes: usize) -> Self {
        Self::build(swarm_id, max_bytes, DEFAULT_RECENT_HISTORY, Vec::new())
    }

    /// **WS3** : construit avec des caps PAR NAMESPACE (en plus du cap global).
    /// `caps` = liste (prefixe, octets_max) ; le cap global s'applique toujours
    /// au total. Permet ex. `/cand/* = 4 Mo`, `/ctrl/* = 64 Ko` (corrige F5).
    pub fn with_namespaces(swarm_id: SwarmId, max_bytes: usize, caps: Vec<(String, usize)>) -> Self {
        Self::build(swarm_id, max_bytes, DEFAULT_RECENT_HISTORY, caps)
    }

    /// **WS3** : variante avec capacité d'historique récent explicite.
    pub fn with_recent_cap(swarm_id: SwarmId, max_bytes: usize, recent_cap: usize) -> Self {
        Self::build(swarm_id, max_bytes, recent_cap, Vec::new())
    }

    fn build(
        swarm_id: SwarmId,
        max_bytes: usize,
        recent_cap: usize,
        mut caps: Vec<(String, usize)>,
    ) -> Self {
        // longest-prefix-first → le 1er prefixe qui matche est le plus spécifique.
        caps.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        Self {
            fields: DashMap::new(),
            swarm_id,
            recent: Mutex::new(VecDeque::new()),
            recent_cap,
            writes_total: AtomicU64::new(0),
            writes_per_agent: DashMap::new(),
            rejections_total: AtomicU64::new(0),
            total_bytes: AtomicUsize::new(0),
            max_bytes,
            namespace_caps: caps,
            ns_bytes: DashMap::new(),
            sink: RwLock::new(None),
            metrics: RwLock::new(None),
            observers: RwLock::new(Vec::new()),
        }
    }

    pub fn swarm_id(&self) -> SwarmId {
        self.swarm_id
    }

    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    pub fn total_bytes(&self) -> usize {
        self.total_bytes.load(Ordering::Relaxed)
    }

    pub fn len(&self) -> usize {
        self.fields.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// **N21** : Unconditional write — increments the version of the
    /// target pointer.  Returns the new version.
    ///
    /// Enforces `MAX_BLACKBOARD_BYTES` (I10 / G8) :
    /// - If the new total size > cap, the write is rejected with
    ///   `SizeLimitExceeded` AND the in-memory state is not mutated.
    pub fn write(
        &self,
        pointer: &str,
        value: serde_json::Value,
        agent_id: AgentId,
    ) -> Result<u64, BlackboardError> {
        validate_pointer(pointer)?;

        // Compute new value bytes.
        let new_bytes = serialised_size(&value);

        // Read current value (if any) to find old bytes + version.
        let (old_bytes, old_version) = match self.fields.get(pointer) {
            Some(e) => (serialised_size(&e.value().value), e.value().version),
            None => (0, 0),
        };

        // Provisional new total.
        let cur_total = self.total_bytes.load(Ordering::Acquire);
        let new_total = cur_total
            .saturating_sub(old_bytes)
            .saturating_add(new_bytes);
        if new_total > self.max_bytes {
            self.note_rejection();
            return Err(BlackboardError::SizeLimitExceeded {
                current: new_total,
                max: self.max_bytes,
            });
        }
        // **WS3** : cap par namespace (en plus du cap global).
        let ns_applied = self.check_namespace_cap(pointer, old_bytes, new_bytes)?;

        let new_version = old_version + 1;
        let bv = BlackboardValue {
            value,
            version: new_version,
            written_by: agent_id.clone(),
            written_at: SystemTime::now(),
        };
        self.fields.insert(pointer.to_string(), bv);
        self.total_bytes.store(new_total, Ordering::Release);
        self.commit_namespace(ns_applied);

        // **WS3/WS5** : compteurs O(1) + ring borné + sink + métriques + observers.
        self.record_write(
            BlackboardWrite {
                pointer: pointer.to_string(),
                version: new_version,
                agent_id,
                at: SystemTime::now(),
                bytes: new_bytes,
            },
            new_total,
        );
        Ok(new_version)
    }

    /// **N21 + F9** : CAS-style write — only succeeds if the stored
    /// version equals `expected_version`.  `expected_version = 0`
    /// means "expected absent" — succeeds iff the pointer is unset.
    ///
    /// **Atomicity** (post-audit fix B-CAS) : the compare + swap is
    /// performed under a single `DashMap::entry` shard guard so
    /// concurrent CAS writers serialise.  Without this, multiple
    /// writers could read the same `actual_version` and both
    /// "succeed", losing updates.
    pub fn write_if(
        &self,
        pointer: &str,
        expected_version: u64,
        value: serde_json::Value,
        agent_id: AgentId,
    ) -> Result<u64, BlackboardError> {
        validate_pointer(pointer)?;
        self.note_cas_attempt(); // **WS5** : chaque write_if = une tentative CAS.
        let new_bytes = serialised_size(&value);

        // Hold the shard lock for the entire compare + swap.
        let entry = self.fields.entry(pointer.to_string());

        use dashmap::mapref::entry::Entry;
        let (old_version, old_bytes) = match &entry {
            Entry::Occupied(o) => (o.get().version, serialised_size(&o.get().value)),
            Entry::Vacant(_) => (0, 0),
        };

        if old_version != expected_version {
            self.note_conflict();
            return Err(BlackboardError::Conflict {
                pointer: pointer.to_string(),
                expected_version,
                actual_version: old_version,
            });
        }

        // Size check (global).
        let cur_total = self.total_bytes.load(Ordering::Acquire);
        let new_total = cur_total
            .saturating_sub(old_bytes)
            .saturating_add(new_bytes);
        if new_total > self.max_bytes {
            self.note_rejection();
            return Err(BlackboardError::SizeLimitExceeded {
                current: new_total,
                max: self.max_bytes,
            });
        }
        // **WS3** : cap par namespace (ns_bytes est une DashMap distincte de
        // `fields` → pas de réentrance sur le shard tenu par `entry`).
        let ns_applied = self.check_namespace_cap(pointer, old_bytes, new_bytes)?;

        let new_version = old_version + 1;
        let bv = BlackboardValue {
            value,
            version: new_version,
            written_by: agent_id.clone(),
            written_at: SystemTime::now(),
        };
        match entry {
            Entry::Occupied(mut o) => {
                o.insert(bv);
            }
            Entry::Vacant(v) => {
                v.insert(bv);
            }
        }
        // `entry` (et donc la garde du shard `fields`) est relâchée ICI ; les
        // hooks ci-dessous ne risquent aucune réentrance sur le même shard.
        self.total_bytes.store(new_total, Ordering::Release);
        self.commit_namespace(ns_applied);

        self.record_write(
            BlackboardWrite {
                pointer: pointer.to_string(),
                version: new_version,
                agent_id,
                at: SystemTime::now(),
                bytes: new_bytes,
            },
            new_total,
        );
        Ok(new_version)
    }

    /// **N21** : Read the current value at a pointer (cloned to drop
    /// the DashMap shard guard immediately — I17).
    pub fn read(&self, pointer: &str) -> Option<BlackboardValue> {
        self.fields.get(pointer).map(|r| r.value().clone())
    }

    /// **N21** : True if a pointer is currently set.
    pub fn contains(&self, pointer: &str) -> bool {
        self.fields.contains_key(pointer)
    }

    /// **N21** : Capture a deterministic snapshot — sorted by
    /// pointer for replay determinism (I13).
    pub fn snapshot(&self) -> BlackboardSnapshot {
        let mut fields: BTreeMap<String, BlackboardValue> = BTreeMap::new();
        for entry in self.fields.iter() {
            fields.insert(entry.key().clone(), entry.value().clone());
        }
        BlackboardSnapshot {
            swarm_id: self.swarm_id,
            captured_at: SystemTime::now(),
            fields,
        }
    }

    /// **WS3** : historique RÉCENT borné (les N dernières écritures), ordre
    /// d'insertion. Pour l'historique complet, brancher un [`WriteSink`].
    pub fn write_history(&self) -> Vec<BlackboardWrite> {
        self.recent
            .lock()
            .map(|h| h.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// **WS3** : nombre EXACT d'écritures (atomique, jamais borné) — O(1).
    pub fn write_count(&self) -> usize {
        self.writes_total.load(Ordering::Relaxed) as usize
    }

    /// **WS3** : nombre EXACT d'écritures par agent — O(1) (DashMap atomique).
    pub fn writes_by_agent(&self, agent_id: &AgentId) -> usize {
        self.writes_per_agent
            .get(agent_id)
            .map(|a| a.load(Ordering::Relaxed) as usize)
            .unwrap_or(0)
    }

    /// **WS3** : écritures rejetées (dépassement de cap global ou namespace).
    pub fn rejections(&self) -> u64 {
        self.rejections_total.load(Ordering::Relaxed)
    }

    /// **WS3** : taille courante du ring d'historique récent.
    pub fn recent_len(&self) -> usize {
        self.recent.lock().map(|h| h.len()).unwrap_or(0)
    }

    /// **WS3** : octets cumulés d'un namespace (0 si non suivi).
    pub fn namespace_bytes(&self, prefix: &str) -> usize {
        self.ns_bytes
            .get(prefix)
            .map(|a| a.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    // ── WS3/WS5 : branchements (seams) ─────────────────────────────────────────

    /// **WS3** : branche un exutoire d'historique (provenance / journal).
    pub fn set_write_sink(&self, sink: Arc<dyn WriteSink>) {
        if let Ok(mut s) = self.sink.write() {
            *s = Some(sink);
        }
    }

    /// **WS5** : branche le registre de métriques du substrat.
    pub fn set_metrics(&self, metrics: Arc<SubstrateMetrics>) {
        if let Ok(mut m) = self.metrics.write() {
            *m = Some(metrics);
        }
    }

    /// **WS5** : enregistre un observateur (appelé à CHAQUE écriture committée).
    pub fn register_observer(&self, observer: Arc<dyn BlackboardObserver>) {
        if let Ok(mut o) = self.observers.write() {
            o.push(observer);
        }
    }

    // ── WS3 : éviction par namespace / epoch ───────────────────────────────────

    /// **WS3** : supprime toutes les cellules dont la clé commence par `prefix`,
    /// décrémente `total_bytes` et remet à zéro les compteurs de namespace
    /// concernés. Sert à réinitialiser les namespaces transients entre runs.
    pub fn evict_namespace(&self, prefix: &str) {
        let keys: Vec<String> = self
            .fields
            .iter()
            .filter(|e| e.key().starts_with(prefix))
            .map(|e| e.key().clone())
            .collect();
        let mut removed = 0usize;
        for k in keys {
            if let Some((_, v)) = self.fields.remove(&k) {
                removed = removed.saturating_add(serialised_size(&v.value));
            }
        }
        // total_bytes -= removed (pattern load/store, cohérent avec write).
        let cur = self.total_bytes.load(Ordering::Acquire);
        self.total_bytes
            .store(cur.saturating_sub(removed), Ordering::Release);
        // remet à zéro les ns_bytes des namespaces couverts.
        for (p, _) in &self.namespace_caps {
            if p.starts_with(prefix) || prefix.starts_with(p.as_str()) {
                if let Some(a) = self.ns_bytes.get(p) {
                    a.store(0, Ordering::Release);
                }
            }
        }
        if let Ok(m) = self.metrics.read() {
            if let Some(m) = m.as_ref() {
                m.set_bb_bytes(self.total_bytes.load(Ordering::Relaxed));
            }
        }
    }

    /// **WS3** : ouvre une nouvelle « epoch » en évinçant des namespaces
    /// transients (ex. `["/cand/", "/claim/"]`) — purge entre deux runs d'essaim.
    pub fn new_epoch(&self, transient_prefixes: &[&str]) {
        for p in transient_prefixes {
            self.evict_namespace(p);
        }
    }

    // ── Privés ─────────────────────────────────────────────────────────────────

    /// Chemin commun post-commit : compteurs O(1), ring borné, sink, métriques,
    /// observers. Appelé HORS de toute garde de shard `fields`.
    fn record_write(&self, w: BlackboardWrite, new_total: usize) {
        self.writes_total.fetch_add(1, Ordering::Relaxed);
        self.writes_per_agent
            .entry(w.agent_id.clone())
            .or_default()
            .fetch_add(1, Ordering::Relaxed);
        // ring borné (corrige F4 : mémoire bornée).
        if self.recent_cap > 0 {
            if let Ok(mut r) = self.recent.lock() {
                while r.len() >= self.recent_cap {
                    r.pop_front();
                }
                r.push_back(w.clone());
            }
        }
        // sink (streaming historique complet hors chemin chaud).
        if let Ok(s) = self.sink.read() {
            if let Some(s) = s.as_ref() {
                s.on_write(&w);
            }
        }
        // métriques (WS5).
        if let Ok(m) = self.metrics.read() {
            if let Some(m) = m.as_ref() {
                m.record_bb_write();
                m.set_bb_bytes(new_total);
            }
        }
        // observers (hook désormais CÂBLÉ).
        if let Ok(obs) = self.observers.read() {
            for o in obs.iter() {
                o.on_write(&w);
            }
        }
    }

    fn note_rejection(&self) {
        self.rejections_total.fetch_add(1, Ordering::Relaxed);
        if let Ok(m) = self.metrics.read() {
            if let Some(m) = m.as_ref() {
                m.record_bb_rejection();
            }
        }
    }

    fn note_conflict(&self) {
        if let Ok(m) = self.metrics.read() {
            if let Some(m) = m.as_ref() {
                m.record_bb_conflict();
            }
        }
    }

    fn note_cas_attempt(&self) {
        if let Ok(m) = self.metrics.read() {
            if let Some(m) = m.as_ref() {
                m.record_bb_cas_attempt();
            }
        }
    }

    /// Index du namespace le plus spécifique couvrant `pointer` (caps triés
    /// longest-first → première correspondance = la plus longue).
    fn ns_match(&self, pointer: &str) -> Option<usize> {
        self.namespace_caps
            .iter()
            .position(|(p, _)| pointer.starts_with(p.as_str()))
    }

    /// Vérifie le cap du namespace ; renvoie `Some((idx, projeté))` à committer,
    /// `None` si aucun namespace, `Err` si dépassé.
    fn check_namespace_cap(
        &self,
        pointer: &str,
        old_bytes: usize,
        new_bytes: usize,
    ) -> Result<Option<(usize, usize)>, BlackboardError> {
        if self.namespace_caps.is_empty() {
            return Ok(None);
        }
        let Some(idx) = self.ns_match(pointer) else {
            return Ok(None);
        };
        let (prefix, cap) = &self.namespace_caps[idx];
        let cur = self
            .ns_bytes
            .get(prefix)
            .map(|a| a.load(Ordering::Acquire))
            .unwrap_or(0);
        let projected = cur.saturating_sub(old_bytes).saturating_add(new_bytes);
        if projected > *cap {
            self.note_rejection();
            return Err(BlackboardError::SizeLimitExceeded {
                current: projected,
                max: *cap,
            });
        }
        Ok(Some((idx, projected)))
    }

    fn commit_namespace(&self, applied: Option<(usize, usize)>) {
        if let Some((idx, projected)) = applied {
            let prefix = &self.namespace_caps[idx].0;
            self.ns_bytes
                .entry(prefix.clone())
                .or_default()
                .store(projected, Ordering::Release);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  JSON pointer validation — RFC 6901 short check
// ─────────────────────────────────────────────────────────────────────

fn validate_pointer(p: &str) -> Result<(), BlackboardError> {
    if p.is_empty() {
        // RFC 6901 allows "" as the root pointer — but we don't
        // allow writing to root because callers should target a
        // specific field.  Reject.
        return Err(BlackboardError::InvalidPointer {
            value: "".into(),
        });
    }
    if !p.starts_with('/') {
        return Err(BlackboardError::InvalidPointer {
            value: p.to_string(),
        });
    }
    Ok(())
}

fn serialised_size(v: &serde_json::Value) -> usize {
    serde_json::to_vec(v).map(|b| b.len()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(s: &str) -> AgentId {
        AgentId::new(s).unwrap()
    }

    fn bb() -> Blackboard {
        Blackboard::new(SwarmId::new())
    }

    // ─── Basic write / read ─────────────────────────────────────────

    #[test]
    fn fresh_blackboard_is_empty() {
        let b = bb();
        assert_eq!(b.len(), 0);
        assert!(b.is_empty());
        assert_eq!(b.total_bytes(), 0);
    }

    #[test]
    fn write_then_read_returns_value() {
        let b = bb();
        let v = b
            .write("/x", serde_json::json!(42), agent("ner_extractor"))
            .unwrap();
        assert_eq!(v, 1);
        let bv = b.read("/x").unwrap();
        assert_eq!(bv.value, serde_json::json!(42));
        assert_eq!(bv.version, 1);
        assert_eq!(bv.written_by.as_str(), "ner_extractor");
    }

    #[test]
    fn write_increments_version_per_pointer() {
        let b = bb();
        let v1 = b
            .write("/x", serde_json::json!(1), agent("a-1"))
            .unwrap();
        let v2 = b
            .write("/x", serde_json::json!(2), agent("a-2"))
            .unwrap();
        assert_eq!(v1, 1);
        assert_eq!(v2, 2);
        let bv = b.read("/x").unwrap();
        assert_eq!(bv.value, serde_json::json!(2));
        assert_eq!(bv.version, 2);
        assert_eq!(bv.written_by.as_str(), "a-2");
    }

    #[test]
    fn write_to_different_pointers_independent() {
        let b = bb();
        b.write("/x", serde_json::json!(1), agent("aaa")).unwrap();
        b.write("/y", serde_json::json!(2), agent("aaa")).unwrap();
        assert_eq!(b.read("/x").unwrap().value, serde_json::json!(1));
        assert_eq!(b.read("/y").unwrap().value, serde_json::json!(2));
    }

    #[test]
    fn read_missing_pointer_returns_none() {
        let b = bb();
        assert!(b.read("/missing").is_none());
    }

    #[test]
    fn contains_reflects_write() {
        let b = bb();
        assert!(!b.contains("/x"));
        b.write("/x", serde_json::json!(1), agent("aaa")).unwrap();
        assert!(b.contains("/x"));
    }

    // ─── JSON pointer validation ────────────────────────────────────

    #[test]
    fn empty_pointer_rejected() {
        let b = bb();
        let err = b
            .write("", serde_json::json!(1), agent("aaa"))
            .unwrap_err();
        assert!(matches!(err, BlackboardError::InvalidPointer { .. }));
    }

    #[test]
    fn pointer_without_leading_slash_rejected() {
        let b = bb();
        let err = b
            .write("x", serde_json::json!(1), agent("aaa"))
            .unwrap_err();
        assert!(matches!(err, BlackboardError::InvalidPointer { .. }));
    }

    #[test]
    fn nested_pointer_accepted() {
        let b = bb();
        assert!(b
            .write("/foo/bar/baz", serde_json::json!(1), agent("aaa"))
            .is_ok());
        assert!(b.contains("/foo/bar/baz"));
    }

    // ─── write_if (CAS) ────────────────────────────────────────────

    #[test]
    fn write_if_expected_zero_succeeds_when_absent() {
        let b = bb();
        let v = b
            .write_if("/x", 0, serde_json::json!(1), agent("aaa"))
            .unwrap();
        assert_eq!(v, 1);
    }

    #[test]
    fn write_if_expected_zero_fails_when_present() {
        let b = bb();
        b.write("/x", serde_json::json!(1), agent("aaa")).unwrap();
        let err = b
            .write_if("/x", 0, serde_json::json!(2), agent("bbb"))
            .unwrap_err();
        match err {
            BlackboardError::Conflict {
                pointer,
                expected_version,
                actual_version,
            } => {
                assert_eq!(pointer, "/x");
                assert_eq!(expected_version, 0);
                assert_eq!(actual_version, 1);
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    #[test]
    fn write_if_matching_version_succeeds() {
        let b = bb();
        b.write("/x", serde_json::json!(1), agent("aaa")).unwrap();
        let v = b
            .write_if("/x", 1, serde_json::json!(2), agent("bbb"))
            .unwrap();
        assert_eq!(v, 2);
    }

    #[test]
    fn write_if_stale_version_fails() {
        let b = bb();
        b.write("/x", serde_json::json!(1), agent("aaa")).unwrap();
        b.write("/x", serde_json::json!(2), agent("bbb")).unwrap();
        // Stale CAS : expected 1, actual 2.
        let err = b
            .write_if("/x", 1, serde_json::json!(3), agent("ccc"))
            .unwrap_err();
        assert!(matches!(err, BlackboardError::Conflict { .. }));
    }

    // ─── Audit log ───────────────────────────────────────────────

    #[test]
    fn write_history_records_each_write() {
        let b = bb();
        b.write("/x", serde_json::json!(1), agent("aaa")).unwrap();
        b.write("/y", serde_json::json!(2), agent("bbb")).unwrap();
        b.write("/x", serde_json::json!(3), agent("aaa")).unwrap();
        let h = b.write_history();
        assert_eq!(h.len(), 3);
        assert_eq!(h[0].pointer, "/x");
        assert_eq!(h[1].pointer, "/y");
        assert_eq!(h[2].pointer, "/x");
        assert_eq!(h[2].version, 2);
    }

    #[test]
    fn writes_by_agent_counts_correctly() {
        let b = bb();
        b.write("/x", serde_json::json!(1), agent("alpha")).unwrap();
        b.write("/y", serde_json::json!(2), agent("alpha")).unwrap();
        b.write("/z", serde_json::json!(3), agent("beta")).unwrap();
        assert_eq!(b.writes_by_agent(&agent("alpha")), 2);
        assert_eq!(b.writes_by_agent(&agent("beta")), 1);
        assert_eq!(b.writes_by_agent(&agent("gamma")), 0);
    }

    #[test]
    fn write_count_total() {
        let b = bb();
        b.write("/x", serde_json::json!(1), agent("aaa")).unwrap();
        b.write("/y", serde_json::json!(2), agent("aaa")).unwrap();
        assert_eq!(b.write_count(), 2);
    }

    // ─── Size cap ────────────────────────────────────────────────

    #[test]
    fn write_under_cap_succeeds() {
        let b = Blackboard::with_cap(SwarmId::new(), 10_000);
        let value = serde_json::json!("x".repeat(100));
        let res = b.write("/a", value, agent("aaa"));
        assert!(res.is_ok());
        assert!(b.total_bytes() > 0);
    }

    #[test]
    fn write_over_cap_rejected() {
        let b = Blackboard::with_cap(SwarmId::new(), 100);
        let big = serde_json::json!("x".repeat(500));
        let err = b.write("/a", big, agent("aaa")).unwrap_err();
        assert!(matches!(err, BlackboardError::SizeLimitExceeded { .. }));
        // State not mutated.
        assert!(!b.contains("/a"));
        assert_eq!(b.total_bytes(), 0);
    }

    #[test]
    fn write_overwriting_releases_old_bytes() {
        let b = Blackboard::with_cap(SwarmId::new(), 200);
        // 100 bytes.
        b.write(
            "/a",
            serde_json::json!("x".repeat(80)),
            agent("aaa"),
        )
        .unwrap();
        let bytes_before = b.total_bytes();
        // Overwrite with 50 bytes ; total should shrink.
        b.write(
            "/a",
            serde_json::json!("y".repeat(20)),
            agent("aaa"),
        )
        .unwrap();
        assert!(b.total_bytes() < bytes_before);
    }

    // ─── Snapshot ────────────────────────────────────────────────

    #[test]
    fn snapshot_captures_all_fields_sorted() {
        let b = bb();
        b.write("/z", serde_json::json!(1), agent("aaa")).unwrap();
        b.write("/a", serde_json::json!(2), agent("aaa")).unwrap();
        b.write("/m", serde_json::json!(3), agent("aaa")).unwrap();
        let s = b.snapshot();
        let keys: Vec<&str> = s.fields.keys().map(|s| s.as_str()).collect();
        assert_eq!(keys, vec!["/a", "/m", "/z"]);
    }

    #[test]
    fn snapshot_after_writes_matches_field_count() {
        let b = bb();
        b.write("/x", serde_json::json!(1), agent("aaa")).unwrap();
        let s = b.snapshot();
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn snapshot_swarm_id_matches_blackboard() {
        let id = SwarmId::new();
        let b = Blackboard::new(id);
        let s = b.snapshot();
        assert_eq!(s.swarm_id, id);
    }

    // ─── Concurrency ─────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_writes_to_different_pointers_safe() {
        let b = std::sync::Arc::new(bb());
        let mut handles = Vec::new();
        for i in 0..50 {
            let b = b.clone();
            handles.push(tokio::spawn(async move {
                let pointer = format!("/key_{i}");
                let agent_name = format!("agent-{}", i % 10);
                b.write(&pointer, serde_json::json!(i), agent(&agent_name))
                    .unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(b.len(), 50);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_writes_to_same_pointer_all_succeed() {
        // write() is unconditional ; under concurrency the *final*
        // version may be lower than the number of writes because
        // multiple writers can read the same `old_version` and both
        // compute `new_version = old + 1` (a benign race for
        // non-CAS writes).  What we DO guarantee :
        // - All writes succeed (no errors).
        // - write_count (audit log) == number of writes.
        // - The final stored version is at most N and at least 1.
        // - The final stored value is one of the written values.
        //
        // Callers needing atomic version increment must use
        // `write_if` (CAS) instead.
        let b = std::sync::Arc::new(bb());
        let mut handles = Vec::new();
        for i in 0..20 {
            let b = b.clone();
            handles.push(tokio::spawn(async move {
                b.write("/shared", serde_json::json!(i), agent("aaa"))
                    .unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        let bv = b.read("/shared").unwrap();
        assert!(bv.version >= 1 && bv.version <= 20);
        assert_eq!(b.write_count(), 20);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_write_if_serialises_version_increment() {
        // write_if (CAS) — only one thread succeeds per version
        // transition.  Conflicts surface as `Conflict` errors.
        let b = std::sync::Arc::new(bb());
        b.write("/shared", serde_json::json!(0), agent("aaa")).unwrap();
        let mut handles = Vec::new();
        for i in 0..20 {
            let b = b.clone();
            handles.push(tokio::spawn(async move {
                // Try CAS from version 1 ; first wins, others
                // get Conflict.
                b.write_if("/shared", 1, serde_json::json!(i), agent("aaa"))
                    .ok()
            }));
        }
        let mut successes = 0;
        for h in handles {
            if h.await.unwrap().is_some() {
                successes += 1;
            }
        }
        // Exactly one CAS succeeded.
        assert_eq!(successes, 1);
        let bv = b.read("/shared").unwrap();
        assert_eq!(bv.version, 2);
    }

    // ─── Misc ────────────────────────────────────────────────────

    #[test]
    fn debug_format_no_panic() {
        let b = bb();
        let _ = format!("{b:?}");
    }

    #[test]
    fn blackboard_value_serde_round_trip() {
        let v = BlackboardValue {
            value: serde_json::json!({"a": 1}),
            version: 5,
            written_by: agent("aaa"),
            written_at: SystemTime::now(),
        };
        let json = serde_json::to_string(&v).unwrap();
        let back: BlackboardValue = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn max_bytes_default_256kb() {
        let b = bb();
        assert_eq!(b.max_bytes(), 262_144);
    }
}
