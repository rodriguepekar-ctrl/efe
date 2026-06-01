// SPDX-License-Identifier: UNLICENSED
//! **Sprint 18 — N26 + G13 + Workflow D** :
//! `GET /v1/llm-provenance` — operator-scoped read endpoint
//! that exposes the existing N17 store via the gateway.
//!
//! Sprint 18 ships a minimal surface : a tenant-id-filtered list
//! within an optional date window.  Sprint 19+ adds the
//! `agent_id` / `publisher_id` / `gold_case_id` filters per
//! architecture § 9.2.

use axum::{
    extract::{Query, State},
    response::IntoResponse,
    Json,
};
use efe_llm_provenance::DateRange;
use efe_nano_sdk::{LlmProvenanceRecord, ModelIdentity, TenantId};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use uuid::Uuid;

use crate::auth::layer::{operator_only_or_403, Claims};
use crate::error::ApiError;
use crate::state::AppState;

/// **N26 + ADR-9** : default page size for the listing.
pub const DEFAULT_PAGE_SIZE: usize = 50;
/// **N26 + ADR-9** : hard maximum page size.
pub const MAX_PAGE_SIZE: usize = 500;

/// **N26 + G13** : query parameters for `GET /v1/llm-provenance`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmProvenanceQuery {
    /// **N26** : tenant id to scope the listing to.  Operators
    /// MAY query other tenants ; if absent, the JWT claim's
    /// tenant is used as a safe default.
    #[serde(default)]
    pub tenant_id: Option<Uuid>,
    /// **N26** : earliest `recorded_at` ; RFC-3339.
    #[serde(default)]
    pub since: Option<String>,
    /// **N26** : latest `recorded_at` (exclusive) ; RFC-3339.
    #[serde(default)]
    pub until: Option<String>,
    /// **N26** : optional `swarm_request_id` filter (Sprint 18
    /// only — Sprint 19 N27 adds the other dimensions).
    #[serde(default)]
    pub swarm_request_id: Option<Uuid>,
    /// **N26 + ADR-9** : page size.  Defaults to
    /// [`DEFAULT_PAGE_SIZE`] ; capped at [`MAX_PAGE_SIZE`].
    #[serde(default)]
    pub limit: Option<usize>,
}

/// **N26 + G13** : response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmProvenanceListResponse {
    /// Wire schema literal — `"llm_provenance.v1"`.
    pub schema: String,
    /// UTC RFC-3339 emission timestamp.
    pub generated_at: String,
    /// Tenant id the query is scoped to.
    pub tenant_id: Uuid,
    /// Date window (UTC RFC-3339).
    pub since: String,
    pub until: String,
    /// Records sorted by `recorded_at` descending.
    pub records: Vec<LlmProvenanceRecord>,
    /// Effective page size (capped).
    pub limit: usize,
}

