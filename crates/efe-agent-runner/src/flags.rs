//! D93 — production `TenantFlagsProvider` backed by Postgres.
//!
//! Reads the `tenant_feature` row for a given tenant and maps it to a
//! `TenantAgentRuntimeFlags`. Used by both :
//!
//! 1. `efe-agent-runner` controller binary (consumes the flags through
//!    the gRPC service layer when reconciling AgentRun CRs).
//! 2. `efe-server` REST gateway (consumes the flags through the
//!    in-process `AgentRuntimeService` wired up in `main.rs` when
//!    `EFE_AGENT_REST_ENABLED=true`).
//!
//! Schema-level details :
//!
//! - Migration 0018 added the M26 graduation columns on `tenant_feature`
//!   (`m26_graduation_acknowledged_at`, `m26_graduation_revoked_at`).
//!   When a tenant has been revoked from STABLE, this provider downgrades
//!   `agent_runtime_enabled` to `false` regardless of the M25 flag — so
//!   the service returns `M25NotEnabledForTenant` cleanly without leaking
//!   the partial-graduation state to the caller.
//! - `allowed_profiles` / `allowed_providers` are stored as TEXT arrays
//!   (`text[]`) in PG ; we deserialise via `sqlx::types::Json` for
//!   compatibility with the existing migration scripts.
//!
//! Caching : intentionally NONE at this layer. The provider is hot enough
//! in the request path that a stale flag value (e.g. DPIA revoked but
//! cache not yet invalidated) is a P1 incident. Callers that need batch
//! lookups should compose a higher-level cache with explicit TTL.

use crate::service::TenantFlagsProvider;
use crate::{AgentRunnerError, AgentRunnerResult, TenantAgentRuntimeFlags, TenantId};
use chrono::{DateTime, Utc};
use sqlx::PgPool;

/// PG-backed `TenantFlagsProvider` for production.
///
/// Holds the pool by `Arc` clone (cheap) — no Mutex needed because
/// `sqlx::PgPool` is internally synchronised.
#[derive(Debug, Clone)]
pub struct PgTenantFlagsProvider {
    pg: PgPool,
}

impl PgTenantFlagsProvider {
    pub fn new(pg: PgPool) -> Self {
        Self { pg }
    }
}

/// Raw row shape, matches the M25 + M26 columns we care about.
///
/// Notes on the columns :
/// - `m26_graduation_acknowledged_at` / `m26_graduation_revoked_at`
///   are added by migration 0018 and may be NULL on tenants who never
///   completed M26 graduation.
/// - `m25_allowed_profiles` and `m25_allowed_providers` are TEXT[]
///   columns in Postgres. `sqlx::query_as` maps them directly to
///   `Vec<String>` when the row decoder is set up correctly.
/// - `m25_microvm_pool_size` may be NULL on legacy rows — we coerce
///   NULL → `2` (a conservative default that matches `Default::default()`
///   on `TenantAgentRuntimeFlags`).
#[derive(sqlx::FromRow, Debug)]
struct TenantFeatureRow {
    m25_agent_runtime_enabled: Option<bool>,
    m25_dpia_signed_at: Option<DateTime<Utc>>,
    m25_dpia_expires_at: Option<DateTime<Utc>>,
    m25_dpia_addendum_hash: Option<String>,
    m25_cost_cap_per_run_usd: Option<f64>,
    m25_timeout_minutes_max: Option<i32>,
    m25_allowed_profiles: Option<Vec<String>>,
    m25_allowed_providers: Option<Vec<String>>,
    m25_memory_enabled: Option<bool>,
    m25_microvm_pool_size: Option<i32>,
    m25_max_concurrent_runs: Option<i32>,
    m26_graduation_acknowledged_at: Option<DateTime<Utc>>,
    m26_graduation_revoked_at: Option<DateTime<Utc>>,
}

impl TenantFeatureRow {
    fn into_flags(self) -> TenantAgentRuntimeFlags {
        // D93 — M26 graduation guard.
        //
        // The runtime is only enabled if BOTH conditions hold :
        //   1. The M25 flag is true (operator has flipped the switch).
        //   2. The tenant either has never graduated OR is currently
        //      acknowledged AND not currently revoked.
        //
        // The graduation revocation takes precedence — if revoked_at >
        // acknowledged_at, the tenant is OFF regardless of M25 flag.
        let revoked_after_ack = match (self.m26_graduation_revoked_at, self.m26_graduation_acknowledged_at) {
            (Some(rev), Some(ack)) => rev > ack,
            (Some(_), None) => true,
            _ => false,
        };
        let runtime_enabled =
            self.m25_agent_runtime_enabled.unwrap_or(false) && !revoked_after_ack;

        TenantAgentRuntimeFlags {
            agent_runtime_enabled: runtime_enabled,
            dpia_signed_at: self.m25_dpia_signed_at,
            dpia_expires_at: self.m25_dpia_expires_at,
            dpia_addendum_hash: self.m25_dpia_addendum_hash,
            cost_cap_per_run_usd: self.m25_cost_cap_per_run_usd.unwrap_or(0.0),
            timeout_minutes_max: self
                .m25_timeout_minutes_max
                .filter(|v| *v > 0)
                .map(|v| v as u32)
                .unwrap_or(60),
            allowed_profiles: self.m25_allowed_profiles.unwrap_or_default(),
            allowed_providers: self.m25_allowed_providers.unwrap_or_default(),
            memory_enabled: self.m25_memory_enabled.unwrap_or(false),
            microvm_pool_size: self
                .m25_microvm_pool_size
                .filter(|v| *v > 0)
                .map(|v| v as u32)
                .unwrap_or(2),
            max_concurrent_runs: self
                .m25_max_concurrent_runs
                .filter(|v| *v > 0)
                .map(|v| v as u32)
                .unwrap_or(5),
        }
    }
}

