//! `AlignmentRegistry` — trait et implémentation `InMemoryRegistry`.
//!
//! Le registre est le **point central** de M9 : stockage versionné,
//! thread-safe, policy-enforced, avec breach log et événements.
//!
//! # Garanties concurrence
//!
//! - `tokio::sync::RwLock` pour le catalogue principal : N lecteurs
//!   simultanés, écriture exclusive.
//! - `tokio::sync::Mutex` pour le breach log : les opérations sont très
//!   brèves (push amortized O(1)).
//! - **Jamais** de lock croisé. Un `register`/`evolve` acquiert le
//!   write lock, fait son boulot, le relâche avant d'émettre l'event
//!   sur le channel (qui ne nécessite aucun lock).
//!
//! # Invariant de version
//!
//! Pour tout `(tenant, task)`, les versions sont **strictement monotones
//! croissantes**. Le registre rejette tout `register` avec une version
//! `v ≤ max(versions_existantes)`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use tokio::sync::{mpsc, Mutex, RwLock};

use crate::alignment::breach::{BreachFilter, BreachLog, BreachReport};
use crate::alignment::canonical::{CanonicalCone, ConeHandle, ConeMetadata};
use crate::alignment::compose::union_cones;
use crate::alignment::events::RegistryEvent;
use crate::alignment::evolution::ConeEvolutionOp;
use crate::alignment::key::ConeKey;
use crate::alignment::policy::{PolicyEngine, RegistryConfig};
use crate::errors::{EfeError, Result};

// =====================================================================
// Traits
// =====================================================================

/// Sous-trait minimal pour les consommateurs de breach reporting (M8).
///
/// Permet à M8 de dépendre d'une surface réduite sans importer le trait
/// complet. Toute implémentation d'[`AlignmentRegistry`] fournit
/// automatiquement [`BreachReporter`] via l'impl blanket ci-dessous.
///
/// La méthode est nommée `submit_breach` (et non `report_breach`) pour
/// éviter l'ambiguïté de résolution de méthode quand à la fois
/// `AlignmentRegistry` et `BreachReporter` sont en scope sur le même
/// type concret.
#[async_trait]
pub trait BreachReporter: Send + Sync + std::fmt::Debug {
    /// Enregistre un rapport de breach.
    ///
    /// # Errors
    ///
    /// Dépend du backend ; `InMemoryRegistry` ne fail jamais.
    async fn submit_breach(&self, report: BreachReport) -> Result<()>;
}

/// Contrat complet du registre.
#[async_trait]
pub trait AlignmentRegistry: Send + Sync + std::fmt::Debug {
    /// Enregistre un nouveau cône à la clé donnée.
    ///
    /// # Contrat
    ///
    /// - `key.version` doit être strictement supérieur à la version
    ///   maximale déjà enregistrée pour `(tenant, task)`.
    /// - Policy vérifiée avant insertion.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] si policy violée, version
    ///   non-monotone, ou quota `max_versions_per_task` dépassé.
    async fn register(&self, key: ConeKey, cone: CanonicalCone) -> Result<ConeHandle>;

    /// Lookup par clé exacte.
    ///
    /// # Errors
    ///
    /// [`EfeError::InvalidConfig`] si la clé n'existe pas.
    async fn get(&self, key: &ConeKey) -> Result<ConeHandle>;

    /// Liste toutes les versions pour `(tenant, task)`, triées.
    async fn list_versions(&self, tenant: &str, task: &str) -> Result<Vec<ConeKey>>;

    /// Applique une opération d'évolution et crée une nouvelle version.
    ///
    /// La nouvelle version est `max_existing_version + 1`.
    ///
    /// # Errors
    ///
    /// [`EfeError::InvalidConfig`] si `from` inexistant, op invalide,
    /// policy violée, quota dépassé.
    async fn evolve(
        &self,
        from: ConeKey,
        op: ConeEvolutionOp,
        metadata_override: Option<ConeMetadata>,
    ) -> Result<ConeHandle>;

    /// Supprime une version. Refuse la suppression de la dernière
    /// version sauf si `force == true`.
    ///
    /// # Errors
    ///
    /// [`EfeError::InvalidConfig`] si clé inconnue ou dernière version
    /// sans `force`.
    async fn delete(&self, key: &ConeKey, force: bool) -> Result<()>;

    /// Retourne jusqu'à `n` breaches récents, filtrés.
    async fn breach_history(
        &self,
        n: usize,
        filter: Option<BreachFilter>,
    ) -> Result<Vec<BreachReport>>;

