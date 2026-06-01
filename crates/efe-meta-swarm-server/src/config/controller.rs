//! Controller-side configuration.  Built once at startup from env
//! vars ; immutable thereafter.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use crate::error::ServerError;
use crate::tls::ProtocolVersion;

use super::{env_bool, env_duration_s, env_or, env_path, env_socket_addr};

/// Strongly-typed controller configuration.
#[derive(Debug, Clone)]
pub struct ControllerConfig {
    pub grpc_bind: SocketAddr,
    pub metrics_bind: SocketAddr,
    pub discovery: DiscoveryMode,
    pub static_participants: Vec<StaticParticipant>,
    pub manifest_path: PathBuf,
    pub max_inflight: usize,
    pub default_max_wall_ms: u32,
    pub worker_threads: usize,
    pub reflection_enabled: bool,
    pub tls_required: bool,
    pub tls_cert_path: Option<PathBuf>,
    pub tls_key_path: Option<PathBuf>,
    pub tls_ca_path: Option<PathBuf>,
    /// PR #16 — `true` requires mutual TLS (server requests and
    /// verifies a client cert) ; `false` is server-side TLS only.
    pub tls_mutual: bool,
    /// PR #16 — minimum acceptable protocol version.
    pub tls_min_protocol: ProtocolVersion,
    /// PR #16 — peer SAN allowlist for mTLS verification.
    pub tls_allowed_peer_sans: Vec<String>,
    /// PR #16 — expected SAN of the participant server cert.
    pub tls_peer_san: Option<String>,
    pub drain_timeout: Duration,
}

/// Discovery mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryMode {
    StaticLocalhost,
    K8sDns,
}

impl std::str::FromStr for DiscoveryMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "static-localhost" => Ok(Self::StaticLocalhost),
            "k8s-dns" => Ok(Self::K8sDns),
            other => Err(format!("invalid discovery mode: {other:?}")),
        }
    }
}

/// One static participant entry parsed from
/// `EFE_META_SWARM_PARTICIPANTS`.
#[derive(Debug, Clone)]
pub struct StaticParticipant {
    pub id: String,
    pub addr: String, // host:port, no scheme
}

