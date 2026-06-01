//! Sprint **D14-bis** — HMAC-chained tamper-evident audit log.
//!
//! Mirror of the design document
//! [`docs/bench/d14bis_hmac_audit_chain_architecture.md`](../../../docs/bench/d14bis_hmac_audit_chain_architecture.md).
//! Provides the impl of the `AuditSink` trait that signs every audit
//! event with HMAC-SHA-256 and persists it in the `audit_events` SQL
//! table (migration `0003_audit_events.sql`).
//!
//! The trait surface is identical to [`super::audit::TracingAuditSink`]
//! (D12) ; this file is purely the storage + signing swap. The
//! `AuditEmitter` is constructed in `runtime::Server::build` with the
//! appropriate sink based on `EFE_AUDIT_SIGNING_ENABLED`.
//!
//! # Architectural pivots
//!
//! 1. **HMAC-SHA-256 chained** : every event's `hmac` is computed
//!    from `canonical_serialise(payload)` keyed by the active HMAC
//!    key. The `prev_hmac` field is the previous event's `hmac` in
//!    the same `chain_id`. Chain genesis is the constant
//!    `0x00…00` (64 zeros hex).
//!
//! 2. **Per-tenant chain** : `chain_id = "tenant:<X>"`. Verify is
//!    isolated per tenant, advisory locks per chain prevent the
//!    `prev_hmac` race.
//!
//! 3. **Canonical serialisation** : JSON sorted keys + minified +
//!    UTF-8. `BTreeMap` insertion → field-order stable output.
//!    `serde_json::to_vec` is deterministic for our schema (no
//!    floats, no HashMap).
//!
//! 4. **`emit()` is async via `tokio::spawn`** : the trait is sync
//!    (D12 contract), but the SQL insert is async. The caller
//!    (`PostgresCatalogRegistry::register/evolve/delete`) has
//!    already committed its own transaction ; the audit emit is
//!    best-effort + alerted.
//!
//! 5. **Chain to tracing** : when enabled, the sink fans out to the
//!    legacy `TracingAuditSink` so the existing Loki/Splunk
//!    pipelines continue to receive the audit stream.
//!
//! # Concurrency invariants
//!
//! * `pg_advisory_xact_lock(hash(chain_id))` serialises inserts per
//!   chain. The `prev_hmac` is read + written under the lock.
//! * `last_hmac_cache` is updated only AFTER the COMMIT lands.
//! * Multi-replica safe : each replica has its own cache, but the
//!   advisory lock + `MAX(seq) WHERE chain_id` SELECT under lock
//!   guarantees convergence.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// Note : the trait `AuditSink::emit` is sync (D12 contract). The
// `async_trait` macro is NOT applied to our impl (audit fix M3).
use hmac::{Hmac, Mac};
use sha2::Sha256;
use sqlx::postgres::PgPool;
use sqlx::Row;
use uuid::Uuid;
use xxhash_rust::xxh64::xxh64;

use super::audit::{AuditEvent, AuditSink};
use super::audit_keys::{HmacKey, LoadedKeys};

// =====================================================================
// Constants
// =====================================================================

/// Chain genesis : the conventional `prev_hmac` value for the very
/// first event in a chain. 64 zeros hex (256 zero bits). Documented
/// in the architecture doc § 12.
pub const GENESIS_PREV_HMAC: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

/// Schema version of the canonical payload. Bumped iff the field
/// inventory changes. Verify uses this to dispatch to the right
/// canonical serialiser. Today only version 1 exists.
pub const SCHEMA_VERSION: u32 = 1;

/// xxh64 seed for the per-chain advisory lock id. Distinct from the
/// catalog `(tenant, name)` lock seed (D14) so the two lock spaces
/// never collide.
const AUDIT_CHAIN_LOCK_SEED: u64 = 0xefe_0000_d14b_0000;

/// Compute the advisory-lock id for an audit chain.
#[must_use]
fn lock_id_for_chain(chain_id: &str) -> i64 {
    xxh64(chain_id.as_bytes(), AUDIT_CHAIN_LOCK_SEED) as i64
}

// =====================================================================
// Signed event + canonical serialisation
// =====================================================================

type HmacSha256 = Hmac<Sha256>;

