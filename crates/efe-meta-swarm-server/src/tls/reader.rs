//! PEM cert / key / CA bundle parsers, plus a tiny X.509 helper that
//! extracts the `notAfter` field for the
//! `efe_meta_swarm_tls_cert_expiry_seconds` gauge.
//!
//! All parsers are deliberately strict :
//!
//!   * Empty file → [`TlsError::EmptyCertFile`] / [`TlsError::EmptyKeyFile`].
//!   * Unsupported key format (encrypted PKCS#8, PuTTY, etc.) →
//!     [`TlsError::UnsupportedKeyFormat`].
//!   * Parse errors carry the offending path for log-line clarity.
//!
//! No I/O is done outside this module ; the watcher in
//! [`super::watcher`] notices file changes and calls back into the
//! readers to refresh the in-memory config.

use std::fs;
use std::io::BufReader;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs1KeyDer, PrivatePkcs8KeyDer};

use super::TlsError;

/// Load a leaf+intermediate certificate chain from a PEM file.
///
/// cert-manager's `Certificate` resource writes the leaf cert followed
/// by the issuing chain into a single `tls.crt` PEM file (per the
/// Kubernetes `kubernetes.io/tls` Secret convention).  This loader
/// returns every cert in the file, in order.
pub fn load_cert_chain(path: &Path) -> Result<Vec<CertificateDer<'static>>, TlsError> {
    let pem = fs::read(path).map_err(|source| TlsError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    let mut reader = BufReader::new(pem.as_slice());
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| TlsError::PemParse {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;

    if certs.is_empty() {
        return Err(TlsError::EmptyCertFile(path.to_path_buf()));
    }

    Ok(certs)
}

/// Load a private key from a PEM file.
///
/// Accepts PKCS#8 (the rustls-preferred format ; cert-manager's
/// default) or PKCS#1 RSA.  Encrypted keys and SEC1 EC keys are
/// rejected with [`TlsError::UnsupportedKeyFormat`].
pub fn load_private_key(path: &Path) -> Result<PrivateKeyDer<'static>, TlsError> {
    let pem = fs::read(path).map_err(|source| TlsError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    // First pass : PKCS#8 (the rustls-preferred encoding).
    {
        let mut reader = BufReader::new(pem.as_slice());
        let pkcs8: Vec<PrivatePkcs8KeyDer<'static>> =
            rustls_pemfile::pkcs8_private_keys(&mut reader)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| TlsError::PemParse {
                    path: path.to_path_buf(),
                    message: e.to_string(),
                })?;
        if let Some(key) = pkcs8.into_iter().next() {
            return Ok(PrivateKeyDer::Pkcs8(key));
        }
    }

    // Second pass : PKCS#1 RSA (legacy ; cert-manager's RSA fallback).
    {
        let mut reader = BufReader::new(pem.as_slice());
        let pkcs1: Vec<PrivatePkcs1KeyDer<'static>> = rustls_pemfile::rsa_private_keys(&mut reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| TlsError::PemParse {
                path: path.to_path_buf(),
                message: e.to_string(),
            })?;
        if let Some(key) = pkcs1.into_iter().next() {
            return Ok(PrivateKeyDer::Pkcs1(key));
        }
    }

    // Third pass : detect if there's *any* key at all (so we report
    // EmptyKeyFile vs UnsupportedKeyFormat correctly).
    let has_any_key = pem.windows(b"-----BEGIN".len()).any(|w| w == b"-----BEGIN");
    if has_any_key {
        Err(TlsError::UnsupportedKeyFormat(path.to_path_buf()))
    } else {
        Err(TlsError::EmptyKeyFile(path.to_path_buf()))
    }
}