impl ControllerConfig {
    /// Build from the environment.  Returns an error if any required
    /// var is missing or malformed.
    pub fn from_env() -> Result<Self, ServerError> {
        let grpc_bind = env_socket_addr("EFE_META_SWARM_CONTROLLER_ADDR", "0.0.0.0:50080")?;
        let metrics_bind = env_socket_addr("EFE_META_SWARM_CONTROLLER_METRICS", "0.0.0.0:9091")?;

        let discovery: DiscoveryMode =
            env_or("EFE_META_SWARM_DISCOVERY", DiscoveryMode::StaticLocalhost)?;

        let static_participants = match std::env::var("EFE_META_SWARM_PARTICIPANTS") {
            Ok(raw) => parse_participants(&raw)?,
            Err(_) => Vec::new(),
        };

        let manifest_path = env_path("EFE_META_SWARM_MANIFEST")
            .unwrap_or_else(|| PathBuf::from("/etc/efe-meta-swarm/manifest.yaml"));

        let max_inflight = env_or("EFE_META_SWARM_MAX_INFLIGHT", 10_usize)?;
        let default_max_wall_ms = env_or("EFE_META_SWARM_DEFAULT_MAX_WALL_MS", 15_000_u32)?;
        let worker_threads = env_or("EFE_META_SWARM_WORKER_THREADS", 2_usize)?;
        let reflection_enabled = env_bool("EFE_META_SWARM_REFLECTION", false)?;

        let tls_required = env_bool("EFE_META_SWARM_TLS_REQUIRED", false)?;
        let tls_cert_path = env_path("EFE_META_SWARM_TLS_CERT");
        let tls_key_path = env_path("EFE_META_SWARM_TLS_KEY");
        let tls_ca_path = env_path("EFE_META_SWARM_TLS_CA");

        if tls_required && (tls_cert_path.is_none() || tls_key_path.is_none()) {
            return Err(ServerError::Config(
                "EFE_META_SWARM_TLS_REQUIRED=true but \
                 EFE_META_SWARM_TLS_CERT or EFE_META_SWARM_TLS_KEY is unset"
                    .into(),
            ));
        }

        // PR #16 — mutual TLS knobs.
        let tls_mutual = env_bool("EFE_META_SWARM_TLS_MUTUAL", false)?;
        let tls_min_protocol_str =
            std::env::var("EFE_META_SWARM_TLS_MIN_PROTOCOL").unwrap_or_else(|_| "1.3".to_string());
        let tls_min_protocol: ProtocolVersion = tls_min_protocol_str
            .parse()
            .map_err(|e: crate::tls::TlsError| ServerError::Config(e.to_string()))?;
        let tls_allowed_peer_sans = std::env::var("EFE_META_SWARM_TLS_ALLOWED_PEER_SANS")
            .map(|raw| {
                raw.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let tls_peer_san = std::env::var("EFE_META_SWARM_TLS_PEER_SAN").ok();

        if tls_required && tls_mutual && tls_allowed_peer_sans.is_empty() {
            return Err(ServerError::Config(
                "EFE_META_SWARM_TLS_MUTUAL=true requires \
                 EFE_META_SWARM_TLS_ALLOWED_PEER_SANS to be a non-empty \
                 comma-separated list of accepted peer SANs (PR #16 AD11)"
                    .into(),
            ));
        }

        let drain_timeout = env_duration_s("EFE_META_SWARM_RELOAD_DRAIN_TIMEOUT_S", 30)?;

        Ok(Self {
            grpc_bind,
            metrics_bind,
            discovery,
            static_participants,
            manifest_path,
            max_inflight,
            default_max_wall_ms,
            worker_threads,
            reflection_enabled,
            tls_required,
            tls_cert_path,
            tls_key_path,
            tls_ca_path,
            tls_mutual,
            tls_min_protocol,
            tls_allowed_peer_sans,
            tls_peer_san,
            drain_timeout,
        })
    }
}

fn parse_participants(raw: &str) -> Result<Vec<StaticParticipant>, ServerError> {
    let mut out = Vec::new();
    for entry in raw.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let (id, addr) = entry.split_once('=').ok_or_else(|| {
            ServerError::Config(format!(
                "EFE_META_SWARM_PARTICIPANTS : malformed entry {entry:?} (expected `id=host:port`)",
            ))
        })?;
        if id.is_empty() || addr.is_empty() {
            return Err(ServerError::Config(format!(
                "EFE_META_SWARM_PARTICIPANTS : empty id or addr in {entry:?}",
            )));
        }
        out.push(StaticParticipant {
            id: id.trim().to_string(),
            addr: addr.trim().to_string(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Use the crate-wide ENV_LOCK so env-touching tests across
    // sibling modules don't race.
    use crate::config::ENV_LOCK;

    /// Helper : clear every controller env var so tests start from a
    /// known state.
    fn clear_controller_env() {
        for v in [
            "EFE_META_SWARM_CONTROLLER_ADDR",
            "EFE_META_SWARM_CONTROLLER_METRICS",
            "EFE_META_SWARM_DISCOVERY",
            "EFE_META_SWARM_PARTICIPANTS",
            "EFE_META_SWARM_MANIFEST",
            "EFE_META_SWARM_MAX_INFLIGHT",
            "EFE_META_SWARM_DEFAULT_MAX_WALL_MS",
            "EFE_META_SWARM_WORKER_THREADS",
            "EFE_META_SWARM_REFLECTION",
            "EFE_META_SWARM_TLS_REQUIRED",
            "EFE_META_SWARM_TLS_CERT",
            "EFE_META_SWARM_TLS_KEY",
            "EFE_META_SWARM_TLS_CA",
            "EFE_META_SWARM_TLS_MUTUAL",
            "EFE_META_SWARM_TLS_MIN_PROTOCOL",
            "EFE_META_SWARM_TLS_ALLOWED_PEER_SANS",
            "EFE_META_SWARM_TLS_PEER_SAN",
            "EFE_META_SWARM_RELOAD_DRAIN_TIMEOUT_S",
        ] {
            std::env::remove_var(v);
        }
    }

    #[test]
    fn parses_three_participants() {
        let raw = "supplier-scoring=localhost:50091,contract-risk=localhost:50092,spend-categorization=localhost:50093";
        let out = parse_participants(raw).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].id, "supplier-scoring");
        assert_eq!(out[0].addr, "localhost:50091");
        assert_eq!(out[2].id, "spend-categorization");
    }

    #[test]
    fn rejects_malformed_entry() {
        let raw = "supplier-scoring";
        let err = parse_participants(raw).unwrap_err();
        assert!(matches!(err, ServerError::Config(_)));
    }

    #[test]
    fn ignores_trailing_comma() {
        let raw = "supplier=localhost:1,";
        let out = parse_participants(raw).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn rejects_empty_id() {
        let raw = "=localhost:50091";
        let err = parse_participants(raw).unwrap_err();
        assert!(matches!(err, ServerError::Config(_)));
    }

    #[test]
    fn parses_discovery_mode() {
        let m: DiscoveryMode = "static-localhost".parse().unwrap();
        assert_eq!(m, DiscoveryMode::StaticLocalhost);
        let m: DiscoveryMode = "k8s-dns".parse().unwrap();
        assert_eq!(m, DiscoveryMode::K8sDns);
        let r: Result<DiscoveryMode, _> = "bogus".parse();
        assert!(r.is_err());
    }

    /// Happy path : every env var unset → reasonable defaults.
    #[test]
    fn from_env_defaults_when_unset() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_controller_env();

        let cfg = ControllerConfig::from_env().expect("defaults must work");
        assert_eq!(cfg.grpc_bind.to_string(), "0.0.0.0:50080");
        assert_eq!(cfg.metrics_bind.to_string(), "0.0.0.0:9091");
        assert_eq!(cfg.discovery, DiscoveryMode::StaticLocalhost);
        assert!(cfg.static_participants.is_empty());
        assert_eq!(cfg.max_inflight, 10);
        assert_eq!(cfg.default_max_wall_ms, 15_000);
        assert_eq!(cfg.worker_threads, 2);
        assert!(!cfg.reflection_enabled);
        assert!(!cfg.tls_required);
        assert_eq!(cfg.drain_timeout.as_secs(), 30);
    }

    /// Happy path : all env vars set → all values parsed.
    #[test]
    fn from_env_parses_full_config() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_controller_env();

        std::env::set_var("EFE_META_SWARM_CONTROLLER_ADDR", "127.0.0.1:9999");
        std::env::set_var("EFE_META_SWARM_CONTROLLER_METRICS", "127.0.0.1:8888");
        std::env::set_var("EFE_META_SWARM_DISCOVERY", "k8s-dns");
        std::env::set_var("EFE_META_SWARM_PARTICIPANTS", "a=h1:1,b=h2:2");
        std::env::set_var("EFE_META_SWARM_MAX_INFLIGHT", "20");
        std::env::set_var("EFE_META_SWARM_DEFAULT_MAX_WALL_MS", "60000");
        std::env::set_var("EFE_META_SWARM_WORKER_THREADS", "8");
        std::env::set_var("EFE_META_SWARM_REFLECTION", "true");
        std::env::set_var("EFE_META_SWARM_RELOAD_DRAIN_TIMEOUT_S", "60");

        let cfg = ControllerConfig::from_env().unwrap();
        assert_eq!(cfg.grpc_bind.to_string(), "127.0.0.1:9999");
        assert_eq!(cfg.metrics_bind.to_string(), "127.0.0.1:8888");
        assert_eq!(cfg.discovery, DiscoveryMode::K8sDns);
        assert_eq!(cfg.static_participants.len(), 2);
        assert_eq!(cfg.max_inflight, 20);
        assert_eq!(cfg.default_max_wall_ms, 60_000);
        assert_eq!(cfg.worker_threads, 8);
        assert!(cfg.reflection_enabled);
        assert_eq!(cfg.drain_timeout.as_secs(), 60);

        clear_controller_env();
    }

    /// `TLS_REQUIRED=true` but cert/key unset → error.
    #[test]
    fn from_env_rejects_tls_required_without_cert() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_controller_env();

        std::env::set_var("EFE_META_SWARM_TLS_REQUIRED", "true");
        // Neither EFE_META_SWARM_TLS_CERT nor EFE_META_SWARM_TLS_KEY is set.

        let err = ControllerConfig::from_env().unwrap_err();
        assert!(
            matches!(err, ServerError::Config(ref m) if m.contains("TLS_REQUIRED")),
            "expected Config error mentioning TLS_REQUIRED, got: {err:?}",
        );

        clear_controller_env();
    }

    /// `TLS_REQUIRED=true` + cert+key set → no error.
    #[test]
    fn from_env_accepts_tls_required_with_cert_and_key() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_controller_env();

        std::env::set_var("EFE_META_SWARM_TLS_REQUIRED", "true");
        std::env::set_var("EFE_META_SWARM_TLS_CERT", "/etc/efe/cert.pem");
        std::env::set_var("EFE_META_SWARM_TLS_KEY", "/etc/efe/key.pem");

        let cfg = ControllerConfig::from_env().unwrap();
        assert!(cfg.tls_required);
        assert_eq!(
            cfg.tls_cert_path.unwrap().to_str(),
            Some("/etc/efe/cert.pem")
        );
        assert_eq!(cfg.tls_key_path.unwrap().to_str(), Some("/etc/efe/key.pem"));

        clear_controller_env();
    }

