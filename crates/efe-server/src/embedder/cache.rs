//! Sprint **D16** — singleflight `EmbedderRegistry`.
//!
//! Mirror of `docs/bench/d16_embedder_bundling_architecture.md` § 8.5.
//!
//! At runtime boot, multiple async tasks may simultaneously try to
//! load the same bundle (the gRPC handler, a warm-up task, the
//! catalog handler verifying a fingerprint). Without singleflight
//! we'd hash the same 500 MiB ONNX file twice, duplicate the ORT
//! session, and burn RAM.
//!
//! The pattern :
//!
//!   1. Fast path : the bundle is already loaded → clone Arc and
//!      return.
//!   2. Slow path : nobody has started loading → declare yourself
//!      leader, do the work, store the result, notify waiters.
//!   3. Concurrent slow path : someone else is the leader → wait
//!      on their `Notify`, then re-check the map.
//!
//! Leader failures are surfaced to all waiters via `LoadError`
//! cloning. We use a `Result<Bundle, Arc<BundleLoadError>>` channel
//! so the error is shareable.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, Notify, RwLock};

use super::bundle::{load_bundle_dir, Bundle, BundleLoadError, LoadPolicy};
use super::trust::TrustStore;

/// String form of a bundle id, used as the registry key. Equivalent
/// to the `manifest.bundle_id` field — we keep the type opaque to
/// avoid leaking serde-deserialise details.
pub type BundleId = String;

/// Outcome shared between the leader and its waiters.
type SharedLoadResult = Result<Bundle, Arc<BundleLoadError>>;

struct InflightEntry {
    notify: Arc<Notify>,
    /// Result, populated by the leader before notifying. `None` while
    /// the leader is still working ; `Some(_)` after the load returns.
    result: Mutex<Option<SharedLoadResult>>,
}

/// Process-wide bundle registry. Cheap to clone — the inner state
/// is `Arc<...>`.
#[derive(Clone)]
pub struct EmbedderRegistry {
    inner: Arc<EmbedderRegistryInner>,
}

struct EmbedderRegistryInner {
    /// Loaded, ready-to-use bundles keyed by id.
    loaded: RwLock<HashMap<BundleId, Bundle>>,
    /// Currently in-flight loads. Singleflight key.
    inflight: Mutex<HashMap<BundleId, Arc<InflightEntry>>>,
}

impl Default for EmbedderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for EmbedderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbedderRegistry")
            .field("loaded_count", &"<async-locked>")
            .finish()
    }
}

impl EmbedderRegistry {
    /// Empty registry. Use [`Self::ensure_loaded`] to add bundles.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(EmbedderRegistryInner {
                loaded: RwLock::new(HashMap::new()),
                inflight: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Lookup a previously-loaded bundle without triggering a load.
    /// Returns `None` if the bundle isn't in the registry yet.
    pub async fn get(&self, id: &str) -> Option<Bundle> {
        self.inner.loaded.read().await.get(id).cloned()
    }

    /// Number of loaded bundles. Used by the
    /// `efe_embedder_active_bundles` gauge.
    pub async fn len(&self) -> usize {
        self.inner.loaded.read().await.len()
    }

    /// Ensure the bundle at `path` (with id `id`) is loaded. If
    /// another task is in the middle of loading the same id, wait
    /// for it instead of starting a duplicate load.
    ///
    /// **Audit fix D16-C2** — race-free wait pattern. The pre-audit
    /// version called `entry.notify.notified().await` AFTER releasing
    /// the inflight mutex, which created a lost-wakeup window : the
    /// leader could call `notify_waiters()` before the non-leader
    /// parked, leaving the non-leader hung forever. We now use the
    /// canonical Tokio "register-then-check" pattern : create the
    /// `Notified` future first (it captures the current notify
    /// state), then check the result, then `await` only if the
    /// result isn't yet there.
    pub async fn ensure_loaded(
        &self,
        id: &str,
        path: PathBuf,
        policy: LoadPolicy,
        trust: Arc<TrustStore>,
    ) -> SharedLoadResult {
        // **Audit fix D16-H2** — default timeout = 60 s (matches the
        // architecture-doc default of `EFE_BUNDLE_LOAD_TIMEOUT_S=60`).
        // Callers that need a different value should call
        // `ensure_loaded_with_timeout` directly. See that function
        // for the rationale + escape hatch.
        self.ensure_loaded_with_timeout(id, path, policy, trust, Duration::from_secs(60))
            .await
    }

