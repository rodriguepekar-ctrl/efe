// SPDX-License-Identifier: UNLICENSED
//! **L'inventaire du store — le primitif sans lequel aucune sauvegarde n'est possible.**
//!
//! ## Ce qui manquait, et pourquoi personne ne l'avait vu
//!
//! [`crate::store::RunStore`] a trois méthodes : `put`, `put_stored`, `get`. **Aucune n'énumère.**
//! On écrit un run, on le relit **par son identifiant**, et c'est tout. Un registre que l'AI Act
//! art. 12 demande de conserver dix ans, et qu'aucun code ne sait **parcourir**, ne peut être ni
//! exporté, ni sauvegardé, ni migré, ni audité en masse. Le trou n'était pas « il manque un script
//! de sauvegarde » : il manquait la seule opération dont un script aurait eu besoin.
//!
//! Il ne se voyait pas parce que rien ne le demande : la passerelle sert des runs un par un, et un
//! trait qui fait exactement ce que ses appelants demandent a l'air complet.
//!
//! ## Un trait SÉPARÉ, et c'est une décision de sûreté
//!
//! Énumérer, c'est lire **tous les tenants**. `RunStore::get` est tenant-scopé pour une raison :
//! l'isolation. Ajouter `inventorier` à `RunStore` mettrait une lecture inter-tenants à portée de
//! toute route qui tient un `Arc<dyn RunStore>` — c'est-à-dire de toute la surface servie, à un
//! oubli de garde près.
//!
//! D'où un trait à part. L'état de la passerelle ne le détient pas ; seul le binaire, en mode
//! exploitant, construit le store concret et peut l'appeler. Une route **ne peut pas** énumérer,
//! pas parce qu'on lui interdit, mais parce qu'elle n'en a pas le type.
//!
//! ## Pagination par CLÉ, jamais par `OFFSET`
//!
//! `LIMIT n OFFSET k` sur une table qui reçoit des écritures **saute et duplique** : une insertion
//! entre deux pages décale tout ce qui suit. Les runs sont *write-once*, mais ils continuent
//! d'arriver pendant l'export d'un registre vivant. La pagination par clé — « strictement après
//! `(created_at, id)` » — est stable sous insertion, et `id` départage les collisions de seconde.
//! Une sauvegarde qui perd silencieusement des lignes est pire que pas de sauvegarde : elle se
//! croit complète.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use efe_swarm_sdk::{AgentCallRecord, LlmProvenanceRecord};

use crate::error::GatewayError;
use crate::store::StoredRun;

/// Le curseur : le dernier couple `(created_at, id)` déjà rendu. `None` = depuis le début.
pub type Curseur = Option<(u64, Uuid)>;

/// **Une ligne d'export** — le run, plus son identité, sous une forme qui traverse un fichier.
///
/// [`StoredRun`] n'est délibérément **pas** `Serialize` : le rendre sérialisable le mettrait à un
/// `Json(...)` de distance d'une réponse HTTP, et il porte le tenant. Cette enveloppe est le seul
/// endroit du dépôt où un run devient du texte, et elle vit hors des routes.
///
/// Les champs reprennent **exactement** les colonnes de `swarm_runs`. Un champ oublié ici serait un
/// champ perdu à la restauration — silencieusement, puisque `serde` le remplirait par défaut.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LigneExport {
    pub id: Uuid,
    pub tenant: String,
    pub records: Vec<AgentCallRecord>,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub llm_records: Vec<LlmProvenanceRecord>,
    #[serde(default)]
    pub n17_leaves: Vec<String>,
    #[serde(default)]
    pub n17_root: Option<String>,
    pub output: Value,
    pub topology: String,
    #[serde(default)]
    pub request: Value,
    /// Époque UNIX en secondes — la même unité que la colonne `created_at`.
    pub created_at: u64,
}

impl LigneExport {
    /// Construit la ligne depuis un run inventorié.
    #[must_use]
    pub(crate) fn depuis(id: Uuid, r: &StoredRun) -> Self {
        Self {
            id,
            tenant: r.tenant.clone(),
            records: r.records.clone(),
            root: r.root.clone(),
            llm_records: r.llm_records.clone(),
            n17_leaves: r.n17_leaves.clone(),
            n17_root: r.n17_root.clone(),
            output: r.output.clone(),
            topology: r.topology.clone(),
            request: r.request.clone(),
            created_at: r.created_at_unix(),
        }
    }

    /// Reconstruit le run. **Aucune re-preuve ici** : l'import écrit ce qu'on lui donne, et c'est
    /// `GET /v1/swarm/verify/:id` qui dira si la chaîne tient — une restauration qui « répare »
    /// silencieusement ce qu'elle relit serait exactement le contraire d'une preuve.
    /// Gardé sur `postgres` **et** `test` : son seul appelant de production est
    /// [`importer_depuis`], qui n'existe que sous `postgres` — sans cette garde il serait du code
    /// mort sous les features par défaut, et la porte de qualité a raison de le dire. `test` le
    /// garde vivant pour l'aller-retour d'enveloppe, qui doit tourner partout.
    #[cfg(any(feature = "postgres", test))]
    #[must_use]
    pub(crate) fn vers_run(&self) -> StoredRun {
        StoredRun {
            tenant: self.tenant.clone(),
            records: self.records.clone(),
            root: self.root.clone(),
            llm_records: self.llm_records.clone(),
            n17_leaves: self.n17_leaves.clone(),
            n17_root: self.n17_root.clone(),
            output: self.output.clone(),
            topology: self.topology.clone(),
            request: self.request.clone(),
            created_at: std::time::UNIX_EPOCH + std::time::Duration::from_secs(self.created_at),
        }
    }

