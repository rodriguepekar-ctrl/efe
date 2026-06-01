//! Sprint **D15** — Tenant registry CRUD layer.
//!
//! Wraps the `tenants` table (`migrations/0004_tenants.sql`) with a
//! sqlx-backed repository used by :
//!   * the `efe-onboard` CLI (preflight, activate, suspend, list) ;
//!   * the OIDC `tenant_strategy = "issuer"` lookup at boot ;
//!   * the periodic `efe_tenants_total{status}` gauge refresh.
//!
//! The schema is fixed by the architecture doc § 4 :
//!
//! ```text
//! tenants(tenant_id PK, display_name, status, idp_issuer,
//!         idp_strategy, idp_meta JSONB, created_at, activated_at,
//!         suspended_at)
//! ```
//!
//! Status FSM (architecture § 4.2) :
//!
//! ```text
//!     ┌─ pending ─┐
//!     ▼           ▼
//!   active ───► suspended
//!     ▲           │
//!     └───────────┘   (re-activation allowed)
//! ```
//!
//! Re-activation is permitted because the audit trail (D14-bis) keeps
//! every transition signed and hash-chained — no information is lost
//! by toggling `status` back to `active`.
//!
//! # Why a dedicated module instead of reusing `catalog`
//!
//! Tenants are *governance metadata*, not a versioned domain object —
//! they have no class hierarchy, no anchors, no policy versioning.
//! Putting them in `catalog` would force them through evolve_to /
//! cone_fit machinery they don't need. Keeping them next to the auth
//! subsystem also means the OIDC `IssuerEntry` lookup can call the
//! store directly without a dependency on the catalog crate.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPool;
use sqlx::Row;
use thiserror::Error;

// =====================================================================
// Domain types
// =====================================================================

/// The three valid lifecycle states (mirror the SQL CHECK constraint).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TenantStatus {
    /// Created but not yet activated — appears in `efe-onboard list`
    /// but tokens for that tenant are rejected.
    Pending,
    /// Activated — RPCs accept tokens for this tenant.
    Active,
    /// Suspended (deliberately or after a contract end) — tokens are
    /// rejected with `permission_denied`.
    Suspended,
}

impl TenantStatus {
    /// String form for SQL.
    #[must_use]
    pub fn as_sql(self) -> &'static str {
        match self {
            TenantStatus::Pending => "pending",
            TenantStatus::Active => "active",
            TenantStatus::Suspended => "suspended",
        }
    }

    /// Parse from SQL.
    pub fn from_sql(s: &str) -> Result<Self, TenantStoreError> {
        match s {
            "pending" => Ok(TenantStatus::Pending),
            "active" => Ok(TenantStatus::Active),
            "suspended" => Ok(TenantStatus::Suspended),
            other => Err(TenantStoreError::Inconsistent(format!(
                "unknown tenant status in db: {other}"
            ))),
        }
    }
}

/// IdP resolution strategy stored alongside the tenant — the OIDC
/// validator reuses [`super::issuers::TenantStrategy`] but the SQL
/// column needs its own enum because we want serde-snake_case here
/// (the JSON / row representation) without polluting the auth-time
/// type with `strum` derives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdpStrategy {
    /// Tenant id read from a JWT custom claim (default).
    Claim,
    /// Tenant id is fixed by the issuer URL (one IdP / org / realm
    /// per tenant).
    Issuer,
}

impl IdpStrategy {
    /// String form for SQL.
    #[must_use]
    pub fn as_sql(self) -> &'static str {
        match self {
            IdpStrategy::Claim => "claim",
            IdpStrategy::Issuer => "issuer",
        }
    }

    /// Parse from SQL.
    pub fn from_sql(s: &str) -> Result<Self, TenantStoreError> {
        match s {
            "claim" => Ok(IdpStrategy::Claim),
            "issuer" => Ok(IdpStrategy::Issuer),
            other => Err(TenantStoreError::Inconsistent(format!(
                "unknown idp_strategy in db: {other}"
            ))),
        }
    }
}

