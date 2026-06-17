// SPDX-License-Identifier: UNLICENSED
//! # `store` — le `MemoryStore` (tenant-scopé, versionné) + l'impl en mémoire
//!
//! Le trait calqué sur le `RunStore` de la passerelle : **async** (l'impl Postgres viendra en
//! MEM-3), **tenant-scopé** (un tenant ne voit jamais la mémoire d'un autre — I-MEM6), et dont
//! le journal des replis est la **source de vérité** (cellules et `memory_root` s'en re-dérivent).
//!
//! ## La porte d'apprentissage (I-MEM3)
//! [`MemoryStore::fold`] ne replie QUE si le run est **propre** (`RunSummary::clean`, le miroir
//! de `SwarmRun::is_clean()` d'ADR-018). Un run tronqué (budget/deadline) ou non vérifié est
//! mis en **quarantaine** : il ne touche jamais le journal. La mémoire n'apprend que du propre.
//!
//! ## Idempotence (I-MEM3 bis)
//! Un `run_id` ne replie qu'**une fois** : un re-fold du même run est un no-op (`folded: false`).
//!
//! ## Déterminisme & re-preuve
//! [`MemoryStore::snapshot`] matérialise les cellules via [`crate::materialize`] (ordre
//! canonique) → `content_hash` reproductible (l'épingle, I-MEM1). [`FoldOutcome::memory_root`]
//! est scellé via [`crate::seal_memory`] avec la **clé HKDF du tenant** — re-prouvable hors ligne.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Mutex;

use async_trait::async_trait;
use efe_nano_sdk::TenantId;
use efe_swarm_provenance::HmacKeySource;
use uuid::Uuid;

use crate::cells::MemorySnapshot;
use crate::chain::{seal_memory, FoldRecord, MemoryWitness};
use crate::family::FamilyKey;
use crate::fold::{materialize, RunSummary};

/// Le journal des replis, indexé par `(tenant, family)` — l'isolation tenant est structurelle
/// (un tenant n'indexe que ses propres entrées). Alias pour lisibilité du champ du store.
type TenantJournals = Mutex<HashMap<(String, FamilyKey), Vec<FoldRecord>>>;

/// Les **témoins racine** persistés (root + feuilles du dernier repli), indexés comme le journal
/// mais conservés SÉPARÉMENT — c'est l'écart entre le journal courant et CE témoin qui détecte une
/// falsification hors-bande (MEM-3.b, I-MEM5). Cf. [`MemoryWitness`].
type TenantWitnesses = Mutex<HashMap<(String, FamilyKey), MemoryWitness>>;

/// Erreurs du store mémoire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemError {
    /// La clé tenant n'a pas pu être dérivée (tenant invalide / source de clé absente).
    KeyDerivation(String),
    /// Un verrou empoisonné (un thread a paniqué en le tenant) — refus prudent.
    Poisoned,
    /// **MEM-3** : erreur d'un store persistant (ex. Postgres) — requête, décodage, connexion.
    Backend(String),
}

impl std::fmt::Display for MemError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemError::KeyDerivation(e) => write!(f, "dérivation de clé tenant échouée : {e}"),
            MemError::Poisoned => write!(f, "verrou mémoire empoisonné"),
            MemError::Backend(e) => write!(f, "store mémoire (backend) : {e}"),
        }
    }
}
impl std::error::Error for MemError {}

/// Ce qu'un run propre demande à plier en mémoire (construit par le SDK en MEM-2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryUpdate {
    /// L'identité du run (idempotence + ordre canonique de la chaîne).
    pub run_id: Uuid,
    pub family: FamilyKey,
    /// La projection du run. Si `summary.clean == false`, le store NE replie PAS (I-MEM3).
    pub summary: RunSummary,
}

/// Le résultat d'un repli : la nouvelle version, le `content_hash` des cellules, le
/// `memory_root` mis à jour, et si le repli a **effectivement** eu lieu (`false` = gardé par
/// la porte clean, ou run_id déjà plié).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldOutcome {
    pub version: u64,
    pub content_hash: [u8; 32],
    pub memory_root: Option<[u8; 32]>,
    pub folded: bool,
}

