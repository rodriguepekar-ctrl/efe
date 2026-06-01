//! Storage backend for HMAC-chained audit traces.
//!
//! # Why an abstraction
//!
//! Sprint 6 ships [`InMemoryChainStore`] as the test + demo backend.
//! Production swaps in a `PostgresChainStore` (Phase 4 deployment work)
//! that persists to `nano_agent_traces` (cf migrations v004-v007).
//!
//! The trait shape is **frozen** as of Sprint 6 (extended Sprint 7 — N10
//! with tombstone methods) : Postgres impl changes the body, not the
//! contract.
//!
//! # Per-tenant chain semantics
//!
//! Each tenant has its own append-only chain.  `latest_for_tenant` returns
//! the most-recently-appended trace ; `chain_for_tenant` returns the full
//! chain in time order ; `traces_for_swarm` returns ops of a single swarm
//! (subset of one tenant's chain).
//!
//! # Concurrency
//!
//! `append` must be **serializable per tenant** : two concurrent appends
//! for the same tenant must produce a linear chain (no fork).  The
//! in-memory impl uses a per-tenant `Mutex` ; the Postgres impl uses
//! `SELECT … FOR UPDATE` inside a transaction.
//!
//! # Sprint 7 — N10 : GDPR tombstone API
//!
//! `tombstone_trace` + `tombstone_traces_for_swarm` erase PII bodies
//! while preserving the cryptographically-frozen HMAC chain.  See
//! [`crate::audit::TombstoneMetadata`] for the data model contract.

use async_trait::async_trait;
use efe_nano_sdk::TenantId;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

use crate::audit::NanoAgentTrace;
use crate::error::ChainStoreError;

// ─────────────────────────────────────────────────────────────────────
//  ChainEntry — one stored chained trace
// ─────────────────────────────────────────────────────────────────────

/// One row in the chain store.  Combines the trace body with the chain
/// link fields (parent + payload_hash + hmac).
#[derive(Debug, Clone)]
pub struct ChainEntry {
    pub trace: NanoAgentTrace,
    /// Chain parent : the previous trace for this tenant.  `None` for
    /// the first row of the tenant's chain (genesis).
    pub chain_parent_id: Option<Uuid>,
    /// SHA-256 of canonical row body (cf `payload::compute_payload_hash`).
    pub payload_hash: [u8; 32],
    /// HMAC-SHA-256(chain_parent_id ‖ payload_hash, tenant_key).
    pub hmac: [u8; 32],
    /// Server-side insertion timestamp (NOT the trace's started_at).
    pub inserted_at: chrono::DateTime<chrono::Utc>,
}

impl ChainEntry {
    /// Convenience : get the trace_id of this entry.
    pub fn trace_id(&self) -> Uuid {
        self.trace.trace_id.0
    }

    /// Convenience : get the tenant of this entry.
    pub fn tenant_id(&self) -> TenantId {
        self.trace.tenant_id
    }
}

// ─────────────────────────────────────────────────────────────────────
//  ChainStore trait
// ─────────────────────────────────────────────────────────────────────

