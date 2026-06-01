//! In-memory `CatalogRegistry` implementation.
//!
//! Architecture document § 9 — single-replica MVP. The store is a
//! `tokio::sync::RwLock<HashMap<(tenant, name), Vec<RegisteredCatalog>>>`
//! ; each `(tenant, name)` chain is a sorted-ascending list of
//! versions.
//!
//! # Concurrency
//!
//! * `register` / `evolve` / `delete` take the write lock.
//! * `get` / `list_*` take the read lock.
//!
//! For the MVP load profile (~10 admin RPCs/sec), the single
//! `RwLock` is fine. A postgres impl will refine this in D13.
//!
//! # SHA-256 placeholder
//!
//! The `content_sha256` field is currently a *placeholder* hash
//! computed from the canonical JSON length — we don't yet have
//! `sha2` as a workspace dep. D12-bis will add it as an additive
//! direct dep on this crate. Until then, the digest is non-
//! cryptographic but stable across re-runs of the same content.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use super::audit::AuditEmitter;
use super::policy::CatalogPolicy;
use super::registry::{
    CatalogClass, CatalogContent, CatalogError, CatalogKey, CatalogMetadata, CatalogPage,
    CatalogRefSummary, CatalogRegistry, ContentDigest, EvolveOp, RegisteredCatalog,
};

// ---------------------------------------------------------------------
// Storage shape
// ---------------------------------------------------------------------

type ChainKey = (String, String); // (tenant, name)
type Chain = Vec<RegisteredCatalog>;

/// Lock-protected store. `BTreeMap` keeps lists ordered, which we
/// rely on for deterministic `list_catalogs` output (sorted by
/// `(tenant, name)`).
type Store = BTreeMap<ChainKey, Chain>;

/// In-memory `CatalogRegistry` impl.
#[derive(Clone)]
pub struct InMemoryRegistry {
    inner: Arc<RwLock<Store>>,
    policy: CatalogPolicy,
    audit: AuditEmitter,
}

impl InMemoryRegistry {
    /// Build with default policy + tracing-backed audit sink.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(BTreeMap::new())),
            policy: CatalogPolicy::default(),
            audit: AuditEmitter::new(),
        }
    }

    /// Build with explicit policy + audit emitter (used in tests).
    pub fn with_policy_and_audit(policy: CatalogPolicy, audit: AuditEmitter) -> Self {
        Self {
            inner: Arc::new(RwLock::new(BTreeMap::new())),
            policy,
            audit,
        }
    }

    /// Number of `(tenant, name)` chains currently stored. Useful
    /// for tests + metrics export.
    pub async fn len(&self) -> usize {
        self.inner.read().await.len()
    }

    /// True when the store is empty.
    pub async fn is_empty(&self) -> bool {
        self.inner.read().await.is_empty()
    }
}

impl Default for InMemoryRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------
// CatalogRegistry impl
// ---------------------------------------------------------------------

#[async_trait]
impl CatalogRegistry for InMemoryRegistry {
    async fn register(
        &self,
        tenant: &str,
        name: &str,
        content: CatalogContent,
        mut metadata: CatalogMetadata,
    ) -> Result<RegisteredCatalog, CatalogError> {
        // **D14 audit fix C1** — validate tenant + name match the
        // canonical regex `^[a-z0-9_-]{1,63}$` before anything else.
        // The Postgres backend's `parent_key` CHECK constraint enforces
        // this regex ; refusing here keeps both backends consistent and
        // avoids the latent failure where `register v1` succeeds and
        // `evolve to v2` fails on a CHECK violation.
        CatalogPolicy::validate_identifier("tenant", tenant)?;
        CatalogPolicy::validate_identifier("name", name)?;
        // Schema first (cheap, fail-fast).
        self.policy.validate_content(&content)?;

        let mut store = self.inner.write().await;

        // Quota: names per tenant.
        let tenant_chains: u32 = store
            .keys()
            .filter(|(t, _)| t == tenant)
            .count()
            .try_into()
            .unwrap_or(u32::MAX);
        // Only enforce names quota if (tenant, name) is NEW.
        let is_new_name = !store.contains_key(&(tenant.to_owned(), name.to_owned()));
        if is_new_name {
            self.policy.check_names_quota(tenant, tenant_chains)?;
        }

        let chain_key: ChainKey = (tenant.to_owned(), name.to_owned());
        let chain = store.entry(chain_key.clone()).or_default();

        // Quota: versions per (tenant, name).
        self.policy
            .check_versions_quota(tenant, name, chain.len() as u32)?;

        let next_version: u32 = chain.last().map(|r| r.key.version + 1).unwrap_or(1);
        let key = CatalogKey {
            tenant: tenant.to_owned(),
            name: name.to_owned(),
            version: next_version,
        };

        // Stamp metadata fields the server controls.
        metadata.evolution_op = "register".to_owned();
        metadata.parent_key = String::new();
        metadata.content_sha256 = compute_digest(&content);

        let registered = RegisteredCatalog {
            key: key.clone(),
            content,
            metadata: metadata.clone(),
            compiled: None, // populated by the embed pool — D12-bis
        };
        chain.push(registered.clone());

        // Audit log.
        self.audit.emit_register(&key, &metadata);

        Ok(registered)
    }

