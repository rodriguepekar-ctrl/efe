//! Sprint **D14** — Postgres-backed [`CatalogRegistry`] implementation.
//!
//! Mirror of the design document
//! [`docs/bench/d14_postgres_catalog_backend_architecture.md`](../../../docs/bench/d14_postgres_catalog_backend_architecture.md).
//! The trait surface is identical to [`super::memory::InMemoryRegistry`]
//! (D12) ; this file is purely the storage swap. The handler in
//! `crates/efe-server/src/services/catalog/mod.rs` (D12-bis) does NOT
//! know which backend it talks to — that's the whole point of the
//! `Arc<dyn CatalogRegistry>` abstraction.
//!
//! # Architectural pivots
//!
//! 1. **sqlx 0.7** in runtime-checked mode (no `query!()` macros) so
//!    the build does not require a live Postgres instance. Compile-time
//!    safety is delegated to the integration tests under
//!    `crates/efe-server/tests/d14_postgres_*.rs`, which run against a
//!    real PG configured via the `EFE_PG_TEST_URL` env var.
//!
//! 2. **Single `catalogs` table** with composite primary key
//!    `(tenant, name, version)`. Cf. § 4 of the architecture doc for
//!    the rationale ; LIST partitioning is reserved for D17+.
//!
//! 3. **`pg_advisory_xact_lock`** keyed on
//!    `xxh64(b"<tenant>/<name>", ADVISORY_LOCK_SEED)` to serialise
//!    monotone version assignment per `(tenant, name)`. Cf. § 5 of the
//!    architecture doc — this is *the* concurrency primitive that lets
//!    `register` / `evolve` race-free under contention without
//!    serialisable isolation. The seed (`0xefe_0000_d014_0000`) belongs
//!    to the EFE namespace so unrelated apps on the same cluster do
//!    not collide on the lock id space.
//!
//! 4. **JSONB for `content_json` and `labels`** : queryable in v1+,
//!    indexable on demand via expression indexes, and avoids the
//!    BYTEA / serialisation back-and-forth.
//!
//! 5. **App-level reference check** for `delete force=false`. We do
//!    NOT use a foreign key with `ON DELETE CASCADE` because the
//!    failure mode would be `fk_violation` (mapped to `Internal`)
//!    rather than the trait-canonical `Conflict` error. The
//!    `parent_key` column is regex-validated by a CHECK constraint
//!    (cf. `migrations/0001_init.sql`).
//!
//! # Error mapping (annexe F of the architecture doc)
//!
//! | sqlx error | PG code | `CatalogError` |
//! |------------|---------|----------------|
//! | `Database` 23505 | unique_violation | `Conflict("(tenant, name, version) already exists")` |
//! | `Database` 23514 | check_violation | `InvalidArgument("schema constraint")` |
//! | `Database` 23502 | not_null_violation | `Internal("missing required field")` |
//! | `Database` 40001 | serialization_failure | `Internal("transaction conflict")` |
//! | `Database` 40P01 | deadlock_detected | `Internal("deadlock")` |
//! | `Database` 53300 | too_many_connections | `Internal("PG pool exhausted")` |
//! | `RowNotFound` | n/a | `NotFound(_)` |
//! | `PoolTimedOut` | n/a | `Internal("pool acquire timeout")` |
//! | `Io(_)` | n/a | `Internal("transient network error")` |
//! | any other | n/a | `Internal(_)` (logged server-side, opaque to client) |
//!
//! # Non-goals (deferred to D15+)
//!
//! * Compaction of old versions (DT-D14.2 backlog).
//! * Materialised view for `list_catalogs` (DT-D14.2 backlog if bench
//!   shows the JSONB-extract aggregation is too slow).
//! * Read replicas for `Get` / `List`.
//! * Postgres Row-Level Security (DT-D14.3 — D17+).

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use sha2::{Digest, Sha256};
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::Row;
use xxhash_rust::xxh64::xxh64;

use crate::catalog::{
    AuditEmitter, CatalogContent, CatalogError, CatalogKey, CatalogMetadata, CatalogPage,
    CatalogPolicy, CatalogRefSummary, CatalogRegistry, CompiledClass, ContentDigest, EmbedderSpec,
    EvolveOp, RegisteredCatalog,
};
use crate::config::CatalogPostgresConfig;

// =====================================================================
// Constants
// =====================================================================

/// Seed for the xxh64 hash used to derive `pg_advisory_xact_lock` ids.
///
/// The constant lives in the EFE namespace so the EFE app's locks
/// never collide with unrelated applications sharing the same cluster.
/// Documented under § 5.3 of the architecture doc ; collision
/// probability between any two `(tenant, name)` pairs is 2⁻⁶⁴
/// (~5 × 10⁻²⁰), and ~3 × 10⁻⁸ over a population of 10⁶ keys
/// (birthday bound).
const ADVISORY_LOCK_SEED: u64 = 0xefe_0000_d014_0000;

// =====================================================================
// Helpers — pure, testable without a database
// =====================================================================

/// Compute the Postgres advisory-lock id for a `(tenant, name)` pair.
///
/// The 64-bit xxh64 output is reinterpreted as `i64` because
/// `pg_advisory_xact_lock` takes `bigint` (signed). The wrap-around
/// is harmless : sign does not affect lock semantics, only equality.
#[must_use]
pub(crate) fn lock_id_for(tenant: &str, name: &str) -> i64 {
    let mut buf = Vec::with_capacity(tenant.len() + 1 + name.len());
    buf.extend_from_slice(tenant.as_bytes());
    buf.push(b'/');
    buf.extend_from_slice(name.as_bytes());
    let h = xxh64(&buf, ADVISORY_LOCK_SEED);
    h as i64
}