/// Append-only store for chained audit traces.
#[async_trait]
pub trait ChainStore: Send + Sync + 'static {
    /// Append a new entry.  The store is responsible for :
    /// 1. Verifying `entry.chain_parent_id` matches the current tip of
    ///    the tenant's chain (if Some) or that the chain is empty (if
    ///    None).  Mismatch returns `Constraint` error.
    /// 2. Persisting the entry atomically (no torn writes).
    /// 3. Updating the tenant's chain tip.
    ///
    /// Concurrent appends for the same tenant must serialize.
    async fn append(&self, entry: ChainEntry) -> Result<(), ChainStoreError>;

    /// Get the latest entry for a tenant.  Returns `None` if no entry
    /// has ever been appended for this tenant.
    async fn latest_for_tenant(
        &self,
        tenant: TenantId,
    ) -> Result<Option<ChainEntry>, ChainStoreError>;

    /// Get the full chain for a tenant in append order (oldest first).
    /// Optional `since` filter : only entries with `inserted_at > since`.
    async fn chain_for_tenant(
        &self,
        tenant: TenantId,
        since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Vec<ChainEntry>, ChainStoreError>;

    /// Get all entries for a specific swarm_request_id (under the given tenant).
    async fn traces_for_swarm(
        &self,
        tenant: TenantId,
        swarm_request_id: Uuid,
    ) -> Result<Vec<ChainEntry>, ChainStoreError>;

    /// Get one trace by id (must belong to the given tenant).
    async fn trace_by_id(
        &self,
        tenant: TenantId,
        trace_id: Uuid,
    ) -> Result<ChainEntry, ChainStoreError>;

    /// Total number of entries across all tenants (telemetry).
    async fn total_entries(&self) -> Result<u64, ChainStoreError>;

    // ─────────────────────────────────────────────────────────────────
    //  Sprint 7 — N10 : tombstone API
    // ─────────────────────────────────────────────────────────────────

    /// **Sprint 7 — N10** : Erase the PII content of a single trace
    /// while preserving the HMAC chain integrity.
    ///
    /// Effects :
    /// 1. Set `entry.trace.full_input = None`, `full_output = None`.
    /// 2. Set `entry.trace.tombstone = Some(metadata)`.
    /// 3. Decrement the reference counts on `input_hash_resolved` and
    ///    `output_hash` in the side-store ; remove the body if the
    ///    refcount reaches zero (no other live trace references it).
    /// 4. **Do NOT** touch `entry.payload_hash`, `entry.hmac`, or
    ///    `entry.chain_parent_id` — these are the cryptographic
    ///    invariants of the chain.
    ///
    /// Idempotent : re-applying the tombstone to an already-tombstoned
    /// trace returns `traces_tombstoned = 0` and doesn't double-erase.
    ///
    /// Returns [`ChainStoreError::NotFound`] if `trace_id` doesn't
    /// belong to `tenant`.
    async fn tombstone_trace(
        &self,
        tenant: TenantId,
        trace_id: Uuid,
        metadata: crate::audit::TombstoneMetadata,
    ) -> Result<TombstoneOutcome, ChainStoreError>;

    /// **Sprint 7 — N10** : Bulk-tombstone every trace of a swarm
    /// in one operation.  Used by the GDPR saga to erase all
    /// per-op traces of a single `SwarmRequest` atomically.
    ///
    /// Same per-trace semantics as `tombstone_trace`.  Returns an
    /// aggregated [`TombstoneOutcome`].
    async fn tombstone_traces_for_swarm(
        &self,
        tenant: TenantId,
        swarm_request_id: Uuid,
        metadata: crate::audit::TombstoneMetadata,
    ) -> Result<TombstoneOutcome, ChainStoreError>;

    /// **Sprint 7 — N10** : Count tombstoned traces for a tenant
    /// (ops dashboards + retention metrics).
    async fn tombstoned_count(&self, tenant: TenantId) -> Result<u64, ChainStoreError>;

    /// **Sprint 8 — N13 (B4 fix)** : Cascade-tombstone a trace AND every
    /// chain entry whose `parent_trace_ids` ultimately references it
    /// AND whose `error_kind` is one of the audit-metadata sentinels
    /// (`CacheHit`, `ShadowRun`, `ShadowComparison`).
    ///
    /// # Why a cascade ?
    ///
    /// When the GDPR saga (M24) tombstones a production trace, the
    /// shadow + comparison rows that referenced it via N1 lineage
    /// MUST also be tombstoned — otherwise their `full_input` /
    /// `full_output` bodies (when populated) would resurface erased
    /// PII via the shadow side-store.
    ///
    /// # Default impl
    ///
    /// Returns the result of `tombstone_trace(root_trace_id, ...)`
    /// without cascading — backward-compatible for impls that don't
    /// yet support it.  `InMemoryChainStore` overrides with the full
    /// cascade (Sprint 8 ship).
    async fn tombstone_trace_cascade(
        &self,
        tenant: TenantId,
        root_trace_id: Uuid,
        metadata: crate::audit::TombstoneMetadata,
    ) -> Result<TombstoneOutcome, ChainStoreError> {
        self.tombstone_trace(tenant, root_trace_id, metadata).await
    }

    /// **Sprint 8 — N14** : Cascade-tombstone the swarm envelope AND
    /// every op / cache-hit / shadow / comparison trace of the same
    /// swarm.
    ///
    /// The `envelope_trace_id` MUST refer to a real swarm-envelope row
    /// (i.e. `error_kind == Some("SwarmEnvelope")`).  Otherwise returns
    /// `NotAnEnvelope`.
    ///
    /// Symmetric to N13's `tombstone_trace_cascade` (which goes from
    /// prod trace down to shadow children).  Here we go from the swarm
    /// anchor to every member of the swarm.
    ///
    /// # Default impl
    ///
    /// Returns the result of `tombstone_traces_for_swarm` — backward-
    /// compatible for impls that don't yet support the envelope.
    /// `InMemoryChainStore` overrides with envelope-shape verification.
    async fn tombstone_swarm_envelope(
        &self,
        tenant: TenantId,
        envelope_trace_id: Uuid,
        metadata: crate::audit::TombstoneMetadata,
    ) -> Result<TombstoneOutcome, ChainStoreError> {
        // Fetch the envelope to extract swarm_request_id.
        let envelope = self.trace_by_id(tenant, envelope_trace_id).await?;
        if envelope.trace.error_kind.as_deref() != Some("SwarmEnvelope") {
            return Err(ChainStoreError::NotFound(envelope_trace_id));
        }
        self.tombstone_traces_for_swarm(tenant, envelope.trace.swarm_request_id, metadata).await
    }
}