/// Le contrat du store mémoire — async, tenant-scopé. L'impl Postgres (MEM-3) implémente le
/// même trait ; rien ici ne suppose un backend particulier.
#[async_trait]
pub trait MemoryStore: Send + Sync {
    /// Lit un **instantané gelé** (version courante) pour `(tenant, family)`. Tenant-scopé.
    async fn snapshot(&self, tenant: &str, family: &FamilyKey) -> Result<MemorySnapshot, MemError>;

    /// **ADR-019 (MEM-3, I-MEM7)** : l'instantané gelé à une version **passée** — l'état
    /// mémoire tel qu'il était après les `version` premiers replis. Permet de **ré-épingler**
    /// l'instantané exact qu'un run a lu, donc de reproduire ses sorties (R1 « sachant
    /// l'instantané », extension d'ADR-017). `version ≥ courant` ⇒ l'instantané courant (clampé).
    /// Le défaut renvoie l'instantané courant — un store sans rétention de versions reste correct
    /// pour la version courante (le seul cas où le caller n'épingle rien).
    async fn snapshot_at(
        &self,
        tenant: &str,
        family: &FamilyKey,
        _version: u64,
    ) -> Result<MemorySnapshot, MemError> {
        self.snapshot(tenant, family).await
    }

    /// Replie un run (si `clean`, I-MEM3 ; idempotent par `run_id`) → nouvelle version chaînée.
    async fn fold(&self, tenant: &str, update: MemoryUpdate) -> Result<FoldOutcome, MemError>;

    /// Le journal des replis (l'évidence pour re-prouver la chaîne hors ligne), tenant-scopé.
    async fn journal(&self, tenant: &str, family: &FamilyKey) -> Result<Vec<FoldRecord>, MemError>;

    /// **ADR-019 (MEM-3.b, I-MEM5)** : le **témoin racine persisté** de la famille (root + feuilles
    /// du dernier repli), conservé SÉPARÉMENT du journal. `None` si la famille n'a jamais replié.
    /// `/verify` compare la re-dérivation du journal courant à CE témoin → détecte + localise une
    /// falsification **hors-bande** (≠ re-sceller le journal contre lui-même, qui est une
    /// tautologie). Défaut `Ok(None)` : un store sans témoin retombe honnêtement sur la
    /// re-dérivation (non tamper-détectante) — mais les deux stores réels (InMemory, Pg) le fournissent.
    async fn witness(&self, tenant: &str, family: &FamilyKey) -> Result<Option<MemoryWitness>, MemError> {
        let _ = (tenant, family);
        Ok(None)
    }
}

/// Store mémoire **en mémoire** (dev / tests / standalone souverain). Le journal par
/// `(tenant, family)` est la source de vérité ; les cellules et le `memory_root` s'en re-dérivent.
pub struct InMemoryMemoryStore {
    /// Source de clé de provenance (dev : `test_key` ; prod : `EnvHkdf` server-side, MEM-3).
    keys: HmacKeySource,
    /// Journal des replis, indexé par `(tenant, family)` — l'isolation tenant est structurelle.
    journals: TenantJournals,
    /// Témoins racine persistés (MEM-3.b) — SÉPARÉS du journal pour la détection hors-bande.
    witnesses: TenantWitnesses,
}

impl Default for InMemoryMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryMemoryStore {
    /// Store dev : clé de test, journal vide.
    pub fn new() -> Self {
        Self { keys: HmacKeySource::test_key(), journals: Mutex::new(HashMap::new()), witnesses: Mutex::new(HashMap::new()) }
    }

    /// Store avec une source de clé explicite (prod : `EnvHkdf` server-side).
    pub fn with_keys(keys: HmacKeySource) -> Self {
        Self { keys, journals: Mutex::new(HashMap::new()), witnesses: Mutex::new(HashMap::new()) }
    }