/// One row of the `tenants` table — domain-friendly form (SQL
/// timestamps mapped to `chrono::DateTime<Utc>` since the workspace
/// already pulls `chrono` transitively via sqlx's `chrono` feature).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tenant {
    /// Canonical tenant id (regex `^[a-z0-9_-]{1,63}$`).
    pub tenant_id: String,
    /// Human-friendly name shown in the dashboard / CLI.
    pub display_name: String,
    /// Lifecycle state.
    pub status: TenantStatus,
    /// OIDC issuer URL (e.g. `https://acme.auth0.com/`).
    pub idp_issuer: String,
    /// How OIDC resolves the tenant id.
    pub idp_strategy: IdpStrategy,
    /// Vendor-specific blob (Auth0 org id, Okta authz server id, …).
    pub idp_meta: serde_json::Value,
    /// Row creation time (UTC).
    pub created_at: DateTime<Utc>,
    /// Time the tenant was first marked `active`. `None` while
    /// `pending`.
    pub activated_at: Option<DateTime<Utc>>,
    /// Time the tenant was last suspended. `None` if never suspended.
    pub suspended_at: Option<DateTime<Utc>>,
}

/// New-tenant payload used by `TenantStore::register`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewTenant {
    /// Canonical tenant id (regex `^[a-z0-9_-]{1,63}$`).
    pub tenant_id: String,
    /// Human-friendly name.
    pub display_name: String,
    /// OIDC issuer URL.
    pub idp_issuer: String,
    /// IdP resolution strategy.
    pub idp_strategy: IdpStrategy,
    /// Vendor-specific blob (defaults to `{}`).
    #[serde(default)]
    pub idp_meta: serde_json::Value,
}

// =====================================================================
// Errors
// =====================================================================

/// Failure modes of the tenant CRUD layer.
#[derive(Debug, Error)]
pub enum TenantStoreError {
    /// `register` was called with a tenant_id that already exists.
    #[error("tenant already registered: {0}")]
    AlreadyExists(String),

    /// `activate` / `suspend` / `get` was called for a non-existent
    /// tenant_id.
    #[error("tenant not found: {0}")]
    NotFound(String),

    /// Argument format violation (regex, empty fields).
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// State machine violation (e.g. `activate` when already
    /// suspended without re-onboarding) — the FSM allows
    /// `pending → active` and `active ↔ suspended` ; everything
    /// else is rejected.
    #[error("invalid status transition for {tenant_id}: {from:?} → {to:?}")]
    InvalidTransition {
        /// Tenant whose transition was rejected.
        tenant_id: String,
        /// Current status fetched from SQL.
        from: TenantStatus,
        /// Requested target status.
        to: TenantStatus,
    },

    /// Row inserted via direct SQL has a value that does not match
    /// any known enum variant.
    #[error("inconsistent row in tenants table: {0}")]
    Inconsistent(String),

    /// Underlying sqlx error (network, syntax, constraint).
    #[error("database error: {0}")]
    Database(String),
}

impl From<sqlx::Error> for TenantStoreError {
    fn from(e: sqlx::Error) -> Self {
        // sqlx surfaces UNIQUE violations as DatabaseError with a
        // PG-specific code "23505". Map that to AlreadyExists when
        // we can ; otherwise classify as a generic database error.
        if let sqlx::Error::Database(db_err) = &e {
            if db_err.code().as_deref() == Some("23505") {
                if let Some(constraint) = db_err.constraint() {
                    return TenantStoreError::AlreadyExists(format!("constraint {constraint}"));
                }
                return TenantStoreError::AlreadyExists(db_err.message().to_string());
            }
        }
        TenantStoreError::Database(e.to_string())
    }
}

// =====================================================================
// TenantStore — the public facade
// =====================================================================

/// `Arc`-wrapped handle injected into the CLI and the runtime.
#[derive(Clone)]
pub struct TenantStore {
    inner: Arc<TenantStoreInner>,
}

struct TenantStoreInner {
    pool: PgPool,
}

impl std::fmt::Debug for TenantStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TenantStore")
            .field("pool_size", &self.inner.pool.size())
            .finish()
    }
}

