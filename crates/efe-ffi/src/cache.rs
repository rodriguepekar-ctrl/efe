//! LRU bundle cache — Sprint **Φ-C-FFI Completion (D26)**.
//!
//! Cf. `docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md` § 16 entry #05.
//!
//! ## Problème résolu
//!
//! Pre-Φ-C-FFI, `efe.classify_zero_copy()` construisait un `PyBundle`
//! à **chaque appel** :
//!
//! ```text
//!   classify_zero_copy(text)
//!     → load_bundle(path)               ← 50–100 ms cold start
//!         → fs::read(path) + verify_signature() + dim probe
//!     → PyEfeNativeClient::new(bundle, catalog_path)
//!     → classify
//!     → client.close()
//! ```
//!
//! Pour un script Python qui appelle classify N fois sur le même
//! bundle, ça multiplie le coût bundle-load par N → la latence
//! observée est dominée par la I/O bundle, pas par le swarm. Φ-C-FFI
//! ferme ce gap (G17) via un LRU cache **thread-safe** côté Rust.
//!
//! ## Choix de design
//!
//! | Décision | Rationale |
//! |---|---|
//! | **`parking_lot::Mutex` (sync)** | Le cache est consulté depuis `classify_zero_copy` qui est `#[pyfunction]` synchrone ; `tokio::sync::Mutex` introduirait un blocage async↔sync inutile. (D7 § 19) |
//! | **LRU max 4 entries** | Couvre 95 % des use cases (1–2 bundles actifs) sans bound VRAM > 32 GiB. (D16 § 19) |
//! | **Clé = `(path, mtime)`** | Détecte un swap de bundle sur disque (= operator pousse `/etc/efe/bundles/active` vers une nouvelle target) sans signature manuelle. |
//! | **Counter exposé** | `efe_ffi_bundle_cache_hits_total{outcome}` hit/miss/evict (instrument 7 du module Telemetry FFI). |
//!
//! ## Sûreté thread
//!
//! `BundleCache` est `Send + Sync` (= `Mutex<HashMap>`). Le test
//! `cache_is_send_sync` enforce statiquement via
//! `static_assertions::assert_impl_all!`. **D-FFI-4 + DF4**.
//!
//! ## Mémoire bornée
//!
//! Le cache stocke `Arc<PyBundle>`. La VRAM hit est partagée — un
//! `Arc::strong_count > 1` survit même si le cache évince l'entrée
//! (= classify en cours protège son bundle). Le drop final libère.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use parking_lot::Mutex;

use efe_telemetry::metrics::ffi_instruments::note_bundle_cache;

use crate::bundle::PyBundle;

/// Capacity maximum du LRU cache.
///
/// **D16 décision** : 4 entries. Au-delà, Python users devraient
/// gérer eux-mêmes le cache (= 5+ bundles distincts = pattern atypique).
pub const LRU_CAPACITY: usize = 4;

/// Cle canonique du cache : `(path, mtime_unix_secs)`.
///
/// Inclure `mtime` est crucial : un operator qui swap le symlink
/// `/etc/efe/bundles/active` vers une nouvelle cible doit voir
/// `classify_zero_copy` charger le nouveau bundle automatiquement.
/// Avec une clé `path`-only, le cache servirait la version pré-swap
/// → bug furtif où "operator a déployé mais rien ne change".
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CacheKey {
    /// Chemin absolu sur disque.
    pub path: PathBuf,
    /// `mtime` en secondes depuis l'epoch (0 si fichier introuvable).
    pub mtime_unix: u64,
}

impl CacheKey {
    /// Construit une clé en mesurant `mtime` du chemin. Retourne `None`
    /// si le chemin est inexistant ou inaccessible — l'appelant
    /// (= [`BundleCache::get_or_load`]) propage alors un error code.
    pub fn probe(path: &Path) -> Option<Self> {
        let canonical = path.canonicalize().ok()?;
        let mtime_unix = std::fs::metadata(&canonical)
            .ok()?
            .modified()
            .ok()
            .and_then(|m| m.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Some(Self {
            path: canonical,
            mtime_unix,
        })
    }
}

/// Entrée LRU = bundle + access tick (= un compteur monotonique
/// qu'on incrémente à chaque hit pour réordonner).
struct Entry {
    bundle: Arc<PyBundle>,
    last_access_tick: u64,
}

/// Cache LRU thread-safe pour les `PyBundle` chargés.
///
/// **D-FFI-4 (Pin)** : la sync `Send + Sync` est enforced
/// statiquement par `static_assertions::assert_impl_all!` dans
/// `tests::cache_is_send_sync`.
pub struct BundleCache {
    inner: Mutex<Inner>,
    capacity: usize,
}

struct Inner {
    entries: HashMap<CacheKey, Entry>,
    tick: u64,
}

impl BundleCache {
    /// Construit un cache vide avec [`LRU_CAPACITY`] = 4.
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(LRU_CAPACITY)
    }