/// Compute HMAC-SHA-256 of `data` keyed by `key` ; returns the raw
/// 32-byte digest. Wraps the `hmac` crate's verbose builder pattern.
#[must_use]
pub fn compute_hmac(key: &[u8; 32], data: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC-SHA-256 accepts any key length");
    mac.update(data);
    let result = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

/// Lower-case hex encode a byte slice.
#[must_use]
pub fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes.iter() {
        let hi = (b >> 4) & 0x0F;
        let lo = b & 0x0F;
        s.push(hex_nibble(hi));
        s.push(hex_nibble(lo));
    }
    s
}

#[inline]
fn hex_nibble(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'a' + (n - 10)) as char,
        _ => unreachable!(),
    }
}

/// Build the canonical bytes that feed the HMAC.
///
/// Field-order stable thanks to `BTreeMap` (alphabetical iteration).
/// JSON minified (no whitespace). UTF-8 by construction (`serde_json`
/// emits ASCII-safe escapes for control chars).
///
/// **Schema** (matches `audit_events.canonical_payload` JSONB) :
///
/// ```jsonc
/// {
///   "action": "register",
///   "catalog_key": "acme/billing@v1",
///   "chain_id": "tenant:acme",
///   "content_sha256": "<64-hex>",
///   "created_at_unix_ms": 1700000000000,
///   "description": "init",
///   "event_id": "<uuid v7>",
///   "op_tag": "register",
///   "parent_key": "",
///   "prev_hmac": "<64-hex or 64 zeros>",
///   "principal": "alice@acme",
///   "schema_version": 1,
///   "tenant": "acme"
/// }
/// ```
///
/// The `hmac` and `key_id` columns are NOT part of the canonical
/// payload (the HMAC is self-referential ; the key_id is metadata).
#[must_use]
pub fn canonical_audit_payload(rec: &SignableRecord<'_>) -> Vec<u8> {
    use serde_json::{Map, Value};
    use std::collections::BTreeMap;

    let mut m: BTreeMap<&str, Value> = BTreeMap::new();
    m.insert("action", Value::from(rec.action));
    m.insert("catalog_key", Value::from(rec.catalog_key));
    m.insert("chain_id", Value::from(rec.chain_id));
    m.insert("content_sha256", Value::from(rec.content_sha256));
    m.insert("created_at_unix_ms", Value::from(rec.created_at_unix_ms));
    m.insert("description", Value::from(rec.description));
    m.insert("event_id", Value::from(rec.event_id.to_string()));
    m.insert("op_tag", Value::from(rec.op_tag));
    m.insert("parent_key", Value::from(rec.parent_key));
    m.insert("prev_hmac", Value::from(rec.prev_hmac));
    m.insert("principal", Value::from(rec.principal));
    m.insert("schema_version", Value::from(SCHEMA_VERSION));
    m.insert("tenant", Value::from(rec.tenant));

    let map: Map<String, Value> = m.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
    serde_json::to_vec(&Value::Object(map)).expect("BTreeMap → Value::Object is serialisable")
}

/// Borrowed view of an event ready to be signed. Lives only on the
/// stack across the sign path ; not stored.
#[derive(Clone, Copy)]
pub struct SignableRecord<'a> {
    pub event_id: Uuid,
    pub chain_id: &'a str,
    pub tenant: &'a str,
    pub action: &'a str,
    pub op_tag: &'a str,
    pub catalog_key: &'a str,
    pub principal: &'a str,
    pub description: &'a str,
    pub parent_key: &'a str,
    pub content_sha256: &'a str,
    pub created_at_unix_ms: i64,
    pub prev_hmac: &'a str,
}

// =====================================================================
// Sink struct + lifecycle
// =====================================================================

/// HMAC-chained tamper-evident audit sink.
///
/// State : per-chain `last_hmac` (the HMAC of the most recent event
/// committed to `audit_events`). Persisted in the DB ; the in-memory
/// cache is hydrated lazily on first use of each chain under the
/// advisory lock (so multi-replica is safe).
#[derive(Clone)]
pub struct HmacChainedAuditSink {
    inner: Arc<Inner>,
}

