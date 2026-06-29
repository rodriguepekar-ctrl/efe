// SPDX-License-Identifier: UNLICENSED
//! **L'intégrité-à-la-lecture** — re-prouver sans re-hacher tout le log à chaque requête, **sans jamais
//! sur-affirmer**.
//!
//! Deux chemins (cf. l'architecture §4) :
//! - **rapide (badge vivant)** : un cache clé-é par `(n_records, manifest_root)`. C'est **sain
//!   cryptographiquement** : le manifest-root **engage** l'ensemble des records (toute mutation casse la
//!   chaîne → root différent). Donc *même root ⟺ mêmes records cold-vérifiés* — un cache-hit O(1) est une
//!   ré-affirmation exacte du dernier verdict froid, pas une approximation.
//! - **froid (autoritaire)** : [`IntegrityMonitor::authoritative`] re-prouve **tout** (genèse→n) via S0.
//!   C'est ce que déclenchent « vérifier », **tout export**, et la cadence de fond. Le moment
//!   juridiquement signifiant ne fait **jamais** confiance au cache.
//!
//! Honnêteté de portée (testée, cf. `I-S2-NO-OVERCLAIM`) : le chemin rapide **ne prétend jamais** plus
//! que le dernier froid ; tout changement de `(len, root)` force un froid ; et le champ `as_of` de
//! l'enveloppe expose la fraîcheur (« vérifié froid il y a N, +Δ depuis »). L'optimisation **O(Δ)**
//! (watermark/delta, qui re-vérifie seulement le suffixe ajouté) est le **chemin d'échelle** nommé : il
//! exige une primitive S0 de continuation de chaîne, et n'est pas nécessaire au beachhead.

use provenance_capture::CaptureStore;
use provenance_dag::Verdict;

/// Le verdict froid mis en cache, avec l'état du store qui l'étaye.
#[derive(Debug, Clone)]
struct Cached {
    n_records: usize,
    manifest_root: String,
    verdict: Verdict,
    as_of: u64,
}

/// Le moniteur d'intégrité d'**un** store (donc d'un tenant). Tient le dernier verdict froid et sait
/// servir un badge rapide quand rien n'a changé.
#[derive(Debug, Default)]
pub struct IntegrityMonitor {
    cached: Option<Cached>,
}

impl IntegrityMonitor {
    pub fn new() -> Self {
        IntegrityMonitor { cached: None }
    }

    /// **Le badge vivant (rapide).** Renvoie `(verdict, as_of)`. Si l'état `(len, root)` du store est
    /// identique au dernier froid mis en cache → cache-hit O(1) (ré-affirmation exacte). Sinon → une
    /// re-preuve **froide** (le changement, p. ex. un append, est ainsi toujours froid-vérifié).
    ///
    /// Portée exacte de la sûreté : `store.manifest_root()` est le champ-chaîne **maintenu à l'append**
    /// d'un store **append-only en mémoire** — pas un re-hash live. Le cache-hit suppose donc l'invariant
    /// d'append en mémoire (cohérent par construction, le store n'étant pas mutable par un tiers). La
    /// falsification qui compte — au **repos** (le JSONL persisté, rechargé) — est rattrapée par le chemin
    /// **froid** ([`Self::authoritative`]), qui re-calcule depuis les records (via S0), et qui tourne à
    /// l'export, au « vérifier », et en cadence. Le rapide ne sert donc jamais que ce que le froid a établi.
    pub fn badge(&mut self, store: &CaptureStore, now: u64) -> (Verdict, u64) {
        let n = store.records().len();
        let root = store.manifest_root();
        if let Some(c) = &self.cached {
            if c.n_records == n && c.manifest_root == root {
                return (c.verdict.clone(), c.as_of); // O(1), root engage les records → sain
            }
        }
        self.refresh_cold(store, now)
    }

    /// **L'attestation autoritaire (froide).** Re-prouve TOUT via S0, met à jour le cache, renvoie
    /// `(verdict, as_of=now)`. À utiliser pour « vérifier », l'export, et la cadence.
    pub fn authoritative(&mut self, store: &CaptureStore, now: u64) -> (Verdict, u64) {
        self.refresh_cold(store, now)
    }

    fn refresh_cold(&mut self, store: &CaptureStore, now: u64) -> (Verdict, u64) {
        let verdict = store.reprove(); // re-preuve FROIDE complète (S0), via la clé encapsulée du store
        let manifest_root = store.manifest_root();
        self.cached = Some(Cached {
            n_records: store.records().len(),
            manifest_root,
            verdict: verdict.clone(),
            as_of: now,
        });
        (verdict, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use provenance_capture::{CaptureStore, LlmCall};
    use provenance_dag::TenantKey;

    fn store_with(n: usize) -> CaptureStore {
        let mut s = CaptureStore::new(TenantKey::from_bytes([9u8; 32]));
        for i in 0..n {
            s.record_llm(&LlmCall { case_id: "C", step: &format!("s{i}"), agent_role: "r", inputs: b"i", model: "m", version: "1", output: "PASS", evidence_steps: &[], timestamp: i as u64 }).unwrap();
        }
        s
    }

    #[test]
    fn cold_then_fast_hit_returns_same_as_of() {
        let s = store_with(3);
        let mut m = IntegrityMonitor::new();
        let (v1, as_of1) = m.authoritative(&s, 100);
        assert!(v1.ok);
        // état inchangé → cache-hit, MÊME as_of (la fraîcheur ne ment pas : c'est le froid de t=100).
        let (v2, as_of2) = m.badge(&s, 200);
        assert!(v2.ok);
        assert_eq!(as_of2, 100, "le badge rapide garde l'as_of du dernier froid");
        let _ = as_of1;
    }

    #[test]
    fn append_forces_a_fresh_cold_proof() {
        let mut s = store_with(2);
        let mut m = IntegrityMonitor::new();
        let (_, as_of1) = m.badge(&s, 100); // 1er appel → froid
        assert_eq!(as_of1, 100);
        // append → (len, root) changent → le prochain badge re-prouve froid avec le nouvel as_of.
        s.record_llm(&LlmCall { case_id: "C", step: "s99", agent_role: "r", inputs: b"i", model: "m", version: "1", output: "PASS", evidence_steps: &[], timestamp: 99 }).unwrap();
        let (v, as_of2) = m.badge(&s, 250);
        assert!(v.ok);
        assert_eq!(as_of2, 250, "un changement force un froid (jamais de cache périmé)");
        assert_eq!(v.n_records, 3);
    }

    #[test]
    fn tamper_is_caught_by_cold_and_never_masked() {
        // On simule un store rechargé depuis un fichier altéré : la re-preuve froide échoue.
        use provenance_dag::{reprove_dag, Record, SealedDag};
        let s = store_with(3);
        let tampered = s.to_jsonl().replacen("PASS", "FAIL", 1);
        let recs: Vec<Record> = SealedDag::records_from_jsonl(&tampered).unwrap();
        let v = reprove_dag(&recs, &TenantKey::from_bytes([9u8; 32]));
        assert!(!v.ok && v.broken_at.is_some(), "l'altération est rattrapée par le froid");
    }
}