    async fn evolve(
        &self,
        from: &CatalogKey,
        op: EvolveOp,
        mut metadata_override: CatalogMetadata,
    ) -> Result<RegisteredCatalog, CatalogError> {
        let mut store = self.inner.write().await;

        let chain_key: ChainKey = (from.tenant.clone(), from.name.clone());
        let chain = store
            .get_mut(&chain_key)
            .ok_or_else(|| CatalogError::NotFound(from.to_wire()))?;

        let source_idx = chain
            .iter()
            .position(|r| r.key == *from)
            .ok_or_else(|| CatalogError::NotFound(from.to_wire()))?;
        // Borrow the source's content immutably.
        let source = chain[source_idx].clone();

        // Validate the op against the source content.
        self.policy.validate_evolve(&source.content, &op)?;

        // Quota check on the chain length BEFORE inserting.
        self.policy
            .check_versions_quota(&from.tenant, &from.name, chain.len() as u32)?;

        // Apply the op to produce the new content.
        let new_content = apply_evolve(&source.content, &op);

        // Re-validate the resulting content as a sanity net (post-op).
        self.policy.validate_content(&new_content)?;

        let next_version = chain
            .last()
            .map(|r| r.key.version + 1)
            .unwrap_or(from.version + 1);
        let new_key = CatalogKey {
            tenant: from.tenant.clone(),
            name: from.name.clone(),
            version: next_version,
        };

        // Stamp server-controlled metadata.
        metadata_override.evolution_op = op.audit_tag().to_owned();
        metadata_override.parent_key = from.to_wire();
        metadata_override.content_sha256 = compute_digest(&new_content);

        let registered = RegisteredCatalog {
            key: new_key.clone(),
            content: new_content,
            metadata: metadata_override.clone(),
            compiled: None,
        };
        chain.push(registered.clone());

        self.audit.emit_evolve(&new_key, &metadata_override, &op);

        Ok(registered)
    }

    async fn get(
        &self,
        key: &CatalogKey,
        include_compiled: bool,
    ) -> Result<RegisteredCatalog, CatalogError> {
        let store = self.inner.read().await;
        let chain_key: ChainKey = (key.tenant.clone(), key.name.clone());
        let chain = store
            .get(&chain_key)
            .ok_or_else(|| CatalogError::NotFound(key.to_wire()))?;
        let r = chain
            .iter()
            .find(|r| r.key == *key)
            .ok_or_else(|| CatalogError::NotFound(key.to_wire()))?;
        let mut out = r.clone();
        if !include_compiled {
            out.compiled = None;
        }
        Ok(out)
    }