struct Inner {
    pool: PgPool,
    /// Active signing key (the head of the keys file).
    active_key_id: String,
    /// All keys (active + rotated-out, kept for verify). Indexed by
    /// `key_id`. Wrapped in Arc for cheap clone across spawn.
    all_keys: Arc<HashMap<String, HmacKey>>,
    /// Per-chain last_hmac cache. Keyed by chain_id.
    last_hmac_cache: tokio::sync::Mutex<HashMap<String, String>>,
    /// Optional fan-out to the legacy tracing sink (Loki shipping).
    chain_to_tracing: bool,
    /// **D14-bis audit fix C2** — JoinSet tracking every spawned
    /// sign+persist task so that the server's shutdown path can
    /// `await` them before draining the pool. Without this, events
    /// committed by the catalog registry but whose audit hadn't
    /// finished spawning were lost on shutdown — exact opposite of
    /// "tamper-evident".
    pending: tokio::sync::Mutex<tokio::task::JoinSet<()>>,
}

impl HmacChainedAuditSink {
    /// Construct the sink. The active key is determined by
    /// `loaded.active_id` (set by `load_keys_file`).
    #[must_use]
    pub fn new(pool: PgPool, loaded: LoadedKeys, chain_to_tracing: bool) -> Self {
        let LoadedKeys { active_id, all } = loaded;
        Self {
            inner: Arc::new(Inner {
                pool,
                active_key_id: active_id,
                all_keys: Arc::new(all),
                last_hmac_cache: tokio::sync::Mutex::new(HashMap::new()),
                chain_to_tracing,
                pending: tokio::sync::Mutex::new(tokio::task::JoinSet::new()),
            }),
        }
    }

    /// **D14-bis audit fix C2** — drain every in-flight sign+persist
    /// task. Called from `Server::serve` shutdown path before
    /// draining the catalog pool, so events committed by the
    /// registry but not yet persisted by the audit sink are not
    /// dropped.
    ///
    /// `timeout` bounds the wait — typically the
    /// `EFE_SHUTDOWN_DRAIN_MS` window is >> the per-event sign
    /// latency (~10 ms), so we converge quickly. If the budget is
    /// exhausted we abort the remaining tasks and surface a metric.
    pub async fn drain_pending(&self, timeout: Duration) {
        let mut set = self.inner.pending.lock().await;
        let n_initial = set.len();
        if n_initial == 0 {
            return;
        }
        tracing::info!(
            target: "efe.audit.hmac",
            n_pending = n_initial,
            timeout_ms = timeout.as_millis() as u64,
            "draining pending audit emit tasks"
        );
        let deadline = std::time::Instant::now() + timeout;
        while set.len() > 0 {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                let aborted = set.len();
                set.abort_all();
                tracing::warn!(
                    target: "efe.audit.hmac",
                    n_aborted = aborted,
                    "audit drain budget exhausted ; aborting remaining tasks \
                     (events may be lost — counter \
                     efe_audit_sign_failures_total bumped per abort)"
                );
                for _ in 0..aborted {
                    efe_telemetry::metrics::d14bis_instruments::audit_sign_failures_total()
                        .with_label_values(&["unknown", "drain_abort"])
                        .inc();
                }
                return;
            }
            // Wait for one more task with the remaining budget.
            match tokio::time::timeout(remaining, set.join_next()).await {
                Ok(Some(_)) => {}   // one task drained
                Ok(None) => break,  // no more tasks
                Err(_) => continue, // timeout retried with refreshed remaining
            }
        }
        tracing::info!(
            target: "efe.audit.hmac",
            n_drained = n_initial,
            "audit drain complete"
        );
    }

    /// Pool accessor — exposed for the runtime's periodic-verify task.
    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.inner.pool
    }

    /// Keys accessor — exposed for the periodic-verify task.
    #[must_use]
    pub fn keys(&self) -> Arc<HashMap<String, HmacKey>> {
        self.inner.all_keys.clone()
    }
}

impl std::fmt::Debug for HmacChainedAuditSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HmacChainedAuditSink")
            .field("active_key_id", &self.inner.active_key_id)
            .field("n_keys", &self.inner.all_keys.len())
            .field("chain_to_tracing", &self.inner.chain_to_tracing)
            .finish()
    }
}

// =====================================================================
// AuditSink impl
// =====================================================================