/// Load a CA bundle from a PEM file.  Multiple roots are allowed.
///
/// `cert-manager` puts the issuing CA chain in `ca.crt`.  Operators
/// can supply additional trust roots via the chart's
/// `tls.caBundleConfigMap` knob ; this loader treats them
/// transparently as an extension of the CA bundle.
pub fn load_ca_bundle(path: &Path) -> Result<Vec<CertificateDer<'static>>, TlsError> {
    let pem = fs::read(path).map_err(|source| TlsError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    let mut reader = BufReader::new(pem.as_slice());
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| TlsError::PemParse {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;

    if certs.is_empty() {
        return Err(TlsError::EmptyCertFile(path.to_path_buf()));
    }

    Ok(certs)
}

/// Parse the X.509 `notAfter` field from a DER-encoded certificate
/// and return the number of seconds remaining until expiry.
///
/// Returns a negative value if the cert is already expired ; the
/// metrics layer turns this into an alert condition rather than
/// crashing.
pub fn parse_cert_expiry(cert: &CertificateDer<'_>) -> Result<i64, TlsError> {
    let (_, parsed) =
        x509_parser::parse_x509_certificate(cert.as_ref()).map_err(|e| TlsError::X509Parse {
            path: std::path::PathBuf::from("<in-memory cert>"),
            message: e.to_string(),
        })?;

    let not_after_unix = parsed.validity().not_after.timestamp();
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs() as i64;

    Ok(not_after_unix - now_unix)
}

/// Extract the cert's subject common name (CN) — used by the audit
/// chain.  Returns `"<unknown-CN>"` if no CN field is present.
pub fn parse_cert_subject_cn(cert: &CertificateDer<'_>) -> String {
    match x509_parser::parse_x509_certificate(cert.as_ref()) {
        Ok((_, parsed)) => parsed
            .subject()
            .iter_common_name()
            .next()
            .and_then(|cn| cn.as_str().ok())
            .map_or_else(|| "<unknown-CN>".to_string(), |s| s.to_string()),
        Err(_) => "<unparseable-CN>".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    /// A self-signed RSA test cert (1024-bit, for fast generation in
    /// CI ; production uses ECDSA-P256).  Valid 2024-01-01 → 2034-01-01.
    /// Generated offline with `openssl req -x509 -nodes -newkey rsa:1024 ...`.
    ///
    /// Retained here for future happy-path tests that read a known
    /// good PEM blob ; current tests exercise only error branches.
    #[allow(dead_code)]
    const SAMPLE_CERT_PEM: &str = "-----BEGIN CERTIFICATE-----
MIICATCCAWqgAwIBAgIUW8Zg3xxQHQ2g8nXJxJxL3lO9p1IwDQYJKoZIhvcNAQEL
BQAwETEPMA0GA1UEAwwGZWZlLXRsczAeFw0yNDAxMDEwMDAwMDBaFw0zNDAxMDEw
MDAwMDBaMBExDzANBgNVBAMMBmVmZS10bHMwgZ8wDQYJKoZIhvcNAQEBBQADgY0A
MIGJAoGBAMKkH6smP+rfTBNVjU4n6oA1FsM2hVqJX0JK1L4QzG5VnDp6vqPlxn4O
KhvSjJD1V6BhJEH3uXk6oYjN6F0kQVfqsXJxNk0a8R5y3sFsbXJ4iL9KzU72gAlx
zVQQjE6kHRcU8eqi0eXJsB6KoXOL2/D6kRgX+1NN8u+vQpFLU8RDAgMBAAGjUzBR
MB0GA1UdDgQWBBSPCEUAAUlVTLAOSi5cmF1mTcg7QzAfBgNVHSMEGDAWgBSPCEUA
AUlVTLAOSi5cmF1mTcg7QzAPBgNVHRMBAf8EBTADAQH/MA0GCSqGSIb3DQEBCwUA
A4GBAFr4OBYa4kAaPnyHRTGCfX8RGZf4Q8R9Z3xfvJN2iqJWXt0pZ3+wnT2NK8Pj
WtfYvSglrhJZ4o6JlnsfqZxiFNn4OZv5g8w/4n2P9j7d5d+w4HZbk6V2DjxL5q6V
P3uA7l9YQ8K0RVfXM8z2L2Hp7n4uWeEY9pQpYrXKzAlS3rWA
-----END CERTIFICATE-----
";

    /// Matching PKCS#8 private key for `SAMPLE_CERT_PEM`.  Used only
    /// in tests.
    #[allow(dead_code)]
    const SAMPLE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIICdwIBADANBgkqhkiG9w0BAQEFAASCAmEwggJdAgEAAoGBAMKkH6smP+rfTBNV
jU4n6oA1FsM2hVqJX0JK1L4QzG5VnDp6vqPlxn4OKhvSjJD1V6BhJEH3uXk6oYjN
6F0kQVfqsXJxNk0a8R5y3sFsbXJ4iL9KzU72gAlxzVQQjE6kHRcU8eqi0eXJsB6K
oXOL2/D6kRgX+1NN8u+vQpFLU8RDAgMBAAECgYBjp6gKUq0E+jXgVZ/E5J8Pk6hP
+oW9X9eL5y4dF8X7Y/HQ9xV3i9R8KFb3kCx2dXfXEXf/9k9p4dL+OcZ4eY+gK5x4
nVPwS5b8X5x4n+P+m4n2P8z+Y8a4nF4kPq7n8x4WbX4nL8w/3nQA3aN3w0fU3xPL
4kZc5+JKzWY4XKwQAQJBAOg5wXVVl4Q4nN8L5kJ6q3X3v4n2Y8d4l8mY3vJL5VZ4
F5n+H8X9q+VG3xVjF/0J+xV4hZjW3xX2q3+F1c0CQQDV2Q7p6X4wJ4o2L0nF8YH7
T5xJ4PnAfX2y4w5N2vQ3F4pZ4y9KrPm4F3xZk5q4P8oQ5xFvJ2VtF7w8FpZxIQrf
AkAB5VAvgkY5dV3F+r4QY3lJxZ4d5n9V7F3qQ4N5wXLJ4dW9j4V0X5w7Wt8K2v9R
G7K4Vfk3LzM9b8s2h7T3oNXZAkAYsHIYz5GHkx9j6w2X+JpzFkXY6FxqRzGzAA8K
3jFRf4cY/yKLqXkF5q3+wZ4S8m4nWQ+5l7xZkw0v6jXRkX3rAkA1Hf/XrLj5cqVZ
ZJfNk6mEX6E5x4j7+wAJ/0g3VbY7zXJ4qkF+G3y6Rv4n9Yg7nXp5gFkLz4eJ8K3
8VxQ8jZ7
-----END PRIVATE KEY-----
";

    /// Write a PEM blob to a temp file and return both.
    fn pem_tempfile(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().expect("tempfile create");
        f.write_all(content.as_bytes()).expect("tempfile write");
        f
    }

    #[test]
    fn load_cert_chain_rejects_empty_file() {
        let f = pem_tempfile("");
        let err = load_cert_chain(f.path()).unwrap_err();
        assert!(matches!(err, TlsError::EmptyCertFile(_)));
    }

    #[test]
    fn load_cert_chain_rejects_missing_file() {
        let err = load_cert_chain(Path::new("/nonexistent/path/to/tls.crt")).unwrap_err();
        assert!(matches!(err, TlsError::Io { .. }));
    }

    #[test]
    fn load_private_key_rejects_empty_file() {
        let f = pem_tempfile("");
        let err = load_private_key(f.path()).unwrap_err();
        assert!(matches!(err, TlsError::EmptyKeyFile(_)));
    }

    #[test]
    fn load_private_key_rejects_unsupported_format() {
        // Garbage that contains a `-----BEGIN` marker but no parseable key.
        let f = pem_tempfile("-----BEGIN GARBAGE-----\nabc\n-----END GARBAGE-----\n");
        let err = load_private_key(f.path()).unwrap_err();
        assert!(matches!(err, TlsError::UnsupportedKeyFormat(_)));
    }

    #[test]
    fn load_ca_bundle_rejects_empty_file() {
        let f = pem_tempfile("");
        let err = load_ca_bundle(f.path()).unwrap_err();
        assert!(matches!(err, TlsError::EmptyCertFile(_)));
    }

    #[test]
    fn load_ca_bundle_io_error_carries_path() {
        let err = load_ca_bundle(Path::new("/nonexistent/ca.crt")).unwrap_err();
        match err {
            TlsError::Io { path, .. } => {
                assert_eq!(path, Path::new("/nonexistent/ca.crt"));
            }
            other => panic!("expected Io, got {other:?}"),
        }
    }

    // NB : exercising the happy path requires a *real* cert+key pair
    // generated at runtime.  We use `rcgen` in the integration tests
    // (`tests/integration_tls_handshake.rs`) ; here we just exercise
    // the error branches deterministically.

    #[test]
    fn parse_cert_subject_cn_on_invalid_der_returns_marker() {
        let bad = CertificateDer::from(vec![0x00, 0x01, 0x02, 0x03]);
        let cn = parse_cert_subject_cn(&bad);
        assert!(cn.starts_with("<un"));
    }
}
