// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : `POST /v1/swarms/run` handler — § 9.2.

use std::time::Instant;

use axum::{extract::State, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use efe_nano_sdk::SwarmId;
use efe_swarm_orchestrator::{SwarmRunConfig, SwarmRunError};

use crate::auth::layer::Claims;
use crate::error::ApiError;
use crate::state::AppState;
use crate::tenancy::check_tenant;

/// **N22** : POST /v1/swarms/run body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmRunRequest {
    /// Tenant id MUST match jwt.tenant_id (G6).
    pub tenant_id: Uuid,
    /// DAG identifier (Sprint 14 : caller-supplied, Sprint 15 :
    /// registry lookup).
    #[serde(default)]
    pub dag_id: Option<String>,
    /// Free-form input handed to every agent's `SwarmAgentContext.
    /// input`.
    pub input: serde_json::Value,
    /// Optional config overrides (timeout, tenant_tier label).
    #[serde(default)]
    pub config: Option<SwarmRunConfigOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SwarmRunConfigOverride {
    #[serde(default)]
    pub global_timeout_ms: Option<u32>,
    #[serde(default)]
    pub emit_envelope: Option<bool>,
    #[serde(default)]
    pub rng_seed: Option<u64>,
    #[serde(default)]
    pub tenant_tier_label: Option<String>,
}

/// **N22** : POST /v1/swarms/run response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmRunResponse {
    pub swarm_id: SwarmId,
    pub status: String,
    pub agents_completed: usize,
    pub agents_errored: usize,
    pub total_cost_microcents: i64,
    pub duration_ms: u64,
    pub bundle: efe_nano_sdk::ContextBundle,
}

/// **N22 + § 10.1 Workflow A** : run a swarm DAG.
///
/// Sprint 14 simplification : the DAG itself is NOT looked up by
/// `dag_id` ; instead, the gateway requires the caller to have
/// pre-registered a DAG via the lib API.  Sprint 15 adds a real
/// DAG registry.
///
/// For Sprint 14 we return 503 if no DAG matches `dag_id` ; the
/// pilot use case shipping in Sprint 14 hard-codes a single
/// reference DAG which the orchestrator runs by default if `dag_id`
/// is `None`.
pub async fn swarms_run_handler(
    State(state): State<AppState>,
    Claims(claims): Claims,
    Json(req): Json<SwarmRunRequest>,
) -> Result<impl IntoResponse, ApiError> {
    // Tenant isolation enforcement (G6).
    check_tenant(&claims, req.tenant_id)?;

    // Build the SwarmRunConfig from defaults + overrides.
    let mut config = SwarmRunConfig::default();
    if let Some(o) = req.config.clone() {
        if let Some(t) = o.global_timeout_ms {
            config.global_timeout_ms = t;
        }
        if let Some(e) = o.emit_envelope {
            config.emit_envelope = e;
        }
        if let Some(s) = o.rng_seed {
            config.rng_seed = Some(s);
        }
        if let Some(l) = o.tenant_tier_label {
            config.tenant_tier_label = l;
        }
    }
    config.swarm_id = SwarmId::new();
    config.dag_id_label = req
        .dag_id
        .clone()
        .unwrap_or_else(|| "default".to_string());

    // **Sprint 14 jour 2 fix** : look up the DAG via the built-in
    // `dag_registry` module.  Default DAG is the 6-agent pre-
    // analysis pipeline ; custom DAGs deferred to Sprint 15.
    let started = Instant::now();
    // **N24** : if the compliance runner is wired, use the
    // per-tier policy DAG ; otherwise fall back to the N23
    // experimental_fast DAG.
    let lookup = if let Some(runner) = state.compliance.as_ref() {
        crate::dag_registry::lookup_dag_wired(req.dag_id.as_deref(), runner.as_ref())
    } else {
        crate::dag_registry::lookup_dag(req.dag_id.as_deref())
    };
    let dag = match lookup {
        crate::dag_registry::DagLookup::Found(d) => d,
        crate::dag_registry::DagLookup::NotFound => {
            return Err(ApiError::NotFound(format!(
                "DAG {:?} not registered ; Sprint 14 ships the 'default' \
                 reference DAG only — Sprint 15 will add a custom registry",
                req.dag_id.clone().unwrap_or_else(|| "default".into())
            )));
        }
    };

    let report = state
        .swarm
        .run(&dag, req.input, config)
        .await
        .map_err(map_swarm_err)?;

    let elapsed_ms = started.elapsed().as_millis() as u64;
    state.metrics.observe_duration_ms("POST", "/v1/swarms/run", elapsed_ms);

    Ok(Json(SwarmRunResponse {
        swarm_id: report.swarm_id,
        status: report.overall.label().into(),
        agents_completed: report.agents_completed(),
        agents_errored: report.agents_errored(),
        total_cost_microcents: report.total_cost_microcents,
        duration_ms: report.duration_ms(),
        bundle: report.bundle,
    }))
}

fn map_swarm_err(e: SwarmRunError) -> ApiError {
    match e {
        SwarmRunError::GlobalTimeout { ms } => {
            ApiError::UpstreamTimeout(format!("swarm timeout {ms} ms"))
        }
        SwarmRunError::AggregatorFailed { message } => ApiError::Internal(message),
        SwarmRunError::AggregatorNotInDag => {
            ApiError::Internal("aggregator missing from DAG".into())
        }
        SwarmRunError::Backend { message } => ApiError::DependencyUnavailable(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_serde_round_trip() {
        let r = SwarmRunRequest {
            tenant_id: Uuid::new_v4(),
            dag_id: Some("pdf-analyser".into()),
            input: serde_json::json!({"text": "x"}),
            config: Some(SwarmRunConfigOverride {
                global_timeout_ms: Some(10_000),
                emit_envelope: Some(true),
                rng_seed: Some(42),
                tenant_tier_label: Some("tier_2".into()),
            }),
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: SwarmRunRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.dag_id, r.dag_id);
        assert_eq!(back.tenant_id, r.tenant_id);
    }

    #[test]
    fn config_override_default_all_none() {
        let o = SwarmRunConfigOverride::default();
        assert!(o.global_timeout_ms.is_none());
        assert!(o.emit_envelope.is_none());
    }

    #[test]
    fn map_swarm_err_global_timeout_504() {
        let e = SwarmRunError::GlobalTimeout { ms: 30_000 };
        let mapped = map_swarm_err(e);
        assert!(matches!(mapped, ApiError::UpstreamTimeout(_)));
    }

    #[test]
    fn map_swarm_err_backend_503() {
        let e = SwarmRunError::Backend {
            message: "pg down".into(),
        };
        let mapped = map_swarm_err(e);
        assert!(matches!(mapped, ApiError::DependencyUnavailable(_)));
    }
}