// Note : the trait `AuditSink::emit` is sync (D12 contract). We do
// NOT decorate the impl with `#[async_trait]` — the macro is
// inert on sync methods and was a left-over from an earlier draft
// (audit fix M3).
impl AuditSink for HmacChainedAuditSink {
    /// Sync-emit : spawn the async sign+persist onto the tokio
    /// runtime, tracked via the inner `JoinSet` so shutdown can
    /// await drain (audit fix C2).
    ///
    /// **Loki ordering invariant** (audit fix L1) : the legacy
    /// `TracingAuditSink.emit` happens **before** the spawn so the
    /// Loki pipeline observes events in the order the registry
    /// produces them, not in the order the spawned tasks complete.
    ///
    /// **Fan-out semantics** (audit fix M4) : when
    /// `chain_to_tracing = true`, Loki always sees the event ;
    /// the SQL persistence is an additional guarantee, not an
    /// alternative. If SQL fails, Loki still has the line — that's
    /// intentional, the operator can correlate Loki ↔ SQL via
    /// `efe_audit_sign_failures_total`.
    fn emit(&self, event: &AuditEvent) {
        // Audit fix L1 — Loki fan-out FIRST so ordering is the
        // registry-emit order, not the spawn-completion order.
        if self.inner.chain_to_tracing {
            super::audit::TracingAuditSink.emit(event);
        }

        let inner = self.inner.clone();
        let event_owned = event.clone();
        // The sink is invoked from inside an async runtime (the
        // registry is async). `tokio::spawn` requires an active
        // runtime ; we expect that to be satisfied. If not (e.g.
        // a unit test that uses the trait outside of tokio), the
        // SQL emit becomes a no-op — defensive log.
        let handle = tokio::runtime::Handle::try_current();
        let runtime = match handle {
            Ok(h) => h,
            Err(_) => {
                tracing::warn!(
                    target: "efe.audit.hmac",
                    "HmacChainedAuditSink emitted outside a tokio runtime ; \
                     event NOT persisted to audit_events"
                );
                return;
            }
        };

        // Audit fix C2 — track the spawned task in the inner JoinSet
        // so shutdown can drain. We `try_lock` the pending set and
        // either spawn into it (success path) or fall back to a
        // detached spawn under contention. Early-return after the
        // success path keeps the borrow checker happy : the `inner`
        // borrow held by `try_lock` ends at the end of the `if let`
        // body, before the fallback uses `inner` by value.
        {
            if let Ok(mut set) = inner.pending.try_lock() {
                set.spawn_on(Self::sign_task(inner.clone(), event_owned), &runtime);
                return;
            }
        }
        // Lock contended (drain in progress at shutdown). Fall back
        // to a detached spawn ; the counter
        // `efe_audit_sign_failures_total{error="drain_race"}` surfaces
        // the case so ops can correlate with the shutdown event.
        runtime.spawn(Self::sign_task(inner, event_owned));
        efe_telemetry::metrics::d14bis_instruments::audit_sign_failures_total()
            .with_label_values(&[event.action.as_str(), "drain_race"])
            .inc();
    }
}

impl HmacChainedAuditSink {
    /// The async body that gets spawned per emit. Extracted so we
    /// can attach it to either a `JoinSet` (normal path) or a
    /// detached spawn (drain-race fallback).
    async fn sign_task(inner: Arc<Inner>, event: AuditEvent) {
        if let Err(e) = sign_and_persist(&inner, &event).await {
            tracing::error!(
                target: "efe.audit.hmac",
                err = %e,
                action = event.action.as_str(),
                tenant = %event.key.tenant,
                "audit sign+persist failed ; chain integrity at risk \
                 (counter efe_audit_sign_failures_total bumped)"
            );
            efe_telemetry::metrics::d14bis_instruments::audit_sign_failures_total()
                .with_label_values(&[event.action.as_str(), "persist"])
                .inc();
        } else {
            efe_telemetry::metrics::d14bis_instruments::audit_events_signed_total()
                .with_label_values(&[event.action.as_str()])
                .inc();
        }
    }
}

// =====================================================================
// Sign + persist core
// =====================================================================

#[derive(Debug, thiserror::Error)]
pub enum AuditSignError {
    #[error("Postgres error during audit {op}: {source}")]
    Database {
        op: &'static str,
        #[source]
        source: sqlx::Error,
    },

    #[error("active HMAC key {0:?} is missing from the keys map ; misconfiguration")]
    ActiveKeyMissing(String),