/// Hex-encode the SHA-256 digest of the canonical JSON serialisation
/// of `content`.
///
/// **Determinism**: the JSON output is field-order stable because
/// `CatalogContent` (and its descendants) only contain structs (which
/// serde-derives in declaration order), `Vec`s (which preserve
/// insertion order), and `BTreeMap`s (which iterate in key order).
/// Two clients sending the same logical `CatalogContent` from
/// different machines produce the same `content_sha256`.
#[must_use]
pub(crate) fn sha256_hex_of_content(content: &CatalogContent) -> String {
    // unwrap : `serde_json::to_vec` only fails on serialiser bugs ;
    // our types are all stable Serialize.
    let bytes = serde_json::to_vec(content).expect("CatalogContent serialise to JSON");
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest = hasher.finalize();
    let mut s = String::with_capacity(digest.len() * 2);
    for b in digest.iter() {
        // explicit two-hex-digit format for stability across rust versions
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

/// Encode a `list_catalogs` page cursor.
///
/// The cursor carries the **last `name` of the previous page** so the
/// next query can do `WHERE name > $cursor`. We base64-encode it so
/// the client treats it as an opaque blob (forward-compat — we may
/// embed (tenant, name) pairs in D17+ without breaking clients).
#[must_use]
pub(crate) fn encode_page_token(last_name: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(last_name.as_bytes())
}

/// Decode a `list_catalogs` page cursor produced by [`encode_page_token`].
///
/// Returns `Err(CatalogError::InvalidArgument)` if the cursor is not
/// valid base64 or contains non-UTF-8 bytes — clients must treat
/// cursors as opaque, so any malformed cursor is a contract violation.
pub(crate) fn decode_page_token(token: &str) -> Result<String, CatalogError> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(token.as_bytes())
        .map_err(|e| CatalogError::InvalidArgument(format!("malformed page_token: {e}")))?;
    String::from_utf8(bytes).map_err(|e| {
        CatalogError::InvalidArgument(format!("malformed page_token (non-utf-8): {e}"))
    })
}

/// Apply an [`EvolveOp`] to a [`CatalogContent`] in place.
///
/// Mirrors `InMemoryRegistry::apply_evolve_op` — the trait contract
/// is identical between the two backends, so this routine can be
/// exercised by the same set of unit tests.
///
/// **Side effect**: when the op is [`EvolveOp::Recompile`], the
/// embedder is replaced AND the caller is expected to invalidate the
/// `compiled_anchors` BYTEA column (set to `NULL`) — the embedder
/// fingerprint changed, the old anchors are no longer applicable.
/// We do not zero them here ; the SQL layer in
/// [`PostgresCatalogRegistry::evolve`] handles the column update.
pub(crate) fn apply_evolve_op(
    content: &mut CatalogContent,
    op: &EvolveOp,
) -> Result<(), CatalogError> {
    match op {
        EvolveOp::AppendClass { class } => {
            if content.classes.iter().any(|c| c.name == class.name) {
                return Err(CatalogError::InvalidArgument(format!(
                    "AppendClass: class {:?} already exists in catalog",
                    class.name
                )));
            }
            content.classes.push(class.clone());
        }
        EvolveOp::RemoveClass { class_name } => {
            let before = content.classes.len();
            content.classes.retain(|c| &c.name != class_name);
            if content.classes.len() == before {
                return Err(CatalogError::InvalidArgument(format!(
                    "RemoveClass: class {class_name:?} not found"
                )));
            }
        }
        EvolveOp::UpdateThresholds { thresholds } => {
            content.thresholds = thresholds.clone();
        }
        EvolveOp::ReplaceClassAnchors {
            class_name,
            canonical_sentences,
        } => {
            let cls = content
                .classes
                .iter_mut()
                .find(|c| &c.name == class_name)
                .ok_or_else(|| {
                    CatalogError::InvalidArgument(format!(
                        "ReplaceClassAnchors: class {class_name:?} not found"
                    ))
                })?;
            if canonical_sentences.is_empty() {
                return Err(CatalogError::InvalidArgument(format!(
                    "ReplaceClassAnchors for {class_name:?} requires \
                     a non-empty canonical_sentences list",
                )));
            }
            cls.canonical_sentences = canonical_sentences.clone();
        }
        EvolveOp::Recompile { new_embedder } => {
            content.embedder = new_embedder.clone();
        }
    }
    Ok(())
}

/// Map a [`sqlx::Error`] to a [`CatalogError`].
///
/// Cf. annexe F of the architecture doc for the full table. The
/// caller is responsible for *not* leaking the inner error message
/// to the client — `Internal` variants log server-side at ERROR level
/// before being returned to the gRPC handler, which sanitises them
/// per the audit fix T-12bis-2 in the D12-bis handler.
fn map_pg_error(op: &'static str, e: sqlx::Error) -> CatalogError {
    if let sqlx::Error::Database(ref db) = e {
        let code = db.code().map(|c| c.into_owned()).unwrap_or_default();
        match code.as_str() {
            "23505" => {
                tracing::warn!(
                    target: "efe.catalog.postgres",
                    op = op,
                    pg_code = %code,
                    detail = %db.message(),
                    "unique violation — concurrent register/evolve race",
                );
                return CatalogError::Conflict(format!(
                    "uniqueness violation in {op}: \
                     (tenant, name, version) already exists",
                ));
            }
            "23514" => {
                tracing::warn!(
                    target: "efe.catalog.postgres",
                    op = op,
                    pg_code = %code,
                    detail = %db.message(),
                    "check constraint violation",
                );
                return CatalogError::InvalidArgument(format!(
                    "schema constraint violated in {op}: {}",
                    db.message()
                ));
            }
            "23502" => {
                tracing::error!(
                    target: "efe.catalog.postgres",
                    op = op,
                    pg_code = %code,
                    detail = %db.message(),
                    "not-null violation — schema bug",
                );
                return CatalogError::Internal(format!(
                    "missing required field in {op}: {}",
                    db.message()
                ));
            }
            "40001" => {
                tracing::warn!(
                    target: "efe.catalog.postgres",
                    op = op,
                    pg_code = %code,
                    "serialization failure — transaction conflict",
                );
                return CatalogError::Internal(format!(
                    "transaction conflict in {op}; retry recommended",
                ));
            }
            "40P01" => {
                tracing::warn!(
                    target: "efe.catalog.postgres",
                    op = op,
                    pg_code = %code,
                    "deadlock detected",
                );
                return CatalogError::Internal(format!(
                    "deadlock detected in {op}; retry recommended",
                ));
            }
            "53300" => {
                tracing::error!(
                    target: "efe.catalog.postgres",
                    op = op,
                    pg_code = %code,
                    "PG too_many_connections — pool exhausted",
                );
                return CatalogError::Internal("PG connection limit reached".to_owned());
            }
            _ => {
                tracing::error!(
                    target: "efe.catalog.postgres",
                    op = op,
                    pg_code = %code,
                    detail = %db.message(),
                    "unhandled PG error",
                );
                return CatalogError::Internal(format!("PG error in {op} (code {code})"));
            }
        }
    }
    if let sqlx::Error::RowNotFound = e {
        return CatalogError::NotFound(format!("{op}: row not found"));
    }
    if let sqlx::Error::PoolTimedOut = e {
        tracing::error!(
            target: "efe.catalog.postgres",
            op = op,
            "pool acquire timeout",
        );
        return CatalogError::Internal(format!("pool acquire timeout in {op} — DB overload"));
    }
    if let sqlx::Error::Io(ref io) = e {
        tracing::error!(
            target: "efe.catalog.postgres",
            op = op,
            io_kind = ?io.kind(),
            "transient I/O error",
        );
        return CatalogError::Internal(format!("transient network error in {op}: {io}"));
    }
    tracing::error!(
        target: "efe.catalog.postgres",
        op = op,
        sqlx_err = %e,
        "uncategorised sqlx error",
    );
    CatalogError::Internal(format!("internal error in {op}"))
}

/// Convert an internal `serde_json::Error` into a `CatalogError::Internal`.
///
/// Used by the JSONB serialisation paths. Should never fire in
/// practice (our types are stable Serialize/Deserialize) but failing
/// gracefully is safer than panicking inside the registry.
fn internal_je<E: std::fmt::Display>(op: &'static str, e: E) -> CatalogError {
    tracing::error!(
        target: "efe.catalog.postgres",
        op = op,
        detail = %e,
        "JSON (de)serialisation error",
    );
    CatalogError::Internal(format!("json error in {op}"))
}

// =====================================================================
// Registry struct + lifecycle
// =====================================================================

/// Postgres-backed implementation of [`CatalogRegistry`].
///
/// Construction is via [`PostgresCatalogRegistry::connect`], which
/// builds the connection pool from a [`CatalogPostgresConfig`] and
/// **does NOT** run migrations — that's a separate explicit call to
/// [`Self::run_migrations`]. The runtime uses both methods in order :
/// 1. `connect` (to fail fast on bad credentials),
/// 2. `run_migrations` (to fail fast on a stale schema).
///
/// Cloning is cheap : the inner [`PgPool`] is `Clone`-by-`Arc`. We
/// expose a clone-by-arc handle so the gRPC handler stack
/// (`CatalogServiceImpl`) can hold an `Arc<dyn CatalogRegistry>`
/// without paying for boxed allocation per RPC.
#[derive(Clone)]
pub struct PostgresCatalogRegistry {
    pool: PgPool,
    policy: CatalogPolicy,
    audit: AuditEmitter,
}

impl std::fmt::Debug for PostgresCatalogRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresCatalogRegistry")
            .field("pool_size", &self.pool.size())
            .field("policy", &self.policy)
            .finish()
    }
}