    async fn list_catalogs(
        &self,
        tenant: &str,
        page_size: u32,
        _page_token: Option<&str>,
    ) -> Result<CatalogPage, CatalogError> {
        let store = self.inner.read().await;
        let mut catalogs: Vec<CatalogRefSummary> = store
            .iter()
            .filter(|((t, _), _)| t == tenant)
            .filter_map(|((_t, name), chain)| {
                chain.last().map(|latest| CatalogRefSummary {
                    tenant: latest.key.tenant.clone(),
                    name: name.clone(),
                    latest_version: latest.key.version,
                    num_classes: latest.content.classes.len() as u32,
                    num_anchors: latest
                        .content
                        .classes
                        .iter()
                        .map(|c| c.canonical_sentences.len() as u32)
                        .sum(),
                })
            })
            .collect();

        // Already sorted by `(tenant, name)` because `BTreeMap`
        // iterates ordered. Apply page_size.
        let page_size_clamped = page_size.clamp(1, 500) as usize;
        let next_token = if catalogs.len() > page_size_clamped {
            // For the in-memory MVP we just truncate ; pagination
            // refinement lands in D13 alongside postgres.
            "next".to_owned()
        } else {
            String::new()
        };
        catalogs.truncate(page_size_clamped);
        Ok(CatalogPage {
            catalogs,
            next_page_token: next_token,
        })
    }

    async fn list_versions(
        &self,
        tenant: &str,
        name: &str,
    ) -> Result<Vec<CatalogKey>, CatalogError> {
        let store = self.inner.read().await;
        let chain_key: ChainKey = (tenant.to_owned(), name.to_owned());
        let chain = store
            .get(&chain_key)
            .ok_or_else(|| CatalogError::NotFound(format!("{tenant}/{name}")))?;
        Ok(chain.iter().map(|r| r.key.clone()).collect())
    }

    async fn delete(
        &self,
        key: &CatalogKey,
        force: bool,
        principal: &str,
    ) -> Result<(), CatalogError> {
        let mut store = self.inner.write().await;
        let chain_key: ChainKey = (key.tenant.clone(), key.name.clone());
        let chain = store
            .get_mut(&chain_key)
            .ok_or_else(|| CatalogError::NotFound(key.to_wire()))?;

        // Locate the target.
        if !chain.iter().any(|r| r.key == *key) {
            return Err(CatalogError::NotFound(key.to_wire()));
        }

        // Reference check — refuse deletion if a successor's
        // parent_key points to this version, unless force=true.
        let referenced = chain.iter().any(|r| r.metadata.parent_key == key.to_wire());
        if referenced && !force {
            return Err(CatalogError::Conflict(format!(
                "{} is referenced by a successor ; pass force=true to cascade",
                key.to_wire(),
            )));
        }

        // Compute the set of versions to remove. Always includes
        // the target ; with `force=true`, also includes every
        // version whose parent chain transitively reaches the
        // target. This handles forked chains correctly — a
        // sibling that branches off from a different ancestor
        // is NOT touched by the cascade.
        let mut to_remove: std::collections::HashSet<u32> = std::collections::HashSet::new();
        to_remove.insert(key.version);

        if force {
            // Iterative closure : keep adding versions whose parent
            // is already in `to_remove`, until no more grow.
            // O(N²) worst case, but N ≤ max_versions_per_name (1000).
            let target_wire = key.to_wire();
            loop {
                let mut grew = false;
                for r in chain.iter() {
                    if to_remove.contains(&r.key.version) {
                        continue;
                    }
                    let parent_wire = &r.metadata.parent_key;
                    // Direct hit on the deleted target.
                    if parent_wire == &target_wire {
                        to_remove.insert(r.key.version);
                        grew = true;
                        continue;
                    }
                    // Indirect hit : parent_key parses to a version
                    // we've already marked for removal.
                    if let Ok(parent_key) = CatalogKey::parse(parent_wire) {
                        if parent_key.tenant == key.tenant
                            && parent_key.name == key.name
                            && to_remove.contains(&parent_key.version)
                        {
                            to_remove.insert(r.key.version);
                            grew = true;
                        }
                    }
                }
                if !grew {
                    break;
                }
            }
        }

        chain.retain(|r| !to_remove.contains(&r.key.version));

        // If the chain is now empty, drop the entry.
        if chain.is_empty() {
            store.remove(&chain_key);
        }

        self.audit.emit_delete(key, principal, force);
        Ok(())
    }
}

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

