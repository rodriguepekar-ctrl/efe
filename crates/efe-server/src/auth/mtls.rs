//! Sprint **D22** (M4 § 10.2) — mTLS configuration.
//!
//! ## Overview
//!
//! Production mTLS is mandatory in Tier-2/3 deployments
//! (`EFE_RUNTIME_MTLS_REQUIRED=true`). The server refuses to boot
//! without a client-CA file when mTLS is required ; the validation lives
//! in [`MtlsConfig::validate`] and the wired-up server config is
//! produced by [`MtlsConfig::build_server_tls_config`].
//!
//! Dev mode (`EFE_RUNTIME_MTLS_REQUIRED=false`, default) keeps the
//! existing TLS-optional posture : if a server cert is configured,
//! TLS is enabled but client cert verification is disabled. If neither
//! is configured, the server runs plain h2c.
//!
//! ## Cert chain (Helm-installed)
//!
//! ```text
//! Trust anchor : Helm cert-manager Issuer
//!    │
//!    └── Root CA (efe-ca)
//!        │
//!        ├── Server cert (efe-server)        ← used by tonic server TLS
//!        │
//!        └── Client certs (per consumer)      ← provisioned per tenant
//!            ├── tenant-acme-client.crt
//!            ├── tenant-bigco-client.crt
//!            └── ...
//! ```
//!
//! ## Audit fix S-A1
//!
//! The previous codebase had no first-class mTLS surface — TLS was
//! configurable via the `tonic::transport::Server::tls_config` API but
//! the operator had no obvious way to enforce client-cert verification
//! and the boot path silently fell back to "TLS-only" if the client-CA
//! file was absent. This module surfaces the requirement as a typed
//! config value (`required: bool`) and a typed builder error
//! ([`MtlsError::MissingClientCaButRequired`]).
//!
//! ## Implementation note
//!
//! The actual integration with `tonic::transport::Server` happens in
//! [`crate::runtime::Server::serve`] and uses
//! `tonic::transport::ServerTlsConfig`. We expose a typed wrapper
//! [`MtlsConfig`] here so that :
//!
//! 1. The validation logic is unit-testable without a live socket.
//! 2. The Helm template can render a `mtls.yaml` ConfigMap whose schema
//!    matches the Rust struct field-for-field.
//! 3. Operators who want to load certs from somewhere other than disk
//!    (Vault, AWS Secrets Manager, …) can construct an
//!    `MtlsConfig::from_pem_bytes` without going through the file path.

use std::path::{Path, PathBuf};

use thiserror::Error;

// `tonic::transport::ServerTlsConfig`, `Certificate` and `Identity` are
// gated behind tonic's `tls` Cargo feature. We re-import them only when
// our own `tls` feature is on (which forwards to `tonic/tls` via the
// crate's feature graph) — otherwise the validate-only logic still
// compiles, exercises the typed errors, and gates the boot path.
#[cfg(feature = "tls")]
use tonic::transport::{Certificate, Identity, ServerTlsConfig};

/// Typed errors for mTLS configuration / build.
#[derive(Debug, Error)]
pub enum MtlsError {
    /// `required = true` but no client-CA file was provided.
    /// **Audit fix S-A1** — this would previously have silently fallen
    /// back to "no client cert verification".
    #[error("mTLS required but no client_ca_path provided")]
    MissingClientCaButRequired,

    /// Could not read a PEM file from disk.
    #[error("I/O error reading {path}: {source}")]
    Io {
        /// Path that failed to read.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// `required = true` but no server cert was provided either.
    #[error("mTLS required but server_cert_path / server_key_path are unset")]
    MissingServerIdentity,
}

/// Typed mTLS configuration. Constructed once at server boot from
/// environment variables (or directly from PEM bytes by tests / non-
/// disk-backed deployments).
///
/// Cloning is cheap : the inner state holds owned bytes, but those
/// bytes are loaded once at boot and never mutated.
#[derive(Debug, Clone, Default)]
pub struct MtlsConfig {
    /// Path to the server cert (PEM-encoded chain). When `None`, the
    /// server runs without TLS.
    pub server_cert_path: Option<PathBuf>,

    /// Path to the server private key (PEM-encoded). When
    /// `server_cert_path` is `Some`, this MUST also be `Some`.
    pub server_key_path: Option<PathBuf>,

    /// Path to the client-CA bundle (PEM-encoded). When `None`, mTLS
    /// is OFF. When `required = true` and this is `None`,
    /// [`Self::validate`] returns an error.
    pub client_ca_path: Option<PathBuf>,

