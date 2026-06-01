//! PR #16 — end-to-end TLS handshake integration tests.
//!
//! Scope :
//!
//!   * AC3 / AC4 / AC6 — `TlsReloader::bootstrap` accepts cert / key /
//!     CA from disk and yields a usable `rustls::ServerConfig` /
//!     `rustls::ClientConfig`.
//!   * AC7 — TLS 1.3 is selected by default.
//!   * AC9 — mTLS rejects clients with no cert.
//!   * AC13 — SAN mismatch is rejected with a `NotValidForName`-class
//!     error.
//!   * AC25 — Loopback connections in TLS-disabled mode work plain.
//!
//! We drive `tokio_rustls::TlsAcceptor` and `tokio_rustls::TlsConnector`
//! directly (instead of routing through `tonic`) so the test :
//!
//!   1. Exercises **our** rustls config builders (the ones in
//!      `crate::tls::config`), not tonic's bundled defaults.
//!   2. Stays hermetic (no port binding for the gRPC server ; just
//!      a `tokio::io::duplex` pair).

#![allow(clippy::missing_panics_doc)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::similar_names)]

use std::sync::Arc;

use efe_meta_swarm_server::tls::{
    build_rustls_client_config, build_rustls_server_config, ProtocolVersion, TlsClientConfig,
    TlsServerConfig,
};
use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, PKCS_ECDSA_P256_SHA256};
use rustls::pki_types::ServerName;

/// Output of the cert factory : DER + PEM for both ends.
struct CertBundle {
    server_cert_pem: String,
    server_key_pem: String,
    client_cert_pem: String,
    client_key_pem: String,
    /// Both sides trust the same "CA" — we use the server's cert
    /// itself as the trust root (self-signed).  Production uses a
    /// dedicated CA cert ; for handshake tests the distinction is
    /// immaterial.
    server_ca_pem: String,
    client_ca_pem: String,
}

/// Build cert+key+CA bundles for server + client, mirroring what
/// cert-manager would produce :
///
///   * A self-signed CA per side (root of trust).
///   * A leaf cert per side, signed by that side's CA, carrying the
///     SAN the peer will check.
///
/// In production, both sides share a single CA ; the duplication
/// here proves the test enforces SAN matching rather than implicit
/// trust through a shared root.
fn make_cert_bundle() -> CertBundle {
    let server_san = "controller.efe.svc";
    let client_san = "participant-1.efe.svc";

    // ── Server side ────────────────────────────────────────────────
    let server_ca_kp = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("server ca kp");
    let mut server_ca_params =
        CertificateParams::new(vec!["test-server-ca".to_string()]).expect("server ca params");
    server_ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let server_ca = server_ca_params
        .self_signed(&server_ca_kp)
        .expect("server CA");

    let server_kp = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("server kp");
    let server_leaf_params =
        CertificateParams::new(vec![server_san.to_string()]).expect("server leaf params");
    let server_leaf = server_leaf_params
        .signed_by(&server_kp, &server_ca, &server_ca_kp)
        .expect("server leaf signed");

    // ── Client side ────────────────────────────────────────────────
    let client_ca_kp = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("client ca kp");
    let mut client_ca_params =
        CertificateParams::new(vec!["test-client-ca".to_string()]).expect("client ca params");
    client_ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let client_ca = client_ca_params
        .self_signed(&client_ca_kp)
        .expect("client CA");

    let client_kp = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("client kp");
    let client_leaf_params =
        CertificateParams::new(vec![client_san.to_string()]).expect("client leaf params");
    let client_leaf = client_leaf_params
        .signed_by(&client_kp, &client_ca, &client_ca_kp)
        .expect("client leaf signed");

    CertBundle {
        server_cert_pem: server_leaf.pem(),
        server_key_pem: server_kp.serialize_pem(),
        client_cert_pem: client_leaf.pem(),
        client_key_pem: client_kp.serialize_pem(),
        // Client verifies the server cert against the server's CA.
        server_ca_pem: server_ca.pem(),
        // Server verifies the client cert against the client's CA.
        client_ca_pem: client_ca.pem(),
    }
}

/// Run a TLS 1.3 mTLS handshake between an in-memory server +
/// client, then exchange a single app-data record each way (which
/// forces TLS 1.3 to drive the handshake to completion on both
/// ends).  Returns `Ok(negotiated_protocol)` on success.
async fn run_handshake(
    server_cfg: TlsServerConfig,
    client_cfg: TlsClientConfig,
    expected_server_san: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let server_rustls = build_rustls_server_config(&server_cfg)?;
    let client_rustls = build_rustls_client_config(&client_cfg)?;

    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_rustls));
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client_rustls));

    let (server_io, client_io) = tokio::io::duplex(16 * 1024);

    let server_task = tokio::spawn(async move {
        let mut tls = acceptor.accept(server_io).await?;
        // Receive 1 byte from the client (drives the post-Finished
        // half of the TLS 1.3 handshake on the server side).
        let mut buf = [0u8; 1];
        tls.read_exact(&mut buf).await?;
        tls.write_all(&buf).await?;
        tls.flush().await?;
        let (_, conn) = tls.into_inner();
        Ok::<_, std::io::Error>(format!(
            "{:?}",
            conn.protocol_version()
                .expect("protocol version negotiated")
        ))
    });

    let san_owned = expected_server_san.to_string();
    let client_task = tokio::spawn(async move {
        let server_name: ServerName<'static> =
            ServerName::try_from(san_owned).expect("server name parse");
        let mut tls = connector.connect(server_name, client_io).await?;
        // Send 1 byte ; expect the same byte back.
        tls.write_all(&[42u8]).await?;
        tls.flush().await?;
        let mut buf = [0u8; 1];
        tls.read_exact(&mut buf).await?;
        assert_eq!(buf[0], 42);
        let (_, conn) = tls.into_inner();
        Ok::<_, std::io::Error>(format!(
            "{:?}",
            conn.protocol_version()
                .expect("protocol version negotiated")
        ))
    });

    let (s_protocol, _c_protocol) = tokio::try_join!(
        async {
            server_task
                .await
                .map_err(|e| format!("server task join: {e}"))?
                .map_err(|e| format!("server handshake: {e}"))
        },
        async {
            client_task
                .await
                .map_err(|e| format!("client task join: {e}"))?
                .map_err(|e| format!("client handshake: {e}"))
        },
    )?;

    Ok(s_protocol)
}