impl PostgresCatalogRegistry {
    /// Connect to Postgres and build the pool described by `cfg`.
    ///
    /// Fail-fast paths :
    /// * DSN parse failure → `CatalogError::Internal`.
    /// * Initial connection refused → `CatalogError::Internal`.
    /// * TLS handshake failure → `CatalogError::Internal` (no leak of
    ///   the inner reason — see `tracing` for diagnostic).
    pub async fn connect(
        cfg: &CatalogPostgresConfig,
        policy: CatalogPolicy,
        audit: AuditEmitter,
    ) -> Result<Self, CatalogError> {
        tracing::info!(
            target: "efe.catalog.postgres",
            dsn = %cfg.dsn_redacted(),
            pool_max = cfg.pool_max_connections,
            pool_min = cfg.pool_min_connections,
            ssl_mode = cfg.ssl_mode.as_dsn_value(),
            "connecting to Postgres",
        );
        let pool = PgPoolOptions::new()
            .max_connections(cfg.pool_max_connections)
            .min_connections(cfg.pool_min_connections)
            .acquire_timeout(Duration::from_millis(cfg.pool_acquire_timeout_ms))
            .max_lifetime(Some(Duration::from_secs(cfg.pool_max_lifetime_s)))
            .idle_timeout(Some(Duration::from_secs(cfg.pool_idle_timeout_s)))
            .test_before_acquire(cfg.pool_test_before_acquire)
            .connect(&cfg.dsn())
            .await
            .map_err(|e| {
                tracing::error!(
                    target: "efe.catalog.postgres",
                    err = %e,
                    "connect failed",
                );
                CatalogError::Internal(format!("Postgres connect failed: {e}"))
            })?;
        tracing::info!(
            target: "efe.catalog.postgres",
            "Postgres pool ready",
        );
        Ok(Self {
            pool,
            policy,
            audit,
        })
    }

    /// Build a registry from an already-constructed pool — used by
    /// the integration tests so they can share a pool across tests
    /// without re-paying the connect cost.
    #[must_use]
    pub fn from_pool(pool: PgPool, policy: CatalogPolicy, audit: AuditEmitter) -> Self {
        Self {
            pool,
            policy,
            audit,
        }
    }

    /// Pool accessor — exposed for the runtime's pool-size metrics
    /// ticker (cf. § 12 of the architecture doc).
    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Apply forward-only migrations from the crate's `migrations/`
    /// directory.
    ///
    /// Idempotent : sqlx tracks applied migrations in
    /// `_sqlx_migrations`, so calling this from N replicas concurrently
    /// is safe (only one instance wins the migration advisory lock).
    pub async fn run_migrations(&self) -> Result<(), CatalogError> {
        tracing::info!(
            target: "efe.catalog.postgres",
            "running migrations from `crates/efe-server/migrations/`",
        );
        let migrator = sqlx::migrate!("./migrations");
        migrator.run(&self.pool).await.map_err(|e| {
            tracing::error!(
                target: "efe.catalog.postgres",
                err = %e,
                "migration failed",
            );
            CatalogError::Internal(format!("migration failed: {e}"))
        })?;
        tracing::info!(
            target: "efe.catalog.postgres",
            "migrations applied",
        );
        Ok(())
    }
}

// =====================================================================
// Row representation
// =====================================================================

/// Internal mirror of the `catalogs` row, hydrated by sqlx via
/// `query!`-free decoding (we use `Row::try_get` for explicit column
/// access — keeps the build hermetic).
struct CatalogRow {
    tenant: String,
    name: String,
    version: i32,
    content_json: serde_json::Value,
    created_at_unix_ms: i64,
    created_by: String,
    description: String,
    parent_key: String,
    evolution_op: String,
    labels: serde_json::Value,
    content_sha256: String,
    compiled_anchors: Option<Vec<u8>>,
    compiled_classes: Option<serde_json::Value>,
    embedder_fingerprint: Option<String>,
}