    /// Enregistre un breach observé. Ne bloque pas sur l'existence du cône.
    async fn report_breach(&self, report: BreachReport) -> Result<()>;

    /// Nom du backend pour télémétrie.
    fn backend_name(&self) -> &'static str;
}

// Blanket BreachReporter pour tout AlignmentRegistry.
#[async_trait]
impl<T: AlignmentRegistry + ?Sized> BreachReporter for T {
    async fn submit_breach(&self, report: BreachReport) -> Result<()> {
        <Self as AlignmentRegistry>::report_breach(self, report).await
    }
}

// =====================================================================
// InMemoryRegistry
// =====================================================================

/// Implémentation MVP en mémoire.
#[derive(Debug)]
pub struct InMemoryRegistry {
    /// Catalogue principal : `ConeKey` → `Arc<CanonicalCone>`.
    inner: RwLock<Inner>,
    /// Ring buffer des breaches.
    breach_log: Mutex<BreachLog>,
    /// Émetteur d'événements (optionnel).
    events_tx: Mutex<Option<mpsc::UnboundedSender<RegistryEvent>>>,
    /// Receveur stocké jusqu'au premier `take_events_receiver`.
    events_rx_slot: Mutex<Option<mpsc::UnboundedReceiver<RegistryEvent>>>,
    /// Moteur de policy.
    policy: PolicyEngine,
    /// Config complète (utile pour accesseurs).
    cfg: RegistryConfig,
}

#[derive(Debug, Default)]
struct Inner {
    by_key: HashMap<ConeKey, Arc<CanonicalCone>>,
    // Clé : "tenant/task". Valeur : versions triées croissant.
    versions_by_task: HashMap<String, Vec<u32>>,
}

impl InMemoryRegistry {
    /// Construit un registre depuis une config.
    ///
    /// # Errors
    ///
    /// [`EfeError::InvalidConfig`] si la config est invalide.
    pub fn new(cfg: RegistryConfig) -> Result<Self> {
        cfg.validate()?;
        let (tx, rx) = mpsc::unbounded_channel();
        Ok(Self {
            inner: RwLock::new(Inner::default()),
            breach_log: Mutex::new(BreachLog::new(cfg.breach_log_capacity)),
            events_tx: Mutex::new(Some(tx)),
            events_rx_slot: Mutex::new(Some(rx)),
            policy: PolicyEngine::new(cfg.policy.clone()),
            cfg,
        })
    }

    /// Construit un registre avec la config par défaut.
    ///
    /// # Errors
    ///
    /// N'échoue pas en pratique (la config par défaut est valide) ; la
    /// signature reste `Result` pour permettre d'éventuels renforcements
    /// de meta-invariants sans breaking change.
    pub fn with_default_config() -> Result<Self> {
        Self::new(RegistryConfig::default())
    }

    /// Retourne la config du registre.
    #[must_use]
    pub fn config(&self) -> &RegistryConfig {
        &self.cfg
    }

    /// Prend le receveur d'événements. Retourne `None` au 2ᵉ appel.
    ///
    /// Pattern : un seul consommateur par registre. M11 appelle cette
    /// méthode une fois au démarrage ; si aucun n'est attaché, les
    /// événements sont envoyés dans le channel en mémoire (capacité
    /// unbounded → pas de blocage) et consommés lors du drop.
    pub async fn take_events_receiver(&self) -> Option<mpsc::UnboundedReceiver<RegistryEvent>> {
        self.events_rx_slot.lock().await.take()
    }

    /// Émet un événement si un émetteur est attaché. Best-effort :
    /// ignore les erreurs (pas de consommateur, channel fermé).
    async fn emit(&self, event: RegistryEvent) {
        if let Some(tx) = self.events_tx.lock().await.as_ref() {
            // Envoi best-effort : on ignore l'erreur si le receveur a drop.
            let _ = tx.send(event);
        }
    }

    // Helpers internes.

    fn max_version_for(inner: &Inner, task_id: &str) -> Option<u32> {
        inner
            .versions_by_task
            .get(task_id)
            .and_then(|v| v.last().copied())
    }

    fn count_for(inner: &Inner, task_id: &str) -> usize {
        inner.versions_by_task.get(task_id).map_or(0, Vec::len)
    }

