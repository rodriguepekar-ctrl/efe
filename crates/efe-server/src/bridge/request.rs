//! `Request` ↔ proto conversion.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::bridge::{decode, spec};
use crate::proto::v1 as proto;
use efe_core::{swarm::TraceContext, EfeError, Request, RequestId, Result};

/// Build a domain [`Request`] from its proto form.
///
/// `tenant_id` comes from the authenticated `Claims`. The `trace_context`
/// is set by the tracing interceptor layer via request extensions and
/// is injected here from the same source (via the `trace_context`
/// argument rather than reading the proto, which does not carry it).
pub fn from_proto(
    proto_req: proto::ExecuteRequest,
    tenant_id: String,
    trace_context: Option<TraceContext>,
) -> Result<Request> {
    let spec_proto = proto_req
        .spec
        .ok_or_else(|| EfeError::InvalidConfig("ExecuteRequest.spec is required".into()))?;
    let spec = spec::from_proto(spec_proto)?;
    let decode_mode = decode::mode_from_proto(proto_req.decode_mode)?;

    let id = if proto_req.request_id.is_empty() {
        RequestId::new()
    } else {
        RequestId::from_raw(proto_req.request_id)
    };

    // Optional deadline: translate wall-clock Timestamp to an `Instant`.
    // If the target instant is in the past we keep `None` — the
    // controller will fail fast via its budget check.
    let deadline = proto_req.deadline.and_then(|ts| {
        let secs = u64::try_from(ts.seconds).ok()?;
        let target = UNIX_EPOCH.checked_add(Duration::new(secs, ts.nanos.max(0) as u32))?;
        let now_sys = SystemTime::now();
        let remaining = target.duration_since(now_sys).ok()?;
        Instant::now().checked_add(remaining)
    });

    let raw_prompt = if proto_req.raw_prompt.is_empty() {
        None
    } else {
        Some(proto_req.raw_prompt)
    };

    // `Request` is `#[non_exhaustive]` cross-crate; the `for_tests`
    // helper is the only public constructor available to us. We
    // immediately overwrite every public field with the proto-supplied
    // values so the resulting request is deterministic.
    let mut req = Request::for_tests(spec);
    req.id = id;
    req.tenant_id = tenant_id;
    req.decode_mode = decode_mode;
    req.raw_prompt = raw_prompt;
    req.deadline = deadline;
    req.trace_context = trace_context;
    Ok(req)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bare_proto_spec() -> proto::SwarmSpec {
        proto::SwarmSpec {
            swarm_id: String::new(),
            problem_embedding: vec![0.0; 4],
            dim_latent: 4,
            population: vec![proto::PopulationEntry {
                template_code: "na-sem".into(),
                count: 4,
            }],
            max_rounds: 4,
            convergence: None,
            cone_source: Some(proto::swarm_spec::ConeSource::AlignmentCone(
                proto::AlignmentCone {
                    anchors: vec![proto::Anchor {
                        values: vec![0.1; 4],
                    }],
                    max_projection_distance: 0.5,
                },
            )),
            budget: None,
        }
    }

    #[test]
    fn from_proto_generates_request_id_when_empty() {
        let p = proto::ExecuteRequest {
            request_id: String::new(),
            raw_prompt: String::new(),
            spec: Some(bare_proto_spec()),
            decode_mode: None,
            deadline: None,
        };
        let r = from_proto(p, "t".into(), None).unwrap();
        assert!(!r.id.as_str().is_empty());
    }

    #[test]
    fn from_proto_preserves_request_id() {
        let p = proto::ExecuteRequest {
            request_id: "abc123".into(),
            raw_prompt: String::new(),
            spec: Some(bare_proto_spec()),
            decode_mode: None,
            deadline: None,
        };
        let r = from_proto(p, "t".into(), None).unwrap();
        assert_eq!(r.id.as_str(), "abc123");
    }

    #[test]
    fn from_proto_sets_tenant_id() {
        let p = proto::ExecuteRequest {
            request_id: String::new(),
            raw_prompt: String::new(),
            spec: Some(bare_proto_spec()),
            decode_mode: None,
            deadline: None,
        };
        let r = from_proto(p, "acme".into(), None).unwrap();
        assert_eq!(r.tenant_id, "acme");
    }

    #[test]
    fn from_proto_keeps_raw_prompt_when_nonempty() {
        let p = proto::ExecuteRequest {
            request_id: String::new(),
            raw_prompt: "hello".into(),
            spec: Some(bare_proto_spec()),
            decode_mode: None,
            deadline: None,
        };
        let r = from_proto(p, "t".into(), None).unwrap();
        assert_eq!(r.raw_prompt.as_deref(), Some("hello"));
    }

    #[test]
    fn from_proto_rejects_missing_spec() {
        let p = proto::ExecuteRequest {
            request_id: String::new(),
            raw_prompt: String::new(),
            spec: None,
            decode_mode: None,
            deadline: None,
        };
        assert!(from_proto(p, "t".into(), None).is_err());
    }

    #[test]
    fn from_proto_past_deadline_is_ignored() {
        let p = proto::ExecuteRequest {
            request_id: String::new(),
            raw_prompt: String::new(),
            spec: Some(bare_proto_spec()),
            decode_mode: None,
            deadline: Some(prost_types::Timestamp {
                seconds: 1,
                nanos: 0,
            }), // past
        };
        let r = from_proto(p, "t".into(), None).unwrap();
        assert!(r.deadline.is_none());
    }

    #[test]
    fn from_proto_sets_trace_context() {
        let p = proto::ExecuteRequest {
            request_id: String::new(),
            raw_prompt: String::new(),
            spec: Some(bare_proto_spec()),
            decode_mode: None,
            deadline: None,
        };
        let tc = TraceContext {
            traceparent: "00-abc-def-01".into(),
            tracestate: None,
        };
        let r = from_proto(p, "t".into(), Some(tc)).unwrap();
        assert!(r.trace_context.is_some());
    }
}
