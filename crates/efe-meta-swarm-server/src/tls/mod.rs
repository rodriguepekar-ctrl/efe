//! TLS / mTLS support for the EFE meta-swarm controller and participants.
//!
//! Implements the design frozen in
//! [`docs/EFE_A1_PHASE2_PR16_ARCHITECTURE.md`][doc] :
//!
//!   * Pure-Rust crypto stack — `rustls` + `tokio-rustls` + `tonic`'s
//!     optional `tls` feature.  No `OpenSSL` FFI in this crate.
//!   * `TLS 1.3`-only by default ; `TLS 1.2` is opt-in via
//!     `EFE_META_SWARM_TLS_MIN_PROTOCOL=1.2`.
//!   * Cipher suite list is the `rustls` `DEFAULT_TLS13_CIPHER_SUITES`
//!     (frozen at chart version — AD30).
//!   * Mutual TLS opt-in via `EFE_META_SWARM_TLS_MUTUAL=true`.  When
//!     enabled, both server and client verify peer cert SANs strictly
//!     against the configured allowlist.
//!   * Hot reload : an `inotify`-backed watcher (with a 5-second
//!     polling fallback on macOS / non-Linux) observes the cert mount
//!     and triggers an atomic [`arc_swap::ArcSwap`] swap of the
//!     active config when cert-manager rotates the underlying
//!     `Secret`.
//!   * Connection drain : in-flight RPCs continue on the old session
//!     keys ; new connections post-rotation pick up the new cert.
//!     30-second grace period (AD10).
//!   * Strict SAN verifier rejects IP-only and wildcard SANs (AD11 /
//!     § 10).
//!
//! ## Module layout
//!
//!   * [`config`]   — strongly-typed [`TlsServerConfig`] +
//!     [`TlsClientConfig`] structs ; rustls config builders.
//!   * [`reader`]   — PEM cert / key / CA bundle parsers.
//!   * [`watcher`]  — file watcher (notify + polling fallback).
//!   * [`reload`]   — [`TlsReloader`] : atomic swap coordinator.
//!   * [`metrics`]  — 4 Prometheus series (handshake duration /
//!     handshake total / cert expiry / reload total).
//!   * [`error`]    — [`TlsError`] enum.
//!
//! ## Lifecycle
//!
//! ```text
//!     startup
//!        ├─ read /etc/efe-meta-swarm/tls/{tls.crt, tls.key, ca.crt}
//!        ├─ build initial TlsServerConfig + TlsClientConfig
//!        ├─ register Prometheus metrics
//!        ├─ spawn cert-expiry sampling loop (60 s cadence)
//!        ├─ spawn inotify watcher (or polling fallback)
//!        └─ wire tonic Server::tls_config + Channel::tls_config
//!
//!     cert change (Day 23 + every ~ 23 days)
//!        ├─ watcher emits CertChangeEvent
//!        ├─ debounce 500 ms (Kubernetes Secrets fire multiple events)
//!        ├─ TlsReloader::handle_cert_change
//!        │     ├─ re-read cert + key + ca
//!        │     ├─ rebuild rustls config
//!        │     └─ ArcSwap atomic store (lock-free for readers)
//!        ├─ tls_reload_total++
//!        └─ OTel span tls.reload recorded
//! ```
//!
//! [doc]: ../../../docs/EFE_A1_PHASE2_PR16_ARCHITECTURE.md

pub mod audit;
pub mod config;
pub mod error;
pub mod metrics;
pub mod reader;
pub mod reload;
pub mod watcher;

pub use audit::{
    default_audit_emitter, AuditRole, MemoryAuditEmitter, NoopAuditEmitter, SharedAuditEmitter,
    TlsAuditEmitter, TlsAuditEvent,
};
pub use config::{
    build_rustls_client_config, build_rustls_server_config, ProtocolVersion, StrictSanVerifier,
    TlsClientConfig, TlsServerConfig,
};
pub use error::TlsError;
pub use metrics::{ReloadOutcome, TlsMetrics};
pub use reader::{load_ca_bundle, load_cert_chain, load_private_key, parse_cert_expiry};
pub use reload::{ReloadResult, TlsReloadEvent, TlsReloader};
pub use watcher::{CertChangeEvent, CertWatcher};

/// Environment-variable name that requests TLS at runtime.  When
/// `true`, the binaries refuse to start without a valid cert+key
/// pair (no plain-text fallback — AD25).
pub const ENV_TLS_REQUIRED: &str = "EFE_META_SWARM_TLS_REQUIRED";

/// Environment variable pointing at the leaf+intermediate cert chain
/// PEM file.  Provisioned by cert-manager at
/// `/etc/efe-meta-swarm/tls/tls.crt` by default.
pub const ENV_TLS_CERT: &str = "EFE_META_SWARM_TLS_CERT";

/// Environment variable pointing at the matching private key PEM
/// file.  Defaults to `/etc/efe-meta-swarm/tls/tls.key`.
pub const ENV_TLS_KEY: &str = "EFE_META_SWARM_TLS_KEY";

/// Environment variable pointing at the CA bundle PEM file used to
/// verify peer certs.  Defaults to `/etc/efe-meta-swarm/tls/ca.crt`.
pub const ENV_TLS_CA: &str = "EFE_META_SWARM_TLS_CA";

/// Environment variable enabling mutual TLS (client cert
/// verification).  Defaults to `false` (server-side TLS only).
pub const ENV_TLS_MUTUAL: &str = "EFE_META_SWARM_TLS_MUTUAL";

