//! PR #16 — backward-compatibility test (AC1).
//!
//! When `EFE_META_SWARM_TLS_REQUIRED=false` (the default), the binary
//! must build without trying to load any cert / key / CA file.  The
//! `ControllerConfig` returned from `from_env` should report
//! `tls_required=false`, `tls_min_protocol=Tls13` (default), and an
//! empty SAN allowlist.

#![allow(clippy::missing_panics_doc)]

use efe_meta_swarm_server::config::controller::ControllerConfig;
use efe_meta_swarm_server::tls::ProtocolVersion;

/// Hold a lock so this test doesn't race with other env-var tests.
static ENV_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn clear_all_tls_env() {
    for v in [
        "EFE_META_SWARM_TLS_REQUIRED",
        "EFE_META_SWARM_TLS_CERT",
        "EFE_META_SWARM_TLS_KEY",
        "EFE_META_SWARM_TLS_CA",
        "EFE_META_SWARM_TLS_MUTUAL",
        "EFE_META_SWARM_TLS_MIN_PROTOCOL",
        "EFE_META_SWARM_TLS_ALLOWED_PEER_SANS",
        "EFE_META_SWARM_TLS_PEER_SAN",
    ] {
        std::env::remove_var(v);
    }
}

#[test]
fn pr16_default_config_tls_off() {
    let _g = ENV_GUARD.lock().unwrap();
    clear_all_tls_env();

    let cfg = ControllerConfig::from_env().expect("from_env");
    assert!(!cfg.tls_required, "tls_required must default to false");
    assert!(!cfg.tls_mutual, "tls_mutual must default to false");
    assert!(cfg.tls_allowed_peer_sans.is_empty());
    assert!(cfg.tls_peer_san.is_none());
    assert_eq!(cfg.tls_min_protocol, ProtocolVersion::Tls13);

    clear_all_tls_env();
}

#[test]
fn pr16_tls_off_ignores_certs_paths() {
    let _g = ENV_GUARD.lock().unwrap();
    clear_all_tls_env();

    // Cert paths set but tls_required=false — that's fine ; the
    // operator can keep the paths in their config and toggle TLS
    // separately.
    std::env::set_var("EFE_META_SWARM_TLS_CERT", "/etc/efe/cert.pem");
    std::env::set_var("EFE_META_SWARM_TLS_KEY", "/etc/efe/key.pem");

    let cfg = ControllerConfig::from_env().expect("from_env");
    assert!(!cfg.tls_required);
    // The paths are still parsed (might be useful for ops scripts that
    // inspect the running config), but no TLS is wired.
    assert_eq!(
        cfg.tls_cert_path.as_deref().unwrap().to_str(),
        Some("/etc/efe/cert.pem")
    );

    clear_all_tls_env();
}
