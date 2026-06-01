// SPDX-License-Identifier: UNLICENSED
//! **Sprint 19 — N27** : `POST /v1/agentic/call-stream` — SSE
//! handler.  See architecture § 5 (sequence diagram), § 8.7
//! (handler skeleton).
//!
//! Behaviour summary :
//! * Authenticated (JWT) ; same `check_tenant` + audit-context
//!   overwrite as the buffered endpoint (G11).
//! * Opens the bridge's streaming surface and forwards the
//!   downstream-visible [`AgenticEvent`] variants as SSE events.
//! * Emits `: keep-alive\n\n` heartbeats every 15 s (`KeepAlive`)
//!   so HTTP/2 proxies stay alive on slow generations.
//! * Tolerates client disconnect : downstream channel drop
//!   propagates via the mpsc → cancellation back into the
//!   bridge (the bridge's spawned task observes the closed
//!   sender and emits a `Completeness::Aborted` record).

use std::convert::Infallible;
use std::time::Duration;

use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
    Json,
};
use futures::stream::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use efe_agentic_bridge::{
    AbortReason, AgenticAuditContext, AgenticEvent, BridgeError,
};
use efe_nano_sdk::{ContextBundle, NanoAgentId, NanoAgentVersion, PiiStrategy, TenantId};

use crate::auth::layer::Claims;
use crate::error::ApiError;
use crate::handlers::agentic::AgenticCallRequest;
use crate::state::AppState;
use crate::tenancy::check_tenant;

/// **N27 + § 17.1** : downstream keep-alive interval.  Default
/// 15 s ; overridable via `EFE_GATEWAY_SSE_KEEPALIVE_MS`.
pub const N27_DEFAULT_SSE_KEEPALIVE_MS: u64 = 15_000;

