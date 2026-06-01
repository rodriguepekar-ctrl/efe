// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : `POST /v1/agentic/call` — § 9.3.

use axum::{extract::State, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use efe_agentic_bridge::{AgenticAuditContext, AgenticTask, BridgeError};
use efe_nano_sdk::{ContextBundle, NanoAgentId, NanoAgentVersion, PiiStrategy, TenantId};

use crate::auth::layer::Claims;
use crate::error::ApiError;
use crate::state::AppState;
use crate::tenancy::check_tenant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgenticCallRequest {
    pub tenant_id: Uuid,
    pub bundle: ContextBundle,
    pub task: AgenticTask,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgenticCallResponse {
    pub text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost_microcents: i64,
    pub model_provider: String,
    pub model_id: String,
}

pub async fn agentic_call_handler(
    State(state): State<AppState>,
    Claims(claims): Claims,
    Json(req): Json<AgenticCallRequest>,
) -> Result<impl IntoResponse, ApiError> {
    check_tenant(&claims, req.tenant_id)?;
    // **N26 + G11 + I12** : OVERWRITE `audit_context.tenant_id`
    // with the JWT claim's tenant.  A malicious client cannot
    // impersonate another tenant in the audit chain via this
    // field.  If the client did not supply an audit_context, we
    // populate a default one tied to the JWT tenant.
    let mut task = req.task;
    let jwt_tenant = TenantId::from_uuid(claims.tenant_id);
    match task.audit_context.as_mut() {
        Some(ctx) => {
            if ctx.tenant_id != jwt_tenant {
                tracing::warn!(
                    jwt_tenant = %claims.tenant_id,
                    supplied_tenant = ?ctx.tenant_id,
                    "N26 G11 audit_context.tenant_id overwritten by JWT claim"
                );
                ctx.tenant_id = jwt_tenant;
            }
        }
        None => {
            task.audit_context = Some(default_audit_context(jwt_tenant, &req.bundle));
        }
    }
    let resp = state
        .agentic
        .call_with_context(&req.bundle, task)
        .await
        .map_err(map_bridge_err)?;
    Ok(Json(AgenticCallResponse {
        text: resp.text,
        input_tokens: resp.usage.input_tokens,
        output_tokens: resp.usage.output_tokens,
        cost_microcents: resp.cost_microcents,
        model_provider: resp.model_identity.provider,
        model_id: resp.model_identity.model_id,
    }))
}

/// **N26 + Annex A3** : default audit context for an
/// `agentic_proxy` direct call.  Used when the client does not
/// supply one (production safety — every Claude dispatch carries
/// a trace).  The redaction token set is read from the bundle's
/// in-flight redactions so PII attribution survives the call.
fn default_audit_context(tenant: TenantId, bundle: &ContextBundle) -> AgenticAuditContext {
    let redaction_token_set: Vec<String> = bundle
        .redactions
        .iter()
        .map(|r| r.redaction_token.clone())
        .collect();
    let pii_strategy_applied = if bundle.redactions.is_empty() {
        PiiStrategy::None
    } else {
        PiiStrategy::PreLlmRedact
    };
    AgenticAuditContext {
        trace_id: uuid::Uuid::new_v4(),
        swarm_request_id: uuid::Uuid::new_v4(),
        op_id: "agentic_proxy".to_string(),
        tenant_id: tenant,
        agent_id: NanoAgentId::new("agentic_proxy")
            .expect("static agent id `agentic_proxy` is valid"),
        agent_version: NanoAgentVersion::new("1.0.0")
            .expect("static version `1.0.0` is valid"),
        prompt_template_id: "agentic_proxy_v1".to_string(),
        prompt_template_version: "1.0.0".to_string(),
        pii_strategy_applied,
        redaction_token_set,
        publisher_id: None,
        publisher_kyc_tier: None,
        gold_case_id: None,
    }
}

/// **N27** : pub re-export of [`map_bridge_err`] for the SSE
/// handler in `agentic_stream.rs`.  Kept as a function (not a
/// `pub use`) so the buffered handler keeps `map_bridge_err`
/// private for clarity, while exposing the shared mapping.
pub(crate) fn map_bridge_err_pub(e: BridgeError) -> ApiError {
    map_bridge_err(e)
}

fn map_bridge_err(e: BridgeError) -> ApiError {
    match e {
        BridgeError::RateLimited => {
            ApiError::DependencyUnavailable("upstream LLM rate-limited".into())
        }
        BridgeError::Authentication => {
            ApiError::Internal("upstream LLM auth failed".into())
        }
        BridgeError::ProtocolViolation { message } => {
            ApiError::Internal(format!("upstream LLM protocol : {message}"))
        }
        BridgeError::BundleTooLarge { size_bytes, max_bytes } => {
            ApiError::PayloadTooLarge(size_bytes, max_bytes)
        }
        BridgeError::Backend { message } => ApiError::DependencyUnavailable(message),
        // **N26 + I5** : pre-call provenance verification failure
        // → fail closed.  Returned to the client as 503 since
        // the LLM dispatch was rejected upstream of the network.
        BridgeError::ProvenanceVerifyFailed { reason } => {
            ApiError::DependencyUnavailable(format!(
                "provenance verify failed before dispatch : {reason}"
            ))
        }
        // **Sprint 19 — N27** : streaming + retry surface errors.
        BridgeError::Cancelled => ApiError::Internal("upstream stream cancelled".into()),
        BridgeError::TotalTimeout => {
            ApiError::DependencyUnavailable("upstream LLM total timeout".into())
        }
        BridgeError::ReadTimeout => {
            ApiError::DependencyUnavailable("upstream LLM read timeout".into())
        }
        BridgeError::PrematureEof => {
            ApiError::DependencyUnavailable("upstream LLM stream ended prematurely".into())
        }
        BridgeError::SseParseError { reason } => {
            ApiError::Internal(format!("SSE parse : {reason}"))
        }
        BridgeError::InvalidResponseType { content_type } => {
            ApiError::Internal(format!("upstream LLM bad content-type : {content_type}"))
        }
        BridgeError::ToolUseJsonInvalid { reason } => {
            ApiError::Internal(format!("tool-use JSON invalid : {reason}"))
        }
        BridgeError::RetryAfterFirstByte => {
            ApiError::Internal("retry refused after first byte downstream".into())
        }
        BridgeError::HttpStatus { status, body } => {
            // 4xx → DependencyUnavailable (the upstream rejected the call) ;
            // 5xx → DependencyUnavailable ; anything else → Internal.
            if (400..500).contains(&status) || (500..600).contains(&status) {
                ApiError::DependencyUnavailable(format!("upstream HTTP {status} : {body}"))
            } else {
                ApiError::Internal(format!("unexpected HTTP {status} : {body}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::SwarmId;

    #[test]
    fn request_serde_round_trip() {
        let r = AgenticCallRequest {
            tenant_id: Uuid::new_v4(),
            bundle: ContextBundle::empty(SwarmId::new()),
            task: AgenticTask::new("hello"),
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: AgenticCallRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tenant_id, r.tenant_id);
    }

    #[test]
    fn map_bridge_err_rate_limited_503() {
        assert!(matches!(
            map_bridge_err(BridgeError::RateLimited),
            ApiError::DependencyUnavailable(_)
        ));
    }

    #[test]
    fn map_bridge_err_bundle_too_large_413() {
        let e = BridgeError::BundleTooLarge {
            size_bytes: 500_000,
            max_bytes: 256 * 1024,
        };
        assert!(matches!(map_bridge_err(e), ApiError::PayloadTooLarge(_, _)));
    }
}
