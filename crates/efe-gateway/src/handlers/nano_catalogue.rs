// SPDX-License-Identifier: UNLICENSED
//! **Sprint 15 — N23 + G13 + Workflow C** : `GET /v1/nano-catalogue`
//! — returns the merged nano-agent catalogue visible to the
//! authenticated tenant.
//!
//! Sprint 15 ships the built-in `efe` namespace (six pre-analysis
//! agents).  Tenant-published nanos land in Sprint 16+ alongside
//! the publish endpoint (`POST /v1/publishers/<id>/publish-nano`).
//! The response shape already carries the `tenant_id` field so
//! callers can disambiguate per-tenant view.

use axum::{extract::State, response::IntoResponse, Json};
use efe_nano_preanalysis::default_catalogue;
use efe_nano_sdk::NanoAgent;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::layer::Claims;
use crate::error::ApiError;
use crate::state::AppState;

/// **N23 + ADR-9** : `GET /v1/nano-catalogue` response shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NanoCatalogueResponse {
    /// Wire schema version literal — `"1.0"` for Sprint 15.
    pub version: String,
    /// UTC RFC-3339 timestamp at which the snapshot was rendered.
    pub generated_at: String,
    /// The tenant id from the JWT — `None` for the operator
    /// namespace, `Some(uuid)` for tenant-scoped requests.
    pub tenant_id: Option<Uuid>,
    /// Built-in (`efe`) + tenant-scoped (`tenant/<uuid>`) entries
    /// the caller is authorised to see.
    pub entries: Vec<NanoAgent>,
    /// Canonical SHA-256 hash of the built-in catalogue (I20 —
    /// callers can detect drift between deployments).
    pub builtin_canonical_hash: String,
}

/// **N23 + G13** : `GET /v1/nano-catalogue` handler.
///
/// - Auth : Bearer JWT required (any tenant or operator token).
/// - Body : `NanoCatalogueResponse` with the full built-in
///   catalogue (six entries in Sprint 15).
/// - Errors : 401 missing/invalid auth (handled upstream by the
///   `Claims` extractor) ; 5xx only on canonical-hash failure.
pub async fn nano_catalogue_handler(
    State(_state): State<AppState>,
    Claims(claims): Claims,
) -> Result<impl IntoResponse, ApiError> {
    // The built-in catalogue is always returned ; tenant-scoped
    // entries land in Sprint 16+ when the publish endpoint
    // exists.  The `tenant_id` field surfaces the JWT subject so
    // callers can audit who issued the read.
    let tenant_id = if claims.tenant_id.is_nil() {
        None
    } else {
        Some(claims.tenant_id)
    };

    let entries: Vec<NanoAgent> = default_catalogue()
        .into_iter()
        .map(|(nano, _runner)| (*nano).clone())
        .collect();

    let response = NanoCatalogueResponse {
        version: "1.0".to_string(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        tenant_id,
        entries,
        builtin_canonical_hash: efe_nano_preanalysis::canonical_hash(),
    };

    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_serde_round_trips() {
        let r = NanoCatalogueResponse {
            version: "1.0".into(),
            generated_at: "2026-05-21T12:00:00Z".into(),
            tenant_id: Some(Uuid::nil()),
            entries: vec![],
            builtin_canonical_hash: "abc".into(),
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: NanoCatalogueResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.version, "1.0");
        assert_eq!(back.entries.len(), 0);
    }

    #[test]
    fn response_shape_includes_canonical_hash_for_drift_detection() {
        let r = NanoCatalogueResponse {
            version: "1.0".into(),
            generated_at: chrono::Utc::now().to_rfc3339(),
            tenant_id: None,
            entries: vec![],
            builtin_canonical_hash: efe_nano_preanalysis::canonical_hash(),
        };
        // Hash is a 64-char hex SHA-256 ; ensure it survives serde.
        let json = serde_json::to_string(&r).unwrap();
        let back: NanoCatalogueResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.builtin_canonical_hash.len(), 64);
    }
}
