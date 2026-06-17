// SPDX-License-Identifier: UNLICENSED
//! # `chain` — la chaîne `memory_root` re-prouvable (le moat appliqué à la mémoire)
//!
//! Le **jumeau exact** de la chaîne N17 (`efe_swarm_provenance::seal_n17` /
//! `verify_n17_chain`, ADR-016) — même crypto, même clé tenant, même structure :
//! ```text
//! feuille_i = SHA-256(canonical_bytes(FoldRecord_i))                  (le contenu, déterministe)
//! hmac_0 = HMAC(genesis ‖ feuille_0, clé) ; hmac_1 = HMAC(hmac_0 ‖ feuille_1, clé) ; …
//!                                                  └── tip = memory_root
//! ```
//! L'**ordre canonique** est par `run_id` (ADR-007 — stable, jamais l'horloge) : la racine est
//! une fonction de l'ENSEMBLE des replis, insensible à l'ordre d'arrivée et à l'ordre de
//! stockage. Une falsification d'un repli **casse la chaîne et est localisée** ; une forge de
//! feuilles sans la clé casse au niveau chaîne (repli HMAC non forgeable). **Re-prouvable hors
//! ligne** : un tiers, avec les seuls `FoldRecord` persistés + la clé tenant, recompose
//! `memory_root` — et peut démontrer que « l'état mémoire est le repli honnête de ces runs ».

use efe_swarm_provenance::chain_hmac;
use uuid::Uuid;

use crate::family::FamilyKey;
use crate::fold::RunSummary;
use crate::{hex32, sha256_content_of};

/// Un **repli** : l'enregistrement, chaîné par HMAC, qu'un run propre a plié son résultat dans
/// la mémoire. Self-contained — il porte la projection (`summary`) que [`crate::materialize`]
/// re-replie pour recomposer les cellules, et que la chaîne hache pour la tamper-evidence.
/// Jumeau du `LlmProvenanceRecord` pour la chaîne N17.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FoldRecord {
    /// L'identité stable du run qui a plié (idempotence + ordre canonique).
    pub run_id: Uuid,
    /// La famille à laquelle ce repli appartient.
    pub family: FamilyKey,
    /// La projection du run (ce que le fold consomme + ce que la feuille hache).
    pub summary: RunSummary,
}

/// Le hash de contenu **déterministe** d'un repli : SHA-256 de sa forme JSON canonique — la MÊME
/// crypto de feuille que la chaîne N17 (le « jumeau exact »). Distinct de l'épingle d'instantané
/// (BLAKE3) : cf. [`crate::sha256_content_of`].
fn leaf_hash(rec: &FoldRecord) -> [u8; 32] {
    sha256_content_of(rec)
}

/// **Ordre canonique** des replis : par `run_id` (déterministe, stable à la relecture — chaque
/// repli porte son id). PAS l'ordre d'arrivée/stockage (qui dépendrait de l'ordonnancement
/// concurrent — la leçon d'ADR-007). Partagé par la chaîne et la matérialisation des cellules.
pub fn canonical_order(journal: &[FoldRecord]) -> Vec<&FoldRecord> {
    let mut v: Vec<&FoldRecord> = journal.iter().collect();
    v.sort_by_key(|r| r.run_id);
    v
}

/// Le résultat d'un scellement mémoire : les feuilles (ordre canonique) + `memory_root`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedMemory {
    /// SHA-256 de contenu par repli, en **ordre canonique** (par `run_id`).
    pub leaf_hashes: Vec<[u8; 32]>,
    /// `memory_root` : le HMAC replié sur les feuilles ordonnées (clé tenant). `None` si vide.
    pub memory_root: Option<[u8; 32]>,
}

impl SealedMemory {
    /// `memory_root` en hex (persistance / fil).
    pub fn root_hex(&self) -> Option<String> {
        self.memory_root.as_ref().map(hex32)
    }
    /// Les feuilles en hex (persistance de localisation).
    pub fn leaves_hex(&self) -> Vec<String> {
        self.leaf_hashes.iter().map(hex32).collect()
    }
}

/// **Scelle** un journal de replis en chaîne HMAC propre (clé tenant déjà dérivée). Ordre
/// canonique, feuille = SHA-256 de contenu, repli HMAC → `memory_root`. Pur, déterministe.
pub fn seal_memory(journal: &[FoldRecord], key: &[u8]) -> SealedMemory {
    let ordered = canonical_order(journal);
    let mut leaf_hashes = Vec::with_capacity(ordered.len());
    let mut tip: Option<[u8; 32]> = None;
    for r in ordered {
        let leaf = leaf_hash(r);
        tip = Some(chain_hmac(tip.as_ref(), &leaf, key));
        leaf_hashes.push(leaf);
    }
    SealedMemory { leaf_hashes, memory_root: tip }
}