impl TenantStore {
    /// Wrap an already-built sqlx pool. The pool MUST point at a DB
    /// where `migrations/0004_tenants.sql` has been applied — call
    /// `crate::catalog::PostgresCatalogRegistry::run_migrations` (or
    /// equivalent) before instantiating the store.
    #[must_use]
    pub fn from_pool(pool: PgPool) -> Self {
        Self {
            inner: Arc::new(TenantStoreInner { pool }),
        }
    }

    /// Pool accessor — used by the CLI to share the pool across
    /// commands without re-connecting.
    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.inner.pool
    }

    // -----------------------------------------------------------
    // CRUD operations
    // -----------------------------------------------------------

    /// Register a new tenant in `pending` state. Idempotent w.r.t.
    /// duplicates : returns [`TenantStoreError::AlreadyExists`] if
    /// the tenant_id already exists.
    pub async fn register(&self, new: NewTenant) -> Result<Tenant, TenantStoreError> {
        validate_tenant_id(&new.tenant_id)?;
        validate_nonempty(&new.display_name, "display_name")?;
        validate_nonempty(&new.idp_issuer, "idp_issuer")?;

        let row = sqlx::query(
            r#"
            INSERT INTO tenants (
                tenant_id, display_name, status, idp_issuer,
                idp_strategy, idp_meta
            )
            VALUES ($1, $2, 'pending', $3, $4, $5)
            RETURNING tenant_id, display_name, status, idp_issuer,
                      idp_strategy, idp_meta, created_at, activated_at,
                      suspended_at
            "#,
        )
        .bind(&new.tenant_id)
        .bind(&new.display_name)
        .bind(&new.idp_issuer)
        .bind(new.idp_strategy.as_sql())
        .bind(if new.idp_meta.is_null() {
            serde_json::json!({})
        } else {
            new.idp_meta.clone()
        })
        .fetch_one(&self.inner.pool)
        .await?;

        row_to_tenant(&row)
    }

    /// Lookup a tenant by id. Returns [`TenantStoreError::NotFound`]
    /// if absent.
    pub async fn get(&self, tenant_id: &str) -> Result<Tenant, TenantStoreError> {
        validate_tenant_id(tenant_id)?;
        let row = sqlx::query(
            r#"
            SELECT tenant_id, display_name, status, idp_issuer,
                   idp_strategy, idp_meta, created_at, activated_at,
                   suspended_at
            FROM tenants
            WHERE tenant_id = $1
            "#,
        )
        .bind(tenant_id)
        .fetch_optional(&self.inner.pool)
        .await?;
        match row {
            Some(r) => row_to_tenant(&r),
            None => Err(TenantStoreError::NotFound(tenant_id.to_string())),
        }
    }

    /// Lookup a tenant by IdP issuer URL. Used by the OIDC
    /// `tenant_strategy = "issuer"` validator. Returns the *first*
    /// row matching `idp_issuer = $1` ; collisions are detected at
    /// `register` time (UNIQUE not enforced — we leave that to
    /// `efe-onboard` business logic).
    pub async fn get_by_issuer(&self, issuer: &str) -> Result<Option<Tenant>, TenantStoreError> {
        validate_nonempty(issuer, "issuer")?;
        let row = sqlx::query(
            r#"
            SELECT tenant_id, display_name, status, idp_issuer,
                   idp_strategy, idp_meta, created_at, activated_at,
                   suspended_at
            FROM tenants
            WHERE idp_issuer = $1
            ORDER BY created_at ASC
            LIMIT 1
            "#,
        )
        .bind(issuer)
        .fetch_optional(&self.inner.pool)
        .await?;
        row.as_ref().map(row_to_tenant).transpose()
    }

    /// List all tenants, ordered by `created_at` ASC. Used by the CLI
    /// `efe-onboard list` and the gauge refresh job.
    pub async fn list(&self) -> Result<Vec<Tenant>, TenantStoreError> {
        let rows = sqlx::query(
            r#"
            SELECT tenant_id, display_name, status, idp_issuer,
                   idp_strategy, idp_meta, created_at, activated_at,
                   suspended_at
            FROM tenants
            ORDER BY created_at ASC
            "#,
        )
        .fetch_all(&self.inner.pool)
        .await?;
        rows.iter().map(row_to_tenant).collect()
    }

    /// Count tenants per status. Cheaper than `list` for the gauge
    /// refresh job because it avoids decoding every row.
    pub async fn count_by_status(&self) -> Result<[(TenantStatus, i64); 3], TenantStoreError> {
        let rows = sqlx::query(
            r#"
            SELECT status, COUNT(*) AS n
            FROM tenants
            GROUP BY status
            "#,
        )
        .fetch_all(&self.inner.pool)
        .await?;
        let mut out = [
            (TenantStatus::Pending, 0_i64),
            (TenantStatus::Active, 0_i64),
            (TenantStatus::Suspended, 0_i64),
        ];
        for r in rows.iter() {
            let s: String = r
                .try_get("status")
                .map_err(|e| TenantStoreError::Database(format!("status column missing: {e}")))?;
            let st = TenantStatus::from_sql(&s)?;
            let n: i64 = r
                .try_get("n")
                .map_err(|e| TenantStoreError::Database(format!("n column missing: {e}")))?;
            for slot in out.iter_mut() {
                if slot.0 == st {
                    slot.1 = n;
                }
            }
        }
        Ok(out)
    }

    // -----------------------------------------------------------
    // FSM transitions
    // -----------------------------------------------------------

    /// Transition `pending → active` or `suspended → active`.
    /// Refuses if the tenant is already `active` (no-op semantics
    /// would mask CLI typos).
    pub async fn activate(&self, tenant_id: &str) -> Result<Tenant, TenantStoreError> {
        validate_tenant_id(tenant_id)?;
        let mut tx = self.inner.pool.begin().await?;
        let cur: Tenant = match sqlx::query(
            r#"
            SELECT tenant_id, display_name, status, idp_issuer,
                   idp_strategy, idp_meta, created_at, activated_at,
                   suspended_at
            FROM tenants
            WHERE tenant_id = $1
            FOR UPDATE
            "#,
        )
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        {
            Some(r) => row_to_tenant(&r)?,
            None => {
                tx.rollback().await.ok();
                return Err(TenantStoreError::NotFound(tenant_id.to_string()));
            }
        };
        match cur.status {
            TenantStatus::Active => {
                tx.rollback().await.ok();
                return Err(TenantStoreError::InvalidTransition {
                    tenant_id: tenant_id.to_string(),
                    from: TenantStatus::Active,
                    to: TenantStatus::Active,
                });
            }
            TenantStatus::Pending | TenantStatus::Suspended => {}
        }
        let row = sqlx::query(
            r#"
            UPDATE tenants
            SET status = 'active',
                activated_at = COALESCE(activated_at, now())
            WHERE tenant_id = $1
            RETURNING tenant_id, display_name, status, idp_issuer,
                      idp_strategy, idp_meta, created_at, activated_at,
                      suspended_at
            "#,
        )
        .bind(tenant_id)
        .fetch_one(&mut *tx)
        .await?;
        let updated = row_to_tenant(&row)?;
        tx.commit().await?;
        Ok(updated)
    }

    /// **Audit fix D15-C1** — fast `SELECT status FROM tenants
    /// WHERE tenant_id = $1` for the OIDC validator hot path.
    ///
    /// Returns `Ok(true)` iff the row exists and `status = 'active'`.
    /// Anything else (NotFound, pending, suspended) → `Ok(false)`.
    /// Returns `Err` only on DB-level failures (network, serialisation).
    ///
    /// Cheaper than `get(...)` because it doesn't decode the full row.
    pub async fn is_tenant_active(&self, tenant_id: &str) -> Result<bool, TenantStoreError> {
        if validate_tenant_id(tenant_id).is_err() {
            // Format violation → not active. We don't surface the
            // error here because the validator already rejected
            // mal-formatted tenant_ids upstream ; this branch only
            // hardens the function against direct misuse.
            return Ok(false);
        }
        let row: Option<String> =
            sqlx::query_scalar(r#"SELECT status FROM tenants WHERE tenant_id = $1"#)
                .bind(tenant_id)
                .fetch_optional(&self.inner.pool)
                .await?;
        match row {
            Some(s) if s == "active" => Ok(true),
            Some(_) | None => Ok(false),
        }
    }

    /// Transition `pending → suspended` or `active → suspended`.
    /// No-op-into-error if already suspended.
    pub async fn suspend(&self, tenant_id: &str) -> Result<Tenant, TenantStoreError> {
        validate_tenant_id(tenant_id)?;
        let mut tx = self.inner.pool.begin().await?;
        let cur: Tenant = match sqlx::query(
            r#"
            SELECT tenant_id, display_name, status, idp_issuer,
                   idp_strategy, idp_meta, created_at, activated_at,
                   suspended_at
            FROM tenants
            WHERE tenant_id = $1
            FOR UPDATE
            "#,
        )
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        {
            Some(r) => row_to_tenant(&r)?,
            None => {
                tx.rollback().await.ok();
                return Err(TenantStoreError::NotFound(tenant_id.to_string()));
            }
        };
        if cur.status == TenantStatus::Suspended {
            tx.rollback().await.ok();
            return Err(TenantStoreError::InvalidTransition {
                tenant_id: tenant_id.to_string(),
                from: TenantStatus::Suspended,
                to: TenantStatus::Suspended,
            });
        }
        let row = sqlx::query(
            r#"
            UPDATE tenants
            SET status = 'suspended',
                suspended_at = now()
            WHERE tenant_id = $1
            RETURNING tenant_id, display_name, status, idp_issuer,
                      idp_strategy, idp_meta, created_at, activated_at,
                      suspended_at
            "#,
        )
        .bind(tenant_id)
        .fetch_one(&mut *tx)
        .await?;
        let updated = row_to_tenant(&row)?;
        tx.commit().await?;
        Ok(updated)
    }
}