    /// **Réservé aux tests** : remplace le journal d'une famille **sans** toucher au témoin —
    /// simule une **falsification hors-bande** du store durable (éditer une ligne en base sans
    /// pouvoir recalculer le témoin HMAC, qui exige la clé tenant). Sert à prouver que `/verify`
    /// compare au **témoin persisté** (et casse), au lieu de re-sceller le journal contre
    /// lui-même (tautologie). Hors test, aucun usage légitime — d'où `#[doc(hidden)]`.
    #[doc(hidden)]
    pub fn overwrite_journal_for_test(&self, tenant: &str, family: &FamilyKey, journal: Vec<FoldRecord>) {
        self.journals
            .lock()
            .expect("verrou journal")
            .insert((tenant.to_string(), family.clone()), journal);
    }

    /// Dérive la clé HKDF du tenant (miroir de `SwarmGatewayState::derive_key`) : `"system"`
    /// → tenant système ; sinon parse l'UUID tenant. Tenant invalide → `KeyDerivation`.
    fn derive(&self, tenant: &str) -> Result<Vec<u8>, MemError> {
        let tid = if tenant == "system" {
            TenantId::system()
        } else {
            TenantId::from_str(tenant).map_err(|e| MemError::KeyDerivation(e.to_string()))?
        };
        self.keys.derive(tid).map_err(|e| MemError::KeyDerivation(e.to_string()))
    }
}

#[async_trait]
impl MemoryStore for InMemoryMemoryStore {
    async fn snapshot(&self, tenant: &str, family: &FamilyKey) -> Result<MemorySnapshot, MemError> {
        // Le content_hash ne dépend que des cellules (pas de la clé) ; l'isolation tenant est
        // assurée par la clé de map (tenant, family) — un tenant ne lit que SON journal.
        let journals = self.journals.lock().map_err(|_| MemError::Poisoned)?;
        let journal = journals.get(&(tenant.to_string(), family.clone())).cloned().unwrap_or_default();
        let version = journal.len() as u64;
        let cells = materialize(&journal);
        Ok(MemorySnapshot::freeze(version, cells))
    }

    async fn snapshot_at(&self, tenant: &str, family: &FamilyKey, version: u64) -> Result<MemorySnapshot, MemError> {
        // Reconstruction d'un instantané PASSÉ : le journal est append-only, donc « version N »
        // = les N premiers replis insérés. On replie ce préfixe (materialize re-trie
        // canoniquement, donc le résultat est stable). `version` au-delà du courant est clampé.
        let journals = self.journals.lock().map_err(|_| MemError::Poisoned)?;
        let journal = journals.get(&(tenant.to_string(), family.clone())).cloned().unwrap_or_default();
        let n = (version as usize).min(journal.len());
        let cells = materialize(&journal[..n]);
        Ok(MemorySnapshot::freeze(n as u64, cells))
    }

    async fn fold(&self, tenant: &str, update: MemoryUpdate) -> Result<FoldOutcome, MemError> {
        // La clé tenant scelle le memory_root (I-MEM6) ; dérivée hors du verrou (échec net).
        let key = self.derive(tenant)?;
        let family = update.family.clone();
        let mut journals = self.journals.lock().map_err(|_| MemError::Poisoned)?;
        let journal = journals.entry((tenant.to_string(), family.clone())).or_default();

        // ① La PORTE (I-MEM3) : un run non-propre ne replie pas — no-op, état inchangé.
        // ② Idempotence : un run_id déjà plié ne replie pas — no-op.
        let already = journal.iter().any(|r| r.run_id == update.run_id);
        let folded = update.summary.clean && !already;
        if folded {
            journal.push(FoldRecord {
                run_id: update.run_id,
                family: update.family.clone(),
                summary: update.summary,
            });
        }

        let cells = materialize(journal);
        let sealed = seal_memory(journal, &key);
        let outcome = FoldOutcome {
            version: journal.len() as u64,
            content_hash: cells.content_hash(),
            memory_root: sealed.memory_root,
            folded,
        };
        // **ATOMICITÉ (revue adversariale)** : on écrit le témoin EN TENANT le verrou du journal.
        // Sinon deux folds concurrents sur la même famille peuvent réordonner journal-push et
        // témoin-insert (le témoin « retarde » le journal → `/verify` lèverait une fausse alarme
        // CountMismatch sur un journal honnête). Ordre de verrou stable journals→witnesses (aucun
        // chemin n'acquiert l'inverse) → pas d'inter-blocage ; aucun `.await` sous les verrous.
        if let Some(w) = MemoryWitness::from_sealed(&sealed) {
            self.witnesses
                .lock()
                .map_err(|_| MemError::Poisoned)?
                .insert((tenant.to_string(), family), w);
        }
        Ok(outcome)
    }

