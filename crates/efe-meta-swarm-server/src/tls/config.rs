//! Strongly-typed TLS configs and the rustls builder functions that
//! turn them into [`rustls::ServerConfig`] / [`rustls::ClientConfig`].
//!
//! ## Design notes
//!
//!   * **No public rustls types in the struct fields** : we keep
//!     `TlsServerConfig` and `TlsClientConfig` as plain owned bytes
//!     (cert PEM + key PEM + CA PEM) so they're cheap to swap via
//!     [`arc_swap::ArcSwap`] in the reload coordinator.  rustls
//!     types are reconstructed at config-build time.
//!   * **Strict SAN verifier** ([`StrictSanVerifier`]) wraps the
//!     rustls default WebPKI verifier and adds an allowlist check on
//!     the peer cert's DNS SANs.  IP SANs and wildcards are
//!     rejected.
//!   * **TLS 1.3-only by default** (`ProtocolVersion::Tls13`) ; TLS
//!     1.2 is opt-in but still drops every RSA-only suite.
//!   * **0-RTT (early data) disabled** for both server and client —
//!     mitigates the replay class of attacks per T45.

use std::str::FromStr;
use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::server::WebPkiClientVerifier;
use rustls::{DigitallySignedStruct, DistinguishedName, RootCertStore, SignatureScheme};

use super::TlsError;

// ─────────────────────────────────────────────────────────────────────
// Public types.
// ─────────────────────────────────────────────────────────────────────

/// Minimum supported TLS protocol version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolVersion {
    /// TLS 1.2.  Opt-in for legacy peers.
    Tls12,
    /// TLS 1.3.  Default.
    Tls13,
}

impl ProtocolVersion {
    /// Return the rustls protocol-version slice for this minimum.
    /// `Tls13` returns just `[TLS13]` ; `Tls12` returns `[TLS12, TLS13]`.
    ///
    /// The slices are `'static` because they live in module-scope
    /// `const` items below — returning a `&[&'static T]` reference
    /// requires the array itself to be `'static`, not a stack
    /// temporary.
    #[must_use]
    pub fn rustls_versions(self) -> &'static [&'static rustls::SupportedProtocolVersion] {
        const TLS13_ONLY: &[&rustls::SupportedProtocolVersion] = &[&rustls::version::TLS13];
        const TLS12_AND_TLS13: &[&rustls::SupportedProtocolVersion] =
            &[&rustls::version::TLS12, &rustls::version::TLS13];
        match self {
            Self::Tls13 => TLS13_ONLY,
            Self::Tls12 => TLS12_AND_TLS13,
        }
    }
}

impl FromStr for ProtocolVersion {
    type Err = TlsError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "1.2" | "TLSv1.2" => Ok(Self::Tls12),
            "1.3" | "TLSv1.3" => Ok(Self::Tls13),
            other => Err(TlsError::InvalidProtocolVersion {
                got: other.to_string(),
            }),
        }
    }
}

impl Default for ProtocolVersion {
    fn default() -> Self {
        Self::Tls13
    }
}

/// Server-side TLS material (owned PEM blobs + policy).
///
/// Owned bytes — no rustls types in the struct itself — so
/// `ArcSwap<TlsServerConfig>` can hot-swap on cert rotation without
/// re-running webpki validation in the swap critical path.
#[derive(Debug, Clone)]
pub struct TlsServerConfig {
    /// Leaf+intermediate cert chain (PEM bytes).
    pub cert_chain_pem: Vec<u8>,
    /// Matching private key (PEM bytes).
    pub key_pem: Vec<u8>,
    /// Optional CA bundle used to verify *client* certs in mTLS.
    /// `None` ⇒ no client cert verification (server-side TLS only).
    pub client_ca_pem: Option<Vec<u8>>,
    /// Minimum protocol version accepted.
    pub min_protocol: ProtocolVersion,
    /// When `true`, the server requires a client cert and verifies
    /// it against `client_ca_pem` + the SAN allowlist.
    pub require_mutual: bool,
    /// Allowed peer SANs.  Only populated when `require_mutual: true`.
    pub allowed_peer_sans: Vec<String>,
}

/// Client-side TLS material (owned PEM blobs + policy).
#[derive(Debug, Clone)]
pub struct TlsClientConfig {
    /// Leaf+intermediate cert chain (PEM bytes) used as the *client*
    /// identity.  Required when the peer (participant) requires mTLS.
    pub cert_chain_pem: Vec<u8>,
    /// Matching private key (PEM bytes).
    pub key_pem: Vec<u8>,
    /// CA bundle used to verify the *server* cert.
    pub server_ca_pem: Vec<u8>,
    /// Minimum protocol version accepted.
    pub min_protocol: ProtocolVersion,
    /// DNS SAN that the server cert MUST present.  Used as the
    /// `tonic` `domain_name` and matched against the cert's SAN list
    /// by `rustls`'s WebPKI verifier.
    pub expected_server_san: String,
}

