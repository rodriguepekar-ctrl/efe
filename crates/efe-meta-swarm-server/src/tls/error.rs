//! Crate-local error type for the TLS module.
//!
//! Every error variant carries enough context (file path, expected vs
//! actual SAN, debounce window, etc.) to be actionable in a log line
//! without a developer having to consult source.

use std::path::PathBuf;

use thiserror::Error;

/// TLS-specific error type.  Distinct from
/// [`crate::error::ServerError`] so the TLS module can be tested in
/// isolation, but converts cleanly via the `From` impl below.
#[derive(Debug, Error)]
pub enum TlsError {
    /// I/O failure while reading a cert / key / CA file.
    #[error("I/O error reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// PEM-decoding failure.  Wraps the message from
    /// `rustls_pemfile`'s `Error` (which is opaque in 2.x).
    #[error("PEM parse error on {path}: {message}")]
    PemParse { path: PathBuf, message: String },

    /// The file existed but contained zero certificates.
    #[error("cert file {0} contained zero certificates")]
    EmptyCertFile(PathBuf),

    /// The file existed but contained zero private keys (neither
    /// PKCS#8 nor PKCS#1 / SEC1).
    #[error("key file {0} contained zero usable private keys (try PKCS#8 PEM)")]
    EmptyKeyFile(PathBuf),

    /// Key file format we can't load.  Currently we accept PKCS#8
    /// (rustls preferred) and PKCS#1 RSA ; anything else (PuTTY,
    /// PGP keys, encrypted PKCS#8, etc.) is rejected.
    #[error("unsupported key format in {0}")]
    UnsupportedKeyFormat(PathBuf),

    /// `rustls::ServerConfig` / `ClientConfig` builder reported an
    /// internal error.
    #[error("rustls config error: {0}")]
    RustlsConfig(String),

    /// Peer cert SAN did not match the allowed-SAN list.
    #[error("SAN mismatch: expected one of {expected:?}, got {actual:?}")]
    SanMismatch {
        expected: Vec<String>,
        actual: Vec<String>,
    },

    /// File watcher could not be initialised or registered.
    #[error("cert watcher error: {0}")]
    Watcher(String),

    /// A reload was requested but did not complete in the allotted
    /// time budget.  Should be impossible under normal operation —
    /// the swap is atomic and lock-free — but ensures we surface
    /// pathological cases (e.g. cert file disappears mid-reload).
    #[error("cert reload exceeded the configured timeout")]
    ReloadTimeout,

    /// X.509 parsing failure during cert-expiry sampling.
    #[error("x509 parse error on {path}: {message}")]
    X509Parse { path: PathBuf, message: String },

    /// Invalid protocol-version value supplied via env var or values
    /// override.  Expected `"1.2"` or `"1.3"`.
    #[error("invalid TLS protocol version {got:?} (expected \"1.2\" or \"1.3\")")]
    InvalidProtocolVersion { got: String },
}

impl From<TlsError> for crate::error::ServerError {
    fn from(value: TlsError) -> Self {
        Self::Config(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;

    #[test]
    fn io_error_displays_path() {
        let err = TlsError::Io {
            path: PathBuf::from("/etc/efe-meta-swarm/tls/tls.crt"),
            source: std::io::Error::new(ErrorKind::NotFound, "no such file"),
        };
        let rendered = err.to_string();
        assert!(rendered.contains("/etc/efe-meta-swarm/tls/tls.crt"));
        assert!(rendered.contains("no such file"));
    }

    #[test]
    fn san_mismatch_displays_both_lists() {
        let err = TlsError::SanMismatch {
            expected: vec!["controller.efe.svc".into()],
            actual: vec!["evil.example.com".into()],
        };
        let rendered = err.to_string();
        assert!(rendered.contains("controller.efe.svc"));
        assert!(rendered.contains("evil.example.com"));
    }

    #[test]
    fn invalid_protocol_version_carries_input() {
        let err = TlsError::InvalidProtocolVersion { got: "1.4".into() };
        let rendered = err.to_string();
        assert!(rendered.contains("1.4"));
        assert!(rendered.contains("1.2"));
        assert!(rendered.contains("1.3"));
    }

    #[test]
    fn empty_cert_file_displays_path() {
        let path = PathBuf::from("/tmp/empty.crt");
        let err = TlsError::EmptyCertFile(path.clone());
        let rendered = err.to_string();
        assert!(rendered.contains("/tmp/empty.crt"));
    }

    #[test]
    fn converts_to_server_error() {
        let tls_err = TlsError::ReloadTimeout;
        let server_err: crate::error::ServerError = tls_err.into();
        // ServerError::Config is the destination per the From impl.
        assert!(matches!(server_err, crate::error::ServerError::Config(_)));
    }
}