    async fn journal(&self, tenant: &str, family: &FamilyKey) -> Result<Vec<FoldRecord>, MemError> {
        let journals = self.journals.lock().map_err(|_| MemError::Poisoned)?;
        Ok(journals.get(&(tenant.to_string(), family.clone())).cloned().unwrap_or_default())
    }

    async fn witness(&self, tenant: &str, family: &FamilyKey) -> Result<Option<MemoryWitness>, MemError> {
        let witnesses = self.witnesses.lock().map_err(|_| MemError::Poisoned)?;
        Ok(witnesses.get(&(tenant.to_string(), family.clone())).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::verify_memory;
    use crate::family::{family_key, FamilyMode};
    use crate::fold::AgentStat;
    use serde_json::json;

    const TA: &str = "system";
    const TB: &str = "11111111-1111-1111-1111-111111111111";

    fn fam() -> FamilyKey {
        family_key(&json!({ "text": "x" }), "consensus", &["g".into()], FamilyMode::Structural)
    }
    fn clean(run: u128, k: u32) -> MemoryUpdate {
        MemoryUpdate { run_id: Uuid::from_u128(run), family: fam(), summary: RunSummary::clean(k) }
    }

    #[tokio::test]
    async fn fold_then_snapshot_reflects_learning() {
        // La mémoire APPREND : après quelques replis, l'instantané porte les priors.
        let store = InMemoryMemoryStore::new();
        for (run, k) in [(1u128, 6u32), (2, 6), (3, 4)] {
            let out = store.fold(TA, clean(run, k)).await.unwrap();
            assert!(out.folded, "run propre → replié");
        }
        let snap = store.snapshot(TA, &fam()).await.unwrap();
        assert_eq!(snap.version, 3);
        assert_eq!(snap.cells.size.histogram.get(&6), Some(&2));
        assert_eq!(snap.cells.size.histogram.get(&4), Some(&1));
        // l'instantané a une racine re-prouvable via le journal + la clé tenant.
        let journal = store.journal(TA, &fam()).await.unwrap();
        let key = HmacKeySource::test_key().derive(TenantId::system()).unwrap();
        let sealed = seal_memory(&journal, &key);
        let v = verify_memory(&journal, &sealed.leaf_hashes, sealed.memory_root, &key);
        assert!(v.intact, "le journal de l'instantané se re-prouve");
    }

    #[tokio::test]
    async fn dirty_run_is_quarantined_not_learned() {
        // I-MEM3 : un run NON-propre (tronqué/non vérifié) ne touche pas la mémoire.
        let store = InMemoryMemoryStore::new();
        let before = store.snapshot(TA, &fam()).await.unwrap();
        let mut dirty = clean(7, 6);
        dirty.summary.clean = false; // budget/deadline-tronqué, ou non vérifié
        let out = store.fold(TA, dirty).await.unwrap();
        assert!(!out.folded, "run sale → NON replié (quarantaine)");
        let after = store.snapshot(TA, &fam()).await.unwrap();
        assert_eq!(before.content_hash, after.content_hash, "la mémoire n'a PAS bougé");
        assert_eq!(after.version, 0);
    }

    #[tokio::test]
    async fn fold_is_idempotent_by_run_id() {
        // I-MEM3 bis : replier le MÊME run deux fois ne compte qu'une fois.
        let store = InMemoryMemoryStore::new();
        let first = store.fold(TA, clean(1, 6)).await.unwrap();
        assert!(first.folded);
        let again = store.fold(TA, clean(1, 6)).await.unwrap();
        assert!(!again.folded, "même run_id → no-op");
        assert_eq!(first.content_hash, again.content_hash, "état inchangé");
        let snap = store.snapshot(TA, &fam()).await.unwrap();
        assert_eq!(snap.version, 1, "un seul repli compté");
    }

    #[tokio::test]
    async fn snapshot_content_hash_evolves_then_pins() {
        // I-MEM1 : chaque repli fait évoluer le content_hash (l'épingle), de façon déterministe.
        let store = InMemoryMemoryStore::new();
        let h0 = store.snapshot(TA, &fam()).await.unwrap().content_hash;
        store.fold(TA, clean(1, 6)).await.unwrap();
        let h1 = store.snapshot(TA, &fam()).await.unwrap().content_hash;
        store.fold(TA, clean(2, 4)).await.unwrap();
        let h2 = store.snapshot(TA, &fam()).await.unwrap().content_hash;
        assert_ne!(h0, h1, "le 1er repli change l'épingle");
        assert_ne!(h1, h2, "le 2e repli change l'épingle");
        // re-lire le même état → même épingle (reproductible).
        let h2b = store.snapshot(TA, &fam()).await.unwrap().content_hash;
        assert_eq!(h2, h2b, "même état → même content_hash (déterministe)");
    }

    #[tokio::test]
    async fn tenants_are_isolated() {
        // I-MEM6 : le tenant A apprend ; le tenant B ne voit RIEN (map scopée) et leurs racines
        // diffèrent (clés HKDF distinctes).
        let store = InMemoryMemoryStore::new();
        let a = store.fold(TA, clean(1, 6)).await.unwrap();
        let b_snap = store.snapshot(TB, &fam()).await.unwrap();
        assert_eq!(b_snap.version, 0, "B ne voit pas la mémoire de A");
        assert_eq!(b_snap.cells, crate::cells::MemoryCells::default());
        // B replie de son côté : racine différente de A (même contenu, clé différente).
        let b = store.fold(TB, clean(1, 6)).await.unwrap();
        assert_eq!(
            store.snapshot(TA, &fam()).await.unwrap().content_hash,
            store.snapshot(TB, &fam()).await.unwrap().content_hash,
            "même contenu → même content_hash (le hash ne dépend pas de la clé)"
        );
        assert_ne!(a.memory_root, b.memory_root, "mais les RACINES diffèrent (clés tenant distinctes)");
    }

    #[tokio::test]
    async fn reputation_accumulates_across_runs() {
        let store = InMemoryMemoryStore::new();
        for run in 1u128..=3 {
            let mut u = clean(run, 5);
            u.summary.agents = vec![
                AgentStat { agent_id: "g".into(), verified: true, won: run == 1 },
                AgentStat { agent_id: "j".into(), verified: false, won: false },
            ];
            store.fold(TA, u).await.unwrap();
        }
        let snap = store.snapshot(TA, &fam()).await.unwrap();
        let g = snap.cells.reputation.of("g").unwrap();
        assert_eq!((g.total, g.verified, g.won), (3, 3, 1), "réputation cumulée sur 3 runs");
        assert_eq!(g.verified_rate_milli(), 1000);
        assert_eq!(g.win_rate_milli(), 333);
    }

    #[tokio::test]
    async fn witness_detects_out_of_band_journal_tampering() {
        // MEM-3.b / I-MEM5 : le store persiste un TÉMOIN racine (root + feuilles), SÉPARÉ du
        // journal. Re-dériver le journal courant et le comparer à CE témoin détecte une édition
        // HORS-BANDE d'une ligne — la falsification CASSE la chaîne et est LOCALISÉE (≠ re-sceller
        // le journal contre lui-même, qui serait une tautologie : intact toujours vrai).
        let store = InMemoryMemoryStore::new();
        for run in 1u128..=3 {
            store.fold(TA, clean(run, 6)).await.unwrap();
        }
        let key = HmacKeySource::test_key().derive(TenantId::system()).unwrap();
        let witness = store.witness(TA, &fam()).await.unwrap().expect("un témoin existe après repli");
        assert_eq!(witness.leaf_hashes.len(), 3, "le témoin couvre les 3 replis");

        // ① journal intact → re-dérivation == témoin (intact).
        let journal = store.journal(TA, &fam()).await.unwrap();
        let ok = verify_memory(&journal, &witness.leaf_hashes, Some(witness.memory_root), &key);
        assert!(ok.intact, "journal non touché → la chaîne se re-prouve contre le témoin");

        // ② falsification HORS-BANDE : on édite le contenu d'un repli, on laisse le témoin.
        let mut tampered = journal.clone();
        tampered[1].summary.k_used = 999; // un k_used impossible — l'édition de ligne en base
        store.overwrite_journal_for_test(TA, &fam(), tampered.clone());
        let bad = verify_memory(&tampered, &witness.leaf_hashes, Some(witness.memory_root), &key);
        assert!(!bad.intact, "édition hors-bande → la chaîne CASSE contre le témoin");
        assert_eq!(
            bad.reason,
            Some(crate::chain::MemoryBreak::RecordContentMutated),
            "rupture localisée au CONTENU (≠ count/forge)"
        );
        assert!(bad.first_break.is_some(), "le repli fautif est LOCALISÉ (run_id)");
        // le témoin, lui, ne bouge pas : sans la clé tenant, il n'est pas re-forgeable.
        assert_eq!(store.witness(TA, &fam()).await.unwrap().unwrap().memory_root, witness.memory_root);
    }

    #[tokio::test]
    async fn snapshot_at_reconstructs_a_genuinely_past_prefix() {
        // I-MEM7 : snapshot_at(N) reconstruit l'état APRÈS les N premiers replis — un préfixe
        // RÉELLEMENT passé, distinct du courant. C'est ce que R1 ré-épingle pour reproduire un run.
        let store = InMemoryMemoryStore::new();
        store.fold(TA, clean(1, 6)).await.unwrap();
        let h1 = store.snapshot(TA, &fam()).await.unwrap().content_hash; // l'épingle à v1
        store.fold(TA, clean(2, 4)).await.unwrap();
        store.fold(TA, clean(3, 8)).await.unwrap();
        let current = store.snapshot(TA, &fam()).await.unwrap();
        assert_eq!(current.version, 3);

        // snapshot_at(1) == l'état historique à v1 (PAS le courant) — la RÉTENTION, pas un repli.
        let past = store.snapshot_at(TA, &fam(), 1).await.unwrap();
        assert_eq!(past.version, 1, "version du préfixe");
        assert_eq!(past.content_hash, h1, "content_hash == l'épingle d'alors (reproductible, I-MEM7)");
        assert_ne!(past.content_hash, current.content_hash, "≠ courant : c'est un PASSÉ distinct");

        // version ≥ courant → clampé au courant (jamais d'erreur, jamais au-delà).
        let clamped = store.snapshot_at(TA, &fam(), 99).await.unwrap();
        assert_eq!(clamped.version, 3, "version ≥ courant → instantané courant (clampé)");
        assert_eq!(clamped.content_hash, current.content_hash);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_folds_keep_witness_in_sync_with_journal() {
        // RÉGRESSION (revue adversariale) : sous folds CONCURRENTS sur la MÊME famille, le témoin
        // persisté reste EXACTEMENT synchrone avec le journal — le témoin est écrit SOUS le verrou
        // du journal. Sinon une course ferait « retarder » le témoin (moins de feuilles que le
        // journal) et /verify lèverait une FAUSSE alarme CountMismatch sur un journal honnête.
        let store = std::sync::Arc::new(InMemoryMemoryStore::new());
        let mut tasks = Vec::new();
        for run in 0u128..32 {
            let s = store.clone();
            tasks.push(tokio::spawn(async move {
                s.fold(TA, clean(run, 6)).await.unwrap();
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }

        let journal = store.journal(TA, &fam()).await.unwrap();
        let witness = store.witness(TA, &fam()).await.unwrap().expect("témoin présent");
        assert_eq!(journal.len(), 32, "32 replis distincts pliés");
        assert_eq!(
            witness.leaf_hashes.len(),
            journal.len(),
            "le témoin couvre EXACTEMENT le journal (aucun retard de course)"
        );
        let key = HmacKeySource::test_key().derive(TenantId::system()).unwrap();
        assert!(
            verify_memory(&journal, &witness.leaf_hashes, Some(witness.memory_root), &key).intact,
            "journal honnête → intègre (pas de fausse alarme de falsification)"
        );
    }
}