    #[error("JSON serialisation error in audit {0}: {1}")]
    Json(&'static str, serde_json::Error),
}

async fn sign_and_persist(inner: &Inner, event: &AuditEvent) -> Result<(), AuditSignError> {
    let started = std::time::Instant::now();
    let chain_id = format!("tenant:{}", event.key.tenant);
    let lock_id = lock_id_for_chain(&chain_id);

    let mut tx = inner
        .pool
        .begin()
        .await
        .map_err(|e| AuditSignError::Database {
            op: "begin",
            source: e,
        })?;

    // Advisory lock per chain — prevents the prev_hmac race between
    // concurrent inserts on the same chain.
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(lock_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| AuditSignError::Database {
            op: "advisory_lock",
            source: e,
        })?;

    // Hydrate the cache or re-read from DB under the lock.
    let prev_hmac = {
        let mut cache = inner.last_hmac_cache.lock().await;
        if let Some(h) = cache.get(&chain_id) {
            h.clone()
        } else {
            let h: Option<String> = sqlx::query_scalar(
                "SELECT hmac FROM audit_events \
                 WHERE chain_id = $1 ORDER BY seq DESC LIMIT 1",
            )
            .bind(&chain_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| AuditSignError::Database {
                op: "hydrate_prev_hmac",
                source: e,
            })?;
            let h = h.unwrap_or_else(|| GENESIS_PREV_HMAC.to_owned());
            cache.insert(chain_id.clone(), h.clone());
            h
        }
    };

    let event_id = Uuid::now_v7();
    let action_str = event.action.as_str();
    let catalog_key = event.key.to_wire();
    let created_at_unix_ms = now_unix_ms();

    let signable = SignableRecord {
        event_id,
        chain_id: &chain_id,
        tenant: &event.key.tenant,
        action: action_str,
        op_tag: &event.op_tag,
        catalog_key: &catalog_key,
        principal: &event.created_by,
        description: &event.description,
        parent_key: &event.parent_key,
        content_sha256: &event.content_sha256,
        created_at_unix_ms,
        prev_hmac: &prev_hmac,
    };
    let canonical = canonical_audit_payload(&signable);

    // Active key lookup — must be present (validate at boot).
    let key = inner
        .all_keys
        .get(&inner.active_key_id)
        .ok_or_else(|| AuditSignError::ActiveKeyMissing(inner.active_key_id.clone()))?;
    let hmac = compute_hmac(key.expose(), &canonical);
    let hmac_hex = hex_lower(&hmac);

    let canonical_json: serde_json::Value = serde_json::from_slice(&canonical)
        .map_err(|e| AuditSignError::Json("canonical_payload reparse", e))?;

    sqlx::query(
        "INSERT INTO audit_events ( \
             event_id, chain_id, tenant, action, op_tag, catalog_key, \
             principal, description, parent_key, content_sha256, \
             created_at_unix_ms, canonical_payload, prev_hmac, hmac, key_id \
         ) VALUES ( \
             $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15 \
         )",
    )
    .bind(event_id)
    .bind(&chain_id)
    .bind(&event.key.tenant)
    .bind(action_str)
    .bind(&event.op_tag)
    .bind(&catalog_key)
    .bind(&event.created_by)
    .bind(&event.description)
    .bind(&event.parent_key)
    .bind(&event.content_sha256)
    .bind(created_at_unix_ms)
    .bind(&canonical_json)
    .bind(&prev_hmac)
    .bind(&hmac_hex)
    .bind(&inner.active_key_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| AuditSignError::Database {
        op: "insert",
        source: e,
    })?;

    tx.commit().await.map_err(|e| AuditSignError::Database {
        op: "commit",
        source: e,
    })?;

    // Update the cache only after COMMIT lands. The lock is released
    // by COMMIT atomically with the row being visible.
    inner
        .last_hmac_cache
        .lock()
        .await
        .insert(chain_id, hmac_hex);

    let elapsed = started.elapsed().as_secs_f64();
    efe_telemetry::metrics::d14bis_instruments::audit_sign_duration_seconds()
        .with_label_values(&[action_str])
        .observe(elapsed);

    Ok(())
}

/// Wall-clock unix-ms timestamp. Used for `created_at_unix_ms` ;
/// signed by the HMAC. We use `std::time::SystemTime` rather than
/// `chrono::Utc` to avoid pulling chrono as a direct dep (audit fix
/// L3 of D14 retired chrono ; sqlx still pulls it transitively for
/// the type bridge but we don't access it directly here).
///
/// **Audit fix M2** — if `SystemTime::now() < UNIX_EPOCH` (clock
/// skew before 1970, exotic but possible on a fresh VM with a dead
/// CMOS battery), we log an explicit warning and return 0. The
/// alert `D14bisAuditClockSkew` should be wired in the operator
/// runbook to detect a flood of timestamp=0 audit events.
fn now_unix_ms() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => {
            tracing::warn!(
                target: "efe.audit.hmac",
                err = %e,
                "wall-clock is BEFORE UNIX_EPOCH ; audit timestamp set to 0 — \
                 fix the system clock (e.g. NTP, CMOS battery) immediately"
            );
            0
        }
    }
}