    /// Same as [`Self::ensure_loaded`] but with an explicit timeout
    /// matching the operator's `EFE_BUNDLE_LOAD_TIMEOUT_S` config.
    /// **Audit fix D16-H2** — the pre-audit version had no timeout
    /// at all, so a slow disk could hang a load forever (and its
    /// waiters with it).
    pub async fn ensure_loaded_with_timeout(
        &self,
        id: &str,
        path: PathBuf,
        policy: LoadPolicy,
        trust: Arc<TrustStore>,
        load_timeout: Duration,
    ) -> SharedLoadResult {
        // Fast path.
        if let Some(b) = self.inner.loaded.read().await.get(id) {
            return Ok(b.clone());
        }
        // Singleflight.
        let (entry, is_leader) = {
            let mut inflight = self.inner.inflight.lock().await;
            if let Some(existing) = inflight.get(id) {
                (existing.clone(), false)
            } else {
                let entry = Arc::new(InflightEntry {
                    notify: Arc::new(Notify::new()),
                    result: Mutex::new(None),
                });
                inflight.insert(id.to_string(), entry.clone());
                (entry, true)
            }
        };
        if !is_leader {
            return wait_for_leader(&entry, id).await;
        }
        // Leader path — drive the load with a hard timeout.
        // **Audit fix D16-H2** — the pre-audit version awaited
        // `load_bundle_dir` with no upper bound. A frozen NFS / OCI
        // pull / signature verify could hang the leader forever and
        // every concurrent waiter with it (lost-wakeup-equivalent
        // operationally).
        let result =
            match tokio::time::timeout(load_timeout, load_bundle_dir(&path, &policy, &trust)).await
            {
                Ok(r) => r,
                Err(_elapsed) => Err(BundleLoadError::Io {
                    path: path.clone(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!(
                            "bundle load exceeded EFE_BUNDLE_LOAD_TIMEOUT_S={}s",
                            load_timeout.as_secs()
                        ),
                    ),
                }),
            };
        let shared: SharedLoadResult = result.map_err(Arc::new);
        // Stash result FIRST (inside the entry's lock), THEN
        // notify_waiters. Order matters : the `wait_for_leader` path
        // pre-arms `Notified` then re-checks the result. So as long
        // as the notify happens after the stash, a late waiter sees
        // the result on its post-notify re-check.
        {
            let mut result_guard = entry.result.lock().await;
            *result_guard = Some(shared.clone());
        }
        entry.notify.notify_waiters();
        // Remove from inflight + insert into loaded inline.
        // **Audit fix D16-M2** — the pre-audit version spawned an
        // extra tokio task to do the same insert that we're doing
        // inline below. Redundant, removed.
        if let Ok(b) = &shared {
            let mut g = self.inner.loaded.write().await;
            g.insert(b.manifest().bundle_id.clone(), b.clone());
        }
        let mut inflight = self.inner.inflight.lock().await;
        inflight.remove(id);
        shared
    }
}

