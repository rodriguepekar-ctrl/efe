// SPDX-License-Identifier: UNLICENSED
//! # `chain` — la **chaîne HMAC par-swarm** (genèse par `swarm_id`, tip = `provenance_root`).
//!
//! Une [`SwarmChain`] enfile les atomes [`AgentCallRecord`] d'un **run d'essaim** en
//! une liste *hash-linkée* :
//!
//! ```text
//! genèse(swarm) ─► rec0.hmac = HMAC(b"genesis" ‖ rec0.payload_hash , clé_tenant)
//!                   rec1.hmac = HMAC(rec0.hmac  ‖ rec1.payload_hash , clé_tenant)
//!                   rec2.hmac = HMAC(rec1.hmac  ‖ rec2.payload_hash , clé_tenant)
//!                                    └──────────────── tip = provenance_root
//! ```
//!
//! **Un seul hash prouve tout le run** : le `provenance_root` (= dernier HMAC) commet,
//! par transitivité, chaque `payload_hash` et chaque lien parent. Muter, supprimer ou
//! réordonner **n'importe quelle** entrée change son HMAC, donc tous les suivants,
//! donc la racine — détecté par [`verify_chain`] **sans coopération du provider**.
//!
//! La clé est **dérivée par tenant** (HKDF, [`efe_audit_core::HmacKeySource`]) côté
//! serveur ; on ne la saisit jamais. Elle est zéroïsée (best-effort) au `Drop`.

use efe_audit_core::{chain_hmac, AuditKeyError, HmacKeySource};
use efe_nano_sdk::{SwarmId, TenantId};

use crate::record::{hex32, AgentCallDraft, AgentCallRecord};

/// Chaîne HMAC d'un run d'essaim. **Autoritaire** sur `(tenant, swarm_id)` : tampon-
/// ne ces champs sur chaque enregistrement scellé.
pub struct SwarmChain {
    tenant: TenantId,
    swarm_id: SwarmId,
    key: Vec<u8>,
    tip: Option<[u8; 32]>,
    next_seq: u64,
    len: usize,
}

impl SwarmChain {
    /// Construit une chaîne à partir d'une **clé déjà dérivée** (32 o, secret serveur).
    pub fn new(tenant: TenantId, swarm_id: SwarmId, key: Vec<u8>) -> Self {
        Self { tenant, swarm_id, key, tip: None, next_seq: 0, len: 0 }
    }

    /// Construit une chaîne en **dérivant** la clé tenant depuis une source (HKDF).
    /// La clé ne transite jamais par l'appelant.
    pub fn from_key_source(
        tenant: TenantId,
        swarm_id: SwarmId,
        src: &HmacKeySource,
    ) -> Result<Self, AuditKeyError> {
        let key = src.derive(tenant)?;
        Ok(Self::new(tenant, swarm_id, key))
    }

    /// Enfile un brouillon : calcule `seq`, lie au tip courant, calcule le HMAC,
    /// avance le tip, et renvoie l'enregistrement **scellé**. La chaîne tampon-ne
    /// son `(tenant, swarm_id)` sur le record (autorité).
    pub fn append(&mut self, mut draft: AgentCallDraft) -> AgentCallRecord {
        draft.swarm_id = self.swarm_id;
        draft.tenant_id = self.tenant;

        let payload_hash = draft.payload_hash();
        let parent = self.tip;
        let hmac = chain_hmac(parent.as_ref(), &payload_hash, &self.key);

        let seq = self.next_seq;
        self.next_seq += 1;
        self.len += 1;
        self.tip = Some(hmac);

        draft.seal(seq, parent, hmac)
    }

    /// Enfile plusieurs brouillons, renvoie les enregistrements scellés (ordre préservé).
    pub fn append_all<I: IntoIterator<Item = AgentCallDraft>>(&mut self, drafts: I) -> Vec<AgentCallRecord> {
        drafts.into_iter().map(|d| self.append(d)).collect()
    }

