// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : `/readyz` — readiness probe — § 9.1 + I17.

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
pub struct ReadyzResponse {
    pub status: &'static str,
    pub subsystems: Vec<SubsystemStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubsystemStatus {
    pub name: String,
    pub ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// **N22 + I17** : Readiness probe.
///
/// Returns 200 if every critical subsystem responds.  Sprint 14
/// considers the gateway "ready" as long as the swarm orchestrator,
/// agentic bridge, publisher API and gold API are wired (Arc-
/// reachable).  Sprint 15 adds Postgres pool ping.
pub async fn readyz_handler(State(_state): State<AppState>) -> Response {
    // Sprint 14 stub : if AppState is constructed at all, the Arc
    // fields are non-null.  Always 200.  Sprint 15 will ping each.
    let subsystems = vec![
        SubsystemStatus {
            name: "swarm_orchestrator".into(),
            ready: true,
            detail: None,
        },
        SubsystemStatus {
            name: "agentic_bridge".into(),
            ready: true,
            detail: None,
        },
        SubsystemStatus {
            name: "publisher_api".into(),
            ready: true,
            detail: None,
        },
        SubsystemStatus {
            name: "gold_api".into(),
            ready: true,
            detail: None,
        },
        SubsystemStatus {
            name: "jwt_validator".into(),
            ready: true,
            detail: None,
        },
    ];
    let all_ready = subsystems.iter().all(|s| s.ready);
    let status_code = if all_ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let body = ReadyzResponse {
        status: if all_ready { "ready" } else { "degraded" },
        subsystems,
    };
    (status_code, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsystem_status_serde_round_trip() {
        let s = SubsystemStatus {
            name: "x".into(),
            ready: false,
            detail: Some("unreachable".into()),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("unreachable"));
        let back: SubsystemStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "x");
        assert!(!back.ready);
    }
}
