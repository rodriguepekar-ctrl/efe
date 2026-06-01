//! Participant-side configuration.

use std::net::SocketAddr;
use std::path::PathBuf;

use crate::error::ServerError;
use crate::tls::ProtocolVersion;

use super::{env_bool, env_or, env_path, env_socket_addr};

/// Strongly-typed participant configuration.
#[derive(Debug, Clone)]
pub struct ParticipantConfig {
    /// Participant id.  REQUIRED.  Must match the manifest's id.
    pub id: String,
    pub grpc_bind: SocketAddr,
    pub metrics_bind: SocketAddr,
    pub worker_threads: usize,
    pub tls_required: bool,
    pub tls_cert_path: Option<PathBuf>,
    pub tls_key_path: Option<PathBuf>,
    pub tls_ca_path: Option<PathBuf>,
    /// PR #16 — `true` means the participant requests + verifies a
    /// client cert from the controller (mutual TLS).
    pub tls_mutual: bool,
    /// PR #16 — minimum acceptable protocol version.
    pub tls_min_protocol: ProtocolVersion,
    /// PR #16 — peer SAN allowlist for mTLS verification.
    pub tls_allowed_peer_sans: Vec<String>,
    /// PR #16 — expected SAN of the controller cert (when we initiate
    /// outbound connections, e.g. for health checks).  Phase 2 leaves
    /// the participant purely passive ; this is reserved for Phase 3.
    pub tls_peer_san: Option<String>,
}

impl ParticipantConfig {
    pub fn from_env() -> Result<Self, ServerError> {
        let id = std::env::var("EFE_META_SWARM_PARTICIPANT_ID").map_err(|_| {
            ServerError::Config(
                "EFE_META_SWARM_PARTICIPANT_ID is REQUIRED for the participant binary".into(),
            )
        })?;
        if id.is_empty() {
            return Err(ServerError::Config(
                "EFE_META_SWARM_PARTICIPANT_ID must be non-empty".into(),
            ));
        }

        let grpc_bind = env_socket_addr("EFE_META_SWARM_PARTICIPANT_ADDR", "0.0.0.0:50090")?;
        let metrics_bind = env_socket_addr("EFE_META_SWARM_PARTICIPANT_METRICS", "0.0.0.0:9092")?;
        let worker_threads = env_or("EFE_META_SWARM_WORKER_THREADS", 1_usize)?;

        let tls_required = env_bool("EFE_META_SWARM_TLS_REQUIRED", false)?;
        let tls_cert_path = env_path("EFE_META_SWARM_TLS_CERT");
        let tls_key_path = env_path("EFE_META_SWARM_TLS_KEY");
        let tls_ca_path = env_path("EFE_META_SWARM_TLS_CA");

        if tls_required && (tls_cert_path.is_none() || tls_key_path.is_none()) {
            return Err(ServerError::Config(
                "EFE_META_SWARM_TLS_REQUIRED=true but cert/key unset".into(),
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
                 comma-separated list (PR #16 AD11)"
                    .into(),
            ));
        }

        Ok(Self {
            id,
            grpc_bind,
            metrics_bind,
            worker_threads,
            tls_required,
            tls_cert_path,
            tls_key_path,
            tls_ca_path,
            tls_mutual,
            tls_min_protocol,
            tls_allowed_peer_sans,
            tls_peer_san,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ENV_LOCK;

    fn clear_participant_env() {
        for v in [
            "EFE_META_SWARM_PARTICIPANT_ID",
            "EFE_META_SWARM_PARTICIPANT_ADDR",
            "EFE_META_SWARM_PARTICIPANT_METRICS",
            "EFE_META_SWARM_WORKER_THREADS",
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
    fn requires_id() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_participant_env();
        let err = ParticipantConfig::from_env().unwrap_err();
        assert!(matches!(err, ServerError::Config(_)));
    }

    #[test]
    fn rejects_empty_id() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_participant_env();
        std::env::set_var("EFE_META_SWARM_PARTICIPANT_ID", "");
        let err = ParticipantConfig::from_env().unwrap_err();
        assert!(matches!(err, ServerError::Config(_)));
        clear_participant_env();
    }

    /// Happy path : only `EFE_META_SWARM_PARTICIPANT_ID` set →
    /// defaults for everything else.
    #[test]
    fn from_env_minimal_happy_path() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_participant_env();
        std::env::set_var("EFE_META_SWARM_PARTICIPANT_ID", "supplier-scoring");
        let cfg = ParticipantConfig::from_env().expect("minimal config");
        assert_eq!(cfg.id, "supplier-scoring");
        assert_eq!(cfg.grpc_bind.to_string(), "0.0.0.0:50090");
        assert_eq!(cfg.metrics_bind.to_string(), "0.0.0.0:9092");
        assert_eq!(cfg.worker_threads, 1);
        assert!(!cfg.tls_required);
        clear_participant_env();
    }

    /// `TLS_REQUIRED=true` without cert/key → error.
    #[test]
    fn from_env_rejects_tls_required_without_cert() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_participant_env();
        std::env::set_var("EFE_META_SWARM_PARTICIPANT_ID", "p");
        std::env::set_var("EFE_META_SWARM_TLS_REQUIRED", "true");
        let err = ParticipantConfig::from_env().unwrap_err();
        assert!(
            matches!(err, ServerError::Config(ref m) if m.contains("cert/key")),
            "got : {err:?}",
        );
        clear_participant_env();
    }

    /// PR #16 — defaults : mutual off, TLS 1.3, empty allowlist.
    #[test]
    fn from_env_pr16_defaults() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_participant_env();
        std::env::set_var("EFE_META_SWARM_PARTICIPANT_ID", "p");
        let cfg = ParticipantConfig::from_env().unwrap();
        assert!(!cfg.tls_mutual);
        assert_eq!(cfg.tls_min_protocol, ProtocolVersion::Tls13);
        assert!(cfg.tls_allowed_peer_sans.is_empty());
        clear_participant_env();
    }

    /// PR #16 — `TLS_MUTUAL=true` without allowed peer SANs → error.
    #[test]
    fn from_env_pr16_mutual_requires_allowed_sans() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_participant_env();
        std::env::set_var("EFE_META_SWARM_PARTICIPANT_ID", "p");
        std::env::set_var("EFE_META_SWARM_TLS_REQUIRED", "true");
        std::env::set_var("EFE_META_SWARM_TLS_CERT", "/etc/efe/cert.pem");
        std::env::set_var("EFE_META_SWARM_TLS_KEY", "/etc/efe/key.pem");
        std::env::set_var("EFE_META_SWARM_TLS_MUTUAL", "true");
        let err = ParticipantConfig::from_env().unwrap_err();
        assert!(matches!(err, ServerError::Config(ref m) if m.contains("ALLOWED_PEER_SANS")));
        clear_participant_env();
    }

    /// PR #16 — invalid protocol version → Config error.
    #[test]
    fn from_env_pr16_rejects_invalid_protocol_version() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_participant_env();
        std::env::set_var("EFE_META_SWARM_PARTICIPANT_ID", "p");
        std::env::set_var("EFE_META_SWARM_TLS_MIN_PROTOCOL", "1.0");
        let err = ParticipantConfig::from_env().unwrap_err();
        assert!(matches!(err, ServerError::Config(_)));
        clear_participant_env();
    }
}