impl CatalogRow {
    /// Hydrate a `CatalogRow` from a `sqlx::postgres::PgRow`.
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, CatalogError> {
        Ok(Self {
            tenant: row
                .try_get("tenant")
                .map_err(|e| internal_je("row.tenant", e))?,
            name: row
                .try_get("name")
                .map_err(|e| internal_je("row.name", e))?,
            version: row
                .try_get("version")
                .map_err(|e| internal_je("row.version", e))?,
            content_json: row
                .try_get("content_json")
                .map_err(|e| internal_je("row.content_json", e))?,
            created_at_unix_ms: row
                .try_get("created_at_unix_ms")
                .map_err(|e| internal_je("row.created_at_unix_ms", e))?,
            created_by: row
                .try_get("created_by")
                .map_err(|e| internal_je("row.created_by", e))?,
            description: row
                .try_get("description")
                .map_err(|e| internal_je("row.description", e))?,
            parent_key: row
                .try_get("parent_key")
                .map_err(|e| internal_je("row.parent_key", e))?,
            evolution_op: row
                .try_get("evolution_op")
                .map_err(|e| internal_je("row.evolution_op", e))?,
            labels: row
                .try_get("labels")
                .map_err(|e| internal_je("row.labels", e))?,
            content_sha256: row
                .try_get("content_sha256")
                .map_err(|e| internal_je("row.content_sha256", e))?,
            compiled_anchors: row
                .try_get("compiled_anchors")
                .map_err(|e| internal_je("row.compiled_anchors", e))?,
            compiled_classes: row
                .try_get("compiled_classes")
                .map_err(|e| internal_je("row.compiled_classes", e))?,
            embedder_fingerprint: row
                .try_get("embedder_fingerprint")
                .map_err(|e| internal_je("row.embedder_fingerprint", e))?,
        })
    }

    /// Materialise into a domain [`RegisteredCatalog`].
    fn into_registered(self, include_compiled: bool) -> Result<RegisteredCatalog, CatalogError> {
        let content: CatalogContent = serde_json::from_value(self.content_json)
            .map_err(|e| internal_je("decode content_json", e))?;
        let labels: BTreeMap<String, String> =
            serde_json::from_value(self.labels).map_err(|e| internal_je("decode labels", e))?;
        let metadata = CatalogMetadata {
            created_at_unix_ms: self.created_at_unix_ms,
            created_by: self.created_by,
            description: self.description,
            parent_key: self.parent_key,
            evolution_op: self.evolution_op,
            labels,
            content_sha256: ContentDigest(self.content_sha256),
        };
        let key = CatalogKey {
            tenant: self.tenant,
            name: self.name,
            version: self.version as u32,
        };
        let compiled = if include_compiled
            && self.compiled_anchors.is_some()
            && self.compiled_classes.is_some()
        {
            Some(decode_compiled(
                &content.embedder,
                self.compiled_anchors.as_deref().unwrap_or(&[]),
                self.compiled_classes.as_ref().unwrap(),
                self.embedder_fingerprint.as_deref().unwrap_or(""),
            )?)
        } else {
            None
        };
        Ok(RegisteredCatalog {
            key,
            content,
            metadata,
            compiled,
        })
    }
}

/// Decode the BYTEA blob + JSONB layout map into a list of
/// [`CompiledClass`] structs. The on-disk format is documented in
/// § 6.1 of the architecture doc :
///
/// ```jsonc
/// // compiled_classes :
/// // {
/// //   "<class_name>": { "offset": 0, "num_anchors": 8 },
/// //   ...
/// // }
/// ```
///
/// `offset` is in **f32 elements** (not bytes) ; one row of
/// `dim` floats × 4 bytes. The decoder slices `compiled_anchors`
/// accordingly.
fn decode_compiled(
    embedder: &EmbedderSpec,
    blob: &[u8],
    layout: &serde_json::Value,
    fingerprint: &str,
) -> Result<Vec<CompiledClass>, CatalogError> {
    let map: BTreeMap<String, serde_json::Value> = serde_json::from_value(layout.clone())
        .map_err(|e| internal_je("decode compiled layout", e))?;
    let dim = embedder.dim as usize;
    let row_bytes = dim * 4; // f32 little-endian
    let mut out = Vec::with_capacity(map.len());
    for (class_name, layout_entry) in map.into_iter() {
        let offset = layout_entry
            .get("offset")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| {
                CatalogError::Internal(format!("compiled_classes[{class_name:?}].offset missing"))
            })? as usize;
        let num_anchors = layout_entry
            .get("num_anchors")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| {
                CatalogError::Internal(format!(
                    "compiled_classes[{class_name:?}].num_anchors missing"
                ))
            })? as u32;
        let start = offset * 4 * dim;
        let end = start + (num_anchors as usize) * row_bytes;
        if end > blob.len() {
            return Err(CatalogError::Internal(format!(
                "compiled_anchors blob truncated for {class_name:?}: \
                 wanted [{start}..{end}], have {}",
                blob.len()
            )));
        }
        out.push(CompiledClass {
            class_name,
            anchors_blob: blob[start..end].to_vec(),
            num_anchors,
            embedder_fingerprint: fingerprint.to_owned(),
        });
    }
    Ok(out)
}

// =====================================================================
// CatalogRegistry impl
// =====================================================================

#[async_trait]
impl CatalogRegistry for PostgresCatalogRegistry {
    async fn register(
        &self,
        tenant: &str,
        name: &str,
        content: CatalogContent,
        mut metadata: CatalogMetadata,
    ) -> Result<RegisteredCatalog, CatalogError> {
        // -----------------------------------------------------------------
        // Step 0 — policy validation (cheap, applied first).
        //
        // **D14 audit fix C1** : refuse tenant / name that don't match
        // the canonical regex `^[a-z0-9_-]{1,63}$`. Without this guard,
        // an operator could register `tenant="ACME"` v1 successfully
        // (the parent_key CHECK passes for the empty parent_key) only
        // to see the next evolve fail with an opaque check_violation
        // because the synthesised parent_key "ACME/name@v1" violates
        // the regex on column `parent_key`.
        // -----------------------------------------------------------------
        CatalogPolicy::validate_identifier("tenant", tenant)?;
        CatalogPolicy::validate_identifier("name", name)?;
        self.policy.validate_content(&content)?;

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| map_pg_error("register/begin", e))?;