// ─────────────────────────────────────────────────────────────────────
// rustls builders.
// ─────────────────────────────────────────────────────────────────────

/// Build a [`rustls::ServerConfig`] from a [`TlsServerConfig`].
///
/// The returned config :
///
///   * Enforces `cfg.min_protocol` as the minimum protocol version.
///   * Uses the rustls default cipher list (frozen per AD30).
///   * Installs a [`StrictSanVerifier`] when `cfg.require_mutual` is on,
///     otherwise installs `NoClientAuth`.
///   * Disables 0-RTT (early data) regardless of protocol version.
pub fn build_rustls_server_config(cfg: &TlsServerConfig) -> Result<rustls::ServerConfig, TlsError> {
    let cert_chain = parse_cert_chain(&cfg.cert_chain_pem)?;
    let key = parse_private_key(&cfg.key_pem)?;

    let provider = Arc::new(rustls::crypto::ring::default_provider());

    let base = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(cfg.min_protocol.rustls_versions())
        .map_err(|e| TlsError::RustlsConfig(e.to_string()))?;

    let verifier: Arc<dyn ClientCertVerifier> = if cfg.require_mutual {
        let ca_pem = cfg.client_ca_pem.as_deref().ok_or_else(|| {
            TlsError::RustlsConfig("mutual TLS requested but client_ca_pem is None".into())
        })?;
        let mut roots = RootCertStore::empty();
        let cas = parse_cert_chain(ca_pem)?;
        for ca in cas {
            roots
                .add(ca)
                .map_err(|e| TlsError::RustlsConfig(e.to_string()))?;
        }
        // `builder_with_provider`, NOT `builder` : the bare form resolves the
        // process-default CryptoProvider from crate features, which PANICS as
        // soon as the dependency graph unifies a second provider into rustls
        // (e.g. `aws-lc-rs` arriving through the Couche 3 M1.d optional
        // `efe-server` dependency).  Pinning ring here keeps every build —
        // default or `core-integration` — on one deterministic provider.
        let inner = WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider)
            .build()
            .map_err(|e| TlsError::RustlsConfig(e.to_string()))?;
        Arc::new(StrictSanVerifier::new(inner, cfg.allowed_peer_sans.clone()))
    } else {
        WebPkiClientVerifier::no_client_auth()
    };

    let mut server = base
        .with_client_cert_verifier(verifier)
        .with_single_cert(cert_chain, key)
        .map_err(|e| TlsError::RustlsConfig(e.to_string()))?;

    // Per T45 : disable 0-RTT (early data) to neutralise replay.
    server.max_early_data_size = 0;

    Ok(server)
}

/// Build a [`rustls::ClientConfig`] from a [`TlsClientConfig`].
///
/// The returned config :
///
///   * Enforces `cfg.min_protocol`.
///   * Installs the WebPKI verifier rooted at `server_ca_pem`.
///   * Configures the client identity (cert chain + key) used when
///     the peer requests a client cert (i.e. mTLS).
///   * Disables 0-RTT.
pub fn build_rustls_client_config(cfg: &TlsClientConfig) -> Result<rustls::ClientConfig, TlsError> {
    let cert_chain = parse_cert_chain(&cfg.cert_chain_pem)?;
    let key = parse_private_key(&cfg.key_pem)?;
    let ca_chain = parse_cert_chain(&cfg.server_ca_pem)?;

    let mut roots = RootCertStore::empty();
    for ca in ca_chain {
        roots
            .add(ca)
            .map_err(|e| TlsError::RustlsConfig(e.to_string()))?;
    }

    let provider = Arc::new(rustls::crypto::ring::default_provider());

    let base = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(cfg.min_protocol.rustls_versions())
        .map_err(|e| TlsError::RustlsConfig(e.to_string()))?
        .with_root_certificates(roots);

    let mut client = base
        .with_client_auth_cert(cert_chain, key)
        .map_err(|e| TlsError::RustlsConfig(e.to_string()))?;

    client.enable_early_data = false;

    Ok(client)
}

// ─────────────────────────────────────────────────────────────────────
// Strict-SAN client cert verifier.
// ─────────────────────────────────────────────────────────────────────

/// Client-cert verifier that runs rustls's WebPKI verification AND
/// then enforces the configured SAN allowlist.
///
/// The allowlist is matched against the cert's DNS SANs.  IP SANs are
/// ignored (rejected upstream in the chart's Cert template — § 10.3).
/// Wildcard SANs are also rejected by the chart side.
///
/// Logic :
///
///   1. Delegate chain validation to the inner WebPKI verifier.
///   2. Pull the DNS SANs from the cert.
///   3. Require at least one DNS SAN ∈ `allowed_sans`.  Otherwise
///      return [`rustls::Error::General`] with a clear message.
#[derive(Debug)]
pub struct StrictSanVerifier {
    inner: Arc<dyn ClientCertVerifier>,
    allowed_sans: Vec<String>,
}