fn sse_keepalive_interval() -> Duration {
    let ms = std::env::var("EFE_GATEWAY_SSE_KEEPALIVE_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(N27_DEFAULT_SSE_KEEPALIVE_MS);
    Duration::from_millis(ms)
}

/// **N27 + § 17.1 + § 17.4 rollout plan** : helper that reads
/// the env var directly.  Used by `main.rs` to populate the
/// `AppState::stream_endpoint_enabled` flag once at boot.  The
/// handler itself reads from the state field (not env) so tests
/// can configure per-instance without racing on global env.
pub fn stream_endpoint_enabled_from_env() -> bool {
    !matches!(
        std::env::var("EFE_BRIDGE_STREAM_ENABLED").as_deref(),
        Ok("0")
    )
}

/// **N27** : SSE handler.
///
/// Returns an `axum::response::sse::Sse` so the response status
/// is 200 with `content-type: text/event-stream`.  Errors that
/// happen before any event is sent (auth, bundle-too-large, JSON
/// parse) propagate as a regular RFC 7807 problem response.
pub async fn agentic_call_stream_handler(
    State(state): State<AppState>,
    Claims(claims): Claims,
    Json(req): Json<AgenticCallRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    // **N27 + § 17.1 kill-switch** : the boot-time-resolved
    // `AppState::stream_endpoint_enabled` flag (read from env
    // `EFE_BRIDGE_STREAM_ENABLED` in main.rs) gates the endpoint.
    // The route remains in the router so OpenAPI consumers can
    // still discover it, but every request returns 404.  Used
    // during canary rollback per architecture § 17.5.
    if !state.stream_endpoint_enabled {
        tracing::warn!(
            "N27 streaming endpoint disabled by AppState::stream_endpoint_enabled=false"
        );
        return Err(ApiError::NotFound(
            "/v1/agentic/call-stream is disabled (EFE_BRIDGE_STREAM_ENABLED=0)".into(),
        ));
    }
    // Same auth / tenant-scoping discipline as the buffered handler
    // — see N26 + G11 + I12 commentary in `agentic.rs`.
    check_tenant(&claims, req.tenant_id)?;
    let mut task = req.task;
    let jwt_tenant = TenantId::from_uuid(claims.tenant_id);
    match task.audit_context.as_mut() {
        Some(ctx) => {
            if ctx.tenant_id != jwt_tenant {
                tracing::warn!(
                    jwt_tenant = %claims.tenant_id,
                    supplied_tenant = ?ctx.tenant_id,
                    "N27 G11 audit_context.tenant_id overwritten by JWT claim"
                );
                ctx.tenant_id = jwt_tenant;
            }
        }
        None => {
            task.audit_context = Some(default_audit_context_stream(jwt_tenant, &req.bundle));
        }
    }

    // Open the bridge stream.  Pre-stream failures (bundle too
    // large, pre-call provenance verify failure) become RFC 7807
    // responses here — the SSE wire never opens.
    let bridge_stream = state
        .agentic
        .call_with_context_stream(&req.bundle, task)
        .await
        .map_err(map_bridge_err)?;

    // Filter to downstream-visible events + map to axum SSE.
    let sse_stream = bridge_stream
        .filter_map(|ev| async move {
            if !ev.is_downstream_visible() {
                return None;
            }
            Some(Ok::<_, Infallible>(encode_event(ev)))
        });

    Ok(Sse::new(sse_stream).keep_alive(
        KeepAlive::new()
            .interval(sse_keepalive_interval())
            .text("keep-alive"),
    ))
}

/// **N27** : convert an [`AgenticEvent`] into an axum SSE
/// [`Event`].  Three event names :
/// * `delta` — incremental text ; data is a raw UTF-8 string.
/// * `done`  — terminal success ; data is a JSON object with
///   token usage + cost + ttft + retry count.
/// * `error` — terminal failure ; data is a JSON object with
///   the error kind + message.
fn encode_event(ev: AgenticEvent) -> Event {
    match ev {
        AgenticEvent::TextDelta(text) => Event::default().event("delta").data(text),
        AgenticEvent::Done {
            stop_reason,
            total_usage,
            model_identity,
            cost_microcents,
            time_to_first_token_ms,
            provider_response_time_ms,
            retry_count,
            retry_outcome,
        } => {
            let payload = serde_json::json!({
                "stop_reason": stop_reason.label(),
                "input_tokens": total_usage.input_tokens,
                "output_tokens": total_usage.output_tokens,
                "cost_microcents": cost_microcents,
                "model_provider": model_identity.provider,
                "model_id": model_identity.model_id,
                "time_to_first_token_ms": time_to_first_token_ms,
                "provider_response_time_ms": provider_response_time_ms,
                "retry_count": retry_count,
                "retry_outcome": retry_outcome.label(),
            });
            Event::default()
                .event("done")
                .json_data(payload)
                .unwrap_or_else(|_| Event::default().event("done").data(""))
        }
        AgenticEvent::Error(e) => {
            let payload = serde_json::json!({
                "kind": bridge_error_kind(&e),
                "message": e.to_string(),
            });
            Event::default()
                .event("error")
                .json_data(payload)
                .unwrap_or_else(|_| Event::default().event("error").data(e.to_string()))
        }
        _ => Event::default().event("internal").data("ignored"),
    }
}

/// **N27** : stable `kind` string for the `event: error` payload.
fn bridge_error_kind(e: &BridgeError) -> &'static str {
    match e {
        BridgeError::RateLimited => "rate_limited",
        BridgeError::Authentication => "authentication",
        BridgeError::ProtocolViolation { .. } => "protocol_violation",
        BridgeError::BundleTooLarge { .. } => "bundle_too_large",
        BridgeError::Backend { .. } => "backend",
        BridgeError::ProvenanceVerifyFailed { .. } => "provenance_verify_failed",
        BridgeError::Cancelled => "cancelled",
        BridgeError::TotalTimeout => "total_timeout",
        BridgeError::ReadTimeout => "read_timeout",
        BridgeError::PrematureEof => "premature_eof",
        BridgeError::SseParseError { .. } => "sse_parse_error",
        BridgeError::InvalidResponseType { .. } => "invalid_response_type",
        BridgeError::ToolUseJsonInvalid { .. } => "tool_use_json_invalid",
        BridgeError::RetryAfterFirstByte => "retry_after_first_byte",
        BridgeError::HttpStatus { .. } => "http_status",
    }
}

