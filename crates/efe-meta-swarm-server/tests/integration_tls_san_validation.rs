//! PR #16 — SAN validation integration tests.
//!
//! Covers AC13 / AC14 / AC15 :
//!
//!   * AC13 — Cert with SAN not matching the expected DNS name is
//!     rejected with `NotValidForName`-class error.
//!   * AC14 — Cert with IP-only SAN (no DNS SAN) is rejected because
//!     the binary side connects via DNS hostnames and our verifier
//!     extracts DNS SANs only.
//!   * AC15 — Wildcard SAN certs are NOT issued by the chart's Cert
//!     resource (covered by the chart-side schema ; this Rust test
//!     verifies that even if a wildcard cert WERE somehow injected,
//!     the strict SAN allowlist still rejects it).
//!
//! Distinct from `integration_tls_handshake.rs` (which proves the
//! happy path) and `integration_tls_mtls_strict.rs` (which proves
//! no-cert clients are rejected) — this file's sole focus is the
//! SAN extraction + match logic in `tls::config::StrictSanVerifier`.

#![allow(clippy::missing_panics_doc)]

use std::sync::Arc;

use efe_meta_swarm_server::tls::{
    build_rustls_client_config, build_rustls_server_config, ProtocolVersion, TlsClientConfig,
    TlsServerConfig,
};
use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, SanType, PKCS_ECDSA_P256_SHA256};
use rustls::pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Build a CA + leaf where the leaf carries arbitrary SANs of the
/// caller's choice.  Returns `(ca_pem, leaf_cert_pem, leaf_key_pem)`.
fn ca_and_leaf_with_sans(sans: Vec<SanType>) -> (String, String, String) {
    let ca_kp = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut ca_params = CertificateParams::new(vec!["san-test-ca".to_string()]).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca = ca_params.self_signed(&ca_kp).unwrap();

    let leaf_kp = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut leaf_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    leaf_params.subject_alt_names = sans;
    let leaf = leaf_params.signed_by(&leaf_kp, &ca, &ca_kp).unwrap();

    (ca.pem(), leaf.pem(), leaf_kp.serialize_pem())
}

async fn run_handshake(
    server_cfg: TlsServerConfig,
    client_cfg: TlsClientConfig,
    server_san: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let server_rustls = build_rustls_server_config(&server_cfg)?;
    let client_rustls = build_rustls_client_config(&client_cfg)?;
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_rustls));
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client_rustls));
    let (s, c) = tokio::io::duplex(16 * 1024);

    let server_task = tokio::spawn(async move {
        let mut tls = acceptor.accept(s).await?;
        let mut b = [0u8; 1];
        tls.read_exact(&mut b).await?;
        tls.write_all(&b).await?;
        tls.flush().await?;
        Ok::<_, std::io::Error>(())
    });

    let san_owned = server_san.to_string();
    let client_task = tokio::spawn(async move {
        let sn: ServerName<'static> = ServerName::try_from(san_owned).expect("server name");
        let mut tls = connector.connect(sn, c).await?;
        tls.write_all(&[3u8]).await?;
        tls.flush().await?;
        let mut b = [0u8; 1];
        tls.read_exact(&mut b).await?;
        Ok::<_, std::io::Error>(())
    });

    tokio::try_join!(
        async {
            server_task
                .await
                .map_err(|e| format!("server join: {e}"))?
                .map_err(|e| format!("server: {e}"))
        },
        async {
            client_task
                .await
                .map_err(|e| format!("client join: {e}"))?
                .map_err(|e| format!("client: {e}"))
        },
    )?;
    Ok(())
}

/// Build a "normal" server side bundle for these tests.
fn normal_server_bundle() -> (String, String, String) {
    ca_and_leaf_with_sans(vec![SanType::DnsName(
        "controller.efe.svc".try_into().unwrap(),
    )])
}

#[tokio::test]
async fn pr16_ac13_client_san_mismatch_rejected() {
    // Server's allowlist accepts ONLY `participant-1.efe.svc` ;
    // client presents `participant-2.efe.svc` instead.
    let (server_ca, server_cert, server_key) = normal_server_bundle();
    let (client_ca, client_cert, client_key) = ca_and_leaf_with_sans(vec![SanType::DnsName(
        "participant-2.efe.svc".try_into().unwrap(),
    )]);

    let server_cfg = TlsServerConfig {
        cert_chain_pem: server_cert.into_bytes(),
        key_pem: server_key.into_bytes(),
        client_ca_pem: Some(client_ca.into_bytes()),
        min_protocol: ProtocolVersion::Tls13,
        require_mutual: true,
        allowed_peer_sans: vec!["participant-1.efe.svc".into()],
    };

    let client_cfg = TlsClientConfig {
        cert_chain_pem: client_cert.into_bytes(),
        key_pem: client_key.into_bytes(),
        server_ca_pem: server_ca.into_bytes(),
        min_protocol: ProtocolVersion::Tls13,
        expected_server_san: "controller.efe.svc".into(),
    };

    let result = run_handshake(server_cfg, client_cfg, "controller.efe.svc").await;
    assert!(result.is_err(), "SAN mismatch must be rejected");
}

