//! Startup-sequence tests for M12.1.
//!
//! These tests exercise the configuration side of the bootstrap
//! (the parts that are meaningful in isolation, i.e. `ServerConfig`
//! parsing and validation). The full-process bootstrap (tokio runtime,
//! telemetry init, listener bind) is exercised by the grpc integration
//! suite in `integration_grpc.rs`, so we do not duplicate that here.
//!
//! M12 §25 acceptance criteria covered:
//!   - CA-M12-01 (boot readiness) — indirectly via the validate step.
//!   - CA-M12-04 (no ambiguous config) — every tested env var either
//!     parses or returns a descriptive error.

use std::env::VarError;
use std::time::Duration;

use efe_server::{ConfigError, ServerConfig};

fn env_of(
    entries: &'static [(&'static str, &'static str)],
) -> impl Fn(&'static str) -> Result<String, VarError> {
    move |var| {
        entries
            .iter()
            .find(|(k, _)| *k == var)
            .map(|(_, v)| v.to_string())
            .ok_or(VarError::NotPresent)
    }
}

// =====================================================================
// Bootstrap happy path
// =====================================================================

#[test]
fn bootstrap_with_empty_env_uses_defaults_and_validates() {
    let cfg = ServerConfig::from_env_with(env_of(&[])).expect("parse ok");
    cfg.validate().expect("default config must validate");
    assert_eq!(cfg.listen_addr.port(), 50051);
    assert!(cfg.enable_admin);
}

#[test]
fn bootstrap_with_full_env_overrides_every_field() {
    let cfg = ServerConfig::from_env_with(env_of(&[
        ("EFE_LISTEN_ADDR", "0.0.0.0:60051"),
        ("EFE_REQUEST_TIMEOUT_MS", "7500"),
        ("EFE_MAX_CONCURRENT_STREAMS", "2048"),
        ("EFE_ENABLE_ADMIN", "false"),
        ("EFE_RATELIMIT_ENABLED", "true"),
        ("EFE_RATELIMIT_CAPACITY", "750"),
        ("EFE_RATELIMIT_REFILL", "250.5"),
        ("EFE_IDEMPOTENCY_ENABLED", "true"),
        ("EFE_IDEMPOTENCY_CAPACITY", "20000"),
        ("EFE_IDEMPOTENCY_TTL_MS", "120000"),
        ("EFE_SHUTDOWN_DRAIN_MS", "15000"),
        ("EFE_SHUTDOWN_FORCE_ABORT_MS", "20000"),
    ]))
    .expect("parse ok");
    cfg.validate().expect("must validate");

    assert_eq!(cfg.listen_addr.port(), 60051);
    assert_eq!(cfg.request_timeout, Duration::from_millis(7500));
    assert_eq!(cfg.max_concurrent_streams, 2048);
    assert!(!cfg.enable_admin);
    assert!(cfg.ratelimit.enabled);
    assert_eq!(cfg.ratelimit.default_capacity, 750);
    assert!((cfg.ratelimit.default_refill_rate - 250.5).abs() < 1e-6);
    assert_eq!(cfg.idempotency.cache_capacity, 20_000);
    assert_eq!(cfg.idempotency.ttl, Duration::from_secs(120));
    assert_eq!(cfg.shutdown.grace_period, Duration::from_secs(15));
    assert_eq!(cfg.shutdown.force_abort_after, Duration::from_secs(20));
}

// =====================================================================
// Fail-fast on malformed input
// =====================================================================

#[test]
fn bootstrap_fails_fast_on_malformed_listen_addr() {
    let err = ServerConfig::from_env_with(env_of(&[("EFE_LISTEN_ADDR", "not-an-addr")]))
        .expect_err("must fail");
    match err {
        ConfigError::Parse { var, .. } => assert_eq!(var, "EFE_LISTEN_ADDR"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn bootstrap_fails_fast_on_negative_timeout() {
    // `u64::from_str` rejects negatives — good.
    let err = ServerConfig::from_env_with(env_of(&[("EFE_REQUEST_TIMEOUT_MS", "-1")]))
        .expect_err("must fail");
    assert!(matches!(
        err,
        ConfigError::Parse {
            var: "EFE_REQUEST_TIMEOUT_MS",
            ..
        }
    ));
}

#[test]
fn bootstrap_fails_validate_when_auth_on_but_no_secret_file_set() {
    // Auth enabled but no file → the default path is missing, which
    // produces a SecretsIo error during parse, not validate.
    let err = ServerConfig::from_env_with(env_of(&[
        ("EFE_AUTH_ENABLED", "true"),
        ("EFE_JWT_SECRETS_FILE", "/definitely/does/not/exist/hs256"),
    ]))
    .expect_err("must fail");
    assert!(matches!(err, ConfigError::SecretsIo { .. }));
}

#[test]
fn bootstrap_validate_rejects_port_zero() {
    let mut cfg = ServerConfig::default();
    cfg.listen_addr = "127.0.0.1:0".parse().unwrap();
    let err = cfg.validate().expect_err("port 0 must be rejected");
    assert!(matches!(
        err,
        ConfigError::Invalid {
            field: "listen_addr",
            ..
        }
    ));
}

// =====================================================================
// Invariants B4 / B5 — pure helpers, no side effects
// =====================================================================

#[test]
fn from_env_is_pure_with_empty_closure() {
    // Invariant B5: parsing is a pure function of the input closure.
    // Calling twice with the same empty closure must produce equal
    // outputs (modulo field-by-field equality on structured types).
    let a = ServerConfig::from_env_with(env_of(&[])).unwrap();
    let b = ServerConfig::from_env_with(env_of(&[])).unwrap();
    assert_eq!(a.listen_addr, b.listen_addr);
    assert_eq!(a.max_concurrent_streams, b.max_concurrent_streams);
    assert_eq!(a.request_timeout, b.request_timeout);
    assert_eq!(a.shutdown.grace_period, b.shutdown.grace_period);
}

#[test]
fn validate_is_idempotent() {
    // Invariant B4 (spirit): validate is a pure check — calling it
    // twice on the same config returns the same verdict both times.
    let cfg = ServerConfig::default();
    assert!(cfg.validate().is_ok());
    assert!(cfg.validate().is_ok());
    assert!(cfg.validate().is_ok());
}
