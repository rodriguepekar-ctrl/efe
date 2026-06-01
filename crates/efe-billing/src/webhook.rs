// SPDX-License-Identifier: UNLICENSED
//! M31 — Stripe webhook signature verification + event decoding.
//!
//! Stripe signs every webhook with HMAC-SHA-256 over a payload of
//! the form `{timestamp}.{body}`. The signature header is :
//!
//! ```text
//!   Stripe-Signature: t=1700000000,v1=<hex>,v1=<hex>
//! ```
//!
//! We verify ALL `v1` signatures (Stripe ships multiple during key
//! rotation) and check the timestamp falls within `tolerance_secs`
//! of `now`.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::error::{BillingError, Result};

/// Verifier configuration.
#[derive(Debug, Clone)]
pub struct WebhookConfig {
    /// Stripe signing secret (`whsec_...`).
    pub signing_secret: String,
    /// Allowed timestamp drift in seconds.
    pub tolerance_secs: i64,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            signing_secret: String::new(),
            tolerance_secs: 300, // 5 minutes per Stripe recommendation
        }
    }
}

/// Decoded Stripe event (subset).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StripeEvent {
    /// `evt_...` identifier.
    pub id: String,
    /// Event type (`invoice.paid`, `customer.subscription.updated`, …).
    #[serde(rename = "type")]
    pub event_type: String,
    /// Unix timestamp.
    pub created: i64,
    /// Event payload.
    pub data: serde_json::Value,
}

/// Verify the `Stripe-Signature` header and decode the JSON body.
/// Returns the parsed event on success.
pub fn verify_stripe_signature(
    raw_body: &[u8],
    signature_header: &str,
    now_unix: i64,
    cfg: &WebhookConfig,
) -> Result<StripeEvent> {
    if cfg.signing_secret.is_empty() {
        return Err(BillingError::Internal {
            reason: "webhook signing secret empty".into(),
        });
    }

    // Parse "t=…,v1=…,v1=…"
    let mut ts: Option<i64> = None;
    let mut sigs: Vec<&str> = Vec::new();
    for part in signature_header.split(',') {
        let mut kv = part.splitn(2, '=');
        match (kv.next(), kv.next()) {
            (Some("t"), Some(v)) => ts = v.parse().ok(),
            (Some("v1"), Some(v)) => sigs.push(v),
            _ => {}
        }
    }
    let ts = ts.ok_or(BillingError::WebhookSignatureInvalid)?;
    if sigs.is_empty() {
        return Err(BillingError::WebhookSignatureInvalid);
    }

    // Timestamp tolerance.
    let drift = (now_unix - ts).abs();
    if drift > cfg.tolerance_secs {
        return Err(BillingError::WebhookTimestampDrift {
            drift_secs: drift,
            tolerance_secs: cfg.tolerance_secs,
        });
    }

    // Compute the expected HMAC.
    let signed_payload = format!("{ts}.");
    let mut mac = Hmac::<Sha256>::new_from_slice(cfg.signing_secret.as_bytes())
        .map_err(|e| BillingError::Internal {
            reason: format!("hmac key: {e}"),
        })?;
    mac.update(signed_payload.as_bytes());
    mac.update(raw_body);
    let computed = mac.finalize().into_bytes();
    let computed_hex = hex::encode(computed);

    // Compare to ALL `v1` signatures (constant-time per pair).
    let mut any_match = false;
    for s in sigs {
        if constant_time_eq(s.as_bytes(), computed_hex.as_bytes()) {
            any_match = true;
            break;
        }
    }
    if !any_match {
        return Err(BillingError::WebhookSignatureInvalid);
    }

    // Decode body.
    let evt: StripeEvent = serde_json::from_slice(raw_body)
        .map_err(|e| BillingError::WebhookPayload(format!("json: {e}")))?;
    Ok(evt)
}

/// Constant-time byte slice comparison.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut acc = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        acc |= x ^ y;
    }
    acc == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use hmac::Mac;

    fn signed_header(secret: &str, ts: i64, body: &[u8]) -> String {
        let signed = format!("{ts}.");
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(signed.as_bytes());
        mac.update(body);
        let hex = hex::encode(mac.finalize().into_bytes());
        format!("t={ts},v1={hex}")
    }

    fn body() -> Vec<u8> {
        br#"{"id":"evt_1","type":"invoice.paid","created":1700000000,"data":{}}"#.to_vec()
    }

    #[test]
    fn verify_happy_path() {
        let cfg = WebhookConfig {
            signing_secret: "whsec_test".into(),
            tolerance_secs: 60,
        };
        let body = body();
        let header = signed_header("whsec_test", 1700000000, &body);
        let evt = verify_stripe_signature(&body, &header, 1700000010, &cfg).unwrap();
        assert_eq!(evt.id, "evt_1");
        assert_eq!(evt.event_type, "invoice.paid");
    }

    #[test]
    fn verify_rejects_wrong_signature() {
        let cfg = WebhookConfig {
            signing_secret: "whsec_test".into(),
            tolerance_secs: 60,
        };
        let body = body();
        let header = signed_header("WRONG_SECRET", 1700000000, &body);
        let err = verify_stripe_signature(&body, &header, 1700000010, &cfg).unwrap_err();
        assert!(matches!(err, BillingError::WebhookSignatureInvalid));
    }

    #[test]
    fn verify_rejects_timestamp_drift() {
        let cfg = WebhookConfig {
            signing_secret: "whsec_test".into(),
            tolerance_secs: 60,
        };
        let body = body();
        let header = signed_header("whsec_test", 1700000000, &body);
        let err =
            verify_stripe_signature(&body, &header, 1700001000, &cfg).unwrap_err();
        assert!(matches!(err, BillingError::WebhookTimestampDrift { .. }));
    }

    #[test]
    fn verify_rejects_empty_secret() {
        let cfg = WebhookConfig {
            signing_secret: "".into(),
            tolerance_secs: 60,
        };
        let body = body();
        let header = signed_header("x", 1700000000, &body);
        let err = verify_stripe_signature(&body, &header, 1700000010, &cfg).unwrap_err();
        assert!(matches!(err, BillingError::Internal { .. }));
    }

    #[test]
    fn verify_accepts_multiple_v1_signatures() {
        let cfg = WebhookConfig {
            signing_secret: "whsec_test".into(),
            tolerance_secs: 60,
        };
        let body = body();
        let real = signed_header("whsec_test", 1700000000, &body);
        // Stripe may include multiple `v1=` ; we splice a bogus one
        // alongside the real one and verify it still passes.
        let header = format!("{},v1=ff", real);
        let evt = verify_stripe_signature(&body, &header, 1700000010, &cfg).unwrap();
        assert_eq!(evt.id, "evt_1");
    }

    #[test]
    fn constant_time_eq_basic() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }
}
