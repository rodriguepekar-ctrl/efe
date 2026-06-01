//! `Response` ↔ proto conversion.

use crate::bridge::{decode, verdict};
use crate::proto::v1 as proto;
use efe_core::Response;

/// Convert a domain [`Response`] into its proto form.
#[must_use]
pub fn to_proto(r: Response) -> proto::ExecuteResponse {
    proto::ExecuteResponse {
        swarm_id: r.swarm_id.to_string(),
        request_id: r.request_id.as_str().to_string(),
        output: Some(decode::output_to_proto(r.output)),
        confidence: r.confidence,
        verdict: verdict::to_proto(&r.verdict) as i32,
        rounds_executed: r.rounds_executed,
        total_ms: r.total_ms,
        alignment_delta: r.alignment_delta,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_core::{types::RequestId, DecodedOutput, Verdict};

    fn sample_response() -> Response {
        let mut r = Response::for_tests(
            RequestId::from_raw("rid-1"),
            DecodedOutput::Text("hi".into()),
        );
        r.confidence = 0.9;
        r.verdict = Verdict::Converged { confidence: 0.9 };
        r.rounds_executed = 3;
        r.total_ms = 42.0;
        r.alignment_delta = 0.1;
        r
    }

    #[test]
    fn to_proto_text_output() {
        let p = to_proto(sample_response());
        assert_eq!(p.request_id, "rid-1");
        assert_eq!(p.rounds_executed, 3);
        assert!((p.total_ms - 42.0).abs() < 1e-6);
        assert!((p.confidence - 0.9).abs() < 1e-6);
        assert!((p.alignment_delta - 0.1).abs() < 1e-6);
        assert_eq!(p.verdict, proto::Verdict::Converged as i32);
        match p.output.and_then(|o| o.payload) {
            Some(proto::decoded_output::Payload::Text(s)) => assert_eq!(s, "hi"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn to_proto_raw_output() {
        let mut r = sample_response();
        r.output = DecodedOutput::Raw(vec![1.0, 2.0, 3.0]);
        let p = to_proto(r);
        match p.output.and_then(|o| o.payload) {
            Some(proto::decoded_output::Payload::Raw(x)) => {
                assert_eq!(x.values, vec![1.0, 2.0, 3.0]);
            }
            _ => panic!("expected raw"),
        }
    }

    #[test]
    fn to_proto_swarm_id_is_valid_uuid_string() {
        let p = to_proto(sample_response());
        // Loose check: uuid string is 36 chars.
        assert_eq!(p.swarm_id.len(), 36);
    }
}