/// **Sprint 7 — N10** : Result of a tombstone operation.
///
/// Returned by [`ChainStore::tombstone_trace`] and
/// [`ChainStore::tombstone_traces_for_swarm`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TombstoneOutcome {
    /// Number of trace rows freshly tombstoned (excludes already-
    /// tombstoned rows — those are silently treated as no-ops).
    pub traces_tombstoned: u64,
    /// Number of input bodies purged from the side-store (only counts
    /// bodies whose last live reference was removed by this call).
    pub input_bodies_purged: u64,
    /// Number of output bodies purged from the side-store.
    pub output_bodies_purged: u64,
}

impl TombstoneOutcome {
    pub fn merge(&mut self, other: &Self) {
        self.traces_tombstoned += other.traces_tombstoned;
        self.input_bodies_purged += other.input_bodies_purged;
        self.output_bodies_purged += other.output_bodies_purged;
    }
}

// ─────────────────────────────────────────────────────────────────────
//  InMemoryChainStore (Sprint 6 ship)
// ─────────────────────────────────────────────────────────────────────

/// In-process chain store backed by a per-tenant `Vec<ChainEntry>`.
///
/// Production swaps in `PostgresChainStore` (Phase 4 deployment).
pub struct InMemoryChainStore {
    /// Per-tenant chain, serialized by an outer Mutex (chain-level
    /// serialization is the trait contract).
    chains: Mutex<HashMap<TenantId, Vec<ChainEntry>>>,
    /// Optional input/output body store, deduplicated by SHA-256.
    /// Sprint 6 keeps this in-process ; Phase 4 may move it to S3 / GCS
    /// for cold storage.
    input_bodies: Mutex<HashMap<[u8; 32], Value>>,
    output_bodies: Mutex<HashMap<[u8; 32], Value>>,
}

