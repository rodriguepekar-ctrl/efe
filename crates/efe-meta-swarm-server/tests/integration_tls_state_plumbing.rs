//! PR #16 — verify the `ControllerState ↔ TlsReloader ↔
//! connect_participants` plumbing is correctly wired.
//!
//! Covers the operational gap caught during the deep-audit pass :
//!
//!   * `ControllerState.tls_reloader` defaults to `None` (backward-compat).
//!   * `set_tls_reloader(...)` populates the field ; clones of the
//!     state retain the same `Arc<TlsReloader>` reference.
//!   * `current_client_config()` is reachable via the cloned state ;
//!     the controller's iterate path can consult it without holding
//!     a mutex.
//!
//! Without this test, the FIX1 plumbing could silently regress in a
//! future refactor (e.g. someone makes `tls_reloader` private).

#![allow(clippy::missing_panics_doc)]

use std::sync::Arc;

use efe_meta_swarm_server::controller::ControllerState;
use efe_meta_swarm_server::tls::{reload::TlsPaths, ProtocolVersion, TlsMetrics, TlsReloader};
use prometheus::Registry;
use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, PKCS_ECDSA_P256_SHA256};
use tempfile::tempdir;

fn make_triplet() -> (String, String, String) {
    let ca_kp = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut ca_params = CertificateParams::new(vec!["plumbing-ca".to_string()]).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca = ca_params.self_signed(&ca_kp).unwrap();
    let leaf_kp = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let leaf_params = CertificateParams::new(vec!["controller.efe.svc".to_string()]).unwrap();
    let leaf = leaf_params.signed_by(&leaf_kp, &ca, &ca_kp).unwrap();
    (leaf.pem(), leaf_kp.serialize_pem(), ca.pem())
}

#[test]
fn pr16_state_tls_reloader_defaults_to_none() {
    use efe_meta_swarm_server::config::controller::{ControllerConfig, DiscoveryMode};

    let cfg = ControllerConfig {
        grpc_bind: "127.0.0.1:0".parse().unwrap(),
        metrics_bind: "127.0.0.1:0".parse().unwrap(),
        discovery: DiscoveryMode::StaticLocalhost,
        static_participants: vec![],
        manifest_path: "/dev/null".into(),
        max_inflight: 10,
        default_max_wall_ms: 15_000,
        worker_threads: 1,
        reflection_enabled: false,
        tls_required: false,
        tls_cert_path: None,
        tls_key_path: None,
        tls_ca_path: None,
        tls_mutual: false,
        tls_min_protocol: ProtocolVersion::Tls13,
        tls_allowed_peer_sans: Vec::new(),
        tls_peer_san: None,
        drain_timeout: std::time::Duration::from_secs(30),
    };
    let state = ControllerState::new_for_tests(cfg);
    assert!(
        state.tls_reloader.is_none(),
        "default ControllerState must have tls_reloader = None (backward-compat with PR #14)"
    );
}

#[tokio::test]
async fn pr16_state_set_tls_reloader_populates_field() {
    use efe_meta_swarm_server::config::controller::{ControllerConfig, DiscoveryMode};

    let cfg = ControllerConfig {
        grpc_bind: "127.0.0.1:0".parse().unwrap(),
        metrics_bind: "127.0.0.1:0".parse().unwrap(),
        discovery: DiscoveryMode::StaticLocalhost,
        static_participants: vec![],
        manifest_path: "/dev/null".into(),
        max_inflight: 10,
        default_max_wall_ms: 15_000,
        worker_threads: 1,
        reflection_enabled: false,
        tls_required: true,
        tls_cert_path: None,
        tls_key_path: None,
        tls_ca_path: None,
        tls_mutual: false,
        tls_min_protocol: ProtocolVersion::Tls13,
        tls_allowed_peer_sans: Vec::new(),
        tls_peer_san: None,
        drain_timeout: std::time::Duration::from_secs(30),
    };

    let dir = tempdir().unwrap();
    let (cert, key, ca) = make_triplet();
    let cert_p = dir.path().join("tls.crt");
    let key_p = dir.path().join("tls.key");
    let ca_p = dir.path().join("ca.crt");
    std::fs::write(&cert_p, cert).unwrap();
    std::fs::write(&key_p, key).unwrap();
    std::fs::write(&ca_p, ca).unwrap();

    let metrics = Arc::new(TlsMetrics::register(&Registry::new()).unwrap());
    let reloader = Arc::new(
        TlsReloader::bootstrap(
            TlsPaths {
                cert_chain: cert_p,
                key: key_p,
                ca: ca_p,
            },
            false,
            vec![],
            "participant.efe.svc".into(),
            ProtocolVersion::Tls13,
            metrics,
        )
        .unwrap(),
    );

    let mut state = ControllerState::new_for_tests(cfg);
    assert!(state.tls_reloader.is_none());

    state.set_tls_reloader(reloader.clone());
    assert!(state.tls_reloader.is_some());

    // A clone of the state still sees the reloader (Arc-shared).
    let clone = state.clone();
    assert!(clone.tls_reloader.is_some());

    // We can pull a client config through the clone.
    let cfg = clone.tls_reloader.as_ref().unwrap().current_client_config();
    assert!(!cfg.cert_chain_pem.is_empty());
    assert!(!cfg.key_pem.is_empty());
    assert!(!cfg.server_ca_pem.is_empty());
}