#[async_trait::async_trait]
impl TenantFlagsProvider for PgTenantFlagsProvider {
    async fn flags_for(&self, tenant: &TenantId) -> AgentRunnerResult<TenantAgentRuntimeFlags> {
        // Single SELECT on `tenant_feature` — RLS is enforced by the
        // session var `app.current_tenant_id` set upstream by the auth
        // middleware. We don't want to risk a query that crosses
        // tenants here, hence we bind the tenant id explicitly even
        // though RLS would also filter it out.
        let row: Option<TenantFeatureRow> = sqlx::query_as(
            r#"SELECT
                  m25_agent_runtime_enabled,
                  m25_dpia_signed_at,
                  m25_dpia_expires_at,
                  m25_dpia_addendum_hash,
                  m25_cost_cap_per_run_usd,
                  m25_timeout_minutes_max,
                  m25_allowed_profiles,
                  m25_allowed_providers,
                  m25_memory_enabled,
                  m25_microvm_pool_size,
                  m25_max_concurrent_runs,
                  m26_graduation_acknowledged_at,
                  m26_graduation_revoked_at
               FROM tenant_feature
               WHERE id = $1"#,
        )
        .bind(&tenant.0)
        .fetch_optional(&self.pg)
        .await?;

        match row {
            Some(r) => Ok(r.into_flags()),
            // Missing row : treat as "feature off for this tenant".
            // The service layer surfaces this as
            // `M25NotEnabledForTenant`, which is the correct UX.
            None => Err(AgentRunnerError::M25NotEnabledForTenant(tenant.0.clone())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn base_row() -> TenantFeatureRow {
        TenantFeatureRow {
            m25_agent_runtime_enabled: Some(true),
            m25_dpia_signed_at: Some(Utc::now() - Duration::days(30)),
            m25_dpia_expires_at: Some(Utc::now() + Duration::days(335)),
            m25_dpia_addendum_hash: Some("abc123".into()),
            m25_cost_cap_per_run_usd: Some(5.0),
            m25_timeout_minutes_max: Some(60),
            m25_allowed_profiles: Some(vec!["research_lite".into()]),
            m25_allowed_providers: Some(vec!["openai".into()]),
            m25_memory_enabled: Some(false),
            m25_microvm_pool_size: Some(4),
            m25_max_concurrent_runs: Some(5),
            m26_graduation_acknowledged_at: None,
            m26_graduation_revoked_at: None,
        }
    }

    #[test]
    fn happy_path_maps_all_fields() {
        let f = base_row().into_flags();
        assert!(f.agent_runtime_enabled);
        assert!(f.dpia_valid());
        assert_eq!(f.cost_cap_per_run_usd, 5.0);
        assert_eq!(f.timeout_minutes_max, 60);
        assert_eq!(f.allowed_profiles, vec!["research_lite".to_string()]);
        assert_eq!(f.allowed_providers, vec!["openai".to_string()]);
        assert_eq!(f.microvm_pool_size, 4);
        assert_eq!(f.max_concurrent_runs, 5);
    }

    #[test]
    fn null_optional_fields_use_defaults() {
        let mut r = base_row();
        r.m25_cost_cap_per_run_usd = None;
        r.m25_timeout_minutes_max = None;
        r.m25_microvm_pool_size = None;
        r.m25_max_concurrent_runs = None;
        r.m25_allowed_profiles = None;
        let f = r.into_flags();
        assert_eq!(f.cost_cap_per_run_usd, 0.0);
        assert_eq!(f.timeout_minutes_max, 60);
        assert_eq!(f.microvm_pool_size, 2);
        assert_eq!(f.max_concurrent_runs, 5);
        assert!(f.allowed_profiles.is_empty());
    }

    #[test]
    fn graduation_revoked_after_ack_disables_runtime() {
        let mut r = base_row();
        r.m26_graduation_acknowledged_at = Some(Utc::now() - Duration::days(10));
        r.m26_graduation_revoked_at = Some(Utc::now() - Duration::days(1));
        let f = r.into_flags();
        assert!(
            !f.agent_runtime_enabled,
            "tenant revoked after ack must have runtime disabled"
        );
    }

    #[test]
    fn graduation_ack_after_revoke_re_enables_runtime() {
        // Edge case : tenant revoked then re-acknowledged.
        let mut r = base_row();
        r.m26_graduation_revoked_at = Some(Utc::now() - Duration::days(10));
        r.m26_graduation_acknowledged_at = Some(Utc::now() - Duration::days(1));
        let f = r.into_flags();
        assert!(
            f.agent_runtime_enabled,
            "ack-after-revoke should re-enable the runtime"
        );
    }

    #[test]
    fn revoked_without_ack_disables() {
        let mut r = base_row();
        r.m26_graduation_revoked_at = Some(Utc::now() - Duration::days(1));
        let f = r.into_flags();
        assert!(!f.agent_runtime_enabled);
    }

    #[test]
    fn m25_disabled_flag_overrides_everything() {
        let mut r = base_row();
        r.m25_agent_runtime_enabled = Some(false);
        let f = r.into_flags();
        assert!(!f.agent_runtime_enabled);
    }

    #[test]
    fn negative_pool_size_falls_back_to_default() {
        let mut r = base_row();
        r.m25_microvm_pool_size = Some(-1);
        let f = r.into_flags();
        assert_eq!(f.microvm_pool_size, 2);
    }
}