    /// Insère un cône déjà policy-checké dans l'état interne sous write
    /// lock. Retourne le handle ou une erreur de contrat
    /// (version non-monotone / quota dépassé). Ne déclenche **aucun**
    /// event ; c'est au caller (register / evolve) d'émettre après
    /// release du lock.
    fn insert_locked(
        inner: &mut Inner,
        key: ConeKey,
        cone: Arc<CanonicalCone>,
        max_versions: u32,
    ) -> Result<ConeHandle> {
        let task_id = key.task_id();
        if let Some(mv) = Self::max_version_for(inner, &task_id) {
            if key.version <= mv {
                return Err(EfeError::InvalidConfig(format!(
                    "register: version {} not > current max {} for {}",
                    key.version, mv, task_id
                )));
            }
        }
        if Self::count_for(inner, &task_id) >= max_versions as usize {
            return Err(EfeError::InvalidConfig(format!(
                "register: task {task_id} reached max_versions_per_task={max_versions}"
            )));
        }
        inner.by_key.insert(key.clone(), cone.clone());
        let vs = inner.versions_by_task.entry(task_id).or_default();
        vs.push(key.version);
        vs.sort_unstable();
        Ok(ConeHandle::new(key, cone))
    }
}

fn now_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[async_trait]
impl AlignmentRegistry for InMemoryRegistry {
    async fn register(&self, key: ConeKey, cone: CanonicalCone) -> Result<ConeHandle> {
        // 1. Policy (hors lock — pur).
        if let Err(e) = self.policy.check(&cone.cone) {
            self.emit(RegistryEvent::PolicyRejected {
                attempted_key: Some(key.clone()),
                reason: e.to_string(),
            })
            .await;
            return Err(e);
        }

        // 2. Write lock + atomic insert via helper.
        let arc = Arc::new(cone);
        let outcome = {
            let mut inner = self.inner.write().await;
            Self::insert_locked(&mut inner, key.clone(), arc, self.cfg.max_versions_per_task)
        }; // write lock released here.

        match outcome {
            Ok(handle) => {
                self.emit(RegistryEvent::Registered {
                    key: handle.key.clone(),
                    at_nanos: now_nanos(),
                })
                .await;
                Ok(handle)
            }
            Err(e) => {
                self.emit(RegistryEvent::PolicyRejected {
                    attempted_key: Some(key),
                    reason: e.to_string(),
                })
                .await;
                Err(e)
            }
        }
    }

    async fn get(&self, key: &ConeKey) -> Result<ConeHandle> {
        let inner = self.inner.read().await;
        inner
            .by_key
            .get(key)
            .cloned()
            .map(|arc| ConeHandle::new(key.clone(), arc))
            .ok_or_else(|| EfeError::InvalidConfig(format!("registry: unknown cone key {key}")))
    }

    async fn list_versions(&self, tenant: &str, task: &str) -> Result<Vec<ConeKey>> {
        let inner = self.inner.read().await;
        let task_id = format!("{tenant}/{task}");
        let versions = inner.versions_by_task.get(&task_id);
        match versions {
            Some(vs) => {
                let mut keys = Vec::with_capacity(vs.len());
                for &v in vs {
                    keys.push(ConeKey::new(tenant, task, v)?);
                }
                Ok(keys)
            }
            None => Ok(Vec::new()),
        }
    }