impl StrictSanVerifier {
    /// Wrap a base WebPKI verifier with the SAN allowlist.
    #[must_use]
    pub fn new(inner: Arc<dyn ClientCertVerifier>, allowed_sans: Vec<String>) -> Self {
        Self {
            inner,
            allowed_sans,
        }
    }
}

impl ClientCertVerifier for StrictSanVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        self.inner.root_hint_subjects()
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        // 1. Chain + signature validation via WebPKI.
        let _verified = self
            .inner
            .verify_client_cert(end_entity, intermediates, now)?;

        // 2. SAN allowlist check.
        let sans = extract_dns_sans(end_entity)?;
        if !sans
            .iter()
            .any(|s| self.allowed_sans.iter().any(|allowed| allowed == s))
        {
            return Err(rustls::Error::General(format!(
                "client cert SAN mismatch: sans={sans:?}, allowed={:?}",
                self.allowed_sans
            )));
        }

        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

// ─────────────────────────────────────────────────────────────────────
// Strict-SAN server cert verifier (controller-as-client → participant).
// ─────────────────────────────────────────────────────────────────────

/// Server-cert verifier symmetric to [`StrictSanVerifier`] : runs
/// rustls's WebPKI verifier, then enforces that the server's cert
/// presents the SAN we dialed.
///
/// This is wired into the controller's `Channel::tls_config` via the
/// tonic `domain_name` setting (which rustls cross-checks against the
/// cert SAN by default).  We additionally surface SAN mismatches as
/// explicit failures with a clear message for log readers.
#[derive(Debug)]
pub struct StrictServerSanVerifier {
    inner: Arc<WebPkiServerVerifier>,
    expected_san: String,
}

impl StrictServerSanVerifier {
    /// Build a wrapper around the configured WebPKI verifier.
    #[must_use]
    pub fn new(inner: Arc<WebPkiServerVerifier>, expected_san: String) -> Self {
        Self {
            inner,
            expected_san,
        }
    }
}

impl ServerCertVerifier for StrictServerSanVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let _verified =
            self.inner
                .verify_server_cert(end_entity, intermediates, server_name, ocsp, now)?;
        let sans = extract_dns_sans(end_entity)?;
        if !sans.iter().any(|s| s == &self.expected_san) {
            return Err(rustls::Error::General(format!(
                "server cert SAN mismatch: sans={sans:?}, expected={:?}",
                self.expected_san
            )));
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

// ─────────────────────────────────────────────────────────────────────
// Helpers.
// ─────────────────────────────────────────────────────────────────────

fn parse_cert_chain(pem: &[u8]) -> Result<Vec<CertificateDer<'static>>, TlsError> {
    let mut reader = std::io::BufReader::new(pem);
    rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| TlsError::PemParse {
            path: std::path::PathBuf::from("<in-memory PEM>"),
            message: e.to_string(),
        })
}

fn parse_private_key(pem: &[u8]) -> Result<PrivateKeyDer<'static>, TlsError> {
    use rustls_pki_types::{PrivatePkcs1KeyDer, PrivatePkcs8KeyDer};

    {
        let mut reader = std::io::BufReader::new(pem);
        let pkcs8: Vec<PrivatePkcs8KeyDer<'static>> =
            rustls_pemfile::pkcs8_private_keys(&mut reader)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| TlsError::PemParse {
                    path: std::path::PathBuf::from("<in-memory PEM>"),
                    message: e.to_string(),
                })?;
        if let Some(key) = pkcs8.into_iter().next() {
            return Ok(PrivateKeyDer::Pkcs8(key));
        }
    }
    {
        let mut reader = std::io::BufReader::new(pem);
        let pkcs1: Vec<PrivatePkcs1KeyDer<'static>> = rustls_pemfile::rsa_private_keys(&mut reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| TlsError::PemParse {
                path: std::path::PathBuf::from("<in-memory PEM>"),
                message: e.to_string(),
            })?;
        if let Some(key) = pkcs1.into_iter().next() {
            return Ok(PrivateKeyDer::Pkcs1(key));
        }
    }
    Err(TlsError::UnsupportedKeyFormat(std::path::PathBuf::from(
        "<in-memory PEM>",
    )))
}