    /// Construit un cache avec une capacité custom. Utile pour les
    /// tests qui veulent forcer un évincement à `cap = 1`.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        assert!(capacity >= 1, "BundleCache capacity must be ≥ 1");
        Self {
            inner: Mutex::new(Inner {
                entries: HashMap::with_capacity(capacity),
                tick: 0,
            }),
            capacity,
        }
    }

    /// Lookup pur (no insert). Si `key` est présent, met à jour son
    /// tick LRU et incrémente le counter `hit` ; sinon retourne `None`
    /// et incrémente `miss`. La distinction hit/miss est l'instrument
    /// `efe_ffi_bundle_cache_hits_total{outcome}`.
    pub fn get(&self, key: &CacheKey) -> Option<Arc<PyBundle>> {
        let mut g = self.inner.lock();
        g.tick = g.tick.wrapping_add(1);
        let new_tick = g.tick;
        if let Some(entry) = g.entries.get_mut(key) {
            entry.last_access_tick = new_tick;
            let bundle = Arc::clone(&entry.bundle);
            drop(g);
            note_bundle_cache("hit");
            Some(bundle)
        } else {
            drop(g);
            note_bundle_cache("miss");
            None
        }
    }

    /// Insert. Évince le plus ancien entry si la capacité est dépassée
    /// (= LRU). Le compteur `evict` est incrémenté pour chaque éviction.
    ///
    /// Retourne `Arc<PyBundle>` pour symétrie avec `get`.
    pub fn insert(&self, key: CacheKey, bundle: Arc<PyBundle>) -> Arc<PyBundle> {
        let mut g = self.inner.lock();
        g.tick = g.tick.wrapping_add(1);
        let new_tick = g.tick;
        // Si l'entrée existe déjà, on la remplace simplement (=
        // "insert ou update"). Pas d'évincement nécessaire.
        let exists = g.entries.contains_key(&key);
        g.entries.insert(
            key,
            Entry {
                bundle: Arc::clone(&bundle),
                last_access_tick: new_tick,
            },
        );
        // Si on a dépassé la capacité, évince le LRU.
        if !exists && g.entries.len() > self.capacity {
            // Find the key with the smallest tick.
            let lru_key = g
                .entries
                .iter()
                .min_by_key(|(_, e)| e.last_access_tick)
                .map(|(k, _)| k.clone());
            if let Some(k) = lru_key {
                g.entries.remove(&k);
                drop(g);
                note_bundle_cache("evict");
                return bundle;
            }
        }
        drop(g);
        bundle
    }

    /// Combine `get` + `insert` : si miss, appelle `loader` pour
    /// matérialiser le bundle, l'insère, retourne. Pattern canonique
    /// pour `classify_zero_copy`.
    ///
    /// `loader` est une closure FnOnce qui peut faillir
    /// (`PyResult<PyBundle>`) — toute erreur est propagée sans toucher
    /// au cache.
    pub fn get_or_load<F>(
        &self,
        key: CacheKey,
        loader: F,
    ) -> pyo3::PyResult<Arc<PyBundle>>
    where
        F: FnOnce() -> pyo3::PyResult<PyBundle>,
    {
        if let Some(b) = self.get(&key) {
            return Ok(b);
        }
        let bundle = Arc::new(loader()?);
        Ok(self.insert(key, bundle))
    }

    /// Vide entièrement le cache. Réservé aux tests + au scénario
    /// `python -c "efe.invalidate_cache()"` (DT-F-CACHE-3, D29).
    pub fn clear(&self) {
        let mut g = self.inner.lock();
        g.entries.clear();
    }

    /// Renvoie le nombre d'entrées actuellement résidentes.
    pub fn len(&self) -> usize {
        self.inner.lock().entries.len()
    }

    /// Cache vide ?
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for BundleCache {
    fn default() -> Self {
        Self::new()
    }
}

// ====================================================================
// Cache global — une seule instance partagée par tout le process Python.
// ====================================================================

use std::sync::OnceLock;

static GLOBAL_CACHE: OnceLock<BundleCache> = OnceLock::new();