    /// PR #16 — `tls_mutual` defaults to false ; `min_protocol` to
    /// TLS 1.3 ; empty allowlist accepted when `mutual` is off.
    #[test]
    fn from_env_pr16_defaults_tls13_and_mutual_off() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_controller_env();

        let cfg = ControllerConfig::from_env().unwrap();
        assert!(!cfg.tls_mutual);
        assert_eq!(cfg.tls_min_protocol, ProtocolVersion::Tls13);
        assert!(cfg.tls_allowed_peer_sans.is_empty());
        assert!(cfg.tls_peer_san.is_none());

        clear_controller_env();
    }

    /// PR #16 — `EFE_META_SWARM_TLS_MIN_PROTOCOL=1.2` parses to Tls12.
    #[test]
    fn from_env_pr16_min_protocol_tls12() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_controller_env();

        std::env::set_var("EFE_META_SWARM_TLS_MIN_PROTOCOL", "1.2");
        let cfg = ControllerConfig::from_env().unwrap();
        assert_eq!(cfg.tls_min_protocol, ProtocolVersion::Tls12);

        clear_controller_env();
    }

    /// PR #16 — invalid TLS protocol version → Config error.
    #[test]
    fn from_env_pr16_min_protocol_rejects_invalid() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_controller_env();

        std::env::set_var("EFE_META_SWARM_TLS_MIN_PROTOCOL", "1.0");
        let err = ControllerConfig::from_env().unwrap_err();
        assert!(matches!(err, ServerError::Config(_)));

        clear_controller_env();
    }

    /// PR #16 — `TLS_MUTUAL=true` + empty SAN list → error.
    #[test]
    fn from_env_pr16_mutual_requires_allowed_sans() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_controller_env();

        std::env::set_var("EFE_META_SWARM_TLS_REQUIRED", "true");
        std::env::set_var("EFE_META_SWARM_TLS_CERT", "/etc/efe/cert.pem");
        std::env::set_var("EFE_META_SWARM_TLS_KEY", "/etc/efe/key.pem");
        std::env::set_var("EFE_META_SWARM_TLS_MUTUAL", "true");
        // no EFE_META_SWARM_TLS_ALLOWED_PEER_SANS set

        let err = ControllerConfig::from_env().unwrap_err();
        assert!(
            matches!(err, ServerError::Config(ref m) if m.contains("ALLOWED_PEER_SANS")),
            "expected ALLOWED_PEER_SANS error, got {err:?}",
        );

        clear_controller_env();
    }

    /// PR #16 — comma-separated SAN list parses.
    #[test]
    fn from_env_pr16_parses_allowed_san_list() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_controller_env();

        std::env::set_var("EFE_META_SWARM_TLS_REQUIRED", "true");
        std::env::set_var("EFE_META_SWARM_TLS_CERT", "/etc/efe/cert.pem");
        std::env::set_var("EFE_META_SWARM_TLS_KEY", "/etc/efe/key.pem");
        std::env::set_var("EFE_META_SWARM_TLS_MUTUAL", "true");
        std::env::set_var(
            "EFE_META_SWARM_TLS_ALLOWED_PEER_SANS",
            "participant-1.efe.svc, participant-2.efe.svc , participant-3.efe.svc",
        );

        let cfg = ControllerConfig::from_env().unwrap();
        assert_eq!(cfg.tls_allowed_peer_sans.len(), 3);
        assert_eq!(cfg.tls_allowed_peer_sans[0], "participant-1.efe.svc");
        assert_eq!(cfg.tls_allowed_peer_sans[2], "participant-3.efe.svc");

        clear_controller_env();
    }

    /// Invalid bind addr → Config error.
    #[test]
    fn from_env_rejects_invalid_bind() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_controller_env();

        std::env::set_var("EFE_META_SWARM_CONTROLLER_ADDR", "not-a-socket-addr");
        let err = ControllerConfig::from_env().unwrap_err();
        assert!(matches!(err, ServerError::Config(_)));

        clear_controller_env();
    }
}