// =====================================================================
// TenantStatusGate impl — D15-C1 wiring into OidcValidator
// =====================================================================

#[async_trait::async_trait]
impl super::oidc::TenantStatusGate for TenantStore {
    async fn is_active(&self, tenant_id: &str) -> Result<bool, String> {
        self.is_tenant_active(tenant_id)
            .await
            .map_err(|e| e.to_string())
    }
}

// =====================================================================
// Row → Tenant conversion + format guards
// =====================================================================

fn row_to_tenant(row: &sqlx::postgres::PgRow) -> Result<Tenant, TenantStoreError> {
    let tenant_id: String = row.try_get("tenant_id").map_err(map_get_err)?;
    let display_name: String = row.try_get("display_name").map_err(map_get_err)?;
    let status_s: String = row.try_get("status").map_err(map_get_err)?;
    let status = TenantStatus::from_sql(&status_s)?;
    let idp_issuer: String = row.try_get("idp_issuer").map_err(map_get_err)?;
    let idp_strategy_s: String = row.try_get("idp_strategy").map_err(map_get_err)?;
    let idp_strategy = IdpStrategy::from_sql(&idp_strategy_s)?;
    let idp_meta: serde_json::Value = row.try_get("idp_meta").map_err(map_get_err)?;
    let created_at: DateTime<Utc> = row.try_get("created_at").map_err(map_get_err)?;
    let activated_at: Option<DateTime<Utc>> = row.try_get("activated_at").map_err(map_get_err)?;
    let suspended_at: Option<DateTime<Utc>> = row.try_get("suspended_at").map_err(map_get_err)?;
    Ok(Tenant {
        tenant_id,
        display_name,
        status,
        idp_issuer,
        idp_strategy,
        idp_meta,
        created_at,
        activated_at,
        suspended_at,
    })
}

