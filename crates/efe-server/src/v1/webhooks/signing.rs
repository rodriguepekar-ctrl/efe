//! HMAC-SHA256 signing for outbound webhook payloads.
//!
//! Header `X-EFE-Webhook-Signature: t=<unix_ts>,v1=<hex_hmac>` where
//! `hmac = HMAC-SHA256(secret, "{ts}.{body}")`. Receivers verify by
//! recomputing the HMAC and comparing in constant time.
//!
//! Inspired by Stripe's webhook signature pattern.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Compute the signature header value : `t=<ts>,v1=<hex>`.
pub fn build_signature_header(secret: &[u8], timestamp_unix: i64, body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    let prefix = format!("{}.", timestamp_unix);
    mac.update(prefix.as_bytes());
    mac.update(body);
    let hex = hex::encode(mac.finalize().into_bytes());
    format!("t={},v1={}", timestamp_unix, hex)
}

/// Verify a signature header. Returns true iff the signature is valid
/// AND the timestamp is within `tolerance_seconds` of now.
pub fn verify_signature_header(
    header: &str,
    secret: &[u8],
    body: &[u8],
    now_unix: i64,
    tolerance_seconds: i64,
) -> bool {
    let mut t: Option<i64> = None;
    let mut v1: Option<&str> = None;
    for part in header.split(',') {
        let mut kv = part.splitn(2, '=');
        let k = kv.next().unwrap_or("");
        let v = kv.next().unwrap_or("");
        match k.trim() {
            "t" => t = v.trim().parse().ok(),
            "v1" => v1 = Some(v.trim()),
            _ => {}
        }
    }
    let ts = match t {
        Some(n) => n,
        None => return false,
    };
    let sig_hex = match v1 {
        Some(s) => s,
        None => return false,
    };
    if (now_unix - ts).abs() > tolerance_seconds {
        return false;
    }
    let expected = hex::decode(sig_hex).ok();
    let mut mac = match HmacSha256::new_from_slice(secret) {
        Ok(m) => m,
        Err(_) => return false,
    };
    let prefix = format!("{}.", ts);
    mac.update(prefix.as_bytes());
    mac.update(body);
    let actual = mac.finalize().into_bytes();
    match expected {
        Some(bytes) => bytes.len() == actual.len() && constant_time_eq(&bytes, &actual),
        None => false,
    }
}

/// Constant-time bytewise equality.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Hash a user-supplied webhook secret for at-rest storage. Uses
/// SHA256 (cheap, deterministic) rather than Argon2 because we don't
/// need brute-force resistance (the raw secret is high-entropy from
/// our `crypto_secret_gen` helper). The hash is what we store ; we
/// never store the raw secret.
pub fn hash_secret(secret: &[u8]) -> String {
    use sha2::Digest;
    let mut h = Sha256::new();
    h.update(secret);
    hex::encode(h.finalize())
}

/// Generate a random 32-byte secret + return both raw + hashed form.
pub fn crypto_secret_gen() -> (Vec<u8>, String) {
    use rand::RngCore;
    let mut bytes = vec![0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let hashed = hash_secret(&bytes);
    (bytes, hashed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_roundtrip_valid() {
        let secret = b"my-shared-secret-xyz";
        let body = b"{\"event\":\"foo\"}";
        let ts = 1_700_000_000;
        let header = build_signature_header(secret, ts, body);
        assert!(header.starts_with("t=1700000000,v1="));
        let ok = verify_signature_header(&header, secret, body, ts + 10, 300);
        assert!(ok, "valid signature should verify");
    }

    #[test]
    fn signature_outside_tolerance_rejected() {
        let secret = b"k";
        let body = b"x";
        let ts = 1_700_000_000;
        let header = build_signature_header(secret, ts, body);
        // 1 hour off ; tolerance 5 min :
        let ok = verify_signature_header(&header, secret, body, ts + 3600, 300);
        assert!(!ok, "signature outside tolerance should reject");
    }

    #[test]
    fn signature_wrong_secret_rejected() {
        let secret_a = b"a";
        let secret_b = b"b";
        let body = b"hello";
        let ts = 1_700_000_000;
        let header = build_signature_header(secret_a, ts, body);
        assert!(!verify_signature_header(&header, secret_b, body, ts, 300));
    }

    #[test]
    fn signature_tampered_body_rejected() {
        let secret = b"k";
        let ts = 1_700_000_000;
        let header = build_signature_header(secret, ts, b"original");
        assert!(!verify_signature_header(
            &header,
            secret,
            b"tampered",
            ts,
            300
        ));
    }

    #[test]
    fn malformed_header_rejected() {
        assert!(!verify_signature_header("garbage", b"k", b"x", 0, 300));
        assert!(!verify_signature_header("t=,v1=", b"k", b"x", 0, 300));
        assert!(!verify_signature_header("v1=abc", b"k", b"x", 0, 300));
    }

    #[test]
    fn hash_secret_is_deterministic() {
        let s = b"a-secret";
        assert_eq!(hash_secret(s), hash_secret(s));
        assert_eq!(hash_secret(s).len(), 64);
    }

    #[test]
    fn crypto_secret_gen_yields_unique() {
        let (s1, h1) = crypto_secret_gen();
        let (s2, h2) = crypto_secret_gen();
        assert_ne!(s1, s2, "random secrets should differ");
        assert_ne!(h1, h2);
        assert_eq!(s1.len(), 32);
    }
}