/// Le **témoin racine persisté** d'une famille (ADR-019 §6.2, MEM-3.b) : la `memory_root` + les
/// feuilles SCELLÉES au dernier repli, conservées **séparément du journal**. C'est ce qui rend
/// `/verify` honnête : on re-dérive le journal courant et on le compare à CE témoin — une édition
/// **hors-bande** d'une ligne (le journal n'est plus celui qui a produit le témoin) casse la
/// chaîne et est **localisée** (I-MEM5). Sans témoin, re-sceller le journal contre lui-même est
/// une **tautologie** (intact toujours vrai). Le jumeau, côté mémoire, de `n17_root` persisté.
/// La racine n'est PAS `Option` : un témoin n'existe que pour un journal **non vide** (≥ 1 repli).
/// `Serialize`/`Deserialize` (les `[u8;32]` → tableaux JSON) pour la persistance durable (Pg JSONB).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MemoryWitness {
    pub memory_root: [u8; 32],
    pub leaf_hashes: Vec<[u8; 32]>,
}

impl MemoryWitness {
    /// Construit le témoin depuis un scellement (`None` si le journal scellé était vide — pas de
    /// racine à attester). À appeler au repli, sur le résultat de [`seal_memory`].
    pub fn from_sealed(sealed: &SealedMemory) -> Option<Self> {
        sealed.memory_root.map(|root| Self { memory_root: root, leaf_hashes: sealed.leaf_hashes.clone() })
    }
    pub fn root_hex(&self) -> String {
        hex32(&self.memory_root)
    }
}

/// Pourquoi la chaîne mémoire casse (jumeau de `N17Break`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryBreak {
    /// Le contenu d'un repli a été muté (feuille recalculée ≠ feuille scellée).
    RecordContentMutated,
    /// Le repli HMAC ne correspond plus à `memory_root` (feuilles/racine forgées sans la clé).
    ChainRootMismatch,
    /// Le nombre de replis relus ≠ le nombre de feuilles scellées (ajout/suppression).
    CountMismatch,
}

/// Verdict de re-preuve de la chaîne mémoire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryChainVerdict {
    /// Chaîne intègre (toutes les feuilles correspondent ET le repli == `memory_root`).
    pub intact: bool,
    /// Nombre de replis vérifiés.
    pub records: usize,
    /// `memory_root` recalculé depuis les replis persistés.
    pub root: Option<[u8; 32]>,
    /// Le repli fautif **localisé** (mutation de contenu), si déterminable — par `run_id`.
    pub first_break: Option<Uuid>,
    /// La nature de la rupture (`None` si intègre).
    pub reason: Option<MemoryBreak>,
}

impl MemoryChainVerdict {
    pub fn root_hex(&self) -> Option<String> {
        self.root.as_ref().map(hex32)
    }
}