    /// La clé de pagination de cette ligne.
    #[must_use]
    pub fn cle(&self) -> (u64, Uuid) {
        (self.created_at, self.id)
    }
}

/// **Parcourir le store.** Volontairement hors de [`crate::store::RunStore`] — cf. l'en-tête.
#[async_trait]
pub trait RunStoreInventaire: Send + Sync {
    /// Au plus `limite` runs, ordonnés par `(created_at, id)` croissant, **strictement après**
    /// `apres`.
    ///
    /// Rendre moins de `limite` lignes ne signifie **pas** « fini » de façon fiable sur tous les
    /// stores ; l'appelant boucle jusqu'à une page **vide**. C'est un tour de boucle de plus, et
    /// c'est la seule condition d'arrêt qui ne puisse pas tronquer une sauvegarde.
    ///
    /// # Errors
    /// Toute défaillance du store sous-jacent.
    async fn inventorier(
        &self,
        apres: Curseur,
        limite: u32,
    ) -> Result<Vec<LigneExport>, GatewayError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::InMemoryRunStore;

    fn run(tenant: &str, t: u64) -> StoredRun {
        StoredRun {
            tenant: tenant.to_string(),
            records: Vec::new(),
            root: None,
            llm_records: Vec::new(),
            n17_leaves: Vec::new(),
            n17_root: None,
            output: serde_json::json!({"ok": true}),
            topology: "test".into(),
            request: serde_json::json!({}),
            created_at: std::time::UNIX_EPOCH + std::time::Duration::from_secs(t),
        }
    }

    #[tokio::test]
    async fn linventaire_rend_tout_dans_lordre_et_sans_doublon() {
        let s = InMemoryRunStore::new();
        let mut ids: Vec<Uuid> = Vec::new();
        for t in 0..7u64 {
            let id = Uuid::new_v4();
            ids.push(id);
            s.insert_stored(id, run("t1", t));
        }

        // Pages de 2 : la boucle s'arrête sur une page VIDE, jamais sur « moins que la limite ».
        let mut vus: Vec<Uuid> = Vec::new();
        let mut cur: Curseur = None;
        loop {
            let page = s.inventorier(cur, 2).await.unwrap();
            if page.is_empty() {
                break;
            }
            cur = page.last().map(super::LigneExport::cle);
            vus.extend(page.iter().map(|l| l.id));
        }
        assert_eq!(vus.len(), 7, "l'inventaire doit rendre TOUS les runs");
        let mut tri = vus.clone();
        tri.sort_unstable();
        tri.dedup();
        assert_eq!(tri.len(), 7, "aucun run ne doit être rendu deux fois");
    }

    /// Le point de sûreté : l'inventaire est un geste d'exploitant, il voit **tous** les tenants.
    /// Ce test le FIXE, pour qu'un futur ajout de filtre par tenant soit un choix conscient et non
    /// une régression silencieuse d'un côté ou de l'autre.
    #[tokio::test]
    async fn linventaire_traverse_les_tenants_et_cest_assume() {
        let s = InMemoryRunStore::new();
        s.insert_stored(Uuid::new_v4(), run("alpha", 1));
        s.insert_stored(Uuid::new_v4(), run("beta", 2));
        let page = s.inventorier(None, 10).await.unwrap();
        let mut tenants: Vec<&str> = page.iter().map(|l| l.tenant.as_str()).collect();
        tenants.sort_unstable();
        assert_eq!(tenants, vec!["alpha", "beta"]);
    }

    /// Contre-épreuve du curseur : sans elle, un `inventorier` qui ignorerait `apres` passerait le
    /// premier test (la boucle finirait quand même) en rendant indéfiniment la même page.
    #[tokio::test]
    async fn contre_epreuve_le_curseur_avance_vraiment() {
        let s = InMemoryRunStore::new();
        for t in 0..3u64 {
            s.insert_stored(Uuid::new_v4(), run("t1", t));
        }
        let p1 = s.inventorier(None, 2).await.unwrap();
        assert_eq!(p1.len(), 2);
        let p2 = s.inventorier(p1.last().map(super::LigneExport::cle), 2).await.unwrap();
        assert_eq!(p2.len(), 1, "la seconde page doit contenir le RESTE, pas le début");
        assert!(!p2.iter().any(|l| p1.iter().any(|a| a.id == l.id)), "pages disjointes");
    }