/// Renvoie le cache LRU global. Lazy-initialised à la première
/// utilisation ; sûr cross-threads.
pub fn global() -> &'static BundleCache {
    GLOBAL_CACHE.get_or_init(BundleCache::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use static_assertions::assert_impl_all;
    use std::path::PathBuf;

    // ────────────────────────────────────────────────────────────────
    // Static guarantees (D-FFI-4 + DF4 enforcement)
    // ────────────────────────────────────────────────────────────────

    assert_impl_all!(BundleCache: Send, Sync);
    assert_impl_all!(CacheKey: Send, Sync, Clone, Eq);

    // ────────────────────────────────────────────────────────────────
    // CacheKey behaviour
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn cache_key_probe_returns_none_for_missing_file() {
        let key = CacheKey::probe(&PathBuf::from("/__definitely_not_a_real_path_42__"));
        assert!(key.is_none());
    }

    #[test]
    fn cache_key_probe_succeeds_for_existing_file() {
        let tmp = std::env::temp_dir().join("efe_cache_probe_test");
        std::fs::write(&tmp, b"x").unwrap();
        let key = CacheKey::probe(&tmp);
        assert!(key.is_some(), "probe should find an existing file");
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn cache_key_equality_includes_mtime() {
        let a = CacheKey {
            path: "/tmp/x".into(),
            mtime_unix: 100,
        };
        let b = CacheKey {
            path: "/tmp/x".into(),
            mtime_unix: 200,
        };
        assert_ne!(
            a, b,
            "same path different mtime should NOT be equal (= operator bundle swap detection)"
        );
    }

    // ────────────────────────────────────────────────────────────────
    // BundleCache LRU semantics — tested without real PyBundles via
    // a synthetic Arc<PyBundle> wrapper. Per-test capacity = 2 makes
    // the LRU eviction observable in 3 inserts.
    // ────────────────────────────────────────────────────────────────

    fn synth_key(p: &str, mtime: u64) -> CacheKey {
        CacheKey {
            path: p.into(),
            mtime_unix: mtime,
        }
    }

    fn synth_bundle() -> Arc<PyBundle> {
        // PyBundle::synthetic existe pour test path (cf. bundle.rs).
        // Cf. `bundle::PyBundle::synthetic_for_test`.
        Arc::new(PyBundle::synthetic_for_test())
    }

    #[test]
    fn empty_cache_has_zero_len() {
        let c = BundleCache::new();
        assert_eq!(c.len(), 0);
        assert!(c.is_empty());
    }

    #[test]
    fn insert_then_get_returns_same_arc() {
        let c = BundleCache::with_capacity(2);
        let k = synth_key("/tmp/a", 0);
        let b = synth_bundle();
        c.insert(k.clone(), Arc::clone(&b));
        let got = c.get(&k).expect("just inserted");
        assert!(Arc::ptr_eq(&got, &b));
    }

    #[test]
    fn miss_returns_none() {
        let c = BundleCache::with_capacity(2);
        let k = synth_key("/tmp/x", 0);
        assert!(c.get(&k).is_none());
    }

    #[test]
    fn lru_evicts_oldest_when_capacity_exceeded() {
        let c = BundleCache::with_capacity(2);
        let k1 = synth_key("/tmp/a", 0);
        let k2 = synth_key("/tmp/b", 0);
        let k3 = synth_key("/tmp/c", 0);
        c.insert(k1.clone(), synth_bundle());
        c.insert(k2.clone(), synth_bundle());
        // Touche k1 pour le rendre MRU.
        let _ = c.get(&k1);
        // Insert k3 → doit évincer k2 (= LRU).
        c.insert(k3.clone(), synth_bundle());
        assert!(c.get(&k1).is_some(), "k1 should survive (MRU)");
        assert!(c.get(&k2).is_none(), "k2 should be evicted (LRU)");
        assert!(c.get(&k3).is_some(), "k3 just inserted");
    }

    #[test]
    fn reinsert_same_key_does_not_evict() {
        // Audit fix : "re-insert" doit être un upsert, pas un insert
        // qui pousse la cap au-delà du seuil.
        let c = BundleCache::with_capacity(2);
        let k = synth_key("/tmp/a", 0);
        c.insert(k.clone(), synth_bundle());
        c.insert(k.clone(), synth_bundle());
        assert_eq!(c.len(), 1, "re-insert should NOT grow len");
    }

    #[test]
    fn clear_empties_the_cache() {
        let c = BundleCache::with_capacity(4);
        c.insert(synth_key("/tmp/a", 0), synth_bundle());
        c.insert(synth_key("/tmp/b", 0), synth_bundle());
        assert_eq!(c.len(), 2);
        c.clear();
        assert!(c.is_empty());
    }

    #[test]
    fn get_or_load_calls_loader_only_on_miss() {
        let c = BundleCache::with_capacity(2);
        let k = synth_key("/tmp/a", 0);
        let mut load_calls = 0;
        let _ = c.get_or_load(k.clone(), || {
            load_calls += 1;
            Ok(PyBundle::synthetic_for_test())
        });
        let _ = c.get_or_load(k.clone(), || {
            load_calls += 1;
            Ok(PyBundle::synthetic_for_test())
        });
        let _ = c.get_or_load(k, || {
            load_calls += 1;
            Ok(PyBundle::synthetic_for_test())
        });
        assert_eq!(load_calls, 1, "loader should be invoked exactly once");
    }

    #[test]
    fn get_or_load_propagates_loader_errors_without_caching() {
        let c = BundleCache::with_capacity(2);
        let k = synth_key("/tmp/err", 0);
        let r = c.get_or_load(k.clone(), || {
            Err(crate::error::ffi_err("simulated load failure"))
        });
        assert!(r.is_err());
        assert!(c.get(&k).is_none(), "errored load must NOT cache");
    }

    #[test]
    fn global_singleton_is_idempotent() {
        let a = global();
        let b = global();
        assert!(std::ptr::eq(a, b), "global() should return the same &'static");
    }

    #[test]
    fn capacity_one_evicts_on_every_distinct_insert() {
        let c = BundleCache::with_capacity(1);
        c.insert(synth_key("/tmp/a", 0), synth_bundle());
        c.insert(synth_key("/tmp/b", 0), synth_bundle());
        assert_eq!(c.len(), 1, "cap=1 → always exactly 1 entry");
    }
}