#[tokio::test]
async fn pr16_ac14_client_ip_only_san_rejected() {
    // Client cert carries ONLY an IP SAN — no DNS SAN.  Our
    // `extract_dns_sans` returns an empty list ; the StrictSanVerifier
    // can't match any allowed DNS SAN against an empty set.
    let (server_ca, server_cert, server_key) = normal_server_bundle();
    let (client_ca, client_cert, client_key) =
        ca_and_leaf_with_sans(vec![SanType::IpAddress("10.0.0.5".parse().unwrap())]);

    let server_cfg = TlsServerConfig {
        cert_chain_pem: server_cert.into_bytes(),
        key_pem: server_key.into_bytes(),
        client_ca_pem: Some(client_ca.into_bytes()),
        min_protocol: ProtocolVersion::Tls13,
        require_mutual: true,
        allowed_peer_sans: vec!["participant-1.efe.svc".into()],
    };

    let client_cfg = TlsClientConfig {
        cert_chain_pem: client_cert.into_bytes(),
        key_pem: client_key.into_bytes(),
        server_ca_pem: server_ca.into_bytes(),
        min_protocol: ProtocolVersion::Tls13,
        expected_server_san: "controller.efe.svc".into(),
    };

    let result = run_handshake(server_cfg, client_cfg, "controller.efe.svc").await;
    assert!(result.is_err(), "IP-only SAN client cert must be rejected");
}

#[tokio::test]
async fn pr16_ac15_wildcard_san_not_accepted_implicitly() {
    // Edge case : even if a wildcard cert WERE somehow injected,
    // our exact-string allowlist rejects it unless the wildcard
    // itself is in the allowlist (which the chart's values.schema
    // also rejects via the `format: hostname` constraint on
    // additionalSans).
    let (server_ca, server_cert, server_key) = normal_server_bundle();
    let (client_ca, client_cert, client_key) =
        ca_and_leaf_with_sans(vec![SanType::DnsName("*.efe.svc".try_into().unwrap())]);

    let server_cfg = TlsServerConfig {
        cert_chain_pem: server_cert.into_bytes(),
        key_pem: server_key.into_bytes(),
        client_ca_pem: Some(client_ca.into_bytes()),
        min_protocol: ProtocolVersion::Tls13,
        require_mutual: true,
        // The chart's schema would never let `*.efe.svc` into this list ;
        // we test that even if it WERE, the verifier still rejects since
        // the client's literal SAN string `*.efe.svc` is being matched
        // against the literal `participant-1.efe.svc`.
        allowed_peer_sans: vec!["participant-1.efe.svc".into()],
    };

    let client_cfg = TlsClientConfig {
        cert_chain_pem: client_cert.into_bytes(),
        key_pem: client_key.into_bytes(),
        server_ca_pem: server_ca.into_bytes(),
        min_protocol: ProtocolVersion::Tls13,
        expected_server_san: "controller.efe.svc".into(),
    };

    let result = run_handshake(server_cfg, client_cfg, "controller.efe.svc").await;
    assert!(
        result.is_err(),
        "wildcard SAN must NOT implicitly match — our verifier requires exact match"
    );
}

#[tokio::test]
async fn pr16_ac13_server_san_mismatch_rejected_by_client() {
    // Server presents `controller.evil.invalid`, client dials
    // `controller.efe.svc` — rustls's WebPKI verifier rejects.
    let (server_ca, server_cert, server_key) = ca_and_leaf_with_sans(vec![SanType::DnsName(
        "controller.evil.invalid".try_into().unwrap(),
    )]);

    let server_cfg = TlsServerConfig {
        cert_chain_pem: server_cert.into_bytes(),
        key_pem: server_key.into_bytes(),
        client_ca_pem: None,
        min_protocol: ProtocolVersion::Tls13,
        require_mutual: false,
        allowed_peer_sans: vec![],
    };

    let (_other_ca, other_cert, other_key) = normal_server_bundle();
    let client_cfg = TlsClientConfig {
        cert_chain_pem: other_cert.into_bytes(),
        key_pem: other_key.into_bytes(),
        server_ca_pem: server_ca.into_bytes(),
        min_protocol: ProtocolVersion::Tls13,
        expected_server_san: "controller.efe.svc".into(),
    };

    let result = run_handshake(server_cfg, client_cfg, "controller.efe.svc").await;
    assert!(
        result.is_err(),
        "server cert SAN must match the dialed name — rustls WebPKI enforces this"
    );
}