fn compute_digest(content: &CatalogContent) -> ContentDigest {
    // Placeholder — non-cryptographic. D12-bis swaps in a real
    // SHA-256 once `sha2` is added to this crate's deps.
    let summary = format!(
        "placeholder-classes={}-dim={}-ch={}-cm={}-mpd={}",
        content.classes.len(),
        content.embedder.dim,
        content.thresholds.confidence_high,
        content.thresholds.confidence_medium,
        content.cone_policy.max_projection_distance,
    );
    ContentDigest(summary)
}

fn apply_evolve(source: &CatalogContent, op: &EvolveOp) -> CatalogContent {
    let mut out = source.clone();
    match op {
        EvolveOp::AppendClass { class } => {
            out.classes.push(class.clone());
        }
        EvolveOp::RemoveClass { class_name } => {
            out.classes.retain(|c| &c.name != class_name);
        }
        EvolveOp::UpdateThresholds { thresholds } => {
            out.thresholds = thresholds.clone();
        }
        EvolveOp::ReplaceClassAnchors {
            class_name,
            canonical_sentences,
        } => {
            for cls in out.classes.iter_mut() {
                if &cls.name == class_name {
                    cls.canonical_sentences = canonical_sentences.clone();
                }
            }
        }
        EvolveOp::Recompile { new_embedder } => {
            out.embedder = new_embedder.clone();
        }
    }
    out
}