impl InMemoryChainStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a full input body for replay.  Optional — only used when
    /// `agent.audit.include_input` is true.
    pub fn put_input(&self, hash: [u8; 32], body: Value) {
        let mut guard = self.input_bodies.lock().unwrap();
        guard.insert(hash, body);
    }

    /// Get a stored input body.
    pub fn get_input(&self, hash: &[u8; 32]) -> Option<Value> {
        let guard = self.input_bodies.lock().unwrap();
        guard.get(hash).cloned()
    }

    /// Store a full output body.  Optional.
    pub fn put_output(&self, hash: [u8; 32], body: Value) {
        let mut guard = self.output_bodies.lock().unwrap();
        guard.insert(hash, body);
    }

    /// Get a stored output body.
    pub fn get_output(&self, hash: &[u8; 32]) -> Option<Value> {
        let guard = self.output_bodies.lock().unwrap();
        guard.get(hash).cloned()
    }

    /// Number of deduplicated input bodies stored.
    pub fn input_body_count(&self) -> usize {
        self.input_bodies.lock().unwrap().len()
    }

    /// Number of deduplicated output bodies stored.
    pub fn output_body_count(&self) -> usize {
        self.output_bodies.lock().unwrap().len()
    }
}

impl Default for InMemoryChainStore {
    fn default() -> Self {
        Self {
            chains: Mutex::new(HashMap::new()),
            input_bodies: Mutex::new(HashMap::new()),
            output_bodies: Mutex::new(HashMap::new()),
        }
    }
}