    async fn evolve(
        &self,
        from: ConeKey,
        op: ConeEvolutionOp,
        metadata_override: Option<ConeMetadata>,
    ) -> Result<ConeHandle> {
        // 1. Lookup parent (read lock, libéré immédiatement).
        let parent_handle = self.get(&from).await?;
        let parent_cone_snapshot = parent_handle.cone.cone.clone();

        // 2. Lookup UnionWith.other AVANT la section critique.
        let other_cone_snapshot: Option<crate::types::AlignmentCone> = match &op {
            ConeEvolutionOp::UnionWith { other } => Some(self.get(other).await?.cone.cone.clone()),
            _ => None,
        };

        // 3. Apply l'op (pur).
        let new_cone = match (&op, other_cone_snapshot.as_ref()) {
            (ConeEvolutionOp::UnionWith { .. }, Some(other)) => {
                union_cones(&parent_cone_snapshot, other)?
            }
            (local_op, _) => local_op.apply(&parent_cone_snapshot)?,
        };

        // 4. Construit la metadata en **préservant les labels du
        //    parent** dans le chemin par défaut ; l'override remplace
        //    tout sauf le lien parent/op qui est forcé.
        let metadata = if let Some(md) = metadata_override {
            md.with_parent(from.clone(), op.clone())
        } else {
            let mut base = ConeMetadata::new(
                parent_handle.cone.metadata.created_by.clone(),
                format!("evolved from {from} via {}", op.kind()),
            )?;
            base.labels = parent_handle.cone.metadata.labels.clone();
            base.with_parent(from.clone(), op.clone())
        };
        let canonical = CanonicalCone::with_metadata(new_cone, metadata);

        // 5. Policy check (hors lock — fail fast).
        if let Err(e) = self.policy.check(&canonical.cone) {
            self.emit(RegistryEvent::PolicyRejected {
                attempted_key: None,
                reason: e.to_string(),
            })
            .await;
            return Err(e);
        }

        // 6. Section critique atomique : write lock → check parent
        //    encore présent → compute next_version → insert.
        //    Protège contre : (a) TOCTOU sur next_version quand
        //    plusieurs evolves concurrents visent le même task ; (b)
        //    orphan evolution si le parent est delete entre 1 et 6.
        let arc = Arc::new(canonical);
        let outcome: Result<(ConeHandle, ConeKey)> = {
            let mut inner = self.inner.write().await;
            if inner.by_key.contains_key(&from) {
                let task_id = from.task_id();
                let next_version =
                    Self::max_version_for(&inner, &task_id).map_or(from.version + 1, |mv| mv + 1);
                match ConeKey::new(&from.tenant, &from.task, next_version) {
                    Ok(new_key) => Self::insert_locked(
                        &mut inner,
                        new_key.clone(),
                        arc,
                        self.cfg.max_versions_per_task,
                    )
                    .map(|h| (h, new_key)),
                    Err(e) => Err(e),
                }
            } else {
                Err(EfeError::InvalidConfig(format!(
                    "evolve: parent {from} was deleted during evolution"
                )))
            }
        }; // write lock released.

        match outcome {
            Ok((handle, new_key)) => {
                self.emit(RegistryEvent::Registered {
                    key: new_key.clone(),
                    at_nanos: now_nanos(),
                })
                .await;
                self.emit(RegistryEvent::Evolved {
                    from,
                    to: new_key,
                    op_kind: op.kind().to_string(),
                })
                .await;
                Ok(handle)
            }
            Err(e) => {
                self.emit(RegistryEvent::PolicyRejected {
                    attempted_key: None,
                    reason: e.to_string(),
                })
                .await;
                Err(e)
            }
        }
    }

    async fn delete(&self, key: &ConeKey, force: bool) -> Result<()> {
        let removed = {
            let mut inner = self.inner.write().await;
            let task_id = key.task_id();
            let vs = inner.versions_by_task.get(&task_id).cloned();
            let Some(versions) = vs else {
                return Err(EfeError::InvalidConfig(format!(
                    "delete: unknown task {task_id}"
                )));
            };
            if versions.len() == 1 && !force {
                return Err(EfeError::InvalidConfig(format!(
                    "delete: refusing to remove last version {key} without force=true"
                )));
            }
            let removed = inner.by_key.remove(key);
            if removed.is_none() {
                return Err(EfeError::InvalidConfig(format!(
                    "delete: unknown cone key {key}"
                )));
            }
            let vs_mut = inner
                .versions_by_task
                .get_mut(&task_id)
                .ok_or(EfeError::Internal(
                    "delete: versions_by_task entry disappeared",
                ))?;
            vs_mut.retain(|&v| v != key.version);
            if vs_mut.is_empty() {
                inner.versions_by_task.remove(&task_id);
            }
            removed
        };
        if removed.is_some() {
            self.emit(RegistryEvent::Deleted { key: key.clone() }).await;
        }
        Ok(())
    }

    async fn breach_history(
        &self,
        n: usize,
        filter: Option<BreachFilter>,
    ) -> Result<Vec<BreachReport>> {
        let log = self.breach_log.lock().await;
        Ok(log.recent(n, filter.as_ref()))
    }