        // -----------------------------------------------------------------
        // Step 1 — advisory lock for this (tenant, name).
        // The lock lives the lifetime of the transaction ; commit /
        // rollback releases it automatically.
        // -----------------------------------------------------------------
        let lock_id = lock_id_for(tenant, name);
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| map_pg_error("register/advisory_lock", e))?;

        // -----------------------------------------------------------------
        // Step 2 — quota checks under the lock.
        // -----------------------------------------------------------------
        let n_names: i64 =
            sqlx::query_scalar("SELECT count(DISTINCT name) FROM catalogs WHERE tenant = $1")
                .bind(tenant)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| map_pg_error("register/count_names", e))?;
        if (n_names as u32) >= self.policy.max_names_per_tenant && {
            // The check is "would inserting a NEW name overflow ?" If
            // the (tenant, name) tuple already exists, we are about to
            // bump version, not create a new chain — so we don't count
            // it against `max_names_per_tenant`.
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM catalogs \
                 WHERE tenant = $1 AND name = $2)",
            )
            .bind(tenant)
            .bind(name)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| map_pg_error("register/exists_name", e))?;
            !exists
        } {
            return Err(CatalogError::QuotaExceeded(format!(
                "tenant={tenant} reached max_names_per_tenant={}",
                self.policy.max_names_per_tenant
            )));
        }
        let n_versions: i64 =
            sqlx::query_scalar("SELECT count(*) FROM catalogs WHERE tenant = $1 AND name = $2")
                .bind(tenant)
                .bind(name)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| map_pg_error("register/count_versions", e))?;
        if (n_versions as u32) >= self.policy.max_versions_per_name {
            return Err(CatalogError::QuotaExceeded(format!(
                "(tenant={tenant}, name={name}) reached max_versions_per_name={}",
                self.policy.max_versions_per_name
            )));
        }

        // -----------------------------------------------------------------
        // Step 3 — compute next version under lock.
        // -----------------------------------------------------------------
        let max_v: Option<i32> =
            sqlx::query_scalar("SELECT MAX(version) FROM catalogs WHERE tenant = $1 AND name = $2")
                .bind(tenant)
                .bind(name)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| map_pg_error("register/max_version", e))?;
        let next_v: i32 = max_v.map(|v| v + 1).unwrap_or(1);

        // -----------------------------------------------------------------
        // Step 4 — serialise content + compute SHA-256.
        // -----------------------------------------------------------------
        let content_json = serde_json::to_value(&content)
            .map_err(|e| internal_je("register/serialise_content", e))?;
        let sha = sha256_hex_of_content(&content);
        let labels_json = serde_json::to_value(&metadata.labels)
            .map_err(|e| internal_je("register/serialise_labels", e))?;

        // Stamp metadata fields the trait wants the registry to own.
        metadata.parent_key = String::new();
        metadata.evolution_op = "register".to_owned();
        metadata.content_sha256 = ContentDigest(sha.clone());

        // -----------------------------------------------------------------
        // Step 5 — INSERT.
        // -----------------------------------------------------------------
        sqlx::query(
            "INSERT INTO catalogs ( \
                 tenant, name, version, \
                 content_json, \
                 created_at_unix_ms, created_by, description, \
                 parent_key, evolution_op, labels, content_sha256 \
             ) VALUES ( \
                 $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11 \
             )",
        )
        .bind(tenant)
        .bind(name)
        .bind(next_v)
        .bind(&content_json)
        .bind(metadata.created_at_unix_ms)
        .bind(&metadata.created_by)
        .bind(&metadata.description)
        .bind("") // parent_key
        .bind("register")
        .bind(&labels_json)
        .bind(&sha)
        .execute(&mut *tx)
        .await
        .map_err(|e| map_pg_error("register/insert", e))?;

        // -----------------------------------------------------------------
        // Step 6 — audit emit. The registry owns the audit emission
        // (per D12 design). The handler counts the metric (D13 fix C-1).
        // -----------------------------------------------------------------
        let key = CatalogKey {
            tenant: tenant.to_owned(),
            name: name.to_owned(),
            version: next_v as u32,
        };
        self.audit.emit_register(&key, &metadata);

        tx.commit()
            .await
            .map_err(|e| map_pg_error("register/commit", e))?;

        Ok(RegisteredCatalog {
            key,
            content,
            metadata,
            compiled: None,
        })
    }

    async fn evolve(
        &self,
        from: &CatalogKey,
        op: EvolveOp,
        mut metadata_override: CatalogMetadata,
    ) -> Result<RegisteredCatalog, CatalogError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| map_pg_error("evolve/begin", e))?;

        let lock_id = lock_id_for(&from.tenant, &from.name);
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| map_pg_error("evolve/advisory_lock", e))?;

        // -----------------------------------------------------------------
        // Step 1 — fetch the source version.
        // -----------------------------------------------------------------
        let src_row = sqlx::query(
            "SELECT \
                 tenant, name, version, content_json, \
                 created_at_unix_ms, created_by, description, \
                 parent_key, evolution_op, labels, content_sha256, \
                 compiled_anchors, compiled_classes, embedder_fingerprint \
             FROM catalogs \
             WHERE tenant = $1 AND name = $2 AND version = $3",
        )
        .bind(&from.tenant)
        .bind(&from.name)
        .bind(from.version as i32)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| map_pg_error("evolve/fetch_src", e))?;

        let src_row = src_row.ok_or_else(|| {
            CatalogError::NotFound(format!("{}/{}@v{}", from.tenant, from.name, from.version))
        })?;
        let src = CatalogRow::from_row(&src_row)?;

        // -----------------------------------------------------------------
        // Step 2 — apply the op.
        // -----------------------------------------------------------------
        let mut new_content: CatalogContent = serde_json::from_value(src.content_json)
            .map_err(|e| internal_je("evolve/decode_src_content", e))?;
        apply_evolve_op(&mut new_content, &op)?;
        self.policy.validate_content(&new_content)?;

        // -----------------------------------------------------------------
        // Step 3 — quota check on versions.
        // -----------------------------------------------------------------
        let n_versions: i64 =
            sqlx::query_scalar("SELECT count(*) FROM catalogs WHERE tenant=$1 AND name=$2")
                .bind(&from.tenant)
                .bind(&from.name)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| map_pg_error("evolve/count_versions", e))?;
        if (n_versions as u32) >= self.policy.max_versions_per_name {
            return Err(CatalogError::QuotaExceeded(format!(
                "(tenant={}, name={}) reached max_versions_per_name={}",
                from.tenant, from.name, self.policy.max_versions_per_name
            )));
        }

        // -----------------------------------------------------------------
        // Step 4 — next version = MAX(version) + 1, NOT src.version + 1.
        //
        // This handles the case of two concurrent `evolve` requests
        // racing from the same `from_key`. The advisory lock makes
        // them serial : the first writer commits to v(N+1), the
        // second waiter then sees v(N+1) and writes v(N+2). Both
        // carry `parent_key = from_key` (the original source), which
        // is exactly the architecture spec D12.
        // -----------------------------------------------------------------
        let max_v: i32 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(version), 0) FROM catalogs \
             WHERE tenant = $1 AND name = $2",
        )
        .bind(&from.tenant)
        .bind(&from.name)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| map_pg_error("evolve/max_version", e))?;
        let next_v = max_v + 1;

        // -----------------------------------------------------------------
        // Step 5 — derive parent_key, sha, labels.
        // -----------------------------------------------------------------
        let parent_key = format!("{}/{}@v{}", from.tenant, from.name, from.version);
        let new_content_json = serde_json::to_value(&new_content)
            .map_err(|e| internal_je("evolve/serialise_content", e))?;
        let sha = sha256_hex_of_content(&new_content);
        let labels_json = serde_json::to_value(&metadata_override.labels)
            .map_err(|e| internal_je("evolve/serialise_labels", e))?;

        // Stamp the controlled fields.
        metadata_override.parent_key = parent_key.clone();
        metadata_override.evolution_op = op.audit_tag().to_owned();
        metadata_override.content_sha256 = ContentDigest(sha.clone());

        // -----------------------------------------------------------------
        // Step 6 — INSERT.
        //
        // Note on Recompile : when the embedder changes, the previous
        // compiled_anchors are no longer applicable. We do NOT carry
        // them over to v(N+1) — the column is left NULL, the embedding
        // pool re-compiles on demand.
        // -----------------------------------------------------------------
        sqlx::query(
            "INSERT INTO catalogs ( \
                 tenant, name, version, \
                 content_json, \
                 created_at_unix_ms, created_by, description, \
                 parent_key, evolution_op, labels, content_sha256 \
             ) VALUES ( \
                 $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11 \
             )",
        )
        .bind(&from.tenant)
        .bind(&from.name)
        .bind(next_v)
        .bind(&new_content_json)
        .bind(metadata_override.created_at_unix_ms)
        .bind(&metadata_override.created_by)
        .bind(&metadata_override.description)
        .bind(&parent_key)
        .bind(op.audit_tag())
        .bind(&labels_json)
        .bind(&sha)
        .execute(&mut *tx)
        .await
        .map_err(|e| map_pg_error("evolve/insert", e))?;

        let key = CatalogKey {
            tenant: from.tenant.clone(),
            name: from.name.clone(),
            version: next_v as u32,
        };
        self.audit.emit_evolve(&key, &metadata_override, &op);

        // Suppress unused warnings — `src` is materialised for future
        // use (e.g. recording the prior fingerprint in audit events).
        let _ = src.embedder_fingerprint;
        let _ = src.compiled_anchors;
        let _ = src.compiled_classes;

        tx.commit()
            .await
            .map_err(|e| map_pg_error("evolve/commit", e))?;

        Ok(RegisteredCatalog {
            key,
            content: new_content,
            metadata: metadata_override,
            compiled: None,
        })
    }

    async fn get(
        &self,
        key: &CatalogKey,
        include_compiled: bool,
    ) -> Result<RegisteredCatalog, CatalogError> {
        let row = sqlx::query(
            "SELECT \
                 tenant, name, version, content_json, \
                 created_at_unix_ms, created_by, description, \
                 parent_key, evolution_op, labels, content_sha256, \
                 compiled_anchors, compiled_classes, embedder_fingerprint \
             FROM catalogs \
             WHERE tenant = $1 AND name = $2 AND version = $3",
        )
        .bind(&key.tenant)
        .bind(&key.name)
        .bind(key.version as i32)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| map_pg_error("get", e))?;

        let row = row.ok_or_else(|| {
            CatalogError::NotFound(format!("{}/{}@v{}", key.tenant, key.name, key.version))
        })?;
        let r = CatalogRow::from_row(&row)?;
        r.into_registered(include_compiled)
    }

    async fn list_catalogs(
        &self,
        tenant: &str,
        page_size: u32,
        page_token: Option<&str>,
    ) -> Result<CatalogPage, CatalogError> {
        let page_size_clamped: i64 = page_size.clamp(1, 500) as i64;
        let after_name = match page_token {
            None => String::new(),
            Some(t) => decode_page_token(t)?,
        };

        // Aggregate latest_version + class/anchor counts per
        // (tenant, name). The JSONB extraction is cost-bounded by the
        // composite index and the (tenant) prefix scan ; if bench
        // shows it's the bottleneck, DT-D14.2 (materialised view) is
        // the prepared mitigation.
        //
        // Rationale for `+1` LIMIT : we fetch one extra row to detect
        // whether there is a next page without a separate count(*).
        let rows = sqlx::query(
            "SELECT \
                 latest_per_name.name, \
                 latest_per_name.latest_version, \
                 latest_row.content_json AS latest_content_json \
             FROM ( \
                 SELECT name, MAX(version) AS latest_version \
                 FROM catalogs \
                 WHERE tenant = $1 AND name > $2 \
                 GROUP BY name \
                 ORDER BY name ASC \
                 LIMIT $3 \
             ) AS latest_per_name \
             JOIN catalogs latest_row \
                 ON latest_row.tenant = $1 \
                 AND latest_row.name = latest_per_name.name \
                 AND latest_row.version = latest_per_name.latest_version \
             ORDER BY latest_per_name.name ASC",
        )
        .bind(tenant)
        .bind(&after_name)
        .bind(page_size_clamped + 1)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| map_pg_error("list_catalogs/fetch", e))?;

        let mut catalogs: Vec<CatalogRefSummary> = Vec::with_capacity(rows.len());
        for row in rows.iter().take(page_size_clamped as usize) {
            let name: String = row
                .try_get("name")
                .map_err(|e| internal_je("list/name", e))?;
            let latest_version: i32 = row
                .try_get("latest_version")
                .map_err(|e| internal_je("list/latest_version", e))?;
            let content_json: serde_json::Value = row
                .try_get("latest_content_json")
                .map_err(|e| internal_je("list/content_json", e))?;
            let content: CatalogContent = serde_json::from_value(content_json)
                .map_err(|e| internal_je("list/decode_content", e))?;
            let num_classes = content.classes.len() as u32;
            let num_anchors: u32 = content
                .classes
                .iter()
                .map(|c| c.canonical_sentences.len() as u32)
                .sum();
            catalogs.push(CatalogRefSummary {
                tenant: tenant.to_owned(),
                name,
                latest_version: latest_version as u32,
                num_classes,
                num_anchors,
            });
        }

        let next_page_token = if rows.len() > page_size_clamped as usize {
            // We fetched one beyond the requested page → there's more.
            // Cursor on the *last* item in the returned page.
            catalogs
                .last()
                .map(|c| encode_page_token(&c.name))
                .unwrap_or_default()
        } else {
            String::new()
        };
        Ok(CatalogPage {
            catalogs,
            next_page_token,
        })
    }

    async fn list_versions(
        &self,
        tenant: &str,
        name: &str,
    ) -> Result<Vec<CatalogKey>, CatalogError> {
        let rows = sqlx::query(
            "SELECT version FROM catalogs \
             WHERE tenant = $1 AND name = $2 \
             ORDER BY version ASC",
        )
        .bind(tenant)
        .bind(name)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| map_pg_error("list_versions/fetch", e))?;

        if rows.is_empty() {
            return Err(CatalogError::NotFound(format!("{tenant}/{name}")));
        }

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let version: i32 = row
                .try_get("version")
                .map_err(|e| internal_je("list_versions/version", e))?;
            out.push(CatalogKey {
                tenant: tenant.to_owned(),
                name: name.to_owned(),
                version: version as u32,
            });
        }
        Ok(out)
    }

    async fn delete(
        &self,
        key: &CatalogKey,
        force: bool,
        principal: &str,
    ) -> Result<(), CatalogError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| map_pg_error("delete/begin", e))?;

        let lock_id = lock_id_for(&key.tenant, &key.name);
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| map_pg_error("delete/advisory_lock", e))?;

        let canonical = format!("{}/{}@v{}", key.tenant, key.name, key.version);

        // Reference check : when force=false, a delete is allowed only
        // if no successor row points to this version via parent_key.
        if !force {
            let referenced: bool = sqlx::query_scalar(
                "SELECT EXISTS( \
                     SELECT 1 FROM catalogs \
                     WHERE tenant = $1 AND name = $2 AND parent_key = $3 \
                 )",
            )
            .bind(&key.tenant)
            .bind(&key.name)
            .bind(&canonical)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| map_pg_error("delete/reference_check", e))?;
            if referenced {
                return Err(CatalogError::Conflict(format!(
                    "{canonical} is referenced by a successor ; \
                     use force=true to cascade",
                )));
            }
        }

        // Collect all versions to delete : either just `key.version`
        // (when force=false), or the recursive transitive closure
        // of descendants (when force=true).
        let to_delete: Vec<i32> = if force {
            // Recursive CTE : start from the seed version, follow
            // child rows that have parent_key = "<tenant>/<name>@v<n>"
            // for any n already in the chain.
            let cte_rows = sqlx::query(
                "WITH RECURSIVE chain(version) AS ( \
                     SELECT version FROM catalogs \
                     WHERE tenant = $1 AND name = $2 AND version = $3 \
                     UNION \
                     SELECT c.version FROM catalogs c \
                     JOIN chain ch ON c.parent_key = \
                         $1 || '/' || $2 || '@v' || ch.version \
                     WHERE c.tenant = $1 AND c.name = $2 \
                 ) \
                 SELECT version FROM chain",
            )
            .bind(&key.tenant)
            .bind(&key.name)
            .bind(key.version as i32)
            .fetch_all(&mut *tx)
            .await
            .map_err(|e| map_pg_error("delete/cascade_collect", e))?;
            cte_rows
                .into_iter()
                .map(|r| {
                    r.try_get::<i32, _>("version")
                        .map_err(|e| internal_je("delete/cte_version", e))
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            vec![key.version as i32]
        };

        if to_delete.is_empty() {
            return Err(CatalogError::NotFound(canonical));
        }

        let rows_affected = sqlx::query(
            "DELETE FROM catalogs \
             WHERE tenant = $1 AND name = $2 \
             AND version = ANY($3::integer[])",
        )
        .bind(&key.tenant)
        .bind(&key.name)
        .bind(&to_delete)
        .execute(&mut *tx)
        .await
        .map_err(|e| map_pg_error("delete/apply", e))?
        .rows_affected();

        if rows_affected == 0 {
            return Err(CatalogError::NotFound(canonical));
        }

        // Audit at the trait level — the gRPC handler also bumps the
        // metric `efe_audit_events_total` separately (D13 fix C-1).
        self.audit.emit_delete(key, principal, force);

        tx.commit()
            .await
            .map_err(|e| map_pg_error("delete/commit", e))?;
        Ok(())
    }
}