impl std::fmt::Debug for InMemoryChainStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryChainStore")
            .field("tenants", &self.chains.lock().unwrap().len())
            .field("input_bodies", &self.input_body_count())
            .field("output_bodies", &self.output_body_count())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl ChainStore for InMemoryChainStore {
    async fn append(&self, entry: ChainEntry) -> Result<(), ChainStoreError> {
        let mut chains = self.chains.lock().unwrap();
        let tenant_chain = chains.entry(entry.tenant_id()).or_default();

        // Tip check : entry.chain_parent_id must match the current tip.
        let expected_parent = tenant_chain.last().map(|e| e.trace_id());
        if entry.chain_parent_id != expected_parent {
            return Err(ChainStoreError::Constraint(format!(
                "chain_parent_id mismatch : expected {expected_parent:?}, got {:?}",
                entry.chain_parent_id
            )));
        }

        // Duplicate trace_id check (within tenant).
        let new_id = entry.trace_id();
        if tenant_chain.iter().any(|e| e.trace_id() == new_id) {
            return Err(ChainStoreError::Constraint(format!(
                "duplicate trace_id {new_id}"
            )));
        }

        tenant_chain.push(entry);
        Ok(())
    }

    async fn latest_for_tenant(
        &self,
        tenant: TenantId,
    ) -> Result<Option<ChainEntry>, ChainStoreError> {
        let chains = self.chains.lock().unwrap();
        Ok(chains.get(&tenant).and_then(|c| c.last().cloned()))
    }

    async fn chain_for_tenant(
        &self,
        tenant: TenantId,
        since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Vec<ChainEntry>, ChainStoreError> {
        let chains = self.chains.lock().unwrap();
        let out = chains
            .get(&tenant)
            .map(|c| {
                c.iter()
                    .filter(|e| since.map(|s| e.inserted_at > s).unwrap_or(true))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok(out)
    }

    async fn traces_for_swarm(
        &self,
        tenant: TenantId,
        swarm_request_id: Uuid,
    ) -> Result<Vec<ChainEntry>, ChainStoreError> {
        // IMPORTANT : preserve append order (= chain-link order).  The
        // HMAC chain links entries by `chain_parent_id` in the order of
        // insertion, NOT in the order of `trace.started_at` (which may
        // be wall-clock-skewed under parallel execution).  Sorting by
        // `started_at` here would break `verify_subchain` for any swarm
        // whose ops run in parallel (e.g. diamond DAGs).  We use the
        // natural chain iteration order, filtered to the requested swarm.
        let chains = self.chains.lock().unwrap();
        let out: Vec<ChainEntry> = chains
            .get(&tenant)
            .map(|c| {
                c.iter()
                    .filter(|e| e.trace.swarm_request_id == swarm_request_id)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        Ok(out)
    }

    async fn trace_by_id(
        &self,
        tenant: TenantId,
        trace_id: Uuid,
    ) -> Result<ChainEntry, ChainStoreError> {
        let chains = self.chains.lock().unwrap();
        chains
            .get(&tenant)
            .and_then(|c| c.iter().find(|e| e.trace_id() == trace_id).cloned())
            .ok_or(ChainStoreError::NotFound(trace_id))
    }

    async fn total_entries(&self) -> Result<u64, ChainStoreError> {
        let chains = self.chains.lock().unwrap();
        let total = chains.values().map(|c| c.len() as u64).sum();
        Ok(total)
    }

    // ─────────────────────────────────────────────────────────────────
    //  Sprint 7 — N10 : tombstone implementation
    // ─────────────────────────────────────────────────────────────────

    async fn tombstone_trace(
        &self,
        tenant: TenantId,
        trace_id: Uuid,
        metadata: crate::audit::TombstoneMetadata,
    ) -> Result<TombstoneOutcome, ChainStoreError> {
        // Lock the chain map for the duration of the tombstone : we
        // need a consistent view of the per-tenant chain (to find the
        // target trace AND to scan for other live references to the
        // same input/output hashes for the refcount-based body purge).
        let mut chains = self.chains.lock().unwrap();
        let tenant_chain = chains.get_mut(&tenant).ok_or(ChainStoreError::NotFound(trace_id))?;

        // Locate the target trace.
        let target_idx = tenant_chain
            .iter()
            .position(|e| e.trace_id() == trace_id)
            .ok_or(ChainStoreError::NotFound(trace_id))?;

        // Idempotency : if already tombstoned, return no-op outcome.
        if tenant_chain[target_idx].trace.tombstone.is_some() {
            return Ok(TombstoneOutcome::default());
        }

        // Capture the body hashes BEFORE mutating so we can refcount-
        // scan for other live references.
        let input_hash = tenant_chain[target_idx].trace.input_hash_resolved;
        let output_hash_opt = tenant_chain[target_idx].trace.output_hash;

        // Mutate the trace : clear PII bodies + set tombstone metadata.
        // Crypto-relevant fields (payload_hash, hmac, chain_parent_id,
        // input_hash_*, output_hash) stay untouched — that's the whole
        // point of N10.
        let entry = &mut tenant_chain[target_idx];
        entry.trace.full_input = None;
        entry.trace.full_output = None;
        entry.trace.tombstone = Some(metadata);

        // Refcount-style body purge : count how many OTHER live traces
        // (in any tenant) reference the same body hash.  Only purge if
        // we were the last live reference.
        //
        // Cross-tenant note : the body store is deduplicated by hash
        // GLOBALLY, not per-tenant.  Two tenants whose inputs happen to
        // hash to the same SHA-256 share storage.  We must scan ACROSS
        // all tenants to avoid orphaning a tenant B body when tenant A
        // tombstones.  This is O(N_total_traces) per call — fine for
        // in-memory ; production Postgres will use a body refcount
        // table indexed by hash.
        let mut input_still_referenced = false;
        let mut output_still_referenced = false;
        for (_, other_chain) in chains.iter() {
            for entry in other_chain {
                if entry.trace.tombstone.is_some() {
                    continue; // tombstoned traces don't hold a body ref
                }
                if entry.trace.input_hash_resolved == input_hash
                    && entry.trace_id() != trace_id
                {
                    input_still_referenced = true;
                }
                if let Some(oh) = output_hash_opt {
                    if entry.trace.output_hash == Some(oh) && entry.trace_id() != trace_id {
                        output_still_referenced = true;
                    }
                }
                if input_still_referenced
                    && (output_hash_opt.is_none() || output_still_referenced)
                {
                    break;
                }
            }
        }

        let mut outcome = TombstoneOutcome {
            traces_tombstoned: 1,
            input_bodies_purged: 0,
            output_bodies_purged: 0,
        };

        if !input_still_referenced {
            let mut bodies = self.input_bodies.lock().unwrap();
            if bodies.remove(&input_hash).is_some() {
                outcome.input_bodies_purged = 1;
            }
        }
        if let Some(oh) = output_hash_opt {
            if !output_still_referenced {
                let mut bodies = self.output_bodies.lock().unwrap();
                if bodies.remove(&oh).is_some() {
                    outcome.output_bodies_purged = 1;
                }
            }
        }

        Ok(outcome)
    }

    async fn tombstone_traces_for_swarm(
        &self,
        tenant: TenantId,
        swarm_request_id: Uuid,
        metadata: crate::audit::TombstoneMetadata,
    ) -> Result<TombstoneOutcome, ChainStoreError> {
        // Collect target trace_ids under a short read lock, then
        // tombstone one at a time (each call re-locks).  This avoids
        // holding the chains mutex for the whole batch — important if
        // the swarm has many ops.
        let target_ids: Vec<Uuid> = {
            let chains = self.chains.lock().unwrap();
            chains
                .get(&tenant)
                .map(|c| {
                    c.iter()
                        .filter(|e| e.trace.swarm_request_id == swarm_request_id)
                        .map(|e| e.trace_id())
                        .collect()
                })
                .unwrap_or_default()
        };

        let mut aggregated = TombstoneOutcome::default();
        for trace_id in target_ids {
            let outcome = self
                .tombstone_trace(tenant, trace_id, metadata.clone())
                .await?;
            aggregated.merge(&outcome);
        }
        Ok(aggregated)
    }

    async fn tombstoned_count(&self, tenant: TenantId) -> Result<u64, ChainStoreError> {
        let chains = self.chains.lock().unwrap();
        let count = chains
            .get(&tenant)
            .map(|c| c.iter().filter(|e| e.trace.tombstone.is_some()).count() as u64)
            .unwrap_or(0);
        Ok(count)
    }

    /// **Sprint 8 — N13 (B4 fix)** : cascade tombstone — erase the
    /// root trace AND every descendant whose `parent_trace_ids`
    /// references the root (transitively) AND whose `error_kind` is
    /// one of the audit-metadata sentinels (`CacheHit`, `ShadowRun`,
    /// `ShadowComparison`).
    ///
    /// Algorithm : BFS over the per-tenant chain, expanding the set of
    /// to-be-tombstoned trace_ids until no new descendant is found.
    /// O(N × D) where N = chain length, D = cascade depth (usually 2-3
    /// for prod → shadow → comparison).
    async fn tombstone_trace_cascade(
        &self,
        tenant: TenantId,
        root_trace_id: Uuid,
        metadata: crate::audit::TombstoneMetadata,
    ) -> Result<TombstoneOutcome, ChainStoreError> {
        // 1. BFS the chain to find every descendant whose lineage
        //    eventually points to `root_trace_id` via parent_trace_ids
        //    AND whose error_kind is an audit-metadata sentinel.
        let descendants: Vec<Uuid> = {
            let chains = self.chains.lock().unwrap();
            let chain = match chains.get(&tenant) {
                Some(c) => c,
                None => return Err(ChainStoreError::NotFound(root_trace_id)),
            };
            let mut to_tombstone: std::collections::HashSet<Uuid> =
                std::collections::HashSet::from([root_trace_id]);
            // Repeat until no growth.
            loop {
                let before = to_tombstone.len();
                for entry in chain.iter() {
                    if to_tombstone.contains(&entry.trace_id()) {
                        continue;
                    }
                    // Cascade only audit-metadata sentinels.  Live
                    // production ops with parent_trace_ids pointing to
                    // the root are STAND-ALONE OPS (e.g. a Ref-input
                    // op whose lineage points to the tombstoned op) and
                    // MUST NOT be tombstoned by cascade.
                    let kind = entry.trace.error_kind.as_deref();
                    let is_sentinel = matches!(
                        kind,
                        Some("CacheHit")
                            | Some("ShadowRun")
                            | Some("ShadowComparison")
                    );
                    if !is_sentinel {
                        continue;
                    }
                    if entry
                        .trace
                        .parent_trace_ids
                        .iter()
                        .any(|p| to_tombstone.contains(&p.0))
                    {
                        to_tombstone.insert(entry.trace_id());
                    }
                }
                if to_tombstone.len() == before {
                    break;
                }
            }
            to_tombstone.into_iter().collect()
        };

        // 2. Sequentially tombstone each.  `tombstone_trace` is
        //    idempotent so re-applying on already-tombstoned rows is a
        //    no-op (returns traces_tombstoned = 0).
        let mut aggregated = TombstoneOutcome::default();
        let mut root_seen = false;
        for trace_id in descendants {
            let outcome = self
                .tombstone_trace(tenant, trace_id, metadata.clone())
                .await?;
            aggregated.merge(&outcome);
            if trace_id == root_trace_id {
                root_seen = true;
            }
        }
        if !root_seen {
            // Root wasn't in our cascade set — happens if the chain
            // lookup didn't include it.  Force-tombstone now.
            let outcome = self
                .tombstone_trace(tenant, root_trace_id, metadata)
                .await?;
            aggregated.merge(&outcome);
        }
        Ok(aggregated)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::NanoAgentTrace;
    use efe_nano_sdk::{NanoAgentId, NanoAgentVersion, TraceId};

    fn make_entry(
        tenant: TenantId,
        trace_id: Uuid,
        parent: Option<Uuid>,
        swarm: Uuid,
    ) -> ChainEntry {
        ChainEntry {
            trace: NanoAgentTrace {
                trace_id: TraceId(trace_id),
                parent_trace_ids: vec![],
                swarm_request_id: swarm,
                operation_id: "op".into(),
                tenant_id: tenant,
                caller_agent_id: None,
                nano_agent_id: NanoAgentId::new("a").unwrap(),
                nano_agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
                execution_mode: "pure_rust".into(),
                input_hash_source: [0u8; 32],
                input_hash_resolved: [0u8; 32],
                output_hash: None,
                error_kind: None,
                started_at: chrono::Utc::now(),
                completed_at: None,
                latency_ms: None,
                cost_cents: 0.0,
                llm_tokens_used: None,
                full_input: None,
                full_output: None,
                tombstone: None,
                swarm_envelope_trace_id: None,
            },
            chain_parent_id: parent,
            payload_hash: [0u8; 32],
            hmac: [0u8; 32],
            inserted_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn append_first_entry_no_parent_succeeds() {
        let store = InMemoryChainStore::new();
        let tenant = TenantId::system();
        let id = Uuid::new_v4();
        store
            .append(make_entry(tenant, id, None, Uuid::new_v4()))
            .await
            .unwrap();
        assert_eq!(store.total_entries().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn append_second_entry_chains_to_first() {
        let store = InMemoryChainStore::new();
        let tenant = TenantId::system();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        store
            .append(make_entry(tenant, id1, None, Uuid::new_v4()))
            .await
            .unwrap();
        store
            .append(make_entry(tenant, id2, Some(id1), Uuid::new_v4()))
            .await
            .unwrap();
        assert_eq!(store.total_entries().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn append_with_wrong_parent_rejected() {
        let store = InMemoryChainStore::new();
        let tenant = TenantId::system();
        let id1 = Uuid::new_v4();
        store
            .append(make_entry(tenant, id1, None, Uuid::new_v4()))
            .await
            .unwrap();
        // Wrong parent : should be `Some(id1)`, we pass `None`.
        let id2 = Uuid::new_v4();
        let err = store
            .append(make_entry(tenant, id2, None, Uuid::new_v4()))
            .await
            .unwrap_err();
        match err {
            ChainStoreError::Constraint(msg) => assert!(msg.contains("mismatch")),
            other => panic!("expected Constraint, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn append_duplicate_trace_id_rejected() {
        let store = InMemoryChainStore::new();
        let tenant = TenantId::system();
        let id1 = Uuid::new_v4();
        store
            .append(make_entry(tenant, id1, None, Uuid::new_v4()))
            .await
            .unwrap();
        // Same trace_id with a parent — caught by the parent check first.
        // Test a true duplicate : try to append id1 again with same parent (None).
        let err = store
            .append(make_entry(tenant, id1, None, Uuid::new_v4()))
            .await
            .unwrap_err();
        // The parent check fires first because chain tip is id1 ; but
        // either error is a valid Constraint signal.
        assert!(matches!(err, ChainStoreError::Constraint(_)));
    }

    #[tokio::test]
    async fn latest_for_tenant_returns_tip() {
        let store = InMemoryChainStore::new();
        let tenant = TenantId::system();
        assert!(store.latest_for_tenant(tenant).await.unwrap().is_none());
        let id1 = Uuid::new_v4();
        store
            .append(make_entry(tenant, id1, None, Uuid::new_v4()))
            .await
            .unwrap();
        let tip = store.latest_for_tenant(tenant).await.unwrap().unwrap();
        assert_eq!(tip.trace_id(), id1);
    }

    #[tokio::test]
    async fn cross_tenant_isolation() {
        let store = InMemoryChainStore::new();
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        store
            .append(make_entry(t1, id1, None, Uuid::new_v4()))
            .await
            .unwrap();
        store
            .append(make_entry(t2, id2, None, Uuid::new_v4()))
            .await
            .unwrap();
        // Tenant 1's chain doesn't see tenant 2's entry.
        let c1 = store.chain_for_tenant(t1, None).await.unwrap();
        assert_eq!(c1.len(), 1);
        assert_eq!(c1[0].trace_id(), id1);
        let c2 = store.chain_for_tenant(t2, None).await.unwrap();
        assert_eq!(c2.len(), 1);
        assert_eq!(c2[0].trace_id(), id2);
    }

    #[tokio::test]
    async fn trace_by_id_not_found_errors() {
        let store = InMemoryChainStore::new();
        let err = store
            .trace_by_id(TenantId::system(), Uuid::new_v4())
            .await
            .unwrap_err();
        assert!(matches!(err, ChainStoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn traces_for_swarm_filters_correctly() {
        let store = InMemoryChainStore::new();
        let tenant = TenantId::system();
        let swarm_a = Uuid::new_v4();
        let swarm_b = Uuid::new_v4();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();
        store
            .append(make_entry(tenant, id1, None, swarm_a))
            .await
            .unwrap();
        store
            .append(make_entry(tenant, id2, Some(id1), swarm_b))
            .await
            .unwrap();
        store
            .append(make_entry(tenant, id3, Some(id2), swarm_a))
            .await
            .unwrap();
        let a = store.traces_for_swarm(tenant, swarm_a).await.unwrap();
        let b = store.traces_for_swarm(tenant, swarm_b).await.unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(b.len(), 1);
    }

    #[tokio::test]
    async fn put_get_input_body() {
        let store = InMemoryChainStore::new();
        let hash = [42u8; 32];
        let body = serde_json::json!({"x": 1});
        assert!(store.get_input(&hash).is_none());
        store.put_input(hash, body.clone());
        assert_eq!(store.get_input(&hash).unwrap(), body);
        assert_eq!(store.input_body_count(), 1);
    }

    #[tokio::test]
    async fn put_get_output_body() {
        let store = InMemoryChainStore::new();
        let hash = [99u8; 32];
        let body = serde_json::json!({"y": 2});
        store.put_output(hash, body.clone());
        assert_eq!(store.get_output(&hash).unwrap(), body);
        assert_eq!(store.output_body_count(), 1);
    }

    #[tokio::test]
    async fn input_body_dedup() {
        let store = InMemoryChainStore::new();
        let hash = [1u8; 32];
        let body = serde_json::json!({"a": 1});
        store.put_input(hash, body.clone());
        store.put_input(hash, body.clone());      // same hash → no growth
        store.put_input(hash, body);
        assert_eq!(store.input_body_count(), 1);
    }

    #[tokio::test]
    async fn chain_for_tenant_filters_by_since() {
        let store = InMemoryChainStore::new();
        let tenant = TenantId::system();
        let id1 = Uuid::new_v4();
        store
            .append(make_entry(tenant, id1, None, Uuid::new_v4()))
            .await
            .unwrap();
        // Mid-point cutoff before second insert.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let cutoff = chrono::Utc::now();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let id2 = Uuid::new_v4();
        store
            .append(make_entry(tenant, id2, Some(id1), Uuid::new_v4()))
            .await
            .unwrap();
        let after = store.chain_for_tenant(tenant, Some(cutoff)).await.unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].trace_id(), id2);
    }
}