    async fn report_breach(&self, report: BreachReport) -> Result<()> {
        {
            let mut log = self.breach_log.lock().await;
            log.push(report.clone());
        }
        self.emit(RegistryEvent::BreachReported { report }).await;
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "in_memory"
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::alignment::breach::BreachContext;
    use crate::types::{AlignmentCone, SwarmId};

    fn make_canonical(k: usize, d: usize) -> CanonicalCone {
        let anchors: Vec<Vec<f32>> = (0..k)
            .map(|i| {
                let mut v = vec![0.0; d];
                v[i % d] = 1.0;
                v
            })
            .collect();
        let cone = AlignmentCone::new(anchors, 0.5).unwrap();
        CanonicalCone::new(cone, "test", "fixture").unwrap()
    }

    fn registry() -> InMemoryRegistry {
        InMemoryRegistry::with_default_config().unwrap()
    }

    // --- register / get ---

    #[tokio::test]
    async fn register_then_get_roundtrip() {
        let reg = registry();
        let key = ConeKey::new("acme", "task", 1).unwrap();
        let canon = make_canonical(2, 4);
        let h = reg.register(key.clone(), canon).await.unwrap();
        assert_eq!(h.key, key);
        let got = reg.get(&key).await.unwrap();
        assert_eq!(got.key, key);
        assert_eq!(got.cone.num_anchors(), 2);
    }

    #[tokio::test]
    async fn get_unknown_errors() {
        let reg = registry();
        let key = ConeKey::new("acme", "task", 1).unwrap();
        assert!(reg.get(&key).await.is_err());
    }

    #[tokio::test]
    async fn register_policy_rejection_propagates() {
        let reg = registry();
        // Cône avec anchor NaN → I7 fail.
        let cone = AlignmentCone::new(vec![vec![f32::NAN, 0.0], vec![0.0, 1.0]], 0.5).unwrap();
        let canonical = CanonicalCone::new(cone, "t", "d").unwrap();
        let key = ConeKey::new("acme", "task", 1).unwrap();
        assert!(reg.register(key, canonical).await.is_err());
    }

    #[tokio::test]
    async fn register_non_monotone_version_rejected() {
        let reg = registry();
        let k1 = ConeKey::new("acme", "task", 1).unwrap();
        let k2 = ConeKey::new("acme", "task", 2).unwrap();
        reg.register(k1.clone(), make_canonical(2, 4))
            .await
            .unwrap();
        reg.register(k2.clone(), make_canonical(2, 4))
            .await
            .unwrap();
        // Re-register v1 after v2 exists : must fail.
        let err = reg.register(k1, make_canonical(2, 4)).await.unwrap_err();
        assert!(err.to_string().contains("not >"));
    }

    #[tokio::test]
    async fn register_hits_max_versions() {
        let mut cfg = RegistryConfig::default();
        cfg.max_versions_per_task = 2;
        let reg = InMemoryRegistry::new(cfg).unwrap();
        for i in 1..=2 {
            let k = ConeKey::new("acme", "t", i).unwrap();
            reg.register(k, make_canonical(2, 4)).await.unwrap();
        }
        let k3 = ConeKey::new("acme", "t", 3).unwrap();
        let err = reg.register(k3, make_canonical(2, 4)).await.unwrap_err();
        assert!(err.to_string().contains("max_versions_per_task"));
    }

    #[tokio::test]
    async fn list_versions_empty_for_unknown() {
        let reg = registry();
        let vs = reg.list_versions("no", "such").await.unwrap();
        assert!(vs.is_empty());
    }

    #[tokio::test]
    async fn list_versions_sorted() {
        // Versions must be registered monotonically (the registry
        // rejects non-monotone writes) ; we register 1..=3 in order and
        // verify list_versions returns them sorted ascending.
        let reg = registry();
        for i in 1..=3u32 {
            let k = ConeKey::new("t", "k", i).unwrap();
            reg.register(k, make_canonical(2, 4)).await.unwrap();
        }
        let vs = reg.list_versions("t", "k").await.unwrap();
        let versions: Vec<u32> = vs.iter().map(|k| k.version).collect();
        assert_eq!(versions, vec![1, 2, 3]);
    }

    // --- evolve ---

    #[tokio::test]
    async fn evolve_creates_next_version() {
        let reg = registry();
        let k1 = ConeKey::new("acme", "t", 1).unwrap();
        reg.register(k1.clone(), make_canonical(2, 4))
            .await
            .unwrap();
        let h = reg
            .evolve(
                k1.clone(),
                ConeEvolutionOp::AppendAnchor {
                    anchor: vec![0.0, 0.0, 1.0, 0.0],
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(h.key.version, 2);
        assert_eq!(h.cone.num_anchors(), 3);
        assert_eq!(h.cone.metadata.parent, Some(k1));
    }

    #[tokio::test]
    async fn evolve_unknown_parent_errors() {
        let reg = registry();
        let k = ConeKey::new("acme", "t", 1).unwrap();
        let outcome = reg
            .evolve(k, ConeEvolutionOp::UpdateRadius { new_radius: 1.0 }, None)
            .await;
        assert!(outcome.is_err());
    }

    #[tokio::test]
    async fn evolve_update_radius() {
        let reg = registry();
        let k = ConeKey::new("acme", "t", 1).unwrap();
        reg.register(k.clone(), make_canonical(2, 4)).await.unwrap();
        let h = reg
            .evolve(k, ConeEvolutionOp::UpdateRadius { new_radius: 1.5 }, None)
            .await
            .unwrap();
        assert!((h.cone.radius() - 1.5).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn evolve_union_with_other() {
        // Construct two cones with *distinct* anchors so the union
        // passes the cosine-duplication invariant (I5).
        let cone_a = AlignmentCone::new(
            vec![vec![1.0, 0.0, 0.0, 0.0], vec![0.0, 1.0, 0.0, 0.0]],
            0.5,
        )
        .unwrap();
        let cone_b = AlignmentCone::new(
            vec![vec![0.0, 0.0, 1.0, 0.0], vec![0.0, 0.0, 0.0, 1.0]],
            0.5,
        )
        .unwrap();
        let canon_a = CanonicalCone::new(cone_a, "t", "a").unwrap();
        let canon_b = CanonicalCone::new(cone_b, "t", "b").unwrap();

        let reg = registry();
        let k1 = ConeKey::new("t", "a", 1).unwrap();
        let k2 = ConeKey::new("t", "b", 1).unwrap();
        reg.register(k1.clone(), canon_a).await.unwrap();
        reg.register(k2.clone(), canon_b).await.unwrap();

        let h = reg
            .evolve(k1.clone(), ConeEvolutionOp::UnionWith { other: k2 }, None)
            .await
            .unwrap();
        assert_eq!(h.cone.num_anchors(), 4);
        assert_eq!(h.key.tenant, "t");
        assert_eq!(h.key.task, "a");
        assert_eq!(h.key.version, 2);
    }

    #[tokio::test]
    async fn evolve_union_missing_other_errors() {
        let reg = registry();
        let k1 = ConeKey::new("t", "a", 1).unwrap();
        let k2 = ConeKey::new("t", "b", 1).unwrap();
        reg.register(k1.clone(), make_canonical(2, 4))
            .await
            .unwrap();
        let outcome = reg
            .evolve(k1, ConeEvolutionOp::UnionWith { other: k2 }, None)
            .await;
        assert!(outcome.is_err());
    }

    // --- delete ---

    #[tokio::test]
    async fn delete_last_without_force_rejected() {
        let reg = registry();
        let k = ConeKey::new("t", "a", 1).unwrap();
        reg.register(k.clone(), make_canonical(2, 4)).await.unwrap();
        assert!(reg.delete(&k, false).await.is_err());
    }

    #[tokio::test]
    async fn delete_last_with_force_removes_task() {
        let reg = registry();
        let k = ConeKey::new("t", "a", 1).unwrap();
        reg.register(k.clone(), make_canonical(2, 4)).await.unwrap();
        reg.delete(&k, true).await.unwrap();
        assert!(reg.list_versions("t", "a").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_unknown_errors() {
        let reg = registry();
        let k = ConeKey::new("t", "a", 1).unwrap();
        assert!(reg.delete(&k, true).await.is_err());
    }

    #[tokio::test]
    async fn delete_intermediate_version_keeps_others() {
        let reg = registry();
        for i in 1..=3 {
            let k = ConeKey::new("t", "a", i).unwrap();
            reg.register(k, make_canonical(2, 4)).await.unwrap();
        }
        let k2 = ConeKey::new("t", "a", 2).unwrap();
        reg.delete(&k2, false).await.unwrap();
        let vs = reg.list_versions("t", "a").await.unwrap();
        let versions: Vec<u32> = vs.iter().map(|k| k.version).collect();
        assert_eq!(versions, vec![1, 3]);
    }

    // --- breach log ---

    #[tokio::test]
    async fn report_and_list_breaches() {
        let reg = registry();
        let r = BreachReport::new(
            ConeKey::new("t", "a", 1).unwrap(),
            SwarmId::new(),
            0.9,
            0.5,
            100,
            BreachContext::Dissolution,
        );
        reg.report_breach(r.clone()).await.unwrap();
        let history = reg.breach_history(10, None).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].cone_key, r.cone_key);
    }

    #[tokio::test]
    async fn breach_history_filter_by_tenant() {
        let reg = registry();
        for tenant in ["a", "b", "a"] {
            let r = BreachReport::new(
                ConeKey::new(tenant, "t", 1).unwrap(),
                SwarmId::new(),
                0.9,
                0.5,
                1,
                BreachContext::Dissolution,
            );
            reg.report_breach(r).await.unwrap();
        }
        let filter = BreachFilter::none().with_tenant("a");
        let hist = reg.breach_history(10, Some(filter)).await.unwrap();
        assert_eq!(hist.len(), 2);
    }

    // --- backend_name ---

    #[tokio::test]
    async fn backend_name_is_in_memory() {
        let reg = registry();
        assert_eq!(reg.backend_name(), "in_memory");
    }

    // --- events channel ---

    #[tokio::test]
    async fn events_channel_delivers_registered() {
        let reg = registry();
        let mut rx = reg.take_events_receiver().await.unwrap();
        let k = ConeKey::new("t", "a", 1).unwrap();
        reg.register(k.clone(), make_canonical(2, 4)).await.unwrap();
        let evt = rx.recv().await.unwrap();
        match evt {
            RegistryEvent::Registered { key, .. } => assert_eq!(key, k),
            other => panic!("expected Registered, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn events_channel_delivers_evolved_and_registered() {
        let reg = registry();
        let mut rx = reg.take_events_receiver().await.unwrap();
        let k = ConeKey::new("t", "a", 1).unwrap();
        reg.register(k.clone(), make_canonical(2, 4)).await.unwrap();
        reg.evolve(k, ConeEvolutionOp::UpdateRadius { new_radius: 1.5 }, None)
            .await
            .unwrap();
        // Expect in order: Registered(v1), Registered(v2), Evolved.
        let e1 = rx.recv().await.unwrap();
        let e2 = rx.recv().await.unwrap();
        let e3 = rx.recv().await.unwrap();
        assert!(matches!(e1, RegistryEvent::Registered { .. }));
        assert!(matches!(e2, RegistryEvent::Registered { .. }));
        assert!(matches!(e3, RegistryEvent::Evolved { .. }));
    }

    #[tokio::test]
    async fn take_events_receiver_twice_returns_none_second_time() {
        let reg = registry();
        assert!(reg.take_events_receiver().await.is_some());
        assert!(reg.take_events_receiver().await.is_none());
    }

    #[tokio::test]
    async fn breach_reporter_trait_works_via_registry() {
        // InMemoryRegistry : Arc<concrete type> se coerce naturellement en
        // Arc<dyn BreachReporter>.
        let reg: Arc<InMemoryRegistry> = Arc::new(registry());
        let reporter: Arc<dyn BreachReporter> = reg.clone();
        let r = BreachReport::new(
            ConeKey::new("t", "a", 1).unwrap(),
            SwarmId::new(),
            0.9,
            0.5,
            1,
            BreachContext::Dissolution,
        );
        // Nommé `submit_breach` sur le sous-trait pour éviter
        // l'ambiguïté avec `AlignmentRegistry::report_breach`.
        reporter.submit_breach(r).await.unwrap();
        let hist = reg.breach_history(10, None).await.unwrap();
        assert_eq!(hist.len(), 1);
    }

    // --- concurrency ---

    // Test du scenario TOCTOU post-fix : plusieurs evolves concurrents
    // sur le même (tenant, task). Avec l'atomicité du write lock dans
    // evolve, tous réussissent en obtenant chacun une version distincte.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_evolve_on_same_task_all_succeed() {
        let reg = Arc::new(registry());
        let k1 = ConeKey::new("t", "shared", 1).unwrap();
        reg.register(k1.clone(), make_canonical(2, 4))
            .await
            .unwrap();

        let mut handles = Vec::new();
        for _ in 0..4 {
            let reg = reg.clone();
            let k = k1.clone();
            handles.push(tokio::spawn(async move {
                reg.evolve(k, ConeEvolutionOp::UpdateRadius { new_radius: 0.7 }, None)
                    .await
            }));
        }
        let mut successes = 0;
        for h in handles {
            if h.await.unwrap().is_ok() {
                successes += 1;
            }
        }
        assert_eq!(
            successes, 4,
            "atomic evolve should let all 4 concurrent evolves succeed"
        );
        let vs = reg.list_versions("t", "shared").await.unwrap();
        assert_eq!(vs.len(), 5, "original v1 + 4 evolutions");
    }

    // Contention explicite sur register de la **même** clé. Un seul doit
    // réussir ; les autres doivent échouer proprement (non-monotone).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_register_same_task_one_wins() {
        let reg = Arc::new(registry());
        let mut handles = Vec::new();
        for _ in 0..4 {
            let reg = reg.clone();
            handles.push(tokio::spawn(async move {
                let k = ConeKey::new("t", "collide", 1).unwrap();
                reg.register(k, make_canonical(2, 4)).await
            }));
        }
        let mut successes = 0;
        for h in handles {
            if h.await.unwrap().is_ok() {
                successes += 1;
            }
        }
        assert_eq!(
            successes, 1,
            "exactly one concurrent register of the same key must win"
        );
    }

    // report_breach accepte un rapport même si le cône a été delete
    // depuis. C'est voulu : le breach log est un journal historique.
    #[tokio::test]
    async fn report_breach_survives_deleted_cone() {
        let reg = registry();
        let k = ConeKey::new("t", "a", 1).unwrap();
        reg.register(k.clone(), make_canonical(2, 4)).await.unwrap();
        reg.delete(&k, true).await.unwrap();

        let r = BreachReport::new(
            k.clone(),
            SwarmId::new(),
            0.9,
            0.5,
            100,
            BreachContext::Dissolution,
        );
        reg.report_breach(r).await.unwrap();
        let hist = reg.breach_history(10, None).await.unwrap();
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].cone_key, k);
    }

    // Evolve refuse proprement si le parent a été delete entre la lookup
    // et la section critique.
    #[tokio::test]
    async fn evolve_detects_parent_deleted_mid_flight() {
        let reg = Arc::new(registry());
        let k = ConeKey::new("t", "a", 1).unwrap();
        reg.register(k.clone(), make_canonical(2, 4)).await.unwrap();
        // Simule un delete *avant* le evolve (l'ordre atomique dans le
        // write lock capture ce cas).
        reg.delete(&k, true).await.unwrap();
        let err = reg
            .evolve(k, ConeEvolutionOp::UpdateRadius { new_radius: 0.8 }, None)
            .await
            .unwrap_err();
        // Message doit mentionner parent absent.
        let msg = err.to_string();
        assert!(
            msg.contains("unknown") || msg.contains("deleted"),
            "expected parent-missing error, got: {msg}"
        );
    }

    // Evolve par défaut préserve les labels du parent dans la metadata.
    #[tokio::test]
    async fn evolve_default_preserves_parent_labels() {
        let reg = registry();
        let cone = AlignmentCone::new(vec![vec![1.0, 0.0], vec![0.0, 1.0]], 0.5).unwrap();
        let md = ConeMetadata::new("bot", "initial")
            .unwrap()
            .with_label("env", "prod")
            .unwrap()
            .with_label("team", "platform")
            .unwrap();
        let canon = CanonicalCone::with_metadata(cone, md);
        let k = ConeKey::new("t", "a", 1).unwrap();
        reg.register(k.clone(), canon).await.unwrap();

        let h = reg
            .evolve(k, ConeEvolutionOp::UpdateRadius { new_radius: 0.9 }, None)
            .await
            .unwrap();
        assert_eq!(
            h.cone.metadata.labels.get("env").map(String::as_str),
            Some("prod")
        );
        assert_eq!(
            h.cone.metadata.labels.get("team").map(String::as_str),
            Some("platform")
        );
    }

    // PolicyRejected event contient le code d'invariant (I1..I7).
    #[tokio::test]
    async fn policy_rejected_event_carries_invariant_code() {
        let reg = registry();
        let mut rx = reg.take_events_receiver().await.unwrap();
        // Cône avec NaN → I7.
        let bad_cone = AlignmentCone::new(vec![vec![f32::NAN, 0.0], vec![0.0, 1.0]], 0.5).unwrap();
        let canon = CanonicalCone::new(bad_cone, "t", "d").unwrap();
        let k = ConeKey::new("t", "a", 1).unwrap();
        let _ = reg.register(k.clone(), canon).await;
        let evt = rx.recv().await.unwrap();
        match evt {
            RegistryEvent::PolicyRejected { reason, .. } => {
                assert!(
                    reason.contains("I7"),
                    "expected reason to mention I7, got: {reason}"
                );
            }
            other => panic!("expected PolicyRejected, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_register_across_tasks() {
        let reg = Arc::new(registry());
        let mut handles = Vec::new();
        for t in 0..4 {
            let reg = reg.clone();
            handles.push(tokio::spawn(async move {
                for i in 1..=8u32 {
                    let k = ConeKey::new("tenant", format!("task{t}"), i).unwrap();
                    reg.register(k, make_canonical(2, 4)).await.unwrap();
                }
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        for t in 0..4 {
            let vs = reg
                .list_versions("tenant", &format!("task{t}"))
                .await
                .unwrap();
            assert_eq!(vs.len(), 8);
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_report_breach_no_loss_when_under_capacity() {
        let reg = Arc::new(registry()); // capacity 1024
        let mut handles = Vec::new();
        for _ in 0..8 {
            let reg = reg.clone();
            handles.push(tokio::spawn(async move {
                for i in 0..64u32 {
                    let r = BreachReport::new(
                        ConeKey::new("t", "a", 1).unwrap(),
                        SwarmId::new(),
                        0.9,
                        0.5,
                        u64::from(i),
                        BreachContext::Dissolution,
                    );
                    reg.report_breach(r).await.unwrap();
                }
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        let hist = reg.breach_history(10_000, None).await.unwrap();
        assert_eq!(hist.len(), 8 * 64);
    }
}