/// **Re-prouve la chaîne mémoire** depuis les SEULS replis persistés + les feuilles scellées +
/// `memory_root` + la clé tenant. Localise un repli muté quand les feuilles n'ont pas été
/// touchées ; sinon casse au niveau chaîne (repli HMAC non forgeable). Pur, hors ligne — la
/// définition même du moat, appliquée à la mémoire (I-MEM5).
pub fn verify_memory(
    journal: &[FoldRecord],
    sealed_leaves: &[[u8; 32]],
    memory_root: Option<[u8; 32]>,
    key: &[u8],
) -> MemoryChainVerdict {
    let ordered = canonical_order(journal);

    // (a) cardinalité : un repli ajouté/supprimé après scellement.
    if ordered.len() != sealed_leaves.len() {
        return MemoryChainVerdict {
            intact: false,
            records: ordered.len(),
            root: None,
            first_break: ordered.first().map(|r| r.run_id),
            reason: Some(MemoryBreak::CountMismatch),
        };
    }

    // (b) contenu : chaque feuille recalculée doit égaler la feuille scellée → localise.
    for (r, sealed) in ordered.iter().zip(sealed_leaves.iter()) {
        if &leaf_hash(r) != sealed {
            return MemoryChainVerdict {
                intact: false,
                records: ordered.len(),
                root: None,
                first_break: Some(r.run_id),
                reason: Some(MemoryBreak::RecordContentMutated),
            };
        }
    }

    // (c) chaîne : rejoue le HMAC sur les feuilles scellées → compare à memory_root.
    let mut tip: Option<[u8; 32]> = None;
    for leaf in sealed_leaves {
        tip = Some(chain_hmac(tip.as_ref(), leaf, key));
    }
    if tip != memory_root {
        return MemoryChainVerdict {
            intact: false,
            records: ordered.len(),
            root: tip,
            first_break: None, // forge de feuilles/racine : pas de localisation fine
            reason: Some(MemoryBreak::ChainRootMismatch),
        };
    }

    MemoryChainVerdict { intact: true, records: ordered.len(), root: tip, first_break: None, reason: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::family::{family_key, FamilyMode};
    use efe_swarm_provenance::HmacKeySource;
    use efe_nano_sdk::TenantId;
    use serde_json::json;

    fn key() -> Vec<u8> {
        HmacKeySource::test_key().derive(TenantId::system()).unwrap()
    }
    fn fam() -> FamilyKey {
        family_key(&json!({ "text": "x" }), "consensus", &["g".into()], FamilyMode::Structural)
    }
    fn rec(run: u128, k: u32) -> FoldRecord {
        FoldRecord { run_id: Uuid::from_u128(run), family: fam(), summary: RunSummary::clean(k) }
    }
    fn three() -> Vec<FoldRecord> {
        vec![rec(1, 4), rec(2, 6), rec(3, 6)]
    }

    #[test]
    fn seal_then_verify_is_intact() {
        // I-MEM5 : sceller puis re-prouver → intègre, root recalculé == scellé.
        let j = three();
        let sealed = seal_memory(&j, &key());
        assert_eq!(sealed.leaf_hashes.len(), 3);
        assert!(sealed.memory_root.is_some());
        let v = verify_memory(&j, &sealed.leaf_hashes, sealed.memory_root, &key());
        assert!(v.intact, "chaîne mémoire saine : {v:?}");
        assert_eq!(v.records, 3);
        assert_eq!(v.root, sealed.memory_root);
        assert!(v.first_break.is_none());
    }

    #[test]
    fn canonical_order_makes_root_input_order_independent() {
        // I-MEM5 + ADR-007 : mêmes replis, ordres d'entrée différents → MÊME memory_root.
        let a = three();
        let mut b = three();
        b.reverse();
        assert_eq!(seal_memory(&a, &key()), seal_memory(&b, &key()), "ordre d'entrée neutre (tri par run_id)");
    }

    #[test]
    fn mutated_fold_breaks_and_localizes() {
        // I-MEM5 : muter le contenu d'un repli (son k_used) casse la chaîne ET localise par run_id.
        let j = three();
        let sealed = seal_memory(&j, &key());
        let mut tampered = j.clone();
        tampered[1].summary.k_used = 999; // contenu muté
        let v = verify_memory(&tampered, &sealed.leaf_hashes, sealed.memory_root, &key());
        assert!(!v.intact);
        assert_eq!(v.reason, Some(MemoryBreak::RecordContentMutated));
        assert_eq!(v.first_break, Some(j[1].run_id), "le repli muté est localisé");
    }

    #[test]
    fn forged_leaf_breaks_at_chain_level() {
        // Un attaquant mute un repli ET réécrit sa feuille pour coller — mais ne peut pas
        // re-HMAC sans la clé → casse au niveau chaîne (pas de localisation fine).
        let j = three();
        let sealed = seal_memory(&j, &key());
        let mut tampered = j.clone();
        tampered[0].summary.k_used = 42;
        // position canonique du repli muté + feuille forgée pour coller au contenu muté.
        let ordered = canonical_order(&j);
        let pos = ordered.iter().position(|r| r.run_id == j[0].run_id).unwrap();
        let mut leaves = sealed.leaf_hashes.clone();
        leaves[pos] = leaf_hash(&tampered[0]);
        let v = verify_memory(&tampered, &leaves, sealed.memory_root, &key());
        assert!(!v.intact);
        assert_eq!(v.reason, Some(MemoryBreak::ChainRootMismatch), "le repli HMAC non forgeable casse");
    }

    #[test]
    fn deleted_and_injected_break_via_count() {
        let j = three();
        let sealed = seal_memory(&j, &key());
        // suppression : relire 2 replis pour 3 feuilles.
        let v = verify_memory(&j[..2], &sealed.leaf_hashes, sealed.memory_root, &key());
        assert_eq!(v.reason, Some(MemoryBreak::CountMismatch));
        // injection : relire 3 replis pour 2 feuilles scellées.
        let s2 = seal_memory(&j[..2], &key());
        let v2 = verify_memory(&j, &s2.leaf_hashes, s2.memory_root, &key());
        assert_eq!(v2.reason, Some(MemoryBreak::CountMismatch));
        assert_eq!(v2.records, 3);
        // localisation en CountMismatch : first_break pointe le 1er repli en ordre canonique
        // (parité avec le test N17 injected_record_breaks_root_via_count_and_localizes).
        let first_canonical = j.iter().map(|r| r.run_id).min().unwrap();
        assert_eq!(v2.first_break, Some(first_canonical), "first_break localise le 1er repli canonique");
        assert_eq!(v.first_break, Some(first_canonical), "idem sur le chemin suppression");
    }

    #[test]
    fn wrong_tenant_key_breaks_the_chain() {
        // I-MEM6 (isolation) : re-prouver avec une AUTRE clé tenant casse la chaîne — la
        // mémoire d'un tenant est inverifiable (donc inexploitable) avec la clé d'un autre.
        let j = three();
        let sealed = seal_memory(&j, &key());
        let other_tenant_key = HmacKeySource::test_key()
            .derive(TenantId::from_uuid(Uuid::from_u128(0xBEEF)))
            .unwrap();
        let v = verify_memory(&j, &sealed.leaf_hashes, sealed.memory_root, &other_tenant_key);
        assert!(!v.intact);
        assert_eq!(v.reason, Some(MemoryBreak::ChainRootMismatch));
    }

    #[test]
    fn empty_journal_seals_and_verifies_vacuously() {
        let sealed = seal_memory(&[], &key());
        assert_eq!(sealed.memory_root, None);
        let v = verify_memory(&[], &[], None, &key());
        assert!(v.intact, "0 repli (famille neuve) = vide trivialement intègre");
    }
}