fn map_get_err(e: sqlx::Error) -> TenantStoreError {
    TenantStoreError::Database(format!("row decode failed: {e}"))
}

/// `^[a-z0-9_-]{1,63}$` — same as the SQL CHECK and the OIDC tenant
/// resolver. Centralised here so the CLI / SQL / OIDC paths cannot
/// drift.
pub fn validate_tenant_id(s: &str) -> Result<(), TenantStoreError> {
    if s.is_empty() {
        return Err(TenantStoreError::InvalidArgument(
            "tenant_id must not be empty".into(),
        ));
    }
    if s.len() > 63 {
        return Err(TenantStoreError::InvalidArgument(format!(
            "tenant_id too long: {} chars (max 63)",
            s.len()
        )));
    }
    if !s
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    {
        return Err(TenantStoreError::InvalidArgument(format!(
            "tenant_id {s:?} must match ^[a-z0-9_-]{{1,63}}$"
        )));
    }
    Ok(())
}

fn validate_nonempty(s: &str, field: &str) -> Result<(), TenantStoreError> {
    if s.trim().is_empty() {
        return Err(TenantStoreError::InvalidArgument(format!(
            "{field} must not be empty"
        )));
    }
    Ok(())
}

// =====================================================================
// Tests — pure-format / FSM unit tests (no DB)
// =====================================================================
//
// The SQL layer is exercised by the `d15_tenants_postgres.rs`
// integration test under `tests/`. Here we only cover the validators
// and the enum round-trips ; that lets the suite stay green even on a
// machine without a Postgres instance.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_id_regex_accepts_canonical() {
        for s in [
            "acme",
            "acme-corp",
            "acme_42",
            "a",
            &"a".repeat(63),
            "0123456789",
        ] {
            validate_tenant_id(s).unwrap_or_else(|e| panic!("{s:?} should be valid: {e}"));
        }
    }

    #[test]
    fn tenant_id_regex_rejects_non_canonical() {
        for s in [
            "",
            &"a".repeat(64),
            "Acme",      // uppercase
            "acme!",     // punctuation
            "acme corp", // space
            "acmé",      // unicode
            "acme.corp", // dot
            "acme/corp", // slash
        ] {
            assert!(validate_tenant_id(s).is_err(), "{s:?} should be rejected");
        }
    }

    #[test]
    fn tenant_status_round_trip() {
        for st in [
            TenantStatus::Pending,
            TenantStatus::Active,
            TenantStatus::Suspended,
        ] {
            let s = st.as_sql();
            assert_eq!(TenantStatus::from_sql(s).unwrap(), st);
        }
    }

    #[test]
    fn tenant_status_unknown_is_inconsistent() {
        let err = TenantStatus::from_sql("retired").unwrap_err();
        assert!(matches!(err, TenantStoreError::Inconsistent(_)));
    }

    #[test]
    fn idp_strategy_round_trip() {
        for s in [IdpStrategy::Claim, IdpStrategy::Issuer] {
            assert_eq!(IdpStrategy::from_sql(s.as_sql()).unwrap(), s);
        }
    }

    #[test]
    fn idp_strategy_unknown_rejected() {
        assert!(IdpStrategy::from_sql("magic").is_err());
    }

    #[test]
    fn nonempty_validator_trims() {
        assert!(validate_nonempty("   ", "x").is_err());
        assert!(validate_nonempty("", "x").is_err());
        assert!(validate_nonempty("ok", "x").is_ok());
    }

    #[test]
    fn invalid_transition_displays_both_states() {
        let e = TenantStoreError::InvalidTransition {
            tenant_id: "acme".into(),
            from: TenantStatus::Active,
            to: TenantStatus::Active,
        };
        let s = format!("{e}");
        assert!(s.contains("acme"));
        assert!(s.contains("Active"));
    }

    #[test]
    fn new_tenant_default_meta_serialises() {
        let nt = NewTenant {
            tenant_id: "acme".into(),
            display_name: "Acme Corp".into(),
            idp_issuer: "https://acme.auth0.com/".into(),
            idp_strategy: IdpStrategy::Claim,
            idp_meta: serde_json::Value::Null,
        };
        let json = serde_json::to_string(&nt).unwrap();
        assert!(json.contains("\"tenant_id\":\"acme\""));
        assert!(json.contains("\"idp_strategy\":\"claim\""));
    }

    #[test]
    fn new_tenant_deserialises_with_default_meta() {
        let json = r#"{
            "tenant_id": "acme",
            "display_name": "Acme",
            "idp_issuer": "https://acme.auth0.com/",
            "idp_strategy": "issuer"
        }"#;
        let nt: NewTenant = serde_json::from_str(json).unwrap();
        assert_eq!(nt.tenant_id, "acme");
        assert_eq!(nt.idp_strategy, IdpStrategy::Issuer);
        assert!(nt.idp_meta.is_null());
    }
}
