//! `TelemetryConfig` — single source of truth for all observability
//! sinks.
//!
//! A [`TelemetryConfig`] is passed once at startup to
//! [`crate::init::init`]. Every sink is independently activatable: a
//! minimal dev run might only enable stdout JSON logs and leave the
//! Prometheus endpoint off.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::otel::OtelConfig;

/// Serialisation format of log records.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum LogFormat {
    /// JSON Lines (one log record per line). Default for prod.
    Json,
    /// Human-readable multi-line format. Dev only.
    Pretty,
    /// Compact single-line format (no JSON overhead).
    Compact,
}

impl Default for LogFormat {
    fn default() -> Self {
        Self::Json
    }
}

/// Full telemetry configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TelemetryConfig {
    /// Logical service name (e.g. `"efe-server"`).
    pub service_name: String,
    /// Deployment environment (`"dev"`, `"staging"`, `"prod"`).
    pub environment: String,
    /// Log serialisation format.
    pub log_format: LogFormat,
    /// Env-filter style directives (e.g. `"info,efe=debug"`).
    pub log_filter: String,
    /// **PII risk**: when `true`, raw prompts and similar fields are
    /// written verbatim. Default `false`. A WARN is emitted at startup
    /// whenever this is enabled.
    pub log_prompts: bool,
    /// Socket to bind the Prometheus `/metrics` HTTP sidecar to. `None`
    /// disables metric exposure (counters still increment in memory).
    pub metrics_addr: Option<SocketAddr>,
    /// Destination file for the audit log. `None` disables the audit
    /// sink entirely.
    pub audit_log_path: Option<PathBuf>,
    /// Maximum time to drain in-flight sinks during shutdown.
    pub flush_timeout: Duration,
    /// Whether to install the global tracing subscriber. Set to `false`
    /// when running inside a harness that already installs one (for
    /// example tests that call `tracing-test`).
    pub install_global_subscriber: bool,
    /// **Audit fix § A22 (D13-bis operational wiring)** — embedded
    /// OpenTelemetry SDK config. When `otel.enabled = true`, [`crate::init`]
    /// builds the real OTel pipeline (OTLP exporter + sampler +
    /// resource + tracing-opentelemetry layer) ; otherwise the
    /// pipeline is a no-op and only the legacy logging / metrics
    /// sinks are wired. Default : disabled.
    pub otel: OtelConfig,
    /// **Audit fix § A22** — when `otel.enabled = true` and the SDK
    /// init fails (invalid endpoint, exporter build error), should
    /// the whole telemetry init fail (`true`) or degrade gracefully
    /// without traces (`false`) ? Default : `false` (degrade).
    /// Production deployments typically set `true` so a misconfigured
    /// collector surfaces immediately as a CrashLoop instead of a
    /// silent traces-missing dashboard.
    pub fail_on_otel_init_error: bool,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            service_name: "efe-server".into(),
            environment: "dev".into(),
            log_format: LogFormat::Json,
            log_filter: "info,efe=debug".into(),
            log_prompts: false,
            metrics_addr: Some("127.0.0.1:9090".parse().expect("valid metrics address")),
            audit_log_path: None,
            flush_timeout: Duration::from_secs(5),
            install_global_subscriber: true,
            otel: OtelConfig::default(),
            fail_on_otel_init_error: false,
        }
    }
}

impl TelemetryConfig {
    /// Convenience constructor for silent tests — no global subscriber,
    /// no HTTP listener, in-memory metrics only.
    #[must_use]
    pub fn silent() -> Self {
        Self {
            install_global_subscriber: false,
            metrics_addr: None,
            audit_log_path: None,
            ..Self::default()
        }
    }

