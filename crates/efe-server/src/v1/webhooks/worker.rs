//! Background delivery worker.
//!
//! Lifecycle :
//! 1. Subscribe to NATS `efe.tenant.>` (all tenants on the per-tenant
//!    JetStream stream).
//! 2. For each event, look up active webhook subscriptions matching the
//!    tenant + topic.
//! 3. For each match, POST the event JSON to the customer URL with
//!    HMAC signature + standard headers.
//! 4. Log every attempt to `webhook_delivery` table.
//! 5. Apply circuit breaker + retry schedule.
//!
//! Concurrency : the worker processes events in parallel up to
//! `MAX_INFLIGHT` (default 64). NATS ack semantics : ack after the
//! delivery is recorded (success OR final failure). Transient
//! failures get nak'd to trigger NATS-level redelivery only if our
//! own retry schedule is exhausted (rare ; usually we just enqueue a
//! follow-up DB job).
//!
//! Note : the actual delivery worker spinning depends on database
//! access + a NATS subscription, which only the operator-binary main.rs
//! wires up. This module exposes the pure pieces (request building,
//! signature header, response classification) for unit testing.

use super::delivery::DeliveryOutcome;
use super::signing;
use crate::v1::webhooks::validation::validate_webhook_url;
use chrono::Utc;
use serde::Serialize;
use std::time::Duration;
use tracing::warn;

/// Per-delivery wall-clock timeout. Bigger than typical Internet
/// latencies but small enough to keep delivery throughput high.
pub const DEFAULT_DELIVERY_TIMEOUT: Duration = Duration::from_secs(10);

/// Maximum body size for the customer's response we keep in the log.
pub const RESPONSE_BODY_LOG_HEAD: usize = 256;

/// Pure builder for the outbound HTTP request. Returns
/// `(headers, body)` ready for any HTTP client. Kept pure for unit
/// testing (no IO).
pub fn build_outbound_request<P: Serialize>(
    webhook_id: &str,
    topic: &str,
    payload: &P,
    secret: &[u8],
    now_unix: i64,
) -> Result<OutboundRequest, BuildRequestError> {
    let body = serde_json::to_vec(payload).map_err(BuildRequestError::Encode)?;
    let signature = signing::build_signature_header(secret, now_unix, &body);
    let headers = vec![
        ("Content-Type", "application/json".to_string()),
        ("X-EFE-Webhook-ID", webhook_id.to_string()),
        ("X-EFE-Webhook-Topic", topic.to_string()),
        ("X-EFE-Webhook-Timestamp", now_unix.to_string()),
        ("X-EFE-Webhook-Signature", signature),
        ("User-Agent", "EFE-Webhook/2.3.0".to_string()),
    ];
    Ok(OutboundRequest { headers, body })
}

#[derive(Debug)]
pub struct OutboundRequest {
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum BuildRequestError {
    #[error("payload encode : {0}")]
    Encode(serde_json::Error),
}

/// Pure : given a customer HTTP response status + body, return the
/// outcome + a head-of-body for the audit log.
pub fn classify_response(status: Option<u16>, body: &[u8]) -> (DeliveryOutcome, String) {
    use crate::v1::webhooks::delivery::classify_status;
    let outcome = classify_status(status);
    let head_len = body.len().min(RESPONSE_BODY_LOG_HEAD);
    let head = String::from_utf8_lossy(&body[..head_len]).into_owned();
    (outcome, head)
}

/// Re-validate the URL at delivery time (DNS rebinding mitigation).
/// Returns true if safe to deliver, false if URL has shifted to a
/// disallowed IP class since registration.
pub fn precheck_url_at_delivery(url: &str) -> bool {
    match validate_webhook_url(url) {
        Ok(()) => true,
        Err(e) => {
            warn!(%url, ?e, "webhook URL re-validation failed at delivery time");
            false
        }
    }
}

/// Current unix epoch seconds. Tiny wrapper so tests can mock.
pub fn now_unix() -> i64 {
    Utc::now().timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_outbound_request_has_all_headers() {
        let payload = json!({"foo": "bar"});
        let req = build_outbound_request(
            "wh_abc",
            "classify.completed",
            &payload,
            b"secret",
            1_700_000_000,
        )
        .unwrap();
        let header_names: Vec<&str> = req.headers.iter().map(|(k, _)| *k).collect();
        for h in &[
            "Content-Type",
            "X-EFE-Webhook-ID",
            "X-EFE-Webhook-Topic",
            "X-EFE-Webhook-Timestamp",
            "X-EFE-Webhook-Signature",
            "User-Agent",
        ] {
            assert!(header_names.contains(h), "missing {}", h);
        }
        // Body is valid JSON
        let parsed: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(parsed["foo"], "bar");
    }

    #[test]
    fn signature_header_format() {
        let req = build_outbound_request("wh_x", "t", &json!({}), b"k", 100).unwrap();
        let sig = req
            .headers
            .iter()
            .find(|(k, _)| *k == "X-EFE-Webhook-Signature")
            .map(|(_, v)| v.clone())
            .unwrap();
        assert!(sig.starts_with("t=100,v1="));
    }

    #[test]
    fn classify_response_truncates_body() {
        let big = vec![b'x'; 1024];
        let (outcome, head) = classify_response(Some(200), &big);
        assert_eq!(outcome, DeliveryOutcome::Ok);
        assert_eq!(head.len(), RESPONSE_BODY_LOG_HEAD);
    }
}