    /// L'aller-retour d'enveloppe. Un champ oublié dans `depuis`/`vers_run` se perdrait en silence
    /// (serde le remplirait par défaut) : c'est le défaut que ce test attrape.
    #[test]
    fn lenveloppe_conserve_ce_quelle_transporte() {
        let mut r = run("t1", 42);
        r.root = Some("abc".into());
        r.n17_root = Some("def".into());
        r.n17_leaves = vec!["f1".into()];
        r.topology = "mode-c".into();
        r.request = serde_json::json!({"k": 3});

        let id = Uuid::new_v4();
        let ligne = LigneExport::depuis(id, &r);
        let texte = serde_json::to_string(&ligne).unwrap();
        let relu: LigneExport = serde_json::from_str(&texte).unwrap();
        let r2 = relu.vers_run();

        assert_eq!(relu.id, id);
        assert_eq!(r2.tenant, r.tenant);
        assert_eq!(r2.root, r.root);
        assert_eq!(r2.n17_root, r.n17_root);
        assert_eq!(r2.n17_leaves, r.n17_leaves);
        assert_eq!(r2.topology, r.topology);
        assert_eq!(r2.request, r.request);
        assert_eq!(r2.output, r.output);
        assert_eq!(r2.created_at_unix(), 42);
    }
}

// ═════════════════════════════════════════════════════════════════════════════════════════════
//  Les deux gestes d'exploitant
// ═════════════════════════════════════════════════════════════════════════════════════════════
//
//  Ils vivent ICI, dans la bibliothèque, et pas dans le binaire : le binaire ne peut pas nommer
//  `StoredRun` (module `store` privé), et c'est très bien ainsi — l'alternative aurait été de
//  rendre public un type qui porte le tenant, à un `Json(...)` d'une réponse HTTP.
//
//  **Postgres uniquement, et ce n'est pas une limitation.** Le store en mémoire meurt avec le
//  processus : il n'a rien à sauvegarder, et prétendre le contraire donnerait une sauvegarde qui
//  se croit faite.

/// Ce qu'un import a réellement fait — jamais un simple « ok ».
#[cfg(feature = "postgres")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bilan {
    /// Lignes lues dans l'entrée.
    pub lues: u64,
    /// Lignes écrites — les autres existaient déjà (`ON CONFLICT DO NOTHING`, write-once).
    pub ecrites: u64,
}

/// **Exporter** tout le store en JSONL, une ligne par run, ordre `(created_at, id)`.
///
/// Rend le nombre de lignes écrites. La boucle s'arrête sur une page **vide** — jamais sur « moins
/// que la limite », qui tronquerait une sauvegarde sur un store qui rend des pages courtes.
///
/// # Errors
/// Une défaillance du store, ou une écriture impossible sur la sortie.
#[cfg(feature = "postgres")]
pub async fn exporter_vers<W: std::io::Write>(
    store: &crate::PgRunStore,
    sortie: &mut W,
) -> Result<u64, GatewayError> {
    const PAGE: u32 = 500;
    let mut curseur: Curseur = None;
    let mut n = 0u64;
    loop {
        let page = store.inventorier(curseur, PAGE).await?;
        if page.is_empty() {
            break;
        }
        curseur = page.last().map(LigneExport::cle);
        for l in &page {
            let ligne = serde_json::to_string(l)
                .map_err(|e| GatewayError::Internal(format!("run {} non sérialisable : {e}", l.id)))?;
            writeln!(sortie, "{ligne}")
                .map_err(|e| GatewayError::Internal(format!("écriture impossible : {e}")))?;
            n += 1;
        }
    }
    Ok(n)
}

/// **Importer** un JSONL produit par [`exporter_vers`].
///
/// Le store est *write-once* (`ON CONFLICT DO NOTHING`) : réimporter est donc **idempotent** et ne
/// peut pas écraser un run vivant. C'est ce qui rend une restauration sûre à rejouer, y compris sur
/// une base qui n'était pas vide.
///
/// Une ligne illisible **arrête** l'import et le dit, avec son numéro. Sauter en silence les lignes
/// qu'on ne comprend pas produirait une restauration partielle qui se croit complète — le défaut
/// exact que l'en-tête de ce module décrit.
///
/// # Errors
/// Une ligne illisible, ou une défaillance du store.
#[cfg(feature = "postgres")]
pub async fn importer_depuis<R: std::io::BufRead>(
    store: &crate::PgRunStore,
    entree: R,
) -> Result<Bilan, GatewayError> {
    use crate::store::RunStore;
    let mut bilan = Bilan { lues: 0, ecrites: 0 };
    for (i, ligne) in entree.lines().enumerate() {
        let ligne = ligne.map_err(|e| GatewayError::Internal(format!("ligne {} illisible : {e}", i + 1)))?;
        if ligne.trim().is_empty() {
            continue;
        }
        let l: LigneExport = serde_json::from_str(&ligne)
            .map_err(|e| GatewayError::Internal(format!("ligne {} n'est pas un run : {e}", i + 1)))?;
        bilan.lues += 1;
        let avant = store.get(l.id, &l.tenant).await?.is_some();
        store.put_stored(l.id, l.vers_run()).await?;
        if !avant {
            bilan.ecrites += 1;
        }
    }
    Ok(bilan)
}