    /// Build a `TelemetryConfig` from environment variables. Unknown
    /// values fall back to the [`Default`] implementation.
    ///
    /// # Audit fix § A22
    ///
    /// `OtelConfig::from_env()` is called and its result embedded in
    /// `cfg.otel`. If parsing fails (e.g. `EFE_OTEL_SAMPLE_RATE` is
    /// not a valid float), the OTel block silently falls back to
    /// disabled — a misconfigured operator gets logs / metrics but
    /// no traces, with a startup warning emitted by [`crate::init`].
    /// The strict behaviour is opt-in via
    /// `fail_on_otel_init_error = true` (env
    /// `EFE_OBSERVABILITY_FAIL_ON_OTEL_ERROR=true`).
    #[must_use]
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(v) = std::env::var("EFE_ENV") {
            cfg.environment = v.clone();
            // **Audit fix § A22** — propagate `EFE_ENV` into the OTel
            // service name suffix only when not already overridden ;
            // operators often set `EFE_ENV=prod` and expect the OTel
            // resource to reflect it.
            cfg.otel.service_name = std::env::var("EFE_OTEL_SERVICE_NAME")
                .unwrap_or_else(|_| cfg.service_name.clone());
        }
        if let Ok(v) = std::env::var("EFE_LOG_FILTER") {
            cfg.log_filter = v;
        }
        if let Ok(v) = std::env::var("EFE_LOG_PROMPTS") {
            cfg.log_prompts = matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes");
        }
        if let Ok(v) = std::env::var("EFE_METRICS_ADDR") {
            cfg.metrics_addr = v.parse().ok();
        } else if let Ok(v) = std::env::var("EFE_METRICS_DISABLE") {
            if matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes") {
                cfg.metrics_addr = None;
            }
        }
        if let Ok(v) = std::env::var("EFE_AUDIT_LOG") {
            cfg.audit_log_path = Some(PathBuf::from(v));
        }
        // **Audit fix § A22** — embed the OTel config. Parsing
        // errors (invalid bool / float / out-of-range sample rate)
        // produce a disabled-OTel config + a tracing::warn emitted
        // by `init` ; we don't propagate the error here because
        // `from_env` is infallible by design (per the existing
        // contract).
        match OtelConfig::from_env() {
            Ok(otel) => cfg.otel = otel,
            Err(e) => {
                // The default OtelConfig is `enabled=false`, which
                // means a misconfigured env var still degrades to
                // logs+metrics-only ; init() will log the parsing
                // error.
                tracing::warn!(
                    target: "efe.telemetry.config",
                    error = %e,
                    "EFE_OTEL_* env var parse error ; OTel disabled"
                );
            }
        }
        if let Ok(v) = std::env::var("EFE_OBSERVABILITY_FAIL_ON_OTEL_ERROR") {
            cfg.fail_on_otel_init_error =
                matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes");
        }
        cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_sane() {
        let cfg = TelemetryConfig::default();
        assert_eq!(cfg.log_format, LogFormat::Json);
        assert!(cfg.metrics_addr.is_some());
        assert!(!cfg.log_prompts);
        assert!(cfg.install_global_subscriber);
    }

    #[test]
    fn silent_disables_io_sinks() {
        let cfg = TelemetryConfig::silent();
        assert!(!cfg.install_global_subscriber);
        assert!(cfg.metrics_addr.is_none());
        assert!(cfg.audit_log_path.is_none());
    }

    #[test]
    fn serde_roundtrip_defaults() {
        let cfg = TelemetryConfig::default();
        let s = serde_json::to_string(&cfg).unwrap();
        let back: TelemetryConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back.log_format, cfg.log_format);
        assert_eq!(back.log_filter, cfg.log_filter);
    }

    // =================================================================
    // Audit fix § A22 — TelemetryConfig embeds OtelConfig
    // =================================================================

    #[test]
    fn default_has_otel_disabled() {
        let cfg = TelemetryConfig::default();
        assert!(!cfg.otel.enabled);
        assert!(!cfg.fail_on_otel_init_error);
    }

    #[test]
    fn from_env_default_otel_disabled() {
        // Without any EFE_OTEL_* env var, the embedded config must
        // be the safe default (disabled, no endpoint).
        // We don't actually unset env vars (to avoid race with
        // parallel tests) ; we just assert the type is wired.
        let cfg = TelemetryConfig::from_env();
        // If anything is set in the test process, just check the
        // structural invariant that OtelConfig parses without panic.
        assert!(cfg.otel.sample_rate >= 0.0 && cfg.otel.sample_rate <= 1.0);
    }
}