/// **N26 + G13** : `GET /v1/llm-provenance` handler.
///
/// - Auth : operator scope required.
/// - Returns 503 when no store is configured (Sprint 16 N24
///   fallback mode — no LLM provenance available).
/// - Sprint 18 implementation : when `swarm_request_id` is
///   provided, uses `store.list_by_swarm`.  Otherwise lists
///   records by model with a sweeping date range (the N17
///   in-memory store has no full tenant scan ; Sprint 19 N27
///   adds the dedicated `list_by_tenant` method).
pub async fn llm_provenance_handler(
    State(state): State<AppState>,
    Claims(claims): Claims,
    Query(query): Query<LlmProvenanceQuery>,
) -> Result<impl IntoResponse, ApiError> {
    operator_only_or_403(&claims)?;

    let store = state.llm_provenance_store.as_ref().ok_or_else(|| {
        ApiError::DependencyUnavailable(
            "llm provenance store not configured ; the gateway is running without N26 \
             wiring (set EFE_GATEWAY_DISABLE_PROVENANCE != 1)".into(),
        )
    })?;

    let tenant_uuid = query.tenant_id.unwrap_or(claims.tenant_id);
    let tenant = TenantId::from_uuid(tenant_uuid);

    let now = SystemTime::now();
    let since_st = parse_rfc3339(query.since.as_deref())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let until_st = parse_rfc3339(query.until.as_deref()).unwrap_or(now);
    let window = DateRange {
        from: since_st,
        to: until_st,
    };

    let limit = query
        .limit
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .min(MAX_PAGE_SIZE);

    // Sprint 18 implementation : a `swarm_request_id` filter is
    // the cheapest path through the in-memory store.  Without
    // it, we fall back to a model-scoped listing using a sweep
    // of common Anthropic models.
    let mut records: Vec<LlmProvenanceRecord> =
        if let Some(swarm_id) = query.swarm_request_id {
            store
                .list_by_swarm(swarm_id, tenant)
                .await
                .map_err(|e| ApiError::Internal(format!("provenance store : {e}")))?
        } else {
            // Sweep the three known Anthropic models.  Sprint 19
            // N27 replaces this with a dedicated tenant scan.
            let mut acc = Vec::new();
            for model in ["claude-sonnet-4-6", "claude-haiku-3-5", "claude-opus-4"] {
                let id = ModelIdentity::hosted("anthropic", model, "2023-06-01");
                if let Ok(rows) = store.list_by_model(&id, tenant, window).await {
                    acc.extend(rows);
                }
            }
            acc
        };

    // Sort by recorded_at DESC + clamp to limit.
    records.sort_by(|a, b| b.recorded_at.cmp(&a.recorded_at));
    if records.len() > limit {
        records.truncate(limit);
    }

    Ok(Json(LlmProvenanceListResponse {
        schema: "llm_provenance.v1".to_string(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        tenant_id: tenant_uuid,
        since: chrono::DateTime::<chrono::Utc>::from(since_st).to_rfc3339(),
        until: chrono::DateTime::<chrono::Utc>::from(until_st).to_rfc3339(),
        records,
        limit,
    }))
}

fn parse_rfc3339(s: Option<&str>) -> Option<SystemTime> {
    let s = s?;
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_default_is_empty() {
        let q = LlmProvenanceQuery::default();
        assert!(q.tenant_id.is_none());
        assert!(q.limit.is_none());
    }

    #[test]
    fn query_serde_round_trip() {
        let q = LlmProvenanceQuery {
            tenant_id: Some(Uuid::new_v4()),
            since: Some("2026-05-01T00:00:00Z".to_string()),
            until: Some("2026-05-31T23:59:59Z".to_string()),
            swarm_request_id: Some(Uuid::new_v4()),
            limit: Some(100),
        };
        let json = serde_json::to_string(&q).unwrap();
        let back: LlmProvenanceQuery = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tenant_id, q.tenant_id);
        assert_eq!(back.limit, q.limit);
    }

    #[test]
    fn response_serde_round_trip() {
        let r = LlmProvenanceListResponse {
            schema: "llm_provenance.v1".into(),
            generated_at: "2026-05-22T10:00:00Z".into(),
            tenant_id: Uuid::nil(),
            since: "2026-05-01T00:00:00Z".into(),
            until: "2026-05-31T23:59:59Z".into(),
            records: vec![],
            limit: 50,
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: LlmProvenanceListResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tenant_id, r.tenant_id);
        assert_eq!(back.limit, r.limit);
    }

    #[test]
    fn parse_rfc3339_valid_returns_some() {
        let st = parse_rfc3339(Some("2026-05-22T10:00:00Z"));
        assert!(st.is_some());
    }

    #[test]
    fn parse_rfc3339_invalid_returns_none() {
        assert!(parse_rfc3339(Some("not a timestamp")).is_none());
    }

    #[test]
    fn parse_rfc3339_none_returns_none() {
        assert!(parse_rfc3339(None).is_none());
    }

    #[test]
    fn max_page_size_constants_consistent() {
        assert_eq!(DEFAULT_PAGE_SIZE, 50);
        assert_eq!(MAX_PAGE_SIZE, 500);
    }
}