    /// **`provenance_root`** : le tip = un seul hash qui prouve tout le run (`None` si vide).
    pub fn root(&self) -> Option<[u8; 32]> {
        self.tip
    }

    /// `provenance_root` en hex (64 car.), pour journalisation / `TopologyOutput`.
    pub fn root_hex(&self) -> Option<String> {
        self.tip.as_ref().map(hex32)
    }

    /// Vérifie une liste d'enregistrements **avec la clé de cette chaîne**.
    pub fn verify(&self, records: &[AgentCallRecord]) -> ChainVerdict {
        verify_chain(records, &self.key)
    }

    pub fn tenant(&self) -> TenantId {
        self.tenant
    }
    pub fn swarm_id(&self) -> SwarmId {
        self.swarm_id
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }
}

impl Drop for SwarmChain {
    fn drop(&mut self) {
        // Zéroïsation best-effort (safe Rust ; durcissement `zeroize` = Inc.4, cf.
        // pattern efe-server `HmacKey`). Empêche de laisser traîner la clé en clair.
        for b in self.key.iter_mut() {
            *b = 0;
        }
    }
}

impl std::fmt::Debug for SwarmChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SwarmChain")
            .field("tenant", &self.tenant)
            .field("swarm_id", &self.swarm_id)
            .field("len", &self.len)
            .field("root", &self.root_hex())
            .finish() // NB : la clé n'est JAMAIS affichée.
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Vérification de l'intégrité de chaîne (précurseur du verify_swarm d'Inc.4)
// ─────────────────────────────────────────────────────────────────────────────

/// Pourquoi une chaîne casse à une entrée donnée.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakReason {
    /// Le `payload_hash` stocké ne correspond pas au contenu (sortie/entrée altérée).
    PayloadHashMismatch,
    /// Le HMAC recalculé ne correspond pas (mauvaise clé, ou HMAC forgé).
    HmacMismatch,
    /// `chain_parent_hmac` ne pointe pas sur le HMAC de l'entrée précédente (réordre).
    ParentLinkMismatch,
    /// La genèse (1ʳᵉ entrée) porte un parent (devrait être `None`).
    GenesisHasParent,
    /// `seq` non strictement croissant (réordre / doublon).
    NonMonotonicSeq,
}

/// L'entrée fautive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainBreak {
    pub index: usize,
    pub seq: u64,
    pub reason: BreakReason,
}

/// Verdict de vérification d'une chaîne.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainVerdict {
    /// Chaîne intègre de bout en bout.
    pub ok: bool,
    /// Nombre d'entrées vérifiées avant verdict.
    pub checked: usize,
    /// `provenance_root` recalculé (tip), si la chaîne est intègre.
    pub root: Option<[u8; 32]>,
    /// Première entrée fautive (forensique), `None` si intègre.
    pub first_break: Option<ChainBreak>,
}