/// **N27** : same as `default_audit_context` in `agentic.rs` but
/// stamps `op_id = "agentic_proxy_stream"` for filtering.
fn default_audit_context_stream(tenant: TenantId, bundle: &ContextBundle) -> AgenticAuditContext {
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
        trace_id: Uuid::new_v4(),
        swarm_request_id: Uuid::new_v4(),
        op_id: "agentic_proxy_stream".to_string(),
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

/// **N27** : pre-stream errors (auth, bundle-too-large, pre-call
/// provenance fail) are mapped to RFC 7807 problem responses.
/// This is intentionally a copy of `agentic::map_bridge_err`
/// because the streaming endpoint surfaces a subset (most
/// streaming errors land mid-stream and are encoded as `event:
/// error`).
fn map_bridge_err(e: BridgeError) -> ApiError {
    // Reuse the buffered endpoint's mapping for pre-stream errors.
    crate::handlers::agentic::map_bridge_err_pub(e)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgenticStreamDoneEnvelope {
    pub stop_reason: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost_microcents: i64,
    pub model_provider: String,
    pub model_id: String,
    pub time_to_first_token_ms: Option<u32>,
    pub provider_response_time_ms: u32,
    pub retry_count: u8,
    pub retry_outcome: String,
}

/// **N27** : metadata about the abort that the gateway can
/// optionally surface to a curious operator via a debug endpoint.
/// Not part of the public SSE wire (T3 — no PII leak).
#[allow(dead_code)]
pub fn abort_reason_label(r: &AbortReason) -> &'static str {
    r.label()
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_agentic_bridge::{StopReason, TokenUsage};
    use efe_nano_sdk::{ModelIdentity, RetryOutcome};

    #[test]
    fn encode_text_delta() {
        let ev = AgenticEvent::TextDelta("Hello".into());
        let e = encode_event(ev);
        let s = format!("{e:?}");
        assert!(s.contains("delta"));
        assert!(s.contains("Hello"));
    }

    #[test]
    fn encode_done_event_carries_all_fields() {
        let ev = AgenticEvent::Done {
            stop_reason: StopReason::EndTurn,
            total_usage: TokenUsage {
                input_tokens: 123,
                output_tokens: 45,
            },
            model_identity: ModelIdentity::hosted(
                "anthropic",
                "claude-sonnet-4-6",
                "2023-06-01",
            ),
            cost_microcents: 9_750,
            time_to_first_token_ms: Some(287),
            provider_response_time_ms: 1_247,
            retry_count: 1,
            retry_outcome: RetryOutcome::RetriedSucceeded,
        };
        let e = encode_event(ev);
        let s = format!("{e:?}");
        assert!(s.contains("done"));
        assert!(s.contains("123"));
        assert!(s.contains("45"));
        assert!(s.contains("9750"));
        assert!(s.contains("287"));
        assert!(s.contains("retried_succeeded"));
    }

    #[test]
    fn encode_error_event_serialises_kind() {
        let ev = AgenticEvent::Error(BridgeError::ReadTimeout);
        let e = encode_event(ev);
        let s = format!("{e:?}");
        assert!(s.contains("error"));
        assert!(s.contains("read_timeout"));
    }

    #[test]
    fn keepalive_interval_default_is_15s() {
        // Unset env first.
        std::env::remove_var("EFE_GATEWAY_SSE_KEEPALIVE_MS");
        assert_eq!(sse_keepalive_interval(), Duration::from_secs(15));
    }

    #[test]
    fn keepalive_interval_honors_env_override() {
        std::env::set_var("EFE_GATEWAY_SSE_KEEPALIVE_MS", "5000");
        assert_eq!(sse_keepalive_interval(), Duration::from_secs(5));
        std::env::remove_var("EFE_GATEWAY_SSE_KEEPALIVE_MS");
    }

    #[test]
    fn bridge_error_kind_taxonomy_stable() {
        assert_eq!(bridge_error_kind(&BridgeError::RateLimited), "rate_limited");
        assert_eq!(
            bridge_error_kind(&BridgeError::TotalTimeout),
            "total_timeout"
        );
        assert_eq!(
            bridge_error_kind(&BridgeError::HttpStatus {
                status: 503,
                body: "x".into()
            }),
            "http_status"
        );
    }

    // ─── Pass-4 kill-switch helper (env → bool) ─────────────────
    //
    // **Pass-4 fix** : these tests are not perfectly hygienic
    // because cargo runs tests concurrently — env vars are
    // process-global.  We guard with a Mutex to serialize.  The
    // production handler does NOT read the env directly anymore ;
    // it reads `AppState::stream_endpoint_enabled` (set ONCE at
    // boot from `stream_endpoint_enabled_from_env()`).  This
    // makes runtime behaviour fully deterministic per-instance
    // and the env tests below are scoped to the helper only.

    use std::sync::Mutex;
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn stream_endpoint_enabled_from_env_default_true() {
        let _g = ENV_MUTEX.lock().unwrap();
        std::env::remove_var("EFE_BRIDGE_STREAM_ENABLED");
        assert!(stream_endpoint_enabled_from_env());
    }

    #[test]
    fn stream_endpoint_enabled_from_env_explicit_0_disables() {
        let _g = ENV_MUTEX.lock().unwrap();
        std::env::set_var("EFE_BRIDGE_STREAM_ENABLED", "0");
        assert!(!stream_endpoint_enabled_from_env());
        std::env::remove_var("EFE_BRIDGE_STREAM_ENABLED");
    }

    #[test]
    fn stream_endpoint_enabled_from_env_explicit_1_enables() {
        let _g = ENV_MUTEX.lock().unwrap();
        std::env::set_var("EFE_BRIDGE_STREAM_ENABLED", "1");
        assert!(stream_endpoint_enabled_from_env());
        std::env::remove_var("EFE_BRIDGE_STREAM_ENABLED");
    }

    #[test]
    fn stream_endpoint_enabled_from_env_invalid_value_keeps_enabled() {
        let _g = ENV_MUTEX.lock().unwrap();
        std::env::set_var("EFE_BRIDGE_STREAM_ENABLED", "anything-else");
        assert!(stream_endpoint_enabled_from_env());
        std::env::remove_var("EFE_BRIDGE_STREAM_ENABLED");
    }
}