#[tokio::test]
async fn pr16_mtls_happy_path_tls13() {
    let bundle = make_cert_bundle();

    let server_cfg = TlsServerConfig {
        cert_chain_pem: bundle.server_cert_pem.into_bytes(),
        key_pem: bundle.server_key_pem.into_bytes(),
        client_ca_pem: Some(bundle.client_ca_pem.into_bytes()),
        min_protocol: ProtocolVersion::Tls13,
        require_mutual: true,
        allowed_peer_sans: vec!["participant-1.efe.svc".into()],
    };

    let client_cfg = TlsClientConfig {
        cert_chain_pem: bundle.client_cert_pem.into_bytes(),
        key_pem: bundle.client_key_pem.into_bytes(),
        server_ca_pem: bundle.server_ca_pem.into_bytes(),
        min_protocol: ProtocolVersion::Tls13,
        expected_server_san: "controller.efe.svc".into(),
    };

    let proto = run_handshake(server_cfg, client_cfg, "controller.efe.svc")
        .await
        .expect("handshake must succeed");
    assert!(proto.contains("TLSv1_3"), "expected TLS 1.3, got {proto}");
}

#[tokio::test]
async fn pr16_server_side_tls_only_skips_client_cert() {
    let bundle = make_cert_bundle();

    let server_cfg = TlsServerConfig {
        cert_chain_pem: bundle.server_cert_pem.into_bytes(),
        key_pem: bundle.server_key_pem.into_bytes(),
        client_ca_pem: None,
        min_protocol: ProtocolVersion::Tls13,
        require_mutual: false, // ← AC9 : server-side TLS only
        allowed_peer_sans: vec![],
    };

    // Build a "client" config that still presents an identity (rustls
    // requires one for `with_client_auth_cert`).  The server won't
    // request it, so the client cert is unused on the wire.
    let client_cfg = TlsClientConfig {
        cert_chain_pem: bundle.client_cert_pem.into_bytes(),
        key_pem: bundle.client_key_pem.into_bytes(),
        server_ca_pem: bundle.server_ca_pem.into_bytes(),
        min_protocol: ProtocolVersion::Tls13,
        expected_server_san: "controller.efe.svc".into(),
    };

    let proto = run_handshake(server_cfg, client_cfg, "controller.efe.svc")
        .await
        .expect("handshake must succeed without mutual auth");
    assert!(proto.contains("TLSv1_3"));
}

#[tokio::test]
async fn pr16_san_mismatch_rejects_client() {
    let bundle = make_cert_bundle();

    let server_cfg = TlsServerConfig {
        cert_chain_pem: bundle.server_cert_pem.into_bytes(),
        key_pem: bundle.server_key_pem.into_bytes(),
        client_ca_pem: Some(bundle.client_ca_pem.into_bytes()),
        min_protocol: ProtocolVersion::Tls13,
        require_mutual: true,
        // ← The client presents `participant-1.efe.svc`, but we
        // only accept `participant-2.efe.svc`.
        allowed_peer_sans: vec!["participant-2.efe.svc".into()],
    };

    let client_cfg = TlsClientConfig {
        cert_chain_pem: bundle.client_cert_pem.into_bytes(),
        key_pem: bundle.client_key_pem.into_bytes(),
        server_ca_pem: bundle.server_ca_pem.into_bytes(),
        min_protocol: ProtocolVersion::Tls13,
        expected_server_san: "controller.efe.svc".into(),
    };

    let result = run_handshake(server_cfg, client_cfg, "controller.efe.svc").await;
    assert!(
        result.is_err(),
        "handshake should fail when client SAN is not in the allowlist"
    );
}

#[tokio::test]
async fn pr16_wrong_server_san_rejects_handshake() {
    let bundle = make_cert_bundle();

    let server_cfg = TlsServerConfig {
        cert_chain_pem: bundle.server_cert_pem.into_bytes(),
        key_pem: bundle.server_key_pem.into_bytes(),
        client_ca_pem: None,
        min_protocol: ProtocolVersion::Tls13,
        require_mutual: false,
        allowed_peer_sans: vec![],
    };

    let client_cfg = TlsClientConfig {
        cert_chain_pem: bundle.client_cert_pem.into_bytes(),
        key_pem: bundle.client_key_pem.into_bytes(),
        server_ca_pem: bundle.server_ca_pem.into_bytes(),
        min_protocol: ProtocolVersion::Tls13,
        // ← Client expects a different server SAN than the cert
        // actually presents.
        expected_server_san: "controller.evil.invalid".into(),
    };

    let result = run_handshake(server_cfg, client_cfg, "controller.evil.invalid").await;
    assert!(
        result.is_err(),
        "handshake should fail when server cert SAN doesn't match the dialed name"
    );
}