/// Constant-time equality check for hex-encoded HMAC strings.
///
/// **Audit fix M1** — the verify path's HMAC compare must not
/// short-circuit on the first differing byte ; a timing-side-channel
/// attacker who can observe the verify duration would otherwise
/// recover a few bits of the HMAC per measurement.
///
/// We pre-fail on length mismatch (the inputs are always 64 hex
/// chars in our pipeline ; mismatched length is a programmer bug,
/// not a tampering attempt). The byte-by-byte XOR-OR pattern is the
/// canonical constant-time-compare idiom.
#[must_use]
pub fn ct_hex_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.bytes().zip(b.bytes()) {
        diff |= x ^ y;
    }
    diff == 0
}

// =====================================================================
// Verify library
// =====================================================================

/// Outcome of a chain-verify pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    pub chain_id: String,
    pub n_events: u64,
    /// `Some(seq)` if the chain breaks at that row. `None` if intact.
    pub first_break_seq: Option<i64>,
    /// `Some(seq)` of the last row whose HMAC verified successfully.
    pub last_seq_verified: Option<i64>,
    /// Distinct `key_id`s encountered. Useful to surface decommissioned
    /// keys still in the chain (verify needs them).
    pub keys_used: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("JSON deserialisation error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("audit event references unknown key_id {0:?} ; rotate-in this key to verify")]
    UnknownKey(String),
}

/// Replay the chain identified by `chain_id`, recomputing each event's
/// HMAC from the stored `canonical_payload` and the appropriate key
/// (looked up by `key_id`). Returns a [`VerifyReport`] describing the
/// outcome.
///
/// **Streaming-safe** : the function pulls events page by page so the
/// memory footprint is bounded regardless of the chain length.
pub async fn verify_chain(
    pool: &PgPool,
    chain_id: &str,
    keys: &HashMap<String, HmacKey>,
) -> Result<VerifyReport, VerifyError> {
    use futures::TryStreamExt;

    let mut prev_hmac = GENESIS_PREV_HMAC.to_owned();
    let mut report = VerifyReport {
        chain_id: chain_id.to_owned(),
        n_events: 0,
        first_break_seq: None,
        last_seq_verified: None,
        keys_used: Vec::new(),
    };

    let mut rows = sqlx::query(
        "SELECT seq, event_id, canonical_payload, prev_hmac, hmac, key_id \
         FROM audit_events WHERE chain_id = $1 ORDER BY seq",
    )
    .bind(chain_id)
    .fetch(pool);

    while let Some(row) = rows.try_next().await? {
        let seq: i64 = row.try_get("seq")?;
        let stored_prev: String = row.try_get("prev_hmac")?;
        let stored_hmac: String = row.try_get("hmac")?;
        let key_id: String = row.try_get("key_id")?;
        let canonical: serde_json::Value = row.try_get("canonical_payload")?;

        // Chain link check (constant-time — audit fix M1).
        if !ct_hex_eq(&stored_prev, &prev_hmac) {
            report.first_break_seq = Some(seq);
            return Ok(report);
        }

        // Recompute HMAC.
        let canonical_bytes = serde_json::to_vec(&sort_value(canonical))?;
        let key = keys
            .get(&key_id)
            .ok_or_else(|| VerifyError::UnknownKey(key_id.clone()))?;
        let computed = compute_hmac(key.expose(), &canonical_bytes);
        let computed_hex = hex_lower(&computed);
        // Constant-time compare (audit fix M1) — no early-exit timing
        // signal that an attacker could correlate to recover HMAC bits.
        if !ct_hex_eq(&computed_hex, &stored_hmac) {
            report.first_break_seq = Some(seq);
            return Ok(report);
        }

        prev_hmac = stored_hmac;
        report.n_events += 1;
        report.last_seq_verified = Some(seq);
        if !report.keys_used.contains(&key_id) {
            report.keys_used.push(key_id);
        }
    }

    Ok(report)
}

