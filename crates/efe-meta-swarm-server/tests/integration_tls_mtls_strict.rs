//! PR #16 — strict mutual-TLS integration test.
//!
//! Covers AC9 :
//!
//!   * `tls.mutual: true` rejects clients that do **NOT** present a
//!     cert.
//!   * `tls.mutual: false` accepts clients without certs.
//!   * Mutual TLS rejects clients whose cert chain doesn't terminate
//!     in the configured CA.
//!
//! Distinct from `integration_tls_handshake.rs` (which proves the
//! happy path) and `integration_tls_san_validation.rs` (which
//! proves SAN matching) — this file's purpose is the
//! "no-cert client → rejected" semantic, which is the **strictest**
//! mTLS bit.

#![allow(clippy::missing_panics_doc)]

use std::sync::Arc;

use efe_meta_swarm_server::tls::{
    build_rustls_client_config, build_rustls_server_config, ProtocolVersion, TlsClientConfig,
    TlsServerConfig,
};
use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, PKCS_ECDSA_P256_SHA256};
use rustls::pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Cert factory : a CA + a leaf with the given SAN.
fn make_ca_and_leaf(san: &str) -> (String, String, String, String) {
    let ca_kp = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut ca_params = CertificateParams::new(vec!["mtls-strict-ca".to_string()]).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca = ca_params.self_signed(&ca_kp).unwrap();

    let leaf_kp = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let leaf_params = CertificateParams::new(vec![san.to_string()]).unwrap();
    let leaf = leaf_params.signed_by(&leaf_kp, &ca, &ca_kp).unwrap();
    (
        ca.pem(),
        leaf.pem(),
        leaf_kp.serialize_pem(),
        ca_kp.serialize_pem(),
    )
}

// (Inline TlsServerConfig construction in each test ; the
// previously-defined helper added more confusion than value.)

/// Drive a single handshake to completion (write + read a byte) ;
/// returns `Ok(())` on success.
async fn handshake_once(
    server_cfg: TlsServerConfig,
    client_cfg: TlsClientConfig,
    server_san: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let server_rustls = build_rustls_server_config(&server_cfg)?;
    let client_rustls = build_rustls_client_config(&client_cfg)?;

    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_rustls));
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client_rustls));
    let (server_io, client_io) = tokio::io::duplex(16 * 1024);

    let server_task = tokio::spawn(async move {
        let mut tls = acceptor.accept(server_io).await?;
        let mut b = [0u8; 1];
        tls.read_exact(&mut b).await?;
        tls.write_all(&b).await?;
        tls.flush().await?;
        Ok::<_, std::io::Error>(())
    });

    let san_owned = server_san.to_string();
    let client_task = tokio::spawn(async move {
        let sn: ServerName<'static> = ServerName::try_from(san_owned).expect("server name");
        let mut tls = connector.connect(sn, client_io).await?;
        tls.write_all(&[1u8]).await?;
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

#[tokio::test]
async fn pr16_mutual_true_rejects_client_with_no_identity() {
    // Server requires mTLS.
    let (server_ca, server_cert, server_key, _server_ca_kp) =
        make_ca_and_leaf("controller.efe.svc");
    let (_client_ca, _client_cert, _client_key, _client_ca_kp) =
        make_ca_and_leaf("participant-1.efe.svc");

    let server_cfg = TlsServerConfig {
        cert_chain_pem: server_cert.into_bytes(),
        key_pem: server_key.into_bytes(),
        client_ca_pem: Some(_client_ca.into_bytes()),
        min_protocol: ProtocolVersion::Tls13,
        require_mutual: true,
        allowed_peer_sans: vec!["participant-1.efe.svc".into()],
    };

    // Build a rustls client config WITHOUT a client identity (no
    // cert + no key) so the server's mTLS demand isn't met.
    // We can't construct a `TlsClientConfig` with empty cert bytes
    // since `parse_private_key` would reject ; instead we build a
    // rustls::ClientConfig directly with no client auth.
    let server_rustls = build_rustls_server_config(&server_cfg).expect("server rustls");

    // A rustls client config with NO client auth.
    let mut roots = rustls::RootCertStore::empty();
    let cas = rustls_pemfile::certs(&mut std::io::BufReader::new(server_ca.as_bytes()))
        .collect::<Result<Vec<_>, _>>()
        .expect("CA parse");
    for ca in cas {
        roots.add(ca).expect("root add");
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let client_rustls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();

    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_rustls));
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client_rustls));
    let (s, c) = tokio::io::duplex(16 * 1024);

    let server_task = tokio::spawn(async move {
        acceptor
            .accept(s)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    });
    let client_task = tokio::spawn(async move {
        let sn = ServerName::try_from("controller.efe.svc".to_string()).expect("san");
        connector
            .connect(sn, c)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    });

    let s_result = server_task.await.expect("server join");
    let c_result = client_task.await.expect("client join");
    assert!(
        s_result.is_err() || c_result.is_err(),
        "mTLS server MUST reject a client with no cert ; got server={s_result:?} client={c_result:?}"
    );
}