// Forward-compat helper to silence the dead-code lint on `class`
// imports — only used inside `apply_evolve` matches above.
#[allow(dead_code)]
fn _touch_imports() -> Option<&'static CatalogClass> {
    None
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::registry::{ConePolicy, EmbedderSpec, ThresholdPolicy};

    fn sample_class(name: &str) -> CatalogClass {
        CatalogClass {
            name: name.to_owned(),
            description: "demo".into(),
            canonical_sentences: vec!["one".into(), "two".into()],
            color: "#000".into(),
            deprecated: false,
        }
    }

    fn sample_content() -> CatalogContent {
        CatalogContent {
            embedder: EmbedderSpec {
                family: "st".into(),
                model: "mini".into(),
                dim: 384,
                normalize: "l2".into(),
            },
            classes: vec![sample_class("a"), sample_class("b")],
            cone_policy: ConePolicy {
                max_projection_distance: 0.7,
            },
            thresholds: ThresholdPolicy {
                confidence_high: 0.8,
                confidence_medium: 0.5,
            },
        }
    }

    fn sample_meta() -> CatalogMetadata {
        CatalogMetadata {
            created_at_unix_ms: 1_700_000_000_000,
            created_by: "alice@acme.com".into(),
            description: "init".into(),
            parent_key: String::new(),
            evolution_op: String::new(),
            labels: BTreeMap::new(),
            content_sha256: ContentDigest(String::new()),
        }
    }

    #[tokio::test]
    async fn register_assigns_v1_then_v2() {
        let reg = InMemoryRegistry::new();
        let v1 = reg
            .register("acme", "billing", sample_content(), sample_meta())
            .await
            .unwrap();
        assert_eq!(v1.key.version, 1);
        assert_eq!(v1.metadata.evolution_op, "register");
        assert_eq!(v1.metadata.parent_key, "");

        let v2 = reg
            .register("acme", "billing", sample_content(), sample_meta())
            .await
            .unwrap();
        assert_eq!(v2.key.version, 2);
    }

    #[tokio::test]
    async fn register_rejects_invalid_content() {
        let reg = InMemoryRegistry::new();
        let mut bad = sample_content();
        bad.classes.pop(); // 1 class — under MIN_CLASSES_PER_CATALOG
        let err = reg
            .register("acme", "billing", bad, sample_meta())
            .await
            .unwrap_err();
        assert!(matches!(err, CatalogError::InvalidArgument(_)));
    }

    #[tokio::test]
    async fn evolve_append_increments_version_and_sets_parent() {
        let reg = InMemoryRegistry::new();
        let v1 = reg
            .register("acme", "billing", sample_content(), sample_meta())
            .await
            .unwrap();
        let v2 = reg
            .evolve(
                &v1.key,
                EvolveOp::AppendClass {
                    class: sample_class("c"),
                },
                sample_meta(),
            )
            .await
            .unwrap();
        assert_eq!(v2.key.version, 2);
        assert_eq!(v2.metadata.parent_key, v1.key.to_wire());
        assert_eq!(v2.metadata.evolution_op, "append_class");
        assert_eq!(v2.content.classes.len(), 3);
    }

    #[tokio::test]
    async fn evolve_remove_reduces_class_count() {
        let reg = InMemoryRegistry::new();
        let v1 = reg
            .register(
                "acme",
                "billing",
                {
                    let mut c = sample_content();
                    c.classes.push(sample_class("c"));
                    c
                },
                sample_meta(),
            )
            .await
            .unwrap();
        let v2 = reg
            .evolve(
                &v1.key,
                EvolveOp::RemoveClass {
                    class_name: "a".into(),
                },
                sample_meta(),
            )
            .await
            .unwrap();
        assert!(!v2.content.classes.iter().any(|c| c.name == "a"));
    }

    #[tokio::test]
    async fn evolve_update_thresholds_only_changes_thresholds() {
        let reg = InMemoryRegistry::new();
        let v1 = reg
            .register("acme", "billing", sample_content(), sample_meta())
            .await
            .unwrap();
        let v2 = reg
            .evolve(
                &v1.key,
                EvolveOp::UpdateThresholds {
                    thresholds: ThresholdPolicy {
                        confidence_high: 0.9,
                        confidence_medium: 0.7,
                    },
                },
                sample_meta(),
            )
            .await
            .unwrap();
        assert_eq!(v2.content.thresholds.confidence_high, 0.9);
        assert_eq!(v2.content.thresholds.confidence_medium, 0.7);
        assert_eq!(v2.content.classes, v1.content.classes);
    }

    #[tokio::test]
    async fn evolve_recompile_swaps_embedder() {
        let reg = InMemoryRegistry::new();
        let v1 = reg
            .register("acme", "billing", sample_content(), sample_meta())
            .await
            .unwrap();
        let new_embedder = EmbedderSpec {
            family: "st".into(),
            model: "mpnet".into(),
            dim: 768,
            normalize: "l2".into(),
        };
        let v2 = reg
            .evolve(
                &v1.key,
                EvolveOp::Recompile {
                    new_embedder: new_embedder.clone(),
                },
                sample_meta(),
            )
            .await
            .unwrap();
        assert_eq!(v2.content.embedder, new_embedder);
    }

    #[tokio::test]
    async fn list_versions_returns_ascending() {
        let reg = InMemoryRegistry::new();
        for _ in 0..3 {
            reg.register("acme", "billing", sample_content(), sample_meta())
                .await
                .unwrap();
        }
        let keys = reg.list_versions("acme", "billing").await.unwrap();
        assert_eq!(keys.len(), 3);
        assert_eq!(keys[0].version, 1);
        assert_eq!(keys[1].version, 2);
        assert_eq!(keys[2].version, 3);
    }

    #[tokio::test]
    async fn list_catalogs_filters_by_tenant() {
        let reg = InMemoryRegistry::new();
        reg.register("acme", "billing", sample_content(), sample_meta())
            .await
            .unwrap();
        reg.register("other", "billing", sample_content(), sample_meta())
            .await
            .unwrap();
        let acme_page = reg.list_catalogs("acme", 100, None).await.unwrap();
        assert_eq!(acme_page.catalogs.len(), 1);
        assert_eq!(acme_page.catalogs[0].tenant, "acme");
        let other_page = reg.list_catalogs("other", 100, None).await.unwrap();
        assert_eq!(other_page.catalogs.len(), 1);
        assert_eq!(other_page.catalogs[0].tenant, "other");
    }

    #[tokio::test]
    async fn get_returns_compiled_only_when_requested() {
        let reg = InMemoryRegistry::new();
        let v1 = reg
            .register("acme", "billing", sample_content(), sample_meta())
            .await
            .unwrap();
        // The in-memory impl never populates compiled in v0.1.12,
        // so this just exercises the include_compiled flag flow.
        let r1 = reg.get(&v1.key, true).await.unwrap();
        assert!(r1.compiled.is_none()); // none was set
        let r2 = reg.get(&v1.key, false).await.unwrap();
        assert!(r2.compiled.is_none());
    }

    #[tokio::test]
    async fn delete_refuses_referenced_without_force() {
        let reg = InMemoryRegistry::new();
        let v1 = reg
            .register("acme", "billing", sample_content(), sample_meta())
            .await
            .unwrap();
        let _v2 = reg
            .evolve(
                &v1.key,
                EvolveOp::AppendClass {
                    class: sample_class("c"),
                },
                sample_meta(),
            )
            .await
            .unwrap();
        // v1 is referenced by v2 ; soft delete must fail.
        let err = reg.delete(&v1.key, false, "alice").await.unwrap_err();
        assert!(matches!(err, CatalogError::Conflict(_)));
        // force=true cascades.
        reg.delete(&v1.key, true, "alice").await.unwrap();
        let keys = reg.list_versions("acme", "billing").await;
        // After cascade, both versions should be gone.
        assert!(matches!(keys, Err(CatalogError::NotFound(_))));
    }

    #[tokio::test]
    async fn delete_cascade_handles_forks_correctly() {
        // Build a forked chain :
        //   v1 ─┬─ v2 (parent=v1)
        //       │   └─ v3 (parent=v2)   ← descends from v2
        //       └─ v4 (parent=v1)       ← sibling of v2, NOT descendant
        //
        // Deleting v2 with force=true MUST drop v2 and v3, but
        // leave v1 and v4 intact.
        let reg = InMemoryRegistry::new();
        let v1 = reg
            .register("acme", "billing", sample_content(), sample_meta())
            .await
            .unwrap();
        let v2 = reg
            .evolve(
                &v1.key,
                EvolveOp::AppendClass {
                    class: sample_class("c"),
                },
                sample_meta(),
            )
            .await
            .unwrap();
        let _v3 = reg
            .evolve(
                &v2.key,
                EvolveOp::AppendClass {
                    class: sample_class("d"),
                },
                sample_meta(),
            )
            .await
            .unwrap();
        let _v4 = reg
            .evolve(
                &v1.key, // FORK : parent is v1, not v2 ; sibling of v2.
                EvolveOp::AppendClass {
                    class: sample_class("e"),
                },
                sample_meta(),
            )
            .await
            .unwrap();

        // Cascade-delete v2 — must drop v2 + v3 only.
        reg.delete(&v2.key, true, "alice").await.unwrap();

        let surviving = reg.list_versions("acme", "billing").await.unwrap();
        // v1 and v4 must survive ; v2 and v3 must be gone.
        let versions: Vec<u32> = surviving.iter().map(|k| k.version).collect();
        assert!(versions.contains(&1), "v1 must survive");
        assert!(
            versions.contains(&4),
            "v4 must survive (sibling, not descendant)"
        );
        assert!(!versions.contains(&2), "v2 must be deleted");
        assert!(
            !versions.contains(&3),
            "v3 must be deleted (descendant of v2)"
        );
    }

    #[tokio::test]
    async fn delete_emits_audit_with_principal() {
        use crate::catalog::audit::test_sink::MemoryAuditSink;

        let sink = MemoryAuditSink::new();
        let emitter = AuditEmitter::with_sink(Arc::new(sink.clone()));
        let reg = InMemoryRegistry::with_policy_and_audit(CatalogPolicy::default(), emitter);
        let v1 = reg
            .register("acme", "billing", sample_content(), sample_meta())
            .await
            .unwrap();
        reg.delete(&v1.key, false, "alice@acme.com").await.unwrap();

        let events = sink.events();
        let delete_event = events
            .iter()
            .find(|e| e.action == crate::catalog::audit::AuditAction::Delete)
            .expect("must emit a delete event");
        assert_eq!(delete_event.created_by, "alice@acme.com");
    }

    #[tokio::test]
    async fn quota_check_caps_tenant_names() {
        let policy = CatalogPolicy {
            max_names_per_tenant: 2,
            ..Default::default()
        };
        let reg = InMemoryRegistry::with_policy_and_audit(policy, AuditEmitter::new());
        reg.register("acme", "a", sample_content(), sample_meta())
            .await
            .unwrap();
        reg.register("acme", "b", sample_content(), sample_meta())
            .await
            .unwrap();
        let err = reg
            .register("acme", "c", sample_content(), sample_meta())
            .await
            .unwrap_err();
        assert!(matches!(err, CatalogError::QuotaExceeded(_)));
    }
}
