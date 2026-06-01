//! PR #20 § 12 — mTLS bootstrap matching PR #16's cert format.
//!
//! Uses tonic 0.12's built-in TLS API (`Identity` + `Certificate` +
//! `ClientTlsConfig`).  Requires the workspace `tonic` dep to enable
//! the `tls` feature — done in `tools/loadgen/Cargo.toml`.

use std::fs;
use std::path::Path;

use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity};

use crate::config::TlsConfig;
use crate::error::{LoadgenError, LoadgenResult};

/// Build a tonic [`ClientTlsConfig`] from the loadgen TLS material.
pub fn build_client_tls(tls: &TlsConfig) -> LoadgenResult<ClientTlsConfig> {
    if tls.insecure {
        // Native roots + no client cert ; suitable for `--tls-insecure`
        // dev mode (rejected in production by config validation).
        return Ok(ClientTlsConfig::new());
    }
    let cert = read_pem_file(&tls.cert_path)?;
    let key = read_pem_file(&tls.key_path)?;
    let ca = read_pem_file(&tls.ca_path)?;
    let identity = Identity::from_pem(&cert, &key);
    let ca_cert = Certificate::from_pem(&ca);
    let mut cfg = ClientTlsConfig::new()
        .ca_certificate(ca_cert)
        .identity(identity);
    if let Some(san) = &tls.expected_san {
        cfg = cfg.domain_name(san.clone());
    }
    Ok(cfg)
}

/// Drop-in helper : build a channel from a URL + optional TLS config.
pub async fn connect_channel(url: &str, tls: Option<&TlsConfig>) -> LoadgenResult<Channel> {
    let mut endpoint = tonic::transport::Endpoint::from_shared(url.to_string()).map_err(|e| {
        LoadgenError::InvalidConfig(format!("bad URL : {url} ({e})"))
    })?;
    if let Some(t) = tls {
        let cfg = build_client_tls(t)?;
        endpoint = endpoint
            .tls_config(cfg)
            .map_err(|e| LoadgenError::TlsBootstrap(e.to_string()))?;
    }
    endpoint.connect().await.map_err(|source| LoadgenError::Connect {
        target: url.to_string(),
        source,
    })
}

/// Read a PEM file into bytes.
fn read_pem_file(path: &Path) -> LoadgenResult<Vec<u8>> {
    fs::read(path).map_err(|source| LoadgenError::TlsFile {
        path: path.to_path_buf(),
        source,
    })
}

/// Strip the `grpc://` / `grpcs://` scheme to detect whether TLS is
/// required.  Informational only — loadgen never derives TLS state
/// from URL alone.
#[must_use]
pub fn url_implies_tls(url: &str) -> bool {
    url.starts_with("grpcs://") || url.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_implies_tls_grpcs_only() {
        assert!(url_implies_tls("grpcs://x:443"));
        assert!(url_implies_tls("https://x:443"));
        assert!(!url_implies_tls("grpc://x:50051"));
        assert!(!url_implies_tls("x:50051"));
    }
}
