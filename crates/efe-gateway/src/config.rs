// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : Configuration — § 8.2.
//!
//! Loaded from env vars (Annex A2) with sensible defaults.

use serde::{Deserialize, Serialize};

/// **N22** : log output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// One JSON object per log line (production default).
    #[default]
    Json,
    /// Human-readable indented output (dev).
    Pretty,
}

/// **N22** : full gateway configuration.
///
/// Loadable from environment variables via [`GatewayConfig::from_env`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    /// Bind address, e.g. `"0.0.0.0:8080"`.
    pub bind: String,
    /// Optional TLS cert/key (PEM paths).  None = plain HTTP.
    pub tls: Option<TlsConfig>,
    /// Authentication mode (HS256 dev / RS256 prod TBD Sprint 14+).
    pub auth: AuthMode,
    /// Per-tenant rate limit (req/s).
    pub rate_limit_rps: u32,
    /// Body size cap in bytes (default 10 MB).
    pub body_limit_bytes: usize,
    /// Max concurrent connections.
    pub max_concurrent_conn: usize,
    /// Graceful shutdown budget (seconds).
    pub graceful_shutdown_secs: u64,
    /// tracing-subscriber filter directive (e.g. "info", "debug").
    pub log_level: String,
    /// Log output format.
    pub log_format: LogFormat,
    /// In-memory idempotency cache size (LRU).
    pub idempotency_cache_size: usize,
    /// Idempotency TTL (seconds).
    pub idempotency_ttl_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    pub cert_path: String,
    pub key_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthMode {
    /// HS256 with a shared symmetric secret (dev / test / CI).
    /// Secret is the **raw** secret string ; the validator hashes
    /// it appropriately.
    Hs256 { secret: String },
}

impl Default for AuthMode {
    fn default() -> Self {
        Self::Hs256 {
            secret: "dev-only-secret-replace-in-production".into(),
        }
    }
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0:8080".into(),
            tls: None,
            auth: AuthMode::default(),
            rate_limit_rps: super::DEFAULT_RATE_LIMIT_RPS,
            body_limit_bytes: super::DEFAULT_BODY_LIMIT_BYTES,
            max_concurrent_conn: 10_000,
            graceful_shutdown_secs: super::DEFAULT_GRACEFUL_SHUTDOWN_SECS,
            log_level: "info".into(),
            log_format: LogFormat::Json,
            idempotency_cache_size: 10_000,
            idempotency_ttl_secs: 86_400,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid env var {var} : {reason}")]
    InvalidEnvVar { var: String, reason: String },

    #[error("missing required env var : {var}")]
    MissingEnvVar { var: String },
}

impl GatewayConfig {
    /// **N22** : Build config from environment variables.
    /// Missing vars use defaults (see Annex A2).
    pub fn from_env() -> Result<Self, ConfigError> {
        let mut c = Self::default();
        if let Ok(b) = std::env::var("EFE_GATEWAY_BIND") {
            c.bind = b;
        }
        if let Ok(rps) = std::env::var("EFE_GATEWAY_RATE_LIMIT_RPS") {
            c.rate_limit_rps = rps.parse().map_err(|_| ConfigError::InvalidEnvVar {
                var: "EFE_GATEWAY_RATE_LIMIT_RPS".into(),
                reason: "not a u32".into(),
            })?;
        }
        if let Ok(b) = std::env::var("EFE_GATEWAY_BODY_LIMIT_BYTES") {
            c.body_limit_bytes = b.parse().map_err(|_| ConfigError::InvalidEnvVar {
                var: "EFE_GATEWAY_BODY_LIMIT_BYTES".into(),
                reason: "not a usize".into(),
            })?;
        }
        if let Ok(m) = std::env::var("EFE_GATEWAY_MAX_CONN") {
            c.max_concurrent_conn = m.parse().map_err(|_| ConfigError::InvalidEnvVar {
                var: "EFE_GATEWAY_MAX_CONN".into(),
                reason: "not a usize".into(),
            })?;
        }
        if let Ok(s) = std::env::var("EFE_GATEWAY_GRACEFUL_SHUTDOWN_SECS") {
            c.graceful_shutdown_secs = s.parse().map_err(|_| ConfigError::InvalidEnvVar {
                var: "EFE_GATEWAY_GRACEFUL_SHUTDOWN_SECS".into(),
                reason: "not a u64".into(),
            })?;
        }
        if let Ok(l) = std::env::var("EFE_GATEWAY_LOG_LEVEL") {
            c.log_level = l;
        }
        if let Ok(f) = std::env::var("EFE_GATEWAY_LOG_FORMAT") {
            c.log_format = match f.as_str() {
                "json" => LogFormat::Json,
                "pretty" => LogFormat::Pretty,
                other => {
                    return Err(ConfigError::InvalidEnvVar {
                        var: "EFE_GATEWAY_LOG_FORMAT".into(),
                        reason: format!("unknown format {other:?}"),
                    });
                }
            };
        }
        if let Ok(s) = std::env::var("EFE_GATEWAY_JWT_SECRET") {
            c.auth = AuthMode::Hs256 { secret: s };
        }
        if let (Ok(cert), Ok(key)) = (
            std::env::var("EFE_GATEWAY_TLS_CERT"),
            std::env::var("EFE_GATEWAY_TLS_KEY"),
        ) {
            c.tls = Some(TlsConfig {
                cert_path: cert,
                key_path: key,
            });
        }
        Ok(c)
    }

    /// **N22** : Validate cross-field invariants.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.body_limit_bytes < 1_024 || self.body_limit_bytes > 100 * 1024 * 1024 {
            return Err(ConfigError::InvalidEnvVar {
                var: "EFE_GATEWAY_BODY_LIMIT_BYTES".into(),
                reason: "must be in [1 KB, 100 MB]".into(),
            });
        }
        if self.rate_limit_rps == 0 {
            return Err(ConfigError::InvalidEnvVar {
                var: "EFE_GATEWAY_RATE_LIMIT_RPS".into(),
                reason: "must be > 0".into(),
            });
        }
        if self.bind.is_empty() {
            return Err(ConfigError::InvalidEnvVar {
                var: "EFE_GATEWAY_BIND".into(),
                reason: "empty".into(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_validates() {
        let c = GatewayConfig::default();
        assert!(c.validate().is_ok());
    }

    #[test]
    fn body_limit_below_floor_rejected() {
        let c = GatewayConfig {
            body_limit_bytes: 100,
            ..GatewayConfig::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn body_limit_above_ceiling_rejected() {
        let c = GatewayConfig {
            body_limit_bytes: 200 * 1024 * 1024,
            ..GatewayConfig::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn rate_limit_zero_rejected() {
        let c = GatewayConfig {
            rate_limit_rps: 0,
            ..GatewayConfig::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn empty_bind_rejected() {
        let c = GatewayConfig {
            bind: "".into(),
            ..GatewayConfig::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn log_format_default_is_json() {
        assert_eq!(LogFormat::default(), LogFormat::Json);
    }

    #[test]
    fn serde_round_trip() {
        let c = GatewayConfig::default();
        let json = serde_json::to_string(&c).unwrap();
        let back: GatewayConfig = serde_json::from_str(&json).unwrap();
        // Ignore comparing structurally ; just spot-check fields.
        assert_eq!(back.bind, c.bind);
        assert_eq!(back.body_limit_bytes, c.body_limit_bytes);
    }
}