    /// Whether mTLS is mandatory. In production this is `true` ; the
    /// server refuses to boot if the client-CA is missing or unreadable.
    pub required: bool,
}

impl MtlsConfig {
    /// Read mTLS config from environment variables :
    ///
    /// | Variable                          | Maps to                |
    /// |-----------------------------------|------------------------|
    /// | `EFE_TLS_SERVER_CERT_PATH`        | `server_cert_path`     |
    /// | `EFE_TLS_SERVER_KEY_PATH`         | `server_key_path`      |
    /// | `EFE_TLS_CLIENT_CA_PATH`          | `client_ca_path`       |
    /// | `EFE_RUNTIME_MTLS_REQUIRED`       | `required` (true/false)|
    ///
    /// Empty / absent variables map to `None` (or `false` for
    /// `required`).
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            server_cert_path: env_path("EFE_TLS_SERVER_CERT_PATH"),
            server_key_path: env_path("EFE_TLS_SERVER_KEY_PATH"),
            client_ca_path: env_path("EFE_TLS_CLIENT_CA_PATH"),
            required: matches!(
                std::env::var("EFE_RUNTIME_MTLS_REQUIRED").ok().as_deref(),
                Some("1" | "true" | "TRUE" | "yes" | "YES")
            ),
        }
    }

    /// `true` when TLS is configured (server cert + key present).
    #[must_use]
    pub fn tls_enabled(&self) -> bool {
        self.server_cert_path.is_some() && self.server_key_path.is_some()
    }

    /// `true` when mTLS verification is wired (client-CA present).
    #[must_use]
    pub fn mtls_enabled(&self) -> bool {
        self.client_ca_path.is_some()
    }

    /// Static config validation : checks the invariants that don't
    /// require disk reads. Use [`Self::build_server_tls_config`] to
    /// also load the PEM files.
    ///
    /// # Errors
    ///
    /// * [`MtlsError::MissingServerIdentity`] when `required = true`
    ///   and the server cert / key are unset.
    /// * [`MtlsError::MissingClientCaButRequired`] when `required = true`
    ///   and `client_ca_path = None`.
    pub fn validate(&self) -> Result<(), MtlsError> {
        if self.required {
            if !self.tls_enabled() {
                return Err(MtlsError::MissingServerIdentity);
            }
            if !self.mtls_enabled() {
                return Err(MtlsError::MissingClientCaButRequired);
            }
        }
        Ok(())
    }

    /// Build the `tonic::transport::ServerTlsConfig` value to pass to
    /// `Server::tls_config(...)`. Returns `Ok(None)` when TLS is OFF
    /// (no server cert configured).
    ///
    /// # Feature gating
    ///
    /// Compiled only when the `tls` Cargo feature is on (forwards to
    /// `tonic/tls`). With the default feature set the validate-only
    /// surface is still available — operators can detect mTLS
    /// misconfigurations at boot without pulling rustls deps into
    /// the dev build.
    ///
    /// # Errors
    ///
    /// * [`MtlsError::Io`] when a PEM file cannot be read.
    /// * [`MtlsError::MissingClientCaButRequired`] (forwarded from
    ///   [`Self::validate`]).
    #[cfg(feature = "tls")]
    pub fn build_server_tls_config(&self) -> Result<Option<ServerTlsConfig>, MtlsError> {
        self.validate()?;
        let (Some(cert), Some(key)) = (&self.server_cert_path, &self.server_key_path) else {
            return Ok(None);
        };
        let cert_pem = read_pem(cert)?;
        let key_pem = read_pem(key)?;
        let identity = Identity::from_pem(cert_pem, key_pem);

        let mut tls = ServerTlsConfig::new().identity(identity);
        if let Some(ca) = &self.client_ca_path {
            let ca_pem = read_pem(ca)?;
            tls = tls.client_ca_root(Certificate::from_pem(ca_pem));
        }
        Ok(Some(tls))
    }
}

fn env_path(key: &str) -> Option<PathBuf> {
    let v = std::env::var(key).ok()?;
    if v.is_empty() {
        None
    } else {
        Some(PathBuf::from(v))
    }
}

/// Read a PEM file from disk. Used internally by the
/// `build_server_tls_config` path which is gated behind the `tls`
/// feature ; we mirror the gating here to avoid a dead-code warning
/// in the no-`tls` build.
#[cfg(feature = "tls")]
fn read_pem(path: &Path) -> Result<Vec<u8>, MtlsError> {
    std::fs::read(path).map_err(|e| MtlsError::Io {
        path: path.to_path_buf(),
        source: e,
    })
}

// `Path` is imported for `env_path` even without TLS — keep the
// `read_pem` helper gated so it doesn't trip dead-code warnings on
// the default build.
#[cfg(not(feature = "tls"))]
#[allow(dead_code)]
fn _read_pem_unused_marker(_p: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_tls_off() {
        let c = MtlsConfig::default();
        assert!(!c.tls_enabled());
        assert!(!c.mtls_enabled());
        // Default is dev-friendly : not required, no certs, validate OK.
        c.validate().unwrap();
    }

    #[test]
    fn required_without_ca_fails() {
        let c = MtlsConfig {
            server_cert_path: Some(PathBuf::from("/nonexistent/server.pem")),
            server_key_path: Some(PathBuf::from("/nonexistent/server.key")),
            client_ca_path: None,
            required: true,
        };
        let err = c.validate().unwrap_err();
        assert!(matches!(err, MtlsError::MissingClientCaButRequired));
    }

    #[test]
    fn required_without_server_identity_fails() {
        let c = MtlsConfig {
            server_cert_path: None,
            server_key_path: None,
            client_ca_path: Some(PathBuf::from("/nonexistent/ca.pem")),
            required: true,
        };
        let err = c.validate().unwrap_err();
        assert!(matches!(err, MtlsError::MissingServerIdentity));
    }

    #[test]
    fn dev_mode_with_tls_only_validates_ok() {
        let c = MtlsConfig {
            server_cert_path: Some(PathBuf::from("/srv/server.pem")),
            server_key_path: Some(PathBuf::from("/srv/server.key")),
            client_ca_path: None,
            required: false,
        };
        c.validate().unwrap();
        assert!(c.tls_enabled());
        assert!(!c.mtls_enabled());
    }

    #[cfg(feature = "tls")]
    #[test]
    fn build_returns_none_when_no_certs() {
        let c = MtlsConfig::default();
        assert!(c.build_server_tls_config().unwrap().is_none());
    }

    #[cfg(feature = "tls")]
    #[test]
    fn build_io_error_for_missing_file() {
        let c = MtlsConfig {
            server_cert_path: Some(PathBuf::from("/definitely/missing.pem")),
            server_key_path: Some(PathBuf::from("/definitely/missing.key")),
            client_ca_path: None,
            required: false,
        };
        let err = c.build_server_tls_config().unwrap_err();
        assert!(matches!(err, MtlsError::Io { .. }));
    }
}