/// Vérifie une chaîne d'enregistrements (ordre du slice = ordre `seq`) avec `key` :
/// 1. intégrité de contenu (`verify_payload_hash`),
/// 2. lien parent (`chain_parent_hmac` == HMAC précédent ; genèse = `None`),
/// 3. HMAC recalculé == stocké,
/// 4. `seq` strictement croissant.
///
/// Renvoie le `root` (tip) si intègre, sinon la première rupture. Ne dépend QUE des
/// enregistrements + la clé tenant : **un vérificateur indépendant** (le moat).
pub fn verify_chain(records: &[AgentCallRecord], key: &[u8]) -> ChainVerdict {
    let mut prev_hmac: Option<[u8; 32]> = None;
    let mut prev_seq: Option<u64> = None;

    for (index, r) in records.iter().enumerate() {
        let brk = |reason| ChainVerdict {
            ok: false,
            checked: index,
            root: None,
            first_break: Some(ChainBreak { index, seq: r.seq, reason }),
        };

        // 1. intégrité de contenu.
        if !r.verify_payload_hash() {
            return brk(BreakReason::PayloadHashMismatch);
        }
        // 2. lien parent.
        if index == 0 && r.chain_parent_hmac.is_some() {
            return brk(BreakReason::GenesisHasParent);
        }
        if index > 0 && r.chain_parent_hmac != prev_hmac {
            return brk(BreakReason::ParentLinkMismatch);
        }
        // 3. HMAC.
        let expected = chain_hmac(r.chain_parent_hmac.as_ref(), &r.payload_hash, key);
        if expected != r.hmac {
            return brk(BreakReason::HmacMismatch);
        }
        // 4. seq strictement croissant.
        if let Some(ps) = prev_seq {
            if r.seq <= ps {
                return brk(BreakReason::NonMonotonicSeq);
            }
        }

        prev_hmac = Some(r.hmac);
        prev_seq = Some(r.seq);
    }

    ChainVerdict { ok: true, checked: records.len(), root: prev_hmac, first_break: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{AgentKind, AgentRole};
    use efe_nano_sdk::{AgentId, Blake3Hash};
    use uuid::Uuid;

    fn key() -> Vec<u8> {
        HmacKeySource::test_key().derive(TenantId::system()).unwrap()
    }

    fn swarm() -> SwarmId {
        SwarmId::from_uuid(Uuid::from_u128(0x2222))
    }

    fn bh(b: u8) -> Blake3Hash {
        Blake3Hash::from_bytes([b; 32])
    }

    /// Brouillon déterministe #i (record_id fixe par i ⇒ chaînes reproductibles).
    fn draft(i: u128) -> AgentCallDraft {
        AgentCallDraft::new(
            swarm(),
            TenantId::system(),
            AgentId::new("generator").unwrap(),
            AgentRole::Generator,
            AgentKind::ModeA,
            bh(i as u8),
            bh((i + 1) as u8),
        )
        .with_record_id(Uuid::from_u128(0x1000 + i))
        .with_variant(i as u64)
    }

    fn chain() -> SwarmChain {
        SwarmChain::new(TenantId::system(), swarm(), key())
    }

    #[test]
    fn append_assigns_monotonic_seq_and_stamps_swarm() {
        let mut c = chain();
        let r0 = c.append(draft(0));
        let r1 = c.append(draft(1));
        assert_eq!(r0.seq, 0);
        assert_eq!(r1.seq, 1);
        assert_eq!(r0.swarm_id, swarm());
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn genesis_has_no_parent_then_links_to_prev_hmac() {
        let mut c = chain();
        let r0 = c.append(draft(0));
        let r1 = c.append(draft(1));
        assert_eq!(r0.chain_parent_hmac, None, "genèse sans parent");
        assert_eq!(r1.chain_parent_hmac, Some(r0.hmac), "lien = HMAC précédent");
    }

    #[test]
    fn root_is_last_hmac_and_advances() {
        let mut c = chain();
        assert_eq!(c.root(), None, "chaîne vide → pas de racine");
        let r0 = c.append(draft(0));
        assert_eq!(c.root(), Some(r0.hmac));
        let r1 = c.append(draft(1));
        assert_eq!(c.root(), Some(r1.hmac));
        assert_ne!(r0.hmac, r1.hmac);
    }

    #[test]
    fn identical_runs_produce_identical_root() {
        let mut a = chain();
        let mut b = chain();
        let ra = a.append_all((0..5).map(draft));
        let rb = b.append_all((0..5).map(draft));
        assert_eq!(a.root(), b.root(), "même clé + mêmes brouillons → même provenance_root");
        assert_eq!(ra.last().unwrap().hmac, rb.last().unwrap().hmac);
    }

    #[test]
    fn different_key_produces_different_root() {
        let mut a = SwarmChain::new(TenantId::system(), swarm(), key());
        let mut b = SwarmChain::new(TenantId::system(), swarm(), vec![0u8; 32]);
        a.append_all((0..3).map(draft));
        b.append_all((0..3).map(draft));
        assert_ne!(a.root(), b.root());
    }

    #[test]
    fn verify_accepts_an_honest_chain() {
        let mut c = chain();
        let recs = c.append_all((0..6).map(draft));
        let v = verify_chain(&recs, &key());
        assert!(v.ok, "chaîne saine acceptée : {v:?}");
        assert_eq!(v.checked, 6);
        assert_eq!(v.root, c.root(), "root du vérificateur == tip de la chaîne");
        assert!(v.first_break.is_none());
    }

    #[test]
    fn verify_detects_content_tampering_at_exact_index() {
        let mut c = chain();
        let mut recs = c.append_all((0..6).map(draft));
        // un attaquant substitue la sortie de l'entrée 3 (sans la clé).
        recs[3].output_hash = bh(0xFF);
        let v = verify_chain(&recs, &key());
        assert!(!v.ok);
        let brk = v.first_break.unwrap();
        assert_eq!(brk.index, 3);
        assert_eq!(brk.reason, BreakReason::PayloadHashMismatch);
    }

    #[test]
    fn verify_detects_reordering() {
        let mut c = chain();
        let mut recs = c.append_all((0..5).map(draft));
        recs.swap(2, 3); // réordonne → liens parents rompus
        let v = verify_chain(&recs, &key());
        assert!(!v.ok);
        // à l'index 2, chain_parent_hmac (de l'ancien rec3) != hmac de rec1.
        assert_eq!(v.first_break.unwrap().reason, BreakReason::ParentLinkMismatch);
    }

    #[test]
    fn verify_with_wrong_key_breaks_at_genesis() {
        let mut c = chain();
        let recs = c.append_all((0..3).map(draft));
        let v = verify_chain(&recs, &vec![0u8; 32]); // mauvaise clé
        assert!(!v.ok);
        let brk = v.first_break.unwrap();
        assert_eq!(brk.index, 0);
        assert_eq!(brk.reason, BreakReason::HmacMismatch);
    }

    #[test]
    fn empty_chain_verifies_vacuously() {
        let v = verify_chain(&[], &key());
        assert!(v.ok);
        assert_eq!(v.checked, 0);
        assert_eq!(v.root, None);
    }

    #[test]
    fn from_key_source_derives_and_builds() {
        let mut c = SwarmChain::from_key_source(TenantId::system(), swarm(), &HmacKeySource::test_key()).unwrap();
        let recs = c.append_all((0..4).map(draft));
        assert!(c.verify(&recs).ok);
        assert_eq!(c.root_hex().unwrap().len(), 64);
    }

    #[test]
    fn single_root_proves_the_whole_run() {
        // La propriété du moat : muter UNE entrée quelconque change le root recalculé.
        let mut honest = chain();
        let recs = honest.append_all((0..8).map(draft));
        let root = honest.root().unwrap();

        for i in 0..recs.len() {
            let mut tampered = recs.clone();
            tampered[i].output_hash = bh(0xAA); // altère l'entrée i
            // Re-sceller honnêtement la chaîne altérée donnerait un AUTRE root :
            let mut rebuilt = chain();
            let mut drafts: Vec<AgentCallDraft> = (0..8).map(draft).collect();
            drafts[i] = drafts[i].clone().with_record_id(Uuid::from_u128(0x9999)); // contenu différent
            let rebuilt_recs = rebuilt.append_all(drafts);
            assert_ne!(rebuilt.root().unwrap(), root, "un seul hash (root) prouve tout le run (entrée {i})");
            // Et le vérificateur rejette la version altérée-mais-non-rescellée :
            assert!(!verify_chain(&tampered, &key()).ok);
            let _ = rebuilt_recs;
        }
    }
}