/// Extract the DNS SANs from an X.509 cert.
///
/// AC14 — IP SANs and wildcard SANs are **actively rejected** : if
/// the cert carries any IP SAN, the verifier returns
/// `rustls::Error::General("... IP SAN forbidden ...")` immediately
/// rather than silently ignoring it.  This defends against an
/// operator manually injecting a cert that bypasses the chart-side
/// schema gate.
///
/// Wildcard DNS SANs (`*.efe.svc`) are similarly rejected so the
/// allowlist matching stays exact-equality only (AC15).
fn extract_dns_sans(cert: &CertificateDer<'_>) -> Result<Vec<String>, rustls::Error> {
    let (_, parsed) = x509_parser::parse_x509_certificate(cert.as_ref())
        .map_err(|e| rustls::Error::General(format!("x509 parse error: {e}")))?;

    let mut dns_names = Vec::new();
    if let Ok(Some(san_ext)) = parsed.subject_alternative_name() {
        for gn in &san_ext.value.general_names {
            match gn {
                x509_parser::extensions::GeneralName::DNSName(name) => {
                    let s = (*name).to_string();
                    if s.starts_with('*') || s.contains("*.") {
                        return Err(rustls::Error::General(format!(
                            "wildcard SAN forbidden : {s:?} (PR #16 AC15)"
                        )));
                    }
                    dns_names.push(s);
                }
                x509_parser::extensions::GeneralName::IPAddress(_) => {
                    return Err(rustls::Error::General(
                        "IP SAN forbidden : peer cert must present DNS SANs only (PR #16 AC14)"
                            .into(),
                    ));
                }
                _ => {
                    // Other GeneralName variants (URI, email, etc.)
                    // are ignored — they don't influence DNS matching.
                }
            }
        }
    }
    Ok(dns_names)
}

// ─────────────────────────────────────────────────────────────────────
// Tests.
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_default_is_tls13() {
        assert_eq!(ProtocolVersion::default(), ProtocolVersion::Tls13);
    }

    #[test]
    fn protocol_version_from_str_accepts_canonical_forms() {
        assert_eq!(
            ProtocolVersion::from_str("1.2").unwrap(),
            ProtocolVersion::Tls12
        );
        assert_eq!(
            ProtocolVersion::from_str("1.3").unwrap(),
            ProtocolVersion::Tls13
        );
        assert_eq!(
            ProtocolVersion::from_str("TLSv1.2").unwrap(),
            ProtocolVersion::Tls12
        );
        assert_eq!(
            ProtocolVersion::from_str("TLSv1.3").unwrap(),
            ProtocolVersion::Tls13
        );
    }

    #[test]
    fn protocol_version_from_str_rejects_invalid() {
        let err = ProtocolVersion::from_str("1.4").unwrap_err();
        assert!(matches!(err, TlsError::InvalidProtocolVersion { got } if got == "1.4"));

        let err = ProtocolVersion::from_str("1.0").unwrap_err();
        assert!(matches!(err, TlsError::InvalidProtocolVersion { .. }));
    }

    #[test]
    fn protocol_version_from_str_trims_whitespace() {
        assert_eq!(
            ProtocolVersion::from_str("  1.3  ").unwrap(),
            ProtocolVersion::Tls13
        );
    }

    #[test]
    fn rustls_versions_tls13_only_returns_single_slot() {
        let v = ProtocolVersion::Tls13.rustls_versions();
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn rustls_versions_tls12_includes_both() {
        let v = ProtocolVersion::Tls12.rustls_versions();
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn build_server_config_fails_with_empty_pem() {
        let cfg = TlsServerConfig {
            cert_chain_pem: b"".to_vec(),
            key_pem: b"".to_vec(),
            client_ca_pem: None,
            min_protocol: ProtocolVersion::Tls13,
            require_mutual: false,
            allowed_peer_sans: vec![],
        };
        let err = build_rustls_server_config(&cfg).unwrap_err();
        // either RustlsConfig or PemParse — both acceptable
        match err {
            TlsError::RustlsConfig(_)
            | TlsError::PemParse { .. }
            | TlsError::UnsupportedKeyFormat(_) => (),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn build_server_config_mutual_without_ca_errors() {
        let cfg = TlsServerConfig {
            cert_chain_pem: b"-----BEGIN CERTIFICATE-----\nQUFB\n-----END CERTIFICATE-----\n"
                .to_vec(),
            key_pem: b"-----BEGIN PRIVATE KEY-----\nQUFB\n-----END PRIVATE KEY-----\n".to_vec(),
            client_ca_pem: None,
            min_protocol: ProtocolVersion::Tls13,
            require_mutual: true,
            allowed_peer_sans: vec!["controller.efe.svc".into()],
        };
        let err = build_rustls_server_config(&cfg).unwrap_err();
        assert!(matches!(
            err,
            TlsError::RustlsConfig(_) | TlsError::PemParse { .. }
        ));
    }
}