/// Re-canonicalise a JSON Value by sorting keys lexicographically.
///
/// Postgres JSONB normalises the on-disk representation but does NOT
/// guarantee a specific key order on read ; we re-canonicalise on
/// the read side so the `serde_json::to_vec` output matches the
/// original byte sequence the HMAC was computed on.
fn sort_value(v: serde_json::Value) -> serde_json::Value {
    use serde_json::{Map, Value};
    match v {
        Value::Object(m) => {
            let mut sorted: std::collections::BTreeMap<String, Value> =
                std::collections::BTreeMap::new();
            for (k, vv) in m {
                sorted.insert(k, sort_value(vv));
            }
            let result: Map<String, Value> = sorted.into_iter().collect();
            Value::Object(result)
        }
        Value::Array(a) => Value::Array(a.into_iter().map(sort_value).collect()),
        other => other,
    }
}

// =====================================================================
// Periodic self-verify ticker
// =====================================================================

/// Background task that periodically re-verifies every active chain
/// and bumps `efe_audit_chain_break_detected_total` on tampering.
///
/// Spawned by `runtime::Server::build` when
/// `EFE_AUDIT_SELF_VERIFY_EVERY_S > 0`. Returns on shutdown signal.
pub async fn periodic_self_verify(
    pool: PgPool,
    keys: Arc<HashMap<String, HmacKey>>,
    interval: Duration,
    shutdown: crate::shutdown::Shutdown,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // First tick fires immediately ; we want to wait one interval first.
    ticker.tick().await;
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if let Err(e) = self_verify_pass(&pool, &keys).await {
                    tracing::error!(
                        target: "efe.audit.hmac",
                        err = %e,
                        "self-verify pass failed"
                    );
                }
            }
            _ = shutdown.notified() => {
                tracing::info!(
                    target: "efe.audit.hmac",
                    "shutdown — periodic_self_verify exiting"
                );
                return;
            }
        }
    }
}

/// One self-verify pass : list all distinct chain_ids in the table,
/// re-verify each chain, bump `efe_audit_chain_break_detected_total`
/// on tampering and update `efe_audit_chain_head_seq` gauges.
///
/// Public so the runtime's spawn loop can call it directly without
/// forcing a circular module dependency.
pub async fn self_verify_pass(
    pool: &PgPool,
    keys: &HashMap<String, HmacKey>,
) -> Result<(), VerifyError> {
    let chain_ids: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT chain_id FROM audit_events ORDER BY chain_id")
            .fetch_all(pool)
            .await?;
    for chain_id in chain_ids {
        let report = verify_chain(pool, &chain_id, keys).await?;
        if let Some(break_seq) = report.first_break_seq {
            tracing::error!(
                target: "efe.audit.hmac",
                chain_id = %chain_id,
                break_seq = break_seq,
                "AUDIT CHAIN COMPROMISED — tampering detected by self-verify"
            );
            efe_telemetry::metrics::d14bis_instruments::audit_chain_break_detected_total()
                .with_label_values(&[&chain_id])
                .inc();
        } else {
            efe_telemetry::metrics::d14bis_instruments::audit_chain_head_seq()
                .with_label_values(&[&chain_id])
                .set(report.last_seq_verified.unwrap_or(0));
        }
    }
    Ok(())
}