/// Race-free wait for the leader's load result.
///
/// **Audit fix D16-C2** — see [`EmbedderRegistry::ensure_loaded`] for
/// the rationale. This helper isolates the register-then-check
/// pattern : we obtain a `Notified` future BEFORE checking the
/// result so that a leader notification between check and await is
/// captured by the future (Tokio's documented behaviour).
async fn wait_for_leader(entry: &Arc<InflightEntry>, _id: &str) -> SharedLoadResult {
    loop {
        // Pre-arm the waiter. `Notified` captures the current
        // notification state; if a notify_waiters fires between this
        // line and the .await, the future polls Ready immediately.
        let notified = entry.notify.notified();
        // Check the result under the lock.
        {
            let result_guard = entry.result.lock().await;
            if let Some(r) = result_guard.as_ref() {
                return r.clone();
            }
        }
        // Result not yet stashed. Wait for the leader's notify.
        notified.await;
        // Loop back to re-check ; usually result is Some after the
        // notify but a defensive re-check protects against spurious
        // wakeups.
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SignaturePolicy;
    use crate::embedder::manifest::{
        BuildInfo, Component, Dtype, Framework, LicenseInfo, Manifest, Pooling, RuntimeConfig,
    };
    use crate::embedder::trust::TrustStore;
    use sha2::{Digest, Sha256};

    fn sha256_hex(b: &[u8]) -> String {
        hex::encode(Sha256::digest(b))
    }

    fn write_minimal_unsigned_bundle(dir: &std::path::Path) -> String {
        let weights = b"weights";
        let tokenizer = b"tokenizer-json";
        std::fs::create_dir_all(dir.join("weights")).unwrap();
        std::fs::create_dir_all(dir.join("tokenizer")).unwrap();
        std::fs::write(dir.join("weights/model.onnx"), weights).unwrap();
        std::fs::write(dir.join("tokenizer/tokenizer.json"), tokenizer).unwrap();
        let manifest = Manifest {
            manifest_version: "1.0".into(),
            bundle_id: "x/y@1.0.0/onnx/64".into(),
            framework: Framework::Onnx,
            dim: 64,
            components: vec![
                Component {
                    path: "weights/model.onnx".into(),
                    sha256: sha256_hex(weights),
                    size_bytes: weights.len() as u64,
                    required: true,
                },
                Component {
                    path: "tokenizer/tokenizer.json".into(),
                    sha256: sha256_hex(tokenizer),
                    size_bytes: tokenizer.len() as u64,
                    required: true,
                },
            ],
            runtime: RuntimeConfig {
                max_seq_len: 256,
                pooling: Pooling::Mean,
                normalize: true,
                dtype: Dtype::Float32,
            },
            build: BuildInfo {
                builder: "test".into(),
                built_at_unix_ms: 0,
                git_sha: "0".repeat(40),
                source_date_epoch: 0,
            },
            license: LicenseInfo {
                spdx_id: "Apache-2.0".into(),
                url: None,
                redistribution_allowed: true,
            },
            model_origin: None,
            sbom_path: None,
        };
        let id = manifest.bundle_id.clone();
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        id
    }

    #[tokio::test]
    async fn loads_once_caches_for_subsequent_calls() {
        let dir = tempfile::tempdir().unwrap();
        let id = write_minimal_unsigned_bundle(dir.path());
        let trust = Arc::new(TrustStore { keys: vec![] });
        let registry = EmbedderRegistry::new();
        let policy = LoadPolicy {
            signature: SignaturePolicy::Skip,
            require_sbom: false,
            ack_non_redistributable: false,
        };
        let b1 = registry
            .ensure_loaded(&id, dir.path().to_path_buf(), policy.clone(), trust.clone())
            .await
            .unwrap();
        let b2 = registry
            .ensure_loaded(&id, dir.path().to_path_buf(), policy.clone(), trust.clone())
            .await
            .unwrap();
        // Both refs point to the same Arc-cached bundle.
        assert_eq!(b1.fingerprint().hex(), b2.fingerprint().hex());
        assert_eq!(registry.len().await, 1);
    }

    #[tokio::test]
    async fn singleflight_dedupes_concurrent_loads() {
        let dir = tempfile::tempdir().unwrap();
        let id = write_minimal_unsigned_bundle(dir.path());
        let trust = Arc::new(TrustStore { keys: vec![] });
        let registry = EmbedderRegistry::new();
        let policy = LoadPolicy {
            signature: SignaturePolicy::Skip,
            require_sbom: false,
            ack_non_redistributable: false,
        };
        // Spawn 5 concurrent loads of the same id.
        let handles: Vec<_> = (0..5)
            .map(|_| {
                let r = registry.clone();
                let path = dir.path().to_path_buf();
                let id = id.clone();
                let policy = policy.clone();
                let trust = trust.clone();
                tokio::spawn(async move { r.ensure_loaded(&id, path, policy, trust).await })
            })
            .collect();
        for h in handles {
            h.await.unwrap().unwrap();
        }
        // Exactly one bundle ended up cached.
        assert_eq!(registry.len().await, 1);
    }

    #[tokio::test]
    async fn get_returns_none_for_unknown_id() {
        let r = EmbedderRegistry::new();
        assert!(r.get("nonexistent").await.is_none());
    }
}