// =====================================================================
// Pool metrics ticker — exported for `runtime::Server::serve`
// =====================================================================

/// Background task that publishes pool-size gauges to the D14
/// instruments registered in `efe_telemetry::metrics::d14_instruments`.
///
/// Loops every 10 s ; should be `tokio::spawn`-ed alongside the
/// service. Returns on the first shutdown signal received via the
/// passed-in `notify` (cf. [`crate::shutdown::Shutdown`]).
pub async fn pool_metrics_ticker(pool: PgPool, shutdown: crate::shutdown::Shutdown) {
    let mut interval = tokio::time::interval(Duration::from_secs(10));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let total = pool.size() as i64;
                let idle  = pool.num_idle() as i64;
                let active = total - idle;
                efe_telemetry::metrics::d14_instruments::pg_pool_active_connections()
                    .with_label_values(&["catalog"])
                    .set(active);
                efe_telemetry::metrics::d14_instruments::pg_pool_idle_connections()
                    .with_label_values(&["catalog"])
                    .set(idle);
            }
            _ = shutdown.notified() => {
                tracing::info!(
                    target: "efe.catalog.postgres",
                    "shutdown — pool_metrics_ticker exiting",
                );
                return;
            }
        }
    }
}

// =====================================================================
// Tests — unit only (no DB required). DB-dependent tests live in
// `crates/efe-server/tests/d14_postgres_*.rs`.
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CatalogClass, ConePolicy, ThresholdPolicy};

    #[test]
    fn lock_id_is_deterministic_for_same_input() {
        let a = lock_id_for("acme", "billing");
        let b = lock_id_for("acme", "billing");
        assert_eq!(a, b, "lock_id must be stable for identical input");
    }

    #[test]
    fn lock_id_differs_for_different_tenants() {
        let a = lock_id_for("acme", "billing");
        let b = lock_id_for("globex", "billing");
        assert_ne!(a, b);
    }

    #[test]
    fn lock_id_differs_for_different_names() {
        let a = lock_id_for("acme", "billing");
        let b = lock_id_for("acme", "support");
        assert_ne!(a, b);
    }

    #[test]
    fn lock_id_differs_when_separator_makes_difference() {
        // Without the explicit '/' separator, "ac" + "me/billing"
        // would hash identically to "acme" + "billing". Verify the
        // separator matters.
        let a = lock_id_for("acme", "billing");
        let b = lock_id_for("ac", "me/billing");
        assert_ne!(a, b, "the '/' separator must be load-bearing");
    }

    #[test]
    fn sha256_is_deterministic() {
        let c = sample_content();
        let a = sha256_hex_of_content(&c);
        let b = sha256_hex_of_content(&c);
        assert_eq!(a, b);
        assert_eq!(a.len(), 64, "SHA-256 hex is 64 chars");
    }

    #[test]
    fn sha256_differs_when_content_differs() {
        let c1 = sample_content();
        let mut c2 = sample_content();
        c2.classes[0].name = "billing-mod".to_owned();
        let a = sha256_hex_of_content(&c1);
        let b = sha256_hex_of_content(&c2);
        assert_ne!(a, b);
    }

    #[test]
    fn page_token_round_trip() {
        let original = "history";
        let encoded = encode_page_token(original);
        let decoded = decode_page_token(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn page_token_round_trip_on_unicode() {
        let original = "billing-é-😀";
        let encoded = encode_page_token(original);
        let decoded = decode_page_token(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn decode_page_token_rejects_garbage() {
        let err = decode_page_token("!!!").unwrap_err();
        assert!(matches!(err, CatalogError::InvalidArgument(_)));
    }

    #[test]
    fn apply_evolve_op_append_class_appends() {
        let mut c = sample_content();
        let n_before = c.classes.len();
        let new_class = CatalogClass {
            name: "shipping".to_owned(),
            description: "S".to_owned(),
            canonical_sentences: vec!["where is my order".to_owned()],
            color: "#aaa".to_owned(),
            deprecated: false,
        };
        apply_evolve_op(&mut c, &EvolveOp::AppendClass { class: new_class }).unwrap();
        assert_eq!(c.classes.len(), n_before + 1);
        assert_eq!(c.classes.last().unwrap().name, "shipping");
    }

    #[test]
    fn apply_evolve_op_append_class_rejects_duplicate() {
        let mut c = sample_content();
        let dup = c.classes[0].clone();
        let err = apply_evolve_op(&mut c, &EvolveOp::AppendClass { class: dup }).unwrap_err();
        assert!(matches!(err, CatalogError::InvalidArgument(_)));
    }

    #[test]
    fn apply_evolve_op_remove_class_removes() {
        let mut c = sample_content();
        let target = c.classes[0].name.clone();
        let n_before = c.classes.len();
        apply_evolve_op(
            &mut c,
            &EvolveOp::RemoveClass {
                class_name: target.clone(),
            },
        )
        .unwrap();
        assert_eq!(c.classes.len(), n_before - 1);
        assert!(c.classes.iter().all(|cl| cl.name != target));
    }

    #[test]
    fn apply_evolve_op_remove_class_rejects_unknown() {
        let mut c = sample_content();
        let err = apply_evolve_op(
            &mut c,
            &EvolveOp::RemoveClass {
                class_name: "no-such-class".to_owned(),
            },
        )
        .unwrap_err();
        assert!(matches!(err, CatalogError::InvalidArgument(_)));
    }

    #[test]
    fn apply_evolve_op_update_thresholds_replaces() {
        let mut c = sample_content();
        apply_evolve_op(
            &mut c,
            &EvolveOp::UpdateThresholds {
                thresholds: ThresholdPolicy {
                    confidence_high: 0.95,
                    confidence_medium: 0.65,
                },
            },
        )
        .unwrap();
        assert!((c.thresholds.confidence_high - 0.95).abs() < 1e-6);
        assert!((c.thresholds.confidence_medium - 0.65).abs() < 1e-6);
    }

    #[test]
    fn apply_evolve_op_replace_class_anchors_replaces() {
        let mut c = sample_content();
        let target = c.classes[0].name.clone();
        apply_evolve_op(
            &mut c,
            &EvolveOp::ReplaceClassAnchors {
                class_name: target.clone(),
                canonical_sentences: vec!["new-1".to_owned(), "new-2".to_owned()],
            },
        )
        .unwrap();
        let cls = c.classes.iter().find(|cl| cl.name == target).unwrap();
        assert_eq!(
            cls.canonical_sentences,
            vec!["new-1".to_owned(), "new-2".to_owned()]
        );
    }

    #[test]
    fn apply_evolve_op_replace_class_anchors_rejects_unknown() {
        let mut c = sample_content();
        let err = apply_evolve_op(
            &mut c,
            &EvolveOp::ReplaceClassAnchors {
                class_name: "no-such-class".to_owned(),
                canonical_sentences: vec!["x".to_owned()],
            },
        )
        .unwrap_err();
        assert!(matches!(err, CatalogError::InvalidArgument(_)));
    }

    #[test]
    fn apply_evolve_op_replace_class_anchors_rejects_empty_sentences() {
        let mut c = sample_content();
        let target = c.classes[0].name.clone();
        let err = apply_evolve_op(
            &mut c,
            &EvolveOp::ReplaceClassAnchors {
                class_name: target,
                canonical_sentences: vec![],
            },
        )
        .unwrap_err();
        assert!(matches!(err, CatalogError::InvalidArgument(_)));
    }

    #[test]
    fn apply_evolve_op_recompile_swaps_embedder() {
        let mut c = sample_content();
        apply_evolve_op(
            &mut c,
            &EvolveOp::Recompile {
                new_embedder: EmbedderSpec {
                    family: "st".into(),
                    model: "mpnet".into(),
                    dim: 768,
                    normalize: "l2".into(),
                },
            },
        )
        .unwrap();
        assert_eq!(c.embedder.dim, 768);
        assert_eq!(c.embedder.model, "mpnet");
    }

    fn sample_content() -> CatalogContent {
        CatalogContent {
            embedder: EmbedderSpec {
                family: "sentence-transformers".into(),
                model: "all-MiniLM-L6-v2".into(),
                dim: 384,
                normalize: "l2".into(),
            },
            classes: vec![
                CatalogClass {
                    name: "billing".into(),
                    description: "B".into(),
                    canonical_sentences: vec!["refund please".into()],
                    color: "#000".into(),
                    deprecated: false,
                },
                CatalogClass {
                    name: "support".into(),
                    description: "S".into(),
                    canonical_sentences: vec!["it doesn't work".into()],
                    color: "#111".into(),
                    deprecated: false,
                },
            ],
            cone_policy: ConePolicy {
                max_projection_distance: 0.7,
            },
            thresholds: ThresholdPolicy {
                confidence_high: 0.85,
                confidence_medium: 0.6,
            },
        }
    }
}