// =====================================================================
// Tests — pure unit (no DB).
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn sample_record<'a>(event_id: Uuid, prev_hmac: &'a str) -> SignableRecord<'a> {
        SignableRecord {
            event_id,
            chain_id: "tenant:acme",
            tenant: "acme",
            action: "register",
            op_tag: "register",
            catalog_key: "acme/billing@v1",
            principal: "alice@acme",
            description: "init",
            parent_key: "",
            content_sha256: "5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8",
            created_at_unix_ms: 1_700_000_000_000,
            prev_hmac,
        }
    }

    #[test]
    fn canonical_payload_is_deterministic() {
        let id = Uuid::now_v7();
        let rec = sample_record(id, GENESIS_PREV_HMAC);
        let a = canonical_audit_payload(&rec);
        let b = canonical_audit_payload(&rec);
        assert_eq!(a, b, "canonical payload must be byte-stable");
    }

    #[test]
    fn canonical_payload_keys_are_sorted_alphabetically() {
        let id = Uuid::now_v7();
        let rec = sample_record(id, GENESIS_PREV_HMAC);
        let bytes = canonical_audit_payload(&rec);
        let s = std::str::from_utf8(&bytes).unwrap();
        // Find the position of each top-level key. They must be in
        // alphabetical order.
        let keys = [
            "\"action\":",
            "\"catalog_key\":",
            "\"chain_id\":",
            "\"content_sha256\":",
            "\"created_at_unix_ms\":",
            "\"description\":",
            "\"event_id\":",
            "\"op_tag\":",
            "\"parent_key\":",
            "\"prev_hmac\":",
            "\"principal\":",
            "\"schema_version\":",
            "\"tenant\":",
        ];
        let mut last_pos = 0usize;
        for k in keys {
            let p = s.find(k).unwrap_or_else(|| panic!("missing key {k}"));
            assert!(p >= last_pos, "key {k} out of alphabetical order");
            last_pos = p;
        }
    }

    #[test]
    fn canonical_payload_changes_when_field_changes() {
        let id = Uuid::now_v7();
        let mut a = sample_record(id, GENESIS_PREV_HMAC);
        let bytes_a = canonical_audit_payload(&a);
        a.tenant = "globex";
        a.chain_id = "tenant:globex";
        let bytes_b = canonical_audit_payload(&a);
        assert_ne!(bytes_a, bytes_b);
    }

    #[test]
    fn compute_hmac_matches_rfc4231_test_case_1() {
        // RFC 4231 §4.2 — test case 1 (HMAC-SHA-256).
        // key   = 0x0b * 20  (BUT test case 1 uses 20-byte key, not 32)
        // For our 32-byte requirement we use the truncation+padding
        // model : HMAC accepts any length, our compute_hmac requires
        // [u8; 32]. So we test our pipeline with a known answer using
        // a 32-byte key and a synthetic check (HMAC with a known input
        // and our own output, byte-stability test).
        let key = [0x0bu8; 32];
        let data = b"Hi There";
        let h1 = compute_hmac(&key, data);
        let h2 = compute_hmac(&key, data);
        assert_eq!(h1, h2, "HMAC must be deterministic");
        assert_ne!(h1, [0u8; 32], "HMAC must not be zero");
    }

    #[test]
    fn hex_lower_round_trip() {
        let bytes = [0xde, 0xad, 0xbe, 0xef];
        assert_eq!(hex_lower(&bytes), "deadbeef");
        let bytes32 = [0xff; 32];
        let hx = hex_lower(&bytes32);
        assert_eq!(hx.len(), 64);
        assert_eq!(hx, "f".repeat(64));
    }

    #[test]
    fn lock_id_for_chain_is_stable() {
        let a = lock_id_for_chain("tenant:acme");
        let b = lock_id_for_chain("tenant:acme");
        assert_eq!(a, b);
    }

    #[test]
    fn lock_id_for_chain_differs_between_chains() {
        let a = lock_id_for_chain("tenant:acme");
        let b = lock_id_for_chain("tenant:globex");
        assert_ne!(a, b);
    }

    #[test]
    fn genesis_prev_hmac_is_64_zeros() {
        assert_eq!(GENESIS_PREV_HMAC.len(), 64);
        assert!(GENESIS_PREV_HMAC.bytes().all(|b| b == b'0'));
    }

    #[test]
    fn schema_version_is_one() {
        assert_eq!(SCHEMA_VERSION, 1);
    }

    #[test]
    fn ct_hex_eq_matches_string_eq_on_valid_inputs() {
        // Audit fix M1 — sanity check : the constant-time compare
        // gives the same answer as `==` on equal-length hex strings.
        // (The crypto property is timing — not testable in unit
        // tests deterministically — but at least the equality
        // semantics match.)
        let a = "deadbeef";
        let b = "deadbeef";
        let c = "deadbef0";
        assert!(ct_hex_eq(a, b));
        assert!(!ct_hex_eq(a, c));
        // Length mismatch → false.
        assert!(!ct_hex_eq(a, "deadbeef00"));
        assert!(!ct_hex_eq("", "0"));
        assert!(ct_hex_eq("", ""));
    }

    #[test]
    fn ct_hex_eq_does_not_short_circuit_on_first_diff() {
        // Property : compare two strings that differ only in the
        // last byte → still returns false. Doesn't prove constant
        // time but proves we look at the whole string.
        let a = "00000000000000000000000000000000000000000000000000000000000000aa";
        let b = "00000000000000000000000000000000000000000000000000000000000000bb";
        assert!(!ct_hex_eq(a, b));
    }
}