#[tokio::test]
async fn pr16_mutual_false_accepts_client_with_no_identity() {
    // Same scenario but with `require_mutual: false` — the server
    // doesn't request a client cert, so a no-identity client works.
    let (server_ca, server_cert, server_key, _) = make_ca_and_leaf("controller.efe.svc");

    let server_cfg = TlsServerConfig {
        cert_chain_pem: server_cert.into_bytes(),
        key_pem: server_key.into_bytes(),
        client_ca_pem: None,
        min_protocol: ProtocolVersion::Tls13,
        require_mutual: false,
        allowed_peer_sans: vec![],
    };

    let server_rustls = build_rustls_server_config(&server_cfg).expect("server rustls");

    let mut roots = rustls::RootCertStore::empty();
    let cas = rustls_pemfile::certs(&mut std::io::BufReader::new(server_ca.as_bytes()))
        .collect::<Result<Vec<_>, _>>()
        .expect("CA parse");
    for ca in cas {
        roots.add(ca).expect("root add");
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let client_rustls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();

    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_rustls));
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client_rustls));
    let (s, c) = tokio::io::duplex(16 * 1024);

    let server_task = tokio::spawn(async move {
        let mut tls = acceptor.accept(s).await.map_err(|e| e.to_string())?;
        let mut b = [0u8; 1];
        tls.read_exact(&mut b).await.map_err(|e| e.to_string())?;
        tls.write_all(&b).await.map_err(|e| e.to_string())?;
        tls.flush().await.map_err(|e| e.to_string())?;
        Ok::<_, String>(())
    });
    let client_task = tokio::spawn(async move {
        let sn = ServerName::try_from("controller.efe.svc".to_string()).expect("san");
        let mut tls = connector.connect(sn, c).await.map_err(|e| e.to_string())?;
        tls.write_all(&[7u8]).await.map_err(|e| e.to_string())?;
        tls.flush().await.map_err(|e| e.to_string())?;
        let mut b = [0u8; 1];
        tls.read_exact(&mut b).await.map_err(|e| e.to_string())?;
        Ok::<_, String>(())
    });

    server_task
        .await
        .expect("server join")
        .expect("server-side TLS without mTLS must succeed");
    client_task
        .await
        .expect("client join")
        .expect("server-side TLS without mTLS must succeed");
}

#[tokio::test]
async fn pr16_mutual_true_rejects_client_signed_by_unknown_ca() {
    // Server's CA bundle trusts ONLY the participant CA ; client
    // presents a cert signed by a SECOND, unrelated CA.
    let (good_client_ca, _good_client_leaf, _good_client_key, _good_client_ca_kp) =
        make_ca_and_leaf("participant-1.efe.svc");
    let (_bad_client_ca, bad_client_leaf, bad_client_key, _bad_client_ca_kp) =
        make_ca_and_leaf("participant-1.efe.svc");
    let (_server_ca, server_cert, server_key, _) = make_ca_and_leaf("controller.efe.svc");

    let server_cfg = TlsServerConfig {
        cert_chain_pem: server_cert.clone().into_bytes(),
        key_pem: server_key.clone().into_bytes(),
        client_ca_pem: Some(good_client_ca.clone().into_bytes()),
        min_protocol: ProtocolVersion::Tls13,
        require_mutual: true,
        allowed_peer_sans: vec!["participant-1.efe.svc".into()],
    };

    // Same SAN, but signed by an unrelated CA.
    let client_cfg = TlsClientConfig {
        cert_chain_pem: bad_client_leaf.into_bytes(),
        key_pem: bad_client_key.into_bytes(),
        server_ca_pem: server_cert.clone().into_bytes(),
        min_protocol: ProtocolVersion::Tls13,
        expected_server_san: "controller.efe.svc".into(),
    };

    let result = handshake_once(server_cfg, client_cfg, "controller.efe.svc").await;
    assert!(
        result.is_err(),
        "mTLS server MUST reject a client cert signed by an unknown CA"
    );
}