/// Environment variable selecting the minimum protocol version.
/// Accepts `"1.2"` or `"1.3"`.  Defaults to `"1.3"`.
pub const ENV_TLS_MIN_PROTOCOL: &str = "EFE_META_SWARM_TLS_MIN_PROTOCOL";

/// Environment variable carrying a comma-separated list of allowed
/// peer SANs.  Used by [`StrictSanVerifier`] when mutual TLS is on.
pub const ENV_TLS_ALLOWED_PEER_SANS: &str = "EFE_META_SWARM_TLS_ALLOWED_PEER_SANS";

/// Environment variable carrying the expected SAN of the **server**
/// the controller dials.  Required when `EFE_META_SWARM_TLS_REQUIRED=true`
/// on the client side (controller-as-client to participant-as-server).
pub const ENV_TLS_PEER_SAN: &str = "EFE_META_SWARM_TLS_PEER_SAN";

/// Default debounce window for the cert-file watcher.  Kubernetes
/// Secret mounts fire multiple inotify events on a single rotation
/// (RENAME + CREATE + WRITE within ~ 100 ms).  A 500 ms window
/// collapses them to a single reload (§ 18.2).
pub const WATCHER_DEBOUNCE_MS: u64 = 500;

/// Default cadence at which the cert-expiry sampling loop refreshes
/// the `efe_meta_swarm_tls_cert_expiry_seconds` gauge.
pub const CERT_EXPIRY_SAMPLE_INTERVAL_S: u64 = 60;

/// Default 30-second drain grace period quoted in AD10.
pub const DEFAULT_DRAIN_GRACE_S: u64 = 30;

/// AD18 / AC25 / T44 — return `true` if the endpoint URL targets a
/// loopback address (IPv4 `127.0.0.0/8` or IPv6 `::1`).
///
/// Connections to loopback bypass TLS regardless of `tls.enabled`.
/// The rationale (per architecture § 30 T44) : in-pod overhead is
/// non-trivial and the threat model for `127.0.0.1` is trivially
/// trusted — anyone able to dial loopback on a Pod already has the
/// same trust scope as the workload itself.  Production K8s does not
/// route via loopback ; this only fires in dev / single-process tests.
///
/// Accepts URI schemes `http://`, `https://`, `grpc://`, and bare
/// `host:port` forms.  Returns `false` on parse failure (fail-safe :
/// when in doubt, keep TLS on).
#[must_use]
pub fn endpoint_is_loopback(endpoint: &str) -> bool {
    use std::net::IpAddr;

    // Strip scheme if present.
    let after_scheme = endpoint
        .split_once("://")
        .map_or(endpoint, |(_scheme, rest)| rest);

    // Strip path/query if present (everything after the first `/` or `?`).
    let authority = after_scheme
        .split(['/', '?'])
        .next()
        .unwrap_or(after_scheme);

    // Split host from port.  Three cases :
    //   1. `[ipv6]:port` — bracket form, port follows the `]`.
    //   2. `ipv6` (bare, no port) — contains `::`, no brackets.
    //   3. `hostname:port` or `ipv4:port` — last colon separates host.
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        // IPv6 bracket form : [::1]:50080
        stripped.split_once(']').map(|(h, _)| h).unwrap_or(stripped)
    } else if authority.contains("::") {
        // Bare IPv6 (no port).  ":: " is the IPv6 abbreviation marker
        // ; if the input contains it AND is not bracketed, treat the
        // whole authority as the host.
        authority
    } else {
        // hostname:port or ipv4:port — split on the last colon.
        authority.rsplit_once(':').map_or(authority, |(h, _)| h)
    };

    // Literal hostname cases.
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }

    // Parse as IP if possible.
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.is_loopback(),
        Ok(IpAddr::V6(v6)) => v6.is_loopback(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod loopback_tests {
    use super::endpoint_is_loopback;

    #[test]
    fn loopback_ipv4_variants_detected() {
        assert!(endpoint_is_loopback("http://127.0.0.1:50080"));
        assert!(endpoint_is_loopback("https://127.0.0.1:50080"));
        assert!(endpoint_is_loopback("grpc://127.0.0.1"));
        assert!(endpoint_is_loopback("127.0.0.1:50080"));
        assert!(endpoint_is_loopback("http://127.1.2.3:50080")); // 127/8 is loopback
    }

    #[test]
    fn loopback_ipv6_variants_detected() {
        assert!(endpoint_is_loopback("http://[::1]:50080"));
        assert!(endpoint_is_loopback("https://[::1]:50080"));
        assert!(endpoint_is_loopback("[::1]:50080"));
        assert!(endpoint_is_loopback("::1"));
    }

    #[test]
    fn localhost_treated_as_loopback() {
        assert!(endpoint_is_loopback("http://localhost:50080"));
        assert!(endpoint_is_loopback("LOCALHOST:50080"));
    }

    #[test]
    fn non_loopback_addresses_are_not_loopback() {
        assert!(!endpoint_is_loopback("http://controller.efe.svc:50080"));
        assert!(!endpoint_is_loopback("http://10.0.0.1:50080"));
        assert!(!endpoint_is_loopback("http://192.168.1.1:50080"));
        assert!(!endpoint_is_loopback("http://[2001:db8::1]:50080"));
    }

    #[test]
    fn malformed_endpoints_are_not_loopback() {
        // Fail-safe : when in doubt, keep TLS on.
        assert!(!endpoint_is_loopback(""));
        assert!(!endpoint_is_loopback("not a url"));
    }

    #[test]
    fn endpoint_with_path_still_detected() {
        assert!(endpoint_is_loopback("http://127.0.0.1:50080/grpc"));
        assert!(endpoint_is_loopback("http://[::1]:50080/grpc"));
    }
}
